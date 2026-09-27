use cranpose_plugin_process::{Process, is_running};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub fn command(directory: &Path, role: &str, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args(["--exact", "support::fixture", "--ignored", "--nocapture"])
        .env("PROCESS_TEST_DIR", directory)
        .env("PROCESS_TEST_ROLE", role)
        .env("PROCESS_TEST_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}
pub fn ready(directory: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !directory.join("ready").exists() {
        assert!(
            Instant::now() < deadline,
            "fixture failed to start: {}",
            directory.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub fn pids(directory: &Path) -> Vec<u32> {
    ["root", "compiler", "application"]
        .into_iter()
        .filter_map(|role| {
            fs::read_to_string(directory.join(format!("{role}.pid")))
                .ok()
                .map(|pid| pid.parse().expect("pid"))
        })
        .collect()
}
pub fn exited(directory: &Path) {
    let pids = pids(directory);
    assert!(!pids.is_empty());
    let deadline = Instant::now() + Duration::from_secs(3);
    while pids.iter().any(|pid| is_running(*pid)) {
        assert!(
            Instant::now() < deadline,
            "descendants still running: {:?}",
            pids.iter()
                .filter(|pid| is_running(**pid))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let before = fs::read(directory.join("application.beat")).expect("application heartbeat");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        before,
        fs::read(directory.join("application.beat")).expect("stopped heartbeat")
    );
}

#[test]
#[ignore = "subprocess fixture, launched by lifecycle tests"]
fn fixture() {
    let Ok(directory) = std::env::var("PROCESS_TEST_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let role = std::env::var("PROCESS_TEST_ROLE").expect("role");
    let mode = std::env::var("PROCESS_TEST_MODE").expect("mode");
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    let ignore = mode == "stubborn";
    ctrlc::set_handler(move || {
        if !ignore {
            signal.store(true, Ordering::Release);
        }
    })
    .expect("signal handler");
    fs::write(
        directory.join(format!("{role}.pid")),
        std::process::id().to_string(),
    )
    .expect("pid file");
    let worker = if role == "root" {
        Some(
            (if mode == "legacy" {
                Process::spawn
            } else {
                Process::spawn_worker
            })({
                let mut child = command(&directory, "compiler", &mode);
                child.stdout(Stdio::inherit()).stderr(Stdio::inherit());
                child
            })
            .expect("compiler"),
        )
    } else {
        None
    };
    if role == "compiler" {
        // Model a real compiler's app: inherited group/job, independent lifetime.
        #[allow(
            clippy::zombie_processes,
            reason = "fixture deliberately leaves descendant cleanup to the owner"
        )]
        let _application = command(&directory, "application", &mode)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("application");
    }
    if role == "application" {
        fs::write(directory.join("application.beat"), "0").expect("heartbeat");
        fs::write(directory.join("ready"), "ready").expect("ready");
    }
    if role == "root" {
        ready(&directory);
        if mode == "parent-exit" {
            std::mem::forget(worker);
            println!("done");
            return;
        }
        if matches!(mode.as_str(), "bad-auth" | "eof" | "handshake") {
            use std::io::Write;
            let mut socket = std::net::TcpStream::connect(
                std::env::var("CRANPOSE_EMBED_ADDRESS").expect("address"),
            )
            .expect("connect");
            fs::write(directory.join("connected"), "connected").expect("connected");
            if mode == "handshake" {
                while !stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(10));
                }
            } else {
                let token = if mode == "bad-auth" {
                    "wrong".into()
                } else {
                    std::env::var("CRANPOSE_EMBED_TOKEN").expect("token")
                };
                let mut hello = vec![1];
                hello.extend(2_u32.to_le_bytes());
                hello.extend((token.len() as u32).to_le_bytes());
                hello.extend(token.as_bytes());
                socket
                    .write_all(&(hello.len() as u32).to_le_bytes())
                    .expect("size");
                socket.write_all(&hello).expect("hello");
            }
        }
    }
    let mut count = 0_u64;
    while !stop.load(Ordering::Acquire) {
        count += 1;
        if role == "application" {
            fs::write(directory.join("application.beat"), count.to_string()).expect("heartbeat");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    #[cfg(unix)]
    if mode == "graceful"
        && let Some(mut worker) = worker
    {
        worker
            .terminate(Duration::from_millis(100))
            .expect("worker shutdown");
        assert!(
            worker
                .wait_for_tree_exit(Duration::from_secs(1))
                .expect("shared scope exit")
        );
        fs::write(directory.join("shared-scope-exited"), "confirmed").expect("exit marker");
    }
}
