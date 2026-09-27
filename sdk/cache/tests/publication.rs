//! Keep subprocess launches separate from unit tests requiring immediate lock reuse:
//! a concurrent fork can briefly retain a sibling test's lock before exec closes it.
use anyhow::{Result, bail, ensure};
use cranpose_plugin_cache::publish_executable;
use std::{fs, path::Path, process::Command, sync::Barrier};

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
