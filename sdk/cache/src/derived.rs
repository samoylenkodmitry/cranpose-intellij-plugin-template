//! Best-effort seeds for generated files, keyed by the inputs that produced them.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const HEADER: &[u8] = b"cranpose-derived-v1\0";

/// An atomic cache for a generated file that callers copy into their own workspace.
/// Missing or damaged records are misses. Consumers still validate the generated
/// file (for example, Cargo checks a cached lockfile against current manifests).
/// This never edits inputs or returns a path that an active consumer should modify.
pub struct DerivedFile {
    path: PathBuf,
}
impl DerivedFile {
    pub fn new(root: &Path, inputs: &[(&str, &[u8])]) -> Result<Self> {
        let mut entries = BTreeMap::new();
        for &(name, value) in inputs {
            ensure!(
                entries.insert(name, value).is_none(),
                "Duplicate cache input: {name}"
            );
        }
        let mut digest = Sha256::new();
        digest.update(HEADER);
        for (name, value) in entries {
            digest.update((name.len() as u64).to_le_bytes());
            digest.update(name.as_bytes());
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value);
        }
        Ok(Self {
            path: root.join(format!("{:x}.seed", digest.finalize())),
        })
    }
    pub fn load(&self) -> Result<Option<Vec<u8>>> {
        let bytes = match sharing_retry(|| fs::read(&self.path)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let Some(bytes) = bytes.strip_prefix(HEADER).filter(|bytes| bytes.len() >= 32) else {
            return Ok(None);
        };
        let (checksum, payload) = bytes.split_at(32);
        Ok((checksum == &Sha256::digest(payload)[..]).then(|| payload.to_owned()))
    }
    pub fn store(&self, bytes: &[u8]) -> Result<()> {
        let parent = self.path.parent().expect("cache record parent");
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(HEADER)?;
        file.write_all(&Sha256::digest(bytes))?;
        file.write_all(bytes)?;
        file.flush()?;
        // Close the temporary handle before exposing it to other publishers.
        let temporary = file.into_temp_path();
        sharing_retry(|| fs::rename(&temporary, &self.path))?;
        Ok(())
    }
}

// Windows can briefly deny replacement while another reader or publisher closes
// its handle. Bound that wait; a persistent denial remains a best-effort cache error.
fn sharing_retry<T>(mut operation: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    loop {
        match operation() {
            Err(error)
                if cfg!(windows)
                    && matches!(error.raw_os_error(), Some(5 | 32 | 33))
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_changes_invalidate_seeds_but_input_order_does_not() {
        let root = tempfile::tempdir().expect("cache fixture operation");
        let first = DerivedFile::new(root.path(), &[("manifest", b"one"), ("lock", b"original")])
            .expect("cache fixture operation");
        assert_eq!(first.load().expect("cache fixture operation"), None);
        first.store(b"generated").expect("cache fixture operation");
        let same = DerivedFile::new(root.path(), &[("lock", b"original"), ("manifest", b"one")])
            .expect("cache fixture operation");
        assert_eq!(
            same.load().expect("cache fixture operation"),
            Some(b"generated".to_vec())
        );
        for inputs in [
            [
                ("manifest", b"two".as_slice()),
                ("lock", b"original".as_slice()),
            ],
            [
                ("manifest", b"one".as_slice()),
                ("lock", b"updated".as_slice()),
            ],
        ] {
            assert_eq!(
                DerivedFile::new(root.path(), &inputs)
                    .expect("cache fixture operation")
                    .load()
                    .expect("cache fixture operation"),
                None
            );
        }
        assert!(DerivedFile::new(root.path(), &[("same", b"a"), ("same", b"b")]).is_err());
    }
    #[test]
    fn truncated_or_corrupted_records_are_misses_and_can_be_replaced() {
        let root = tempfile::tempdir().expect("cache fixture operation");
        let cache =
            DerivedFile::new(root.path(), &[("input", b"data")]).expect("cache fixture operation");
        cache.store(b"complete").expect("cache fixture operation");
        let mut bytes = fs::read(&cache.path).expect("cache fixture operation");
        *bytes.last_mut().expect("stored record has a payload") ^= 1;
        fs::write(&cache.path, bytes).expect("cache fixture operation");
        assert_eq!(cache.load().expect("cache fixture operation"), None);
        fs::write(&cache.path, HEADER).expect("cache fixture operation");
        assert_eq!(cache.load().expect("cache fixture operation"), None);
        cache.store(b"repaired").expect("cache fixture operation");
        assert_eq!(
            cache.load().expect("cache fixture operation"),
            Some(b"repaired".to_vec())
        );
    }
    #[cfg(windows)]
    #[test]
    fn sharing_conflicts_retry_but_persistent_denial_preserves_the_old_record() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().expect("cache directory");
        let cache = DerivedFile::new(root.path(), &[("input", b"same")]).expect("cache");
        cache.store(b"before").expect("initial record");
        let hold = || {
            fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&cache.path)
                .expect("exclusive handle")
        };
        let file = hold();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            drop(file);
        });
        cache.store(b"after").expect("retry after handle closed");
        release.join().expect("release handle");
        let file = hold();
        let started = std::time::Instant::now();
        assert!(cache.store(b"blocked").is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        drop(file);
        assert_eq!(
            cache.load().expect("intact record"),
            Some(b"after".to_vec())
        );
        assert_eq!(
            fs::read_dir(root.path()).expect("cache directory").count(),
            1
        );
    }
    #[test]
    fn concurrent_publishers_never_expose_partial_payloads() {
        let root = tempfile::tempdir().expect("cache fixture operation");
        std::thread::scope(|scope| {
            for value in 1..=8 {
                let root = root.path();
                scope.spawn(move || {
                    let cache = DerivedFile::new(root, &[("input", b"same")])
                        .expect("cache fixture operation");
                    for _ in 0..8 {
                        cache
                            .store(&vec![value; 4096])
                            .expect("cache fixture operation");
                        let bytes = cache
                            .load()
                            .expect("cache fixture operation")
                            .expect("complete record");
                        assert_eq!(bytes.len(), 4096);
                        assert!(bytes.iter().all(|byte| *byte == bytes[0]));
                    }
                });
            }
        });
    }
}
