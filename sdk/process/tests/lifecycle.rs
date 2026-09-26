mod support;
use cranpose_plugin_process::{Process, capture, is_running};
use std::{
    process::Stdio,
    time::{Duration, Instant},
};

#[test]
fn graceful_stop_covers_compiler_and_application() {
    let directory = tempfile::tempdir().expect("fixture");
    let mut process =
        Process::spawn(support::command(directory.path(), "root", "graceful")).expect("root");
    support::ready(directory.path());
    let started = Instant::now();
    process.terminate(Duration::from_secs(2)).expect("shutdown");
    assert!(started.elapsed() < Duration::from_secs(1));
    support::exited(directory.path());
}
#[test]
fn stubborn_descendants_are_forced_after_bounded_grace() {
    let directory = tempfile::tempdir().expect("fixture");
    let mut process =
        Process::spawn(support::command(directory.path(), "root", "stubborn")).expect("root");
    support::ready(directory.path());
    let started = Instant::now();
    process
        .terminate(Duration::from_millis(100))
        .expect("force-stop");
    assert!(started.elapsed() < Duration::from_secs(1));
    support::exited(directory.path());
}
#[test]
fn abruptly_killed_runner_still_has_owned_descendants() {
    let directory = tempfile::tempdir().expect("fixture");
    let mut process =
        Process::spawn(support::command(directory.path(), "root", "stubborn")).expect("root");
    support::ready(directory.path());
    process.kill().expect("abrupt runner exit");
    process.terminate(Duration::ZERO).expect("cleanup");
    support::exited(directory.path());
}
#[test]
fn dropping_one_preview_preserves_another() {
    let other_dir = tempfile::tempdir().expect("other");
    let other =
        Process::spawn(support::command(other_dir.path(), "root", "stubborn")).expect("other root");
    support::ready(other_dir.path());
    for _ in 0..3 {
        let directory = tempfile::tempdir().expect("fixture");
        let process =
            Process::spawn(support::command(directory.path(), "root", "stubborn")).expect("root");
        support::ready(directory.path());
        drop(process);
        support::exited(directory.path());
        assert!(support::pids(other_dir.path()).into_iter().all(is_running));
    }
    drop(other);
    support::exited(other_dir.path());
}
#[test]
fn standalone_worker_owns_its_compiler_tree() {
    let directory = tempfile::tempdir().expect("fixture");
    let process = Process::spawn_worker(support::command(directory.path(), "compiler", "stubborn"))
        .expect("standalone compiler");
    support::ready(directory.path());
    drop(process);
    support::exited(directory.path());
}
#[test]
fn capture_timeout_and_cancellation_close_descendant_pipes() {
    for cancel in [false, true] {
        let directory = tempfile::tempdir().expect("fixture");
        let command = support::command(directory.path(), "root", "stubborn");
        let started = Instant::now();
        let result = capture(command, None, Duration::from_millis(500), || {
            cancel && directory.path().join("ready").exists()
        });
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        support::exited(directory.path());
    }
}
#[test]
fn completed_parent_closes_inherited_pipes_and_descendants() {
    let directory = tempfile::tempdir().expect("fixture");
    let mut command = support::command(directory.path(), "root", "parent-exit");
    command.stdout(Stdio::piped());
    let started = Instant::now();
    let output = capture(command, None, Duration::from_secs(5), || false).expect("capture");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("done"));
    assert!(started.elapsed() < Duration::from_secs(3));
    support::exited(directory.path());
}

/// Reproduce the 0.6.1 host's two-second wait + abrupt runner-group kill, then
/// measure the owned lifecycle. This intentionally creates isolated orphan
/// fixtures and cleans only their known compiler group before each assertion.
#[cfg(unix)]
#[test]
#[ignore = "manual before/after benchmark, including isolated legacy orphan reproduction"]
fn shutdown_latency_benchmark() {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::Pid,
    };
    struct LegacyCompiler(u32);
    impl Drop for LegacyCompiler {
        fn drop(&mut self) {
            let _ = killpg(Pid::from_raw(self.0 as i32), Signal::SIGKILL);
        }
    }
    for mode in ["legacy", "graceful", "stubborn"] {
        for sample in 0..5 {
            let directory = tempfile::tempdir().expect("fixture");
            let mut process =
                Process::spawn(support::command(directory.path(), "root", mode)).expect("root");
            support::ready(directory.path());
            let pids = support::pids(directory.path());
            let legacy_cleanup = (mode == "legacy").then(|| LegacyCompiler(pids[1]));
            let started = Instant::now();
            if mode == "legacy" {
                // Match Session::close before this fix: wait without a signal.
                std::thread::sleep(Duration::from_secs(2));
                process
                    .terminate(Duration::ZERO)
                    .expect("legacy force-stop");
            } else {
                process
                    .terminate(Duration::from_millis(200))
                    .expect("owned stop");
            }
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            let survivors = pids.iter().filter(|pid| is_running(**pid)).count();
            drop(legacy_cleanup);
            support::exited(directory.path());
            println!(
                "{{\"mode\":\"{mode}\",\"sample\":{sample},\"shutdownMs\":{elapsed},\"survivorsBeforeFixtureCleanup\":{survivors}}}"
            );
            assert_eq!(survivors, if mode == "legacy" { 2 } else { 0 });
        }
    }
}
