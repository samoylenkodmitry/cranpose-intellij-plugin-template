//! Isolated Studio UI measurements over the production embedded protocol.
use anyhow::{Context, Result, ensure};
use clap::Args;
use cranpose_ide_host::{process::Process, protocol::Packet};
use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

#[derive(Args)]
pub struct Options {
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    log: PathBuf,
    #[arg(long)]
    report: PathBuf,
    #[arg(long, default_value_t=1000, value_parser=clap::value_parser!(u32).range(1..=10000))]
    nodes: u32,
    #[arg(long, default_value_t=20, value_parser=clap::value_parser!(u32).range(1..=120))]
    seconds: u32,
    #[arg(long, default_value_t=5, value_parser=clap::value_parser!(u32).range(1..=30))]
    settle_seconds: u32,
    /// Fail if unchanged visible snapshots trigger rendering after settling.
    #[arg(long)]
    require_quiet: bool,
}
fn message(host: &crate::hot_smoke::Host, channel: &str, value: Value) -> Result<()> {
    host.send(Packet::message(channel, &value.to_string()))
}
fn snapshot(host: &crate::hot_smoke::Host, nodes: &[Value], request: u64) -> Result<()> {
    message(
        host,
        "studio.child",
        json!({"session":1,"event":"message",
        "channel":"cranpose.inspector.v2.snapshot",
        "payload":json!({"schema":2,"requestId":request,"captureMicros":request,"nodes":nodes}).to_string()}),
    )
}
pub fn run(options: Options) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let token = format!("inspection-{:032x}", rand::random::<u128>());
    let log = fs::File::create(&options.log)?;
    let mut command = Command::new(options.binary.canonicalize()?);
    command
        .env("CRANPOSE_STUDIO", "1")
        .env("CRANPOSE_EMBED_ADDRESS", listener.local_addr()?.to_string())
        .env("CRANPOSE_EMBED_TOKEN", &token)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    let mut child = Process::spawn(command)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let stream = loop {
        if let Ok((stream, _)) = listener.accept() {
            break stream;
        }
        ensure!(
            child.try_wait()?.is_none(),
            "UI exited; see {}",
            options.log.display()
        );
        ensure!(Instant::now() < deadline, "UI connection timed out");
        thread::sleep(Duration::from_millis(10));
    };
    let host = crate::hot_smoke::Host::new(stream, &token)?;
    host.send(
        Packet::new(1)
            .int(0)
            .int(1000)
            .int(800)
            .float(1.0)
            .float(60.0),
    )?;
    message(
        &host,
        "studio.init",
        json!({"root":"/fixture","cache":"/cache","settings":{"inspect":true}}),
    )?;
    message(
        &host,
        "cranpose.project",
        json!({"targets":[{"manifest":"/fixture/Cargo.toml","packageName":"fixture","name":"fixture","kind":"bin"}]}),
    )?;
    message(&host, "studio.command", json!({"action":"build"}))?;
    message(
        &host,
        "studio.child",
        json!({"session":1,"event":"connected"}),
    )?;
    let mut nodes:Vec<_>=(0..options.nodes).map(|id| json!({"id":format!("node-{id}"),
        "parent":if id==0 {None} else {Some("node-0")},"kind":"Text", "text":format!("Inspection item {id}"),
        "width":100,"height":24,"sources":[{"name":"Item","file":"src/main.rs","line":42}],
        "modifiers":[{"name":"padding","properties":[{"name":"all","value":"12"}]}]})).collect();
    let mut request = 1;
    snapshot(&host, &nodes, request)?;
    verify_text(&host, "Text · Inspection item 0")?;
    let mut phases = Vec::new();
    for visible in [true, false] {
        host.send(Packet::new(13).int(0).byte(u8::from(visible)))?;
        // Include normal inspection traffic while settling to warm all parsing/rendering paths.
        phase(&host, &nodes, &mut request, options.settle_seconds)?;
        let before = crate::process_metrics::sample(&[child.id()])?[0];
        let frames = host.frames.load(Ordering::Relaxed);
        let started = Instant::now();
        let messages = phase(&host, &nodes, &mut request, options.seconds)?;
        let elapsed = started.elapsed().as_secs_f64();
        let cpu = crate::process_metrics::sample(&[child.id()])?[0] - before;
        let rendered = host.frames.load(Ordering::Relaxed) - frames;
        if options.require_quiet {
            ensure!(
                rendered == 0,
                "Unchanged snapshots caused {rendered} frames"
            );
        }
        phases.push(
            json!({"visible":visible,"elapsedSeconds":elapsed,"cpuSeconds":cpu,
            "cpuPercentOfOneCore":cpu/elapsed*100.0,"frames":rendered,"snapshots":messages}),
        );
    }
    // Quiet frames must not hide real changes or leave a re-shown inspector stale.
    host.send(Packet::new(13).int(0).byte(1))?;
    phase(&host, &nodes, &mut request, options.settle_seconds)?;
    let before = host.frames.load(Ordering::Relaxed);
    nodes[0]["text"] = json!("Changed layout");
    request += 1;
    snapshot(&host, &nodes, request)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while host.frames.load(Ordering::Relaxed) == before && Instant::now() < deadline {
        drain(&host)?;
        thread::sleep(Duration::from_millis(10));
    }
    ensure!(
        host.frames.load(Ordering::Relaxed) > before,
        "Changed layout did not render"
    );
    verify_text(&host, "Text · Changed layout")?;
    child.terminate(Duration::from_secs(2))?;
    ensure!(
        child.wait_for_tree_exit(Duration::from_secs(2))?,
        "UI descendants survived shutdown"
    );
    let result = json!({"result":"passed","nodes":options.nodes,"phases":phases,
        "changedLayoutRendered":true,"cpuClockResolutionSeconds":crate::process_metrics::RESOLUTION,
        "snapshotIntervalMs":500,"settleSeconds":options.settle_seconds});
    fs::write(&options.report, serde_json::to_vec_pretty(&result)?)?;
    println!("{result}");
    Ok(())
}
fn verify_text(host: &crate::hot_smoke::Host, expected: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut next = Instant::now();
    while Instant::now() < deadline {
        if Instant::now() >= next {
            message(host, "cranpose.inspector.v2.request", json!(1))?;
            next = Instant::now() + Duration::from_millis(200);
        }
        for response in host.messages.try_iter() {
            let (channel, payload, _) = response?;
            if channel == "cranpose.inspector.v2.snapshot"
                && payload["nodes"]
                    .as_array()
                    .is_some_and(|nodes| nodes.iter().any(|node| node["text"] == expected))
            {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!("Inspector UI did not display {expected:?}")
}

fn drain(host: &crate::hot_smoke::Host) -> Result<()> {
    for message in host.messages.try_iter() {
        message.context("UI connection")?;
    }
    Ok(())
}
fn phase(
    host: &crate::hot_smoke::Host,
    nodes: &[Value],
    request: &mut u64,
    seconds: u32,
) -> Result<u32> {
    let deadline = Instant::now() + Duration::from_secs(u64::from(seconds));
    let mut next = Instant::now();
    let mut count = 0;
    while Instant::now() < deadline {
        if Instant::now() >= next {
            *request += 1;
            snapshot(host, nodes, *request)?;
            count += 1;
            next += Duration::from_millis(500);
        }
        drain(host)?;
        thread::sleep(Duration::from_millis(10));
    }
    Ok(count)
}
