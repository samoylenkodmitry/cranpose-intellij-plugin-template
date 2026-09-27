//! Real OS-lock and interrupted-owner checks on all CI platforms.
use cranpose_plugin_cache::WorkspaceLease;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(root: &std::path::Path) -> anyhow::Result<(Worker, PathBuf)> {
    let mut worker = Worker(
        Command::new(std::env::current_exe()?)
            .args(["--exact", "lease_worker", "--ignored", "--nocapture"])
            .env("CRANPOSE_LEASE_TEST_ROOT", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let path = loop {
        if let Ok(bytes) = fs::read_to_string(root.join("ready")) {
            break PathBuf::from(bytes);
        }
        anyhow::ensure!(
            worker.0.try_wait()?.is_none(),
            "worker exited before acquiring lease"
        );
        anyhow::ensure!(Instant::now() < deadline, "worker did not acquire lease");
        thread::sleep(Duration::from_millis(10));
    };
    Ok((worker, path))
}
#[test]
fn another_process_cannot_reset_a_live_or_interrupted_workspace() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let (mut worker, path) = start(root.path())?;
    let other = WorkspaceLease::acquire(&root.path().join("cache"), b"project")?;
    assert_ne!(other.path(), path);
    assert_eq!(fs::read_to_string(path.join("active"))?, "still in use");
    worker.0.kill()?;
    worker.0.wait()?;
    // The OS lock was released by process exit. The busy marker must still prevent reuse.
    let next = WorkspaceLease::acquire(&root.path().join("cache"), b"project")?;
    assert_ne!(next.path(), path);
    assert_eq!(fs::read_to_string(path.join("active"))?, "still in use");
    Ok(())
}
#[test]
fn clean_owner_exit_allows_same_path_with_fresh_contents() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let (mut worker, path) = start(root.path())?;
    worker
        .0
        .stdin
        .take()
        .expect("worker stdin")
        .write_all(b"complete\n")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = worker.0.try_wait()? {
            anyhow::ensure!(status.success(), "worker failed");
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "worker did not finish");
        thread::sleep(Duration::from_millis(10));
    }
    let next = WorkspaceLease::acquire(&root.path().join("cache"), b"project")?;
    assert!(next.reused());
    assert_eq!(next.path(), path);
    assert!(!path.join("active").exists());
    Ok(())
}
#[test]
#[ignore = "subprocess fixture"]
fn lease_worker() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::var_os("CRANPOSE_LEASE_TEST_ROOT").expect("worker root"));
    let lease = WorkspaceLease::acquire(&root.join("cache"), b"project")?;
    fs::write(lease.path().join("active"), "still in use")?;
    fs::write(
        root.join("ready.tmp"),
        lease.path().to_string_lossy().as_bytes(),
    )?;
    fs::rename(root.join("ready.tmp"), root.join("ready"))?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    anyhow::ensure!(line.trim() == "complete", "worker cancelled");
    lease.complete()
}
