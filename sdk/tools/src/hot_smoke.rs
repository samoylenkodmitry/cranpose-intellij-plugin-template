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
    process::{Command, Stdio},
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
    /// Additional alternating edits, with save-to-ack and save-to-snapshot timings.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u32).range(0..=100))]
    pub measure_rounds: u32,
    /// Simulate ignored build output during each measured edit (fixture only).
    #[arg(long, default_value_t = 0, requires = "fixture")]
    pub background_noise_ms: u64,
    /// Measure startup without making source edits.
    #[arg(long)]
    pub startup_only: bool,
    /// Seconds per idle phase: visible, inspecting every 500 ms, and hidden.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u32).range(0..=120))]
    pub idle_seconds: u32,
    /// Include Cargo fingerprint/rebuild reasons in the compiler log.
    #[arg(long)]
    pub build_diagnostics: bool,
    /// Fail if either unchanged hot-reload support crate was recompiled.
    #[arg(long, requires = "build_diagnostics")]
    pub require_cached_support: bool,
    /// Write the test result and raw timing samples as JSON.
    #[arg(long)]
    pub report: Option<PathBuf>,
}
struct Cleanup {
    child: process::Process,
    source: PathBuf,
    original: String,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Err(error) = fs::write(&self.source, &self.original) {
            eprintln!("Restore fixture: {error}");
        }
        let _ = self.child.terminate(Duration::from_secs(2));
    }
}
struct Host {
    writer: Arc<Mutex<TcpStream>>,
    messages: mpsc::Receiver<Result<(String, Value, Instant)>>,
    frames: Arc<AtomicUsize>,
    runtime: Value,
    applied_at: Option<Instant>,
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
                                .send(Ok((
                                    channel,
                                    serde_json::from_str(&payload)?,
                                    Instant::now(),
                                )))
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
            applied_at: None,
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
        self.snapshot_after(expected, seconds, None)
    }
    fn snapshot_after(
        &mut self,
        expected: &[&str],
        seconds: u64,
        after_generation: Option<u64>,
    ) -> Result<Value> {
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
                    let (channel, payload, received_at) = result?;
                    if channel == "cranpose.dev.applied" {
                        println!("{}", json!({"runtime":payload}));
                        self.runtime = payload;
                        self.applied_at = Some(received_at);
                    } else if channel == "cranpose.inspector.v2.snapshot" && !self.runtime.is_null()
                    {
                        let nodes = payload["nodes"].as_array().context("Inspector nodes")?;
                        // A recomposed layout may reach the host before the runtime's
                        // acknowledgement. Wait for both instead of assuming channel order.
                        let acknowledged = after_generation.is_none_or(|previous| {
                            self.runtime["generation"]
                                .as_u64()
                                .is_some_and(|current| current > previous)
                        });
                        if acknowledged
                            && expected.iter().all(|text| {
                                nodes.iter().any(|node| node["text"].as_str() == Some(text))
                            })
                        {
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
    if options.build_diagnostics {
        command.env("CARGO_LOG", "cargo::core::compiler::fingerprint=info");
    }
    if let Some(dx) = options.dx {
        command.env("CRANPOSE_DX", dx.canonicalize()?);
    }

    let started = Instant::now();
    let mut cleanup = Cleanup {
        child: process::Process::spawn(command)?,
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
    let startup_ms = started.elapsed().as_secs_f64() * 1000.0;
    let pid = host.runtime["pid"].clone();
    let pids = preview_pids(&cleanup, &host, &options.log)?;
    let build_log = fs::read_to_string(&options.log)?;
    let support_builds = support_builds(&build_log);
    let cargo_build_ms = cargo_build_ms(&build_log);
    if options.require_cached_support {
        ensure!(
            support_builds.len() == 2 && support_builds.iter().all(|build| build["fresh"] == true),
            "Hot-reload support crates were rebuilt or freshness evidence is missing: {support_builds:?}"
        );
    }
    let idle = if options.idle_seconds > 0 {
        profile_idle(&mut host, &pids, options.idle_seconds)?
    } else {
        Vec::new()
    };
    if options.startup_only {
        let shutdown_ms = stop_preview(&mut cleanup.child, &pids)?;
        let result = json!({"result":"passed", "mode":"startup", "startupMs":startup_ms,
            "idle":idle, "shutdownMs":shutdown_ms, "stoppedPids":pids, "snapshotPollMs":200, "supportBuilds":support_builds, "cargoReportedBuildMs":cargo_build_ms});
        write_report(options.report, &result)?;
        return Ok(());
    }
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
    let mut timings = Vec::new();
    for round in 0..=options.measure_rounds {
        // Alternate same-length literals so every sample requires a new patch while
        // preserving closure identities, source positions and the remembered count.
        let label = if round.is_multiple_of(2) {
            "Add two!!"
        } else {
            "Plus two!"
        };
        let source = patched.replace("Add two!!", label);
        let previous_generation = host.runtime["generation"].as_u64().unwrap_or(0);
        let noise = Noise::start(&workspace, options.background_noise_ms)?;
        let saved = Instant::now();
        fs::write(&cleanup.source, source)?;
        host.snapshot_after(&["Count: 3", label], 120, Some(previous_generation))?;
        let observed = Instant::now();
        let applied = host
            .applied_at
            .context("Missing patch acknowledgement timestamp")?;
        ensure!(
            host.runtime["pid"] == pid,
            "Measured edit restarted the application"
        );
        ensure!(
            host.runtime["generation"].as_u64().unwrap_or(0) > previous_generation,
            "Measured edit reused an earlier acknowledgement"
        );
        ensure!(applied >= saved, "Patch acknowledgement predates the edit");
        let sample = json!({"round":round, "saveToAppliedMs":(applied-saved).as_secs_f64()*1000.0,
            "saveToSnapshotMs":(observed-saved).as_secs_f64()*1000.0,
            "generation":host.runtime["generation"]});
        println!("{}", json!({"timing":sample}));
        timings.push(sample);
        noise.finish()?;
    }
    if !options.measure_rounds.is_multiple_of(2) {
        fs::write(&cleanup.source, &patched)?;
        host.snapshot(&["Count: 3", "Add two!!"], 120)?;
    }
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
    let shutdown_ms = stop_preview(&mut cleanup.child, &pids)?;
    let result = json!({"result":"passed","pid":pid,"frames":host.frames.load(Ordering::Relaxed),"state":8,"connection":"unchanged","recovery":"compiler error, invalid syntax, and state type change",
        "startupMs":startup_ms, "backgroundNoiseMs":options.background_noise_ms,
        "snapshotPollMs":200, "patches":timings, "shutdownMs":shutdown_ms, "stoppedPids":pids, "idle":idle, "supportBuilds":support_builds, "cargoReportedBuildMs":cargo_build_ms});
    write_report(options.report, &result)
}
fn cargo_build_ms(log: &str) -> Option<f64> {
    log.lines().find_map(|line| {
        if !line.contains("Finished") || !line.contains(" profile ") {
            return None;
        }
        let (_, elapsed) = line.rsplit_once("target(s) in ")?;
        if elapsed.trim().is_empty() {
            return None;
        }
        let seconds = elapsed.split_whitespace().try_fold(0.0, |total, part| {
            let (number, scale) = if let Some(value) = part.strip_suffix('s') {
                (value, 1.0)
            } else {
                (part.strip_suffix('m')?, 60.0)
            };
            let value = number.parse::<f64>().ok()?;
            (value.is_finite() && value >= 0.0).then_some(total + value * scale)
        })?;
        Some(seconds * 1000.0)
    })
}
fn support_builds(log: &str) -> Vec<Value> {
    let mut crates = std::collections::BTreeMap::new();
    for event in log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        let Some(name) = event["target"]["name"].as_str() else {
            continue;
        };
        if event["reason"] == "compiler-artifact"
            && matches!(name, "cranpose_dev_macros" | "cranpose_dev_runtime")
        {
            let fresh = event["fresh"].as_bool().unwrap_or(false);
            crates
                .entry(name.to_owned())
                .and_modify(|previous| *previous &= fresh)
                .or_insert(fresh);
        }
    }
    crates
        .into_iter()
        .map(|(name, fresh)| json!({"crate":name,"fresh":fresh}))
        .collect()
}
fn write_report(report: Option<PathBuf>, result: &Value) -> Result<()> {
    if let Some(report) = report {
        fs::write(report, serde_json::to_vec_pretty(result)?)?;
    }
    println!("{result}");
    Ok(())
}
fn preview_pids(cleanup: &Cleanup, host: &Host, log: &std::path::Path) -> Result<[u32; 3]> {
    let compiler = fs::read_to_string(log)?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["cranposeDev"] == "compiler")
        .and_then(|event| event["pid"].as_u64())
        .context("Missing compiler process identity")?;
    Ok([
        cleanup.child.id(),
        compiler.try_into()?,
        host.runtime["pid"]
            .as_u64()
            .context("application PID")?
            .try_into()?,
    ])
}
fn stop_preview(child: &mut process::Process, pids: &[u32; 3]) -> Result<f64> {
    let stopped = Instant::now();
    child.terminate(Duration::from_secs(2))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while pids.iter().any(|pid| process::is_running(*pid)) {
        ensure!(
            Instant::now() < deadline,
            "Preview descendants survived shutdown: {pids:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
    Ok(stopped.elapsed().as_secs_f64() * 1000.0)
}
fn profile_idle(host: &mut Host, pids: &[u32; 3], seconds: u32) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for mode in ["visible", "inspecting", "hidden"] {
        host.send(Packet::new(13).int(0).byte(u8::from(mode != "hidden")))?;
        // Let visibility and the initial frame settle outside the measured window.
        thread::sleep(Duration::from_millis(500));
        while host.messages.try_recv().is_ok() {}
        let before = super::process_metrics::sample(pids)?;
        let frames = host.frames.load(Ordering::Relaxed);
        let started = Instant::now();
        let mut request = started;
        let mut requests = 0;
        while started.elapsed() < Duration::from_secs(u64::from(seconds)) {
            if mode == "inspecting" && Instant::now() >= request {
                host.send(Packet::message(
                    "cranpose.inspector.v2.request",
                    r#"{"requestId":1}"#,
                ))?;
                request = Instant::now() + Duration::from_millis(500);
                requests += 1;
            }
            while let Ok(message) = host.messages.try_recv() {
                message?;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let after = super::process_metrics::sample(pids)?;
        let elapsed = started.elapsed().as_secs_f64();
        let processes: Vec<_> = ["runner", "compiler", "application"].iter().enumerate().map(|(index, role)| {
            let cpu = (after[index] - before[index]).max(0.0);
            json!({"role":role,"pid":pids[index],"cpuSeconds":cpu,"percentOfOneCore":100.0*cpu/elapsed})
        }).collect();
        result.push(json!({"mode":mode,"seconds":elapsed,"processes":processes,
            "frames":host.frames.load(Ordering::Relaxed)-frames,"inspectorRequests":requests,
            "cpuClockResolutionSeconds":super::process_metrics::RESOLUTION}));
    }
    host.send(Packet::new(13).int(0).byte(1))?;
    Ok(result)
}

fn wait_log(
    path: &std::path::Path,
    offset: u64,
    marker: &str,
    child: &mut process::Process,
) -> Result<()> {
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

/// Join before the next edit so a sample never inherits another sample's noise.
struct Noise(Option<thread::JoinHandle<std::io::Result<()>>>);
impl Noise {
    fn finish(mut self) -> Result<()> {
        if let Some(worker) = self.0.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("Background noise worker panicked"))??;
        }
        Ok(())
    }
    fn start(workspace: &std::path::Path, millis: u64) -> Result<Self> {
        if millis == 0 {
            return Ok(Self(None));
        }
        let directory = workspace.join("target/cranpose-reload-benchmark");
        fs::create_dir_all(&directory)?;
        Ok(Self(Some(thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(millis) {
                fs::write(
                    directory.join("noise.json"),
                    start.elapsed().as_nanos().to_string(),
                )?;
                thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        }))))
    }
}
impl Drop for Noise {
    fn drop(&mut self) {
        if let Some(worker) = self.0.take() {
            match worker.join() {
                Ok(Ok(())) => {}
                result => eprintln!("Background noise failed: {result:?}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cargo_reported_timing_is_optional_and_handles_minutes() {
        assert_eq!(
            cargo_build_ms("Finished `desktop-dev` profile [unoptimized] target(s) in 1.47s"),
            Some(1470.0)
        );
        assert_eq!(
            cargo_build_ms("Finished `desktop-dev` profile [unoptimized] target(s) in 1m 03s"),
            Some(63000.0)
        );
        // Cargo colors the status word when CI forces terminal colors.
        assert_eq!(
            cargo_build_ms("\x1b[1m\x1b[92m    Finished\x1b[0m `desktop-dev` profile [unoptimized + debuginfo] target(s) in 3.52s"),
            Some(3520.0)
        );
        assert_eq!(cargo_build_ms("unrelated log output"), None);
    }
    #[test]
    fn freshness_does_not_hide_an_earlier_rebuild() {
        let artifact = |name, fresh| {
            json!({"reason":"compiler-artifact", "target":{"name":name}, "fresh":fresh}).to_string()
        };
        let log = [
            artifact("cranpose_dev_macros", false),
            artifact("cranpose_dev_runtime", true),
            artifact("cranpose_dev_macros", true),
            artifact("application", false),
        ]
        .join("\n");
        assert_eq!(
            support_builds(&log),
            vec![
                json!({"crate":"cranpose_dev_macros","fresh":false}),
                json!({"crate":"cranpose_dev_runtime","fresh":true})
            ]
        );
    }
    #[test]
    fn measured_snapshot_waits_for_acknowledgement_in_either_message_order() {
        for snapshot_first in [true, false] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
            let stream =
                TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
            let (_peer, _) = listener.accept().expect("accept");
            let (sender, messages) = mpsc::sync_channel(8);
            let mut host = Host {
                writer: Arc::new(Mutex::new(stream)),
                messages,
                frames: Arc::new(AtomicUsize::new(0)),
                runtime: json!({"generation":0,"pid":42}),
                applied_at: None,
            };
            let snapshot = json!({"nodes":[{"text":"edited"}]});
            let now = Instant::now();
            if snapshot_first {
                sender
                    .send(Ok((
                        "cranpose.inspector.v2.snapshot".into(),
                        snapshot.clone(),
                        now,
                    )))
                    .expect("snapshot");
                sender
                    .send(Ok((
                        "cranpose.dev.applied".into(),
                        json!({"generation":0,"pid":42}),
                        now,
                    )))
                    .expect("stale ack");
            }
            sender
                .send(Ok((
                    "cranpose.dev.applied".into(),
                    json!({"generation":1,"pid":42}),
                    now,
                )))
                .expect("ack");
            sender
                .send(Ok(("cranpose.inspector.v2.snapshot".into(), snapshot, now)))
                .expect("snapshot");
            host.snapshot_after(&["edited"], 1, Some(0))
                .expect("confirmed snapshot");
            assert_eq!(host.runtime["generation"], 1);
            assert_eq!(host.applied_at, Some(now));
        }
    }
}
