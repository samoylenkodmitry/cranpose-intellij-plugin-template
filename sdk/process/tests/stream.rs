use anyhow::Result;
use cranpose_plugin_process::{self as process, Cancellation, ExecutionEvent};
use std::{
    fs,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn fixture(mode: &str, directory: &std::path::Path) -> Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--ignored", "--exact", "process_fixture", "--nocapture"])
        .env("CRANPOSE_TEST_MODE", mode)
        .env("CRANPOSE_TEST_DIRECTORY", directory);
    Ok(command)
}
#[test]
#[ignore = "subprocess helper"]
fn process_fixture() -> Result<()> {
    let mode = std::env::var("CRANPOSE_TEST_MODE")?;
    let root = std::path::PathBuf::from(
        std::env::var_os("CRANPOSE_TEST_DIRECTORY").expect("fixture directory"),
    );
    match mode.as_str() {
        "fail" => panic!("expected process failure"),
        "output" => {
            println!("{}", "x".repeat(100_000));
            eprintln!("stderr sentinel");
        }
        "tree" => {
            let mut child = fixture("leaf", &root)?.spawn()?;
            fs::write(root.join("child.pid"), child.id().to_string())?;
            child.wait()?;
        }
        "leaf" => loop {
            fs::write(root.join("heartbeat"), format!("{:?}", Instant::now()))?;
            thread::sleep(Duration::from_millis(10));
        },
        _ => panic!("unknown fixture"),
    }
    Ok(())
}

#[test]
fn drains_output_in_bounded_chunks_and_reports_failure() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut count = 0;
    let mut stderr = false;
    process::execute(
        fixture("output", root.path())?,
        Duration::from_secs(10),
        &Cancellation::default(),
        |text| {
            assert!(text.len() <= 8192);
            count += text.len();
            stderr |= text.contains("stderr sentinel");
        },
    )?;
    assert!(count >= 100_000 && stderr);
    assert!(
        process::execute(
            fixture("fail", root.path())?,
            Duration::from_secs(10),
            &Cancellation::default(),
            |_| {}
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn cancellation_stops_descendants_and_is_bounded() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cancel = Cancellation::default();
    let signal = cancel.clone();
    let marker = root.path().join("heartbeat");
    let worker_marker = marker.clone();
    let canceller = thread::spawn(move || {
        let start = Instant::now();
        while !worker_marker.exists() && start.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
        signal.cancel();
    });
    let start = Instant::now();
    let result = process::execute(
        fixture("tree", root.path())?,
        Duration::from_secs(10),
        &cancel,
        |_| {},
    );
    canceller.join().expect("canceller");
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_secs(7));
    assert!(root.path().join("child.pid").exists());
    let before = fs::read(&marker)?;
    thread::sleep(Duration::from_millis(150));
    assert_eq!(before, fs::read(marker)?);
    let pid = fs::read_to_string(root.path().join("child.pid"))?.parse()?;
    assert!(!cranpose_plugin_process::is_running(pid));
    Ok(())
}

#[test]
fn timeout_and_pre_cancel_never_report_success() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        process::execute(
            fixture("tree", root.path())?,
            Duration::from_secs(1),
            &cancel,
            |_| {}
        )
        .is_err()
    );
    assert!(!root.path().join("child.pid").exists());
    assert!(
        process::execute(
            fixture("tree", root.path())?,
            Duration::from_millis(150),
            &Cancellation::default(),
            |_| {}
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn observes_one_start_before_output_and_no_start_on_spawn_failure() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut starts = Vec::new();
    let mut output = false;
    process::execute_observed(
        fixture("output", root.path())?,
        Duration::from_secs(10),
        &Cancellation::default(),
        |event| match event {
            ExecutionEvent::Started { pid } => {
                assert!(!output);
                starts.push(pid);
            }
            ExecutionEvent::Output { .. } => {
                assert_eq!(starts.len(), 1);
                output = true;
            }
        },
    )?;
    assert!(output);
    assert_eq!(starts.len(), 1);
    assert!(!process::is_running(starts[0]));

    for cancelled in [false, true] {
        let cancel = Cancellation::default();
        if cancelled {
            cancel.cancel();
        }
        let mut events = 0;
        let command = if cancelled {
            fixture("tree", root.path())?
        } else {
            Command::new(root.path().join("missing executable"))
        };
        assert!(
            process::execute_observed(command, Duration::from_secs(5), &cancel, |_| {
                events += 1;
            })
            .is_err()
        );
        assert_eq!(events, 0);
    }
    Ok(())
}

#[test]
fn start_observer_can_cancel_without_waiting_for_application_output() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cancel = Cancellation::default();
    let mut started = None;
    let start = Instant::now();
    let result = process::execute_observed(
        fixture("leaf", root.path())?,
        Duration::from_secs(10),
        &cancel,
        |event| {
            if let ExecutionEvent::Started { pid } = event {
                started = Some(pid);
                cancel.cancel();
            }
        },
    );
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(!process::is_running(started.expect("start notification")));
    Ok(())
}
