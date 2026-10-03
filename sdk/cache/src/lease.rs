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
        let slot = slot.canonicalize()?;
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
    /// Build output owned exclusively by this lease, outside its copied sources.
    /// The caller creates this directory when needed. Its contents survive clean
    /// lease reuse, while concurrent and abandoned leases retain separate paths.
    /// Only access it while holding the lease; stop all users before `complete`.
    pub fn artifacts_path(&self) -> PathBuf {
        self.slot.join("artifacts")
    }
    /// Give a self-contained tool a stable executable path belonging to this lease.
    /// Cargo includes its workspace-wrapper path in artifact hashes: separate aliases
    /// let overlapping workspaces retain independent incremental outputs while sharing
    /// dependency builds. A symlink would canonicalize back to the shared source.
    ///
    /// Call before starting users of this lease. The source must remain immutable while
    /// users run; a hard link avoids copying large tools, with a copy fallback across
    /// filesystems. Re-stage on each acquisition to pick up a replaced source executable.
    /// The source's executable permissions and filename (including .exe) are preserved.
    pub fn stage_executable(&self, source: &Path) -> Result<PathBuf> {
        let source = source.canonicalize().context("resolve tool executable")?;
        ensure!(source.is_file(), "Tool executable is not a regular file");
        ensure!(
            !source.starts_with(&self.slot),
            "Tool source belongs to this lease"
        );
        let identity = Sha256::digest(source.as_os_str().as_encoded_bytes());
        let directory = self.slot.join("tools").join(format!("{identity:x}"));
        fs::create_dir_all(&directory)?;
        let alias = directory.join(source.file_name().context("tool filename")?);
        match fs::symlink_metadata(&alias) {
            Ok(metadata) => {
                ensure!(
                    metadata.file_type().is_file(),
                    "Tool alias is not a regular file"
                );
                fs::remove_file(&alias)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if fs::hard_link(&source, &alias).is_err() {
            copy_executable(&source, &alias)?;
        }
        Ok(alias)
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

fn copy_executable(source: &Path, destination: &Path) -> Result<()> {
    let mut output = File::create_new(destination)?;
    let mut input = File::open(source)?;
    std::io::copy(&mut input, &mut output)?;
    output.set_permissions(input.metadata()?.permissions())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn executable_aliases_follow_lease_ownership_and_refresh() -> Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("compiler.exe");
        fs::write(&source, "first tool")?;
        let cache = root.path().join("cache");
        let first = WorkspaceLease::acquire(&cache, b"project")?;
        let first_alias = first.stage_executable(&source)?;
        let second = WorkspaceLease::acquire(&cache, b"project")?;
        let second_alias = second.stage_executable(&source)?;
        assert_ne!(first_alias, second_alias);
        assert_ne!(first_alias.canonicalize()?, source.canonicalize()?);
        assert_eq!(first_alias.file_name(), source.file_name());
        first.complete()?;
        // Model an atomic tool update after the first owner's processes have exited.
        fs::remove_file(&source)?;
        fs::write(&source, "second tool")?;
        let reused = WorkspaceLease::acquire(&cache, b"project")?;
        assert_eq!(reused.stage_executable(&source)?, first_alias);
        assert_eq!(fs::read_to_string(&first_alias)?, "second tool");
        assert_eq!(fs::read_to_string(&second_alias)?, "first tool");
        Ok(())
    }
    #[test]
    fn copied_executables_preserve_contents_and_permissions_without_overwriting() -> Result<()> {
        let root = tempfile::tempdir()?;
        let source = std::env::current_exe()?;
        let destination = root.path().join("copy.exe");
        copy_executable(&source, &destination)?;
        assert_eq!(fs::read(&source)?, fs::read(&destination)?);
        assert_eq!(
            fs::metadata(&source)?.permissions(),
            fs::metadata(&destination)?.permissions()
        );
        assert!(copy_executable(&source, &destination).is_err());
        Ok(())
    }
    #[test]
    fn alias_rejects_non_files_and_lease_owned_sources() -> Result<()> {
        let root = tempfile::tempdir()?;
        let lease = WorkspaceLease::acquire(root.path(), b"project")?;
        assert!(lease.stage_executable(root.path()).is_err());
        let source = lease.path().join("tool");
        fs::write(&source, "tool")?;
        assert!(lease.stage_executable(&source).is_err());
        Ok(())
    }
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
