use anyhow::{Context, Result};
use std::{fs, io::Read, path::Path};

/// Stage and validate an executable before publishing it without replacement.
/// The caller must authenticate downloaded contents before calling this function.
/// Validation may execute the candidate: all writable handles are closed first.
/// Concurrent publishers validate and reuse the winner; failed candidates are removed.
pub fn publish_executable(
    destination: &Path,
    mut contents: impl Read,
    validate: impl Fn(&Path) -> Result<()>,
) -> Result<()> {
    if destination.try_exists()? {
        return validate(destination);
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut staging = tempfile::Builder::new()
        .prefix(".executable-")
        .suffix(std::env::consts::EXE_SUFFIX)
        .tempfile_in(parent)?;
    std::io::copy(&mut contents, staging.as_file_mut())?;
    staging.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staging
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    // Linux rejects exec while any writable descriptor for this inode is open.
    // Keep path ownership for cleanup, but close the NamedTempFile's descriptor.
    let staging = staging.into_temp_path();
    validate(&staging).context("validate staged executable")?;
    if let Err(error) = staging.persist_noclobber(destination) {
        if destination.try_exists()? {
            return validate(destination).context("validate concurrent executable");
        }
        return Err(error.error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{bail, ensure};
    use std::{process::Command, sync::Barrier};

    fn execute(path: &Path) -> Result<()> {
        let result = Command::new(path).arg("--list").output()?;
        ensure!(result.status.success(), "test executable failed");
        Ok(())
    }

    #[test]
    fn staged_and_published_rust_binaries_can_execute_immediately() -> Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("tool.exe");
        let executable = fs::read(std::env::current_exe()?)?;
        publish_executable(&destination, executable.as_slice(), execute)?;
        execute(&destination)?;
        assert_eq!(fs::read(&destination)?, executable);
        // Reuse does not copy over an already validated executable.
        publish_executable(&destination, b"invalid replacement".as_slice(), execute)?;
        assert_eq!(fs::read(&destination)?, executable);
        assert_eq!(fs::read_dir(root.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn rejected_candidates_are_removed_and_existing_files_are_preserved() -> Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("tool.exe");
        let reject = |_: &Path| bail!("invalid tool");
        assert!(publish_executable(&destination, b"invalid".as_slice(), reject).is_err());
        assert_eq!(fs::read_dir(root.path())?.count(), 0);
        fs::write(&destination, b"existing")?;
        assert!(publish_executable(&destination, b"replacement".as_slice(), reject).is_err());
        assert_eq!(fs::read(&destination)?, b"existing");
        Ok(())
    }

    #[test]
    fn concurrent_publishers_execute_candidates_and_reuse_one_winner() -> Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("tool.exe");
        let executable = fs::read(std::env::current_exe()?)?;
        let barrier = Barrier::new(2);
        std::thread::scope(|scope| {
            let publishers: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        publish_executable(&destination, executable.as_slice(), |candidate| {
                            if candidate != destination {
                                barrier.wait();
                            }
                            execute(candidate)
                        })
                    })
                })
                .collect();
            for publisher in publishers {
                publisher.join().expect("publisher")?;
            }
            Ok::<_, anyhow::Error>(())
        })?;
        execute(&destination)?;
        assert_eq!(fs::read_dir(root.path())?.count(), 1);
        Ok(())
    }
}
