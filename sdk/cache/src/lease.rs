//! Private working directories with explicit, exclusive reuse after clean shutdown.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, TryLockError},
    path::{Path, PathBuf},
};

const READY: &[u8] = b"cranpose-workspace-v1\nready\n";
const BUSY: &[u8] = b"cranpose-workspace-v1\nbusy\n";

/// An exclusively owned directory whose path can survive successive compiler runs.
///
/// `acquire` clears previous contents only after obtaining the OS lock and checking
/// an explicit clean-shutdown marker. Dropping a lease without `complete` abandons
/// it: a crashed owner's descendants may still be using the directory. Such slots
/// are never reused automatically. Callers may clear their cache once all users stop.
#[derive(Debug)]
pub struct WorkspaceLease {
    slot: PathBuf,
    directory: PathBuf,
    _lock: File,
    reused: bool,
}
impl WorkspaceLease {
    /// Identity should include the canonical project path and a format/version tag.
    /// Busy slots are skipped without waiting; concurrent callers get distinct paths.
    pub fn acquire(root: &Path, identity: &[u8]) -> Result<Self> {
        let namespace = root.join(format!("{:x}", Sha256::digest(identity)));
        fs::create_dir_all(&namespace)?;
        for entry in fs::read_dir(&namespace)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir()
                || !entry.file_name().to_string_lossy().starts_with("slot-")
            {
                continue;
            }
            let slot = entry.path();
            let Ok(lock) = File::options()
                .read(true)
                .write(true)
                .open(slot.join("lease"))
            else {
                continue;
            };
            match lock.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
            if fs::read(slot.join("state")).ok().as_deref() != Some(READY) {
                continue;
            }
            return Self::claim(slot, lock, true);
        }
        let slot = tempfile::Builder::new()
            .prefix("slot-")
            .tempdir_in(&namespace)?
            .keep();
        let lock = File::create_new(slot.join("lease"))?;
        lock.try_lock().context("lock new workspace slot")?;
        Self::claim(slot, lock, false)
    }
    fn claim(slot: PathBuf, lock: File, reused: bool) -> Result<Self> {
        // Claim before clearing; interruption at any point leaves an unusable slot.
        fs::write(slot.join("state"), BUSY)?;
        let directory = slot.join("workspace");
        match fs::symlink_metadata(&directory) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "Workspace cache is not an owned directory"
                );
                fs::remove_dir_all(&directory)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::create_dir(&directory)?;
        Ok(Self {
            slot,
            directory: directory.canonicalize()?,
            _lock: lock,
            reused,
        })
    }
    pub fn path(&self) -> &Path {
        &self.directory
    }
    pub fn reused(&self) -> bool {
        self.reused
    }
    /// Release for reuse only after every process using this directory has exited.
    /// This consumes the lease; ordinary Drop intentionally never marks it ready.
    pub fn complete(self) -> Result<()> {
        fs::write(self.slot.join("state"), READY)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_and_abandoned_slots_are_not_reused() -> Result<()> {
        let root = tempfile::tempdir()?;
        let first = WorkspaceLease::acquire(root.path(), b"project")?;
        let path = first.path().to_owned();
        fs::write(path.join("old.rs"), "old input")?;
        let second = WorkspaceLease::acquire(root.path(), b"project")?;
        assert_ne!(second.path(), path);
        drop(second); // Models a failure: its slot must never be reused.
        first.complete()?;
        let reused = WorkspaceLease::acquire(root.path(), b"project")?;
        assert!(reused.reused());
        assert_eq!(reused.path(), path);
        assert!(!path.join("old.rs").exists());
        let third = WorkspaceLease::acquire(root.path(), b"project")?;
        assert!(!third.reused());
        assert_ne!(third.path(), path);
        Ok(())
    }
    #[test]
    fn project_identities_and_foreign_directories_stay_separate() -> Result<()> {
        let root = tempfile::tempdir()?;
        let first = WorkspaceLease::acquire(root.path(), b"first")?;
        let path = first.path().to_owned();
        first.complete()?;
        let foreign = path
            .parent()
            .context("slot")?
            .parent()
            .context("namespace")?
            .join("slot-foreign");
        fs::create_dir(&foreign)?;
        fs::write(foreign.join("keep"), "foreign")?;
        let second = WorkspaceLease::acquire(root.path(), b"second")?;
        assert_ne!(second.path(), path);
        assert_eq!(fs::read_to_string(foreign.join("keep"))?, "foreign");
        Ok(())
    }
}
