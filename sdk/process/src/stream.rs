//! Bounded output and cancellation. Every subprocess belongs to an owned process scope.
use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            bail!("Cancelled")
        }
        Ok(())
    }
}

/// Events delivered synchronously on the caller's worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionEvent<'a> {
    /// The OS created the owned process. This does not promise application readiness.
    Started {
        pid: u32,
    },
    Output {
        text: &'a str,
    },
}

pub fn execute(
    command: Command,
    timeout: Duration,
    cancel: &Cancellation,
    mut output: impl FnMut(&str),
) -> Result<()> {
    execute_observed(command, timeout, cancel, |event| {
        if let ExecutionEvent::Output { text } = event {
            output(text);
        }
    })
}

/// Stream output and notify the caller after successful process creation, before
/// any output callback. Failed spawn and pre-cancellation emit no Started event.
/// Keep callbacks short; they run on this worker and may request cancellation.
pub fn execute_observed(
    mut command: Command,
    timeout: Duration,
    cancel: &Cancellation,
    mut observe: impl FnMut(ExecutionEvent<'_>),
) -> Result<()> {
    cancel.check()?;
    let program = command.get_program().to_string_lossy().into_owned();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::Process::spawn(command).with_context(|| format!("Start {program}"))?;
    observe(ExecutionEvent::Started { pid: child.id() });
    let (tx, rx) = mpsc::sync_channel(64);
    let pipes: Vec<Box<dyn Read + Send>> = vec![
        Box::new(child.stdout.take().context("stdout pipe")?),
        Box::new(child.stderr.take().context("stderr pipe")?),
    ];
    let readers: Vec<_> = pipes
        .into_iter()
        .map(|pipe| {
            let tx = tx.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(pipe);
                loop {
                    let bytes = match reader.fill_buf() {
                        Ok([]) | Err(_) => break,
                        Ok(bytes) => bytes,
                    };
                    let count = bytes
                        .iter()
                        .position(|b| *b == b'\n')
                        .map_or(bytes.len(), |n| n + 1)
                        .min(8192);
                    let text = String::from_utf8_lossy(&bytes[..count])
                        .trim_end()
                        .to_owned();
                    reader.consume(count);
                    if tx.send(text).is_err() {
                        break;
                    }
                }
            })
        })
        .collect();
    drop(tx);
    let start = Instant::now();
    let mut status = None;
    let mut failure = None;
    loop {
        if failure.is_none() && (cancel.is_cancelled() || start.elapsed() >= timeout) {
            failure = Some(if cancel.is_cancelled() {
                "Cancelled"
            } else {
                "Process timed out"
            });
            child.terminate(Duration::from_millis(500))?;
        }
        if status.is_none() {
            status = child.try_wait()?;
            if status.is_some() {
                child.terminate(Duration::from_millis(100))?;
            }
        }
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(line) => observe(ExecutionEvent::Output { text: &line }),
            Err(mpsc::RecvTimeoutError::Disconnected) if status.is_some() => break,
            Err(_) => {}
        }
    }
    anyhow::ensure!(
        child.wait_for_tree_exit(Duration::from_secs(1))?,
        "Process descendants did not exit: {program}"
    );
    for reader in readers {
        reader
            .join()
            .map_err(|_| anyhow::anyhow!("Output reader failed"))?;
    }
    if let Some(message) = failure {
        bail!("{message}: {program}")
    }
    let status = status.context("Missing process exit status")?;
    if !status.success() {
        bail!("{program} exited with {status}")
    }
    Ok(())
}
