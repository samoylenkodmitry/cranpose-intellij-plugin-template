//! End-to-end hot reload test over the production embedded protocol.
use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use cranpose_ide_host::{
    process,
    protocol::{self, Event, Packet},
};
use rand::RngCore;
use serde_json::{Value, json};
use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Args)]
pub struct Options {
    #[arg(long)]
    pub runner: PathBuf,
    #[arg(long, required_unless_present = "fixture", conflicts_with = "fixture")]
    pub workspace: Option<PathBuf>,
    #[arg(long, value_parser=["counter","library"])]
    pub fixture: Option<String>,
    #[arg(long)]
    pub cache: PathBuf,
    #[arg(long)]
    pub log: PathBuf,
    #[arg(long, default_value = "src/main.rs")]
    pub source: PathBuf,
    #[arg(long)]
    pub dx: Option<PathBuf>,
}
struct Cleanup {
    child: Child,
    source: PathBuf,
    original: String,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Err(error) = fs::write(&self.source, &self.original) {
            eprintln!("Restore fixture: {error}");
        }
        process::kill_tree(&mut self.child);
        let _ = self.child.wait();
    }
}
struct Host {
    writer: Arc<Mutex<TcpStream>>,
    messages: mpsc::Receiver<Result<(String, Value)>>,
    frames: Arc<AtomicUsize>,
    runtime: Value,
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self
            .writer
            .lock()
            .expect("socket")
            .shutdown(std::net::Shutdown::Both);
    }
}
impl Host {
    fn new(mut stream: TcpStream, token: &str) -> Result<Self> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        ensure!(
            matches!(protocol::read(&mut stream)?, Some(Event::Hello(2, ref value)) if value == token),
            "Invalid preview authentication"
        );
        stream.set_read_timeout(None)?;
        let writer = Arc::new(Mutex::new(stream.try_clone()?));
        let frames = Arc::new(AtomicUsize::new(0));
        let (sender, messages) = mpsc::sync_channel(64);
        let output = writer.clone();
        let count = frames.clone();
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                loop {
                    match protocol::read(&mut stream)?.context("Preview connection closed")? {
                        Event::Frame(frame) => {
                            count.fetch_add(1, Ordering::Relaxed);
                            Packet::new(11)
                                .int(frame.surface)
                                .int(frame.id)
                                .send(&mut *output.lock().expect("socket"))?;
                        }
                        Event::Message(channel, payload) => {
                            if sender
                                .send(Ok((channel, serde_json::from_str(&payload)?)))
                                .is_err()
                            {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
            })();
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        });
        let host = Self {
            writer,
            messages,
            frames,
            runtime: Value::Null,
        };
        host.send(
            Packet::new(1)
                .int(0)
                .int(320)
                .int(240)
                .float(1.0)
                .float(60.0),
        )?;
        host.send(Packet::new(13).int(0).byte(1))?;
        Ok(host)
    }
    fn send(&self, packet: Packet) -> Result<()> {
        packet.send(&mut *self.writer.lock().expect("socket"))
    }
    fn click(&self, x: f32, y: f32) -> Result<()> {
        for kind in [2, 3, 4] {
            self.send(Packet::new(kind).int(0).float(x).float(y))?;
        }
        Ok(())
    }
    fn snapshot(&mut self, expected: &[&str], seconds: u64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut next = Instant::now();
        while Instant::now() < deadline {
            if Instant::now() >= next {
                self.send(Packet::message(
                    "cranpose.inspector.v2.request",
                    r#"{"requestId":1}"#,
                ))?;
                next = Instant::now() + Duration::from_millis(200);
            }
            match self.messages.recv_timeout(Duration::from_millis(200)) {
                Ok(result) => {
                    let (channel, payload) = result?;
                    if channel == "cranpose.dev.applied" {
                        println!("{}", json!({"runtime":payload}));
                        self.runtime = payload;
                    } else if channel == "cranpose.inspector.v2.snapshot" && !self.runtime.is_null()
                    {
                        let nodes = payload["nodes"].as_array().context("Inspector nodes")?;
                        if expected.iter().all(|text| {
                            nodes.iter().any(|node| node["text"].as_str() == Some(text))
                        }) {
                            return Ok(payload);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => return Err(error.into()),
            }
        }
        bail!(
            "Preview never displayed {expected:?}; {} frames",
            self.frames.load(Ordering::Relaxed)
        )
    }
}
pub fn run(options: Options) -> Result<()> {
    let fixture_dir = tempfile::tempdir()?;
    let workspace = if let Some(workspace) = options.workspace {
        workspace
    } else {
        copy_fixture(
            &crate::root()
                .join("dev-runner/tests/fixtures")
                .join(options.fixture.context("fixture")?),
            fixture_dir.path(),
        )?;
        fixture_dir.path().to_owned()
    };
    let source = workspace.join(&options.source);
    let original = fs::read_to_string(&source)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    fs::create_dir_all(&options.cache)?;
    let config = json!({"root":workspace.canonicalize()?, "cache":options.cache.canonicalize()?, "package":"cranpose-hot-counter", "target":"cranpose-hot-counter", "kind":"bin", "hotReload":true});
    let log = fs::File::create(&options.log)?;
    let mut command = Command::new(options.runner.canonicalize()?);
    command
        .arg(config.to_string())
        .current_dir(&workspace)
        .env("CRANPOSE_EMBED_ADDRESS", listener.local_addr()?.to_string())
        .env("CRANPOSE_EMBED_TOKEN", &token)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    if let Some(dx) = options.dx {
        command.env("CRANPOSE_DX", dx.canonicalize()?);
    }
    process::isolate(&mut command);
    let mut cleanup = Cleanup {
        child: command.spawn()?,
        source,
        original,
    };
    let deadline = Instant::now() + Duration::from_secs(900);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        ensure!(
            cleanup.child.try_wait()?.is_none(),
            "Compiler exited; see {}",
            options.log.display()
        );
        ensure!(
            Instant::now() < deadline,
            "Preview did not connect; see {}",
            options.log.display()
        );
        thread::sleep(Duration::from_millis(100));
    };
    let mut host = Host::new(stream, &token)?;
    let snapshot = host.snapshot(&["Count: 0", "Increment"], 30)?;
    let pid = host.runtime["pid"].clone();
    let label = snapshot["nodes"]
        .as_array()
        .context("nodes")?
        .iter()
        .find(|n| n["text"] == "Increment")
        .context("button")?;
    let x =
        (label["x"].as_f64().context("x")? + label["width"].as_f64().context("width")? / 2.) as f32;
    let y = (label["y"].as_f64().context("y")? + label["height"].as_f64().context("height")? / 2.)
        as f32;
    for count in 1..=3 {
        host.click(x, y)?;
        host.snapshot(&[&format!("Count: {count}")], 10)?;
    }
    let patched = cleanup
        .original
        .replace("\"Increment\"", "\"Add two!!\"")
        .replace("count.get() + 1", "count.get() + 2");
    ensure!(patched != cleanup.original, "Fixture lacks patch point");
    fs::write(&cleanup.source, &patched)?;
    host.snapshot(&["Count: 3", "Add two!!"], 120)?;
    ensure!(host.runtime["pid"] == pid, "Application restarted");
    ensure!(
        host.runtime["generation"].as_u64().unwrap_or(0) > 0,
        "No patch acknowledgement"
    );
    host.click(x, y)?;
    host.snapshot(&["Count: 5", "Add two!!"], 10)?;
    let offset = fs::metadata(&options.log)?.len();
    fs::write(
        &cleanup.source,
        patched.replace("count.get() + 2", "count.get() + 999999999999999999999"),
    )?;
    wait_log(
        &options.log,
        offset,
        "literal out of range",
        &mut cleanup.child,
    )?;
    host.snapshot(&["Count: 5", "Add two!!"], 10)?;
    let offset = fs::metadata(&options.log)?.len();
    fs::write(&cleanup.source, format!("{patched}\nfn incompatible( {{"))?;
    wait_log(&options.log, offset, "restartRequired", &mut cleanup.child)?;
    host.click(x, y)?;
    host.snapshot(&["Count: 7", "Add two!!"], 10)?;
    let offset = fs::metadata(&options.log)?.len();
    fs::write(&cleanup.source, patched.replace("0_i32", "0_i64"))?;
    wait_log(&options.log, offset, "restartRequired", &mut cleanup.child)?;
    host.snapshot(&["Count: 7", "Add two!!"], 10)?;
    fs::write(&cleanup.source, &cleanup.original)?;
    host.snapshot(&["Count: 7", "Increment"], 120)?;
    host.click(x, y)?;
    host.snapshot(&["Count: 8", "Increment"], 10)?;
    ensure!(
        host.runtime["pid"] == pid,
        "Recovery restarted the application"
    );
    println!(
        "{}",
        json!({"result":"passed","pid":pid,"frames":host.frames.load(Ordering::Relaxed),"state":8,"connection":"unchanged","recovery":"compiler error, invalid syntax, and state type change"})
    );
    Ok(())
}
fn wait_log(path: &std::path::Path, offset: u64, marker: &str, child: &mut Child) -> Result<()> {
    use std::io::{Read, Seek};
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        let mut file = fs::File::open(path)?;
        file.seek(std::io::SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if String::from_utf8_lossy(&bytes).contains(marker) {
            return Ok(());
        }
        ensure!(
            child.try_wait()?.is_none(),
            "Compiler stopped during recovery"
        );
        thread::sleep(Duration::from_millis(100));
    }
    bail!("Missing compiler event: {marker}")
}

fn copy_fixture(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() == "target" {
            continue;
        }
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if path.is_dir() {
            copy_fixture(&path, &target)?;
        } else {
            fs::copy(path, target)?;
        }
    }
    Ok(())
}
