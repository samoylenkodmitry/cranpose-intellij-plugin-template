//! Staged, cancellable generation from a pinned Git template.
use anyhow::{Context, Result, ensure};
use std::{fs, io::Write, path::Path, process::Command, time::Duration};
pub const SHOWCASE_REPOSITORY: &str = "https://github.com/samoylenkodmitry/cranpose-showcase";
pub const SHOWCASE_REVISION: &str = "dc439faf9fd019191ccbf1c6fa70f2c523ae1163";

pub fn validate_destination(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    ensure!(root.is_dir(), "Project location is not a directory");
    for item in fs::read_dir(root)? {
        let item = item?;
        let name = item.file_name();
        let name = name.to_string_lossy();
        ensure!(
            name == ".idea" || name == ".git" || name.ends_with(".iml"),
            "Project location contains {name}; choose an empty directory"
        );
    }
    Ok(())
}

/// Fetch before installing any project files. Never runs template scripts.
pub fn generate(
    root: &Path,
    cache: &Path,
    repository: &str,
    revision: &str,
    cancel: impl Fn() -> bool,
) -> Result<()> {
    validate_destination(root)?;
    fs::create_dir_all(cache)?;
    let stage = tempfile::Builder::new()
        .prefix("cranpose-starter-")
        .tempdir_in(cache)?;
    for args in [
        vec!["init", "--quiet"],
        vec!["fetch", "--quiet", "--depth", "1", repository, revision],
        vec![
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "--detach",
            "FETCH_HEAD",
        ],
    ] {
        ensure!(!cancel(), "Project generation cancelled");
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(stage.path())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_LFS_SKIP_SMUDGE", "1");
        let output =
            cranpose_plugin_process::capture(command, None, Duration::from_secs(90), &cancel)
                .context("Fetch Cranpose starter with Git")?;
        ensure!(
            output.status.success(),
            "Git could not prepare starter: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    ensure!(
        stage.path().join("Cargo.toml").is_file() && stage.path().join("src").is_dir(),
        "Starter has no Cargo application"
    );
    fs::remove_dir_all(stage.path().join(".git"))?;
    install(stage.path(), root, &cancel)?;
    Ok(())
}
fn install(source: &Path, destination: &Path, cancel: &impl Fn() -> bool) -> Result<()> {
    validate_destination(destination)?;
    fs::create_dir_all(destination)?;
    let mut created = Vec::new();
    fn copy(
        source: &Path,
        destination: &Path,
        created: &mut Vec<std::path::PathBuf>,
        cancel: &impl Fn() -> bool,
    ) -> Result<()> {
        for entry in fs::read_dir(source)? {
            ensure!(!cancel(), "Project generation cancelled");
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "Starter contains a symlink: {}",
                entry.path().display()
            );
            let target = destination.join(entry.file_name());
            if kind.is_dir() {
                fs::create_dir(&target)?;
                created.push(target.clone());
                copy(&entry.path(), &target, created, cancel)?;
            } else if kind.is_file() {
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                created.push(target.clone());
                std::io::copy(&mut fs::File::open(entry.path())?, &mut output)?;
                output.flush()?;
                fs::set_permissions(&target, entry.metadata()?.permissions())?;
            } else {
                anyhow::bail!("Unsupported starter file");
            }
        }
        Ok(())
    }
    if let Err(error) = copy(source, destination, &mut created, cancel) {
        for path in created.into_iter().rev() {
            if path.is_dir() {
                let _ = fs::remove_dir(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
        return Err(error);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_existing_code_and_preserves_ide_metadata() {
        let temp = tempfile::tempdir().expect("temp");
        let src = temp.path().join("source");
        let dst = temp.path().join("destination");
        fs::create_dir_all(&src).expect("source");
        fs::write(src.join("Cargo.toml"), "starter").expect("file");
        fs::create_dir_all(dst.join(".idea")).expect("idea");
        install(&src, &dst, &|| false).expect("install");
        assert_eq!(
            fs::read_to_string(dst.join("Cargo.toml")).expect("manifest"),
            "starter"
        );
        assert!(install(&src, &dst, &|| false).is_err());
        assert!(dst.join(".idea").is_dir());
    }
    #[test]
    fn cancellation_does_not_leave_partial_files() {
        let temp = tempfile::tempdir().expect("temp");
        let src = temp.path().join("source");
        let dst = temp.path().join("destination");
        fs::create_dir(&src).expect("source");
        for n in 0..3 {
            fs::write(src.join(n.to_string()), "fixture").expect("file");
        }
        let count = std::cell::Cell::new(0);
        assert!(
            install(&src, &dst, &|| {
                count.set(count.get() + 1);
                count.get() > 1
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(dst).expect("destination").count(), 0);
    }
}
