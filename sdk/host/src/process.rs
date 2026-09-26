//! Bounded cancellable subprocess capture shared by metadata and stability analysis.
use anyhow::{Result, ensure};
use std::{
    io::{Read, Write},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
pub fn capture(
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    cancel: impl Fn() -> bool,
) -> Result<Output> {
    isolate(&mut command);
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            stdin.write_all(&input)?;
        }
        Ok(())
    });
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = std::thread::spawn(move || read(stdout));
    let err = std::thread::spawn(move || read(stderr));
    let deadline = Instant::now() + timeout;
    let result = loop {
        if cancel() || Instant::now() > deadline {
            kill_tree(&mut child);
            let _ = child.wait();
            break Err(anyhow::anyhow!("Native command cancelled or timed out"));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                kill_tree(&mut child);
                let _ = child.wait();
                break Err(error.into());
            }
        }
    };
    // Close inherited pipe handles left by descendants before joining readers.
    kill_tree(&mut child);
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader panicked"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panicked"))??;
    let status = result?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("stdin writer panicked"))??;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
fn read(mut input: impl Read) -> Result<Vec<u8>> {
    let mut output = vec![];
    let mut buffer = [0; 8192];
    let mut overflow = false;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if output.len() + count <= 64 * 1024 * 1024 {
            output.extend_from_slice(&buffer[..count]);
        } else {
            overflow = true;
        }
    }
    ensure!(!overflow, "Native command output exceeds 64 MiB");
    Ok(output)
}

pub fn isolate(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000200);
    }
}
pub fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn timeout_closes_descendant_pipes() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60 & wait"]);
        let start = Instant::now();
        assert!(capture(command, None, Duration::from_millis(50), || false).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn completed_command_with_inherited_pipe_does_not_hang() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60 & echo done"]);
        let output = capture(command, None, Duration::from_secs(2), || false).expect("capture");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"done\n");
    }
}
