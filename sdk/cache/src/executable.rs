use anyhow::{Context, Result};
use std::{
    fs,
    io::{ErrorKind, Read},
    path::Path,
    time::{Duration, Instant},
};

const BUSY_RETRY_BUDGET: Duration = Duration::from_millis(500);

/// Stage and validate an executable before publishing it without replacement.
/// The caller must authenticate downloaded contents before calling this function.
/// Validation may execute the candidate: all writable handles are closed first.
/// Concurrent publishers validate and reuse the winner; failed candidates are removed.
/// ExecutableFileBusy is retried for up to 500 ms: an unrelated concurrent fork
/// can briefly inherit a writable descriptor until exec closes it. Other errors
/// return immediately. The validator itself must provide any execution timeout.
pub fn publish_executable(
    destination: &Path,
    mut contents: impl Read,
    validate: impl Fn(&Path) -> Result<()>,
) -> Result<()> {
    if destination.try_exists()? {
        return validate_ready(destination, &validate);
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
    validate_ready(&staging, &validate).context("validate staged executable")?;
    if let Err(error) = staging.persist_noclobber(destination) {
        if destination.try_exists()? {
            return validate_ready(destination, &validate)
                .context("validate concurrent executable");
        }
        return Err(error.error.into());
    }
    Ok(())
}

fn validate_ready(path: &Path, validate: &impl Fn(&Path) -> Result<()>) -> Result<()> {
    let started = Instant::now();
    loop {
        match validate(path) {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == ErrorKind::ExecutableFileBusy) =>
            {
                let Some(remaining) = BUSY_RETRY_BUDGET.checked_sub(started.elapsed()) else {
                    return Err(error);
                };
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            result => return result,
        }
    }
}
