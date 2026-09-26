//! Bounded cancellable subprocess capture shared by metadata and stability analysis.
use crate::Process;
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
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Process::spawn(command)?;
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
            let _ = child.terminate(Duration::ZERO);
            break Err(anyhow::anyhow!("Native command cancelled or timed out"));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let _ = child.terminate(Duration::ZERO);
                break Err(error.into());
            }
        }
    };
    // Close inherited pipe handles left by descendants before joining readers.
    let _ = child.terminate(Duration::ZERO);
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
