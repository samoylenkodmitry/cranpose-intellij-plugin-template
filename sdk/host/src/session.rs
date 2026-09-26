//! Authenticated loopback sessions. No JVM access occurs on worker threads.
use crate::protocol::{self, Event, Packet};
use anyhow::{Context, Result, ensure};
use rand::RngCore;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub enum SessionEvent {
    Connected,
    Data(Event),
    Log(String),
    Stopped(String),
}
pub struct Session {
    pub events: Receiver<SessionEvent>,
    outgoing: SyncSender<Packet>,
    stream: Arc<Mutex<Option<TcpStream>>>,
    child: Arc<Mutex<Option<crate::process::Process>>>,
    closed: Arc<AtomicBool>,
}
#[derive(Clone)]
pub struct Options {
    pub command: Vec<String>,
    pub directory: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub timeout: Duration,
}
impl Session {
    pub fn start(options: Options) -> Result<Self> {
        ensure!(!options.command.is_empty(), "Empty Cranpose command");
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let mut random = [0; 24];
        rand::rng().fill_bytes(&mut random);
        let token = random
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        let mut command = Command::new(&options.command[0]);
        command
            .args(&options.command[1..])
            .envs(options.environment)
            .env("CRANPOSE_EMBED_ADDRESS", listener.local_addr()?.to_string())
            .env("CRANPOSE_EMBED_TOKEN", &token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = options.directory {
            command.current_dir(directory);
        }
        let mut process =
            crate::process::Process::spawn(command).context("Start Cranpose native process")?;
        let (sender, events) = mpsc::sync_channel(32);
        for pipe in [
            process
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
            process
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let sender = sender.clone();
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
                    let line = String::from_utf8_lossy(&bytes[..count])
                        .trim_end()
                        .to_owned();
                    reader.consume(count);
                    if sender.send(SessionEvent::Log(line)).is_err() {
                        break;
                    }
                }
            });
        }
        let (outgoing, packets) = mpsc::sync_channel::<Packet>(1024);
        let stream = Arc::new(Mutex::new(None));
        let child = Arc::new(Mutex::new(Some(process)));
        let closed = Arc::new(AtomicBool::new(false));
        let result = Self {
            events,
            outgoing,
            stream: stream.clone(),
            child: child.clone(),
            closed: closed.clone(),
        };
        thread::spawn(move || {
            let run = || -> Result<()> {
                let deadline = Instant::now() + options.timeout;
                let mut socket = loop {
                    if closed.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    ensure!(Instant::now() < deadline, "Cranpose connection timed out");
                    if let Some(child) = child.lock().expect("child lock").as_mut() {
                        ensure!(
                            child.try_wait()?.is_none(),
                            "Cranpose exited before connecting"
                        );
                    }
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(50))
                        }
                        Err(e) => return Err(e.into()),
                    }
                };
                socket.set_nonblocking(false)?;
                socket.set_nodelay(true)?;
                socket.set_read_timeout(Some(Duration::from_secs(5)))?;
                // Make cancellation able to interrupt an incomplete handshake too.
                {
                    let mut active = stream.lock().expect("stream lock");
                    if closed.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    *active = Some(socket.try_clone()?);
                }
                match protocol::read(&mut socket)? {
                    Some(Event::Hello(version, received)) => ensure!(
                        version == protocol::VERSION
                            && constant_eq(token.as_bytes(), received.as_bytes()),
                        "Cranpose authentication/version mismatch"
                    ),
                    _ => anyhow::bail!("Missing Cranpose hello"),
                }
                socket.set_read_timeout(None)?;
                socket.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut writer = socket.try_clone()?;
                thread::spawn(move || {
                    while let Ok(packet) = packets.recv() {
                        if packet.send(&mut writer).is_err() {
                            break;
                        }
                    }
                    let _ = writer.shutdown(std::net::Shutdown::Both);
                });
                send(&sender, SessionEvent::Connected)?;
                while !closed.load(Ordering::Acquire) {
                    match protocol::read(&mut socket)? {
                        Some(event) => send(&sender, SessionEvent::Data(event))?,
                        None => break,
                    }
                }
                Ok(())
            };
            let message = run()
                .err()
                .map(|e| format!("{e:#}"))
                .unwrap_or_else(|| "Cranpose process stopped".into());
            closed.store(true, Ordering::Release);
            if let Some(socket) = stream.lock().expect("stream lock").take() {
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
            stop_child(&child);
            let _ = sender.send(SessionEvent::Stopped(message));
        });
        Ok(result)
    }
    pub fn send(&self, packet: Packet) {
        // Never wait for a native process on the IDE event thread. A wedged
        // consumer is disconnected rather than silently losing keyboard events.
        if self.outgoing.try_send(packet).is_err() {
            self.close();
        }
    }
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(stream) = self.stream.lock().expect("stream lock").take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let child = self.child.clone();
        thread::spawn(move || stop_child(&child));
    }
}
fn stop_child(child: &Mutex<Option<crate::process::Process>>) {
    // Release the lock before waiting so the network worker and close() can race safely.
    let process = child.lock().expect("child lock").take();
    if let Some(mut process) = process {
        let _ = process.terminate(Duration::from_secs(2));
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}
fn send(sender: &SyncSender<SessionEvent>, event: SessionEvent) -> Result<()> {
    sender
        .send(event)
        .map_err(|_| anyhow::anyhow!("Surface disposed"))
}
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0, |different, (a, b)| different | (a ^ b))
        == 0
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_validation() {
        assert!(constant_eq(b"token", b"token"));
        assert!(!constant_eq(b"token", b"taken"));
        assert!(!constant_eq(b"token", b"token2"));
    }
}
