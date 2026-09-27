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
    /// Measure content changes through the real UI and verify the resulting labels.
    #[arg(long, default_value_t=0, value_parser=clap::value_parser!(u32).range(0..=50))]
    change_rounds: u32,
    /// Stream changed layouts without requesting UI snapshots during the CPU sample.
    #[arg(long, default_value_t=0, value_parser=clap::value_parser!(u32).range(0..=120))]
    change_seconds: u32,
    /// Exercise end-of-list scrolling, collapse at the end and expansion.
    #[arg(long)]
    exercise_tree: bool,
    /// Structural budget for the composed UI, independent of machine speed.
    #[arg(long)]
    max_ui_nodes: Option<usize>,
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
    let initial_cpu = crate::process_metrics::sample(&[child.id()])?[0];
    let initial_started = Instant::now();
    snapshot(&host, &nodes, request)?;
    let initial_view = verify_text(&host, "Text · Inspection item 0")?;
    check_node_budget(&initial_view, options.max_ui_nodes)?;
    let initial_ms = initial_started.elapsed().as_secs_f64() * 1000.0;
    let initial_cpu_seconds = crate::process_metrics::sample(&[child.id()])?[0] - initial_cpu;
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
    let mut changes = Vec::new();
    for round in 0..options.change_rounds {
        // Change every label to exercise real tree composition, not an unchanged poll.
        for (index, node) in nodes.iter_mut().enumerate() {
            node["text"] = json!(format!("Update {round} item {index}"));
        }
        request += 1;
        let cpu_before = crate::process_metrics::sample(&[child.id()])?[0];
        let started = Instant::now();
        snapshot(&host, &nodes, request)?;
        let view = verify_text(&host, &format!("Text · Update {round} item 0"))?;
        check_node_budget(&view, options.max_ui_nodes)?;
        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let cpu_seconds = crate::process_metrics::sample(&[child.id()])?[0] - cpu_before;
        changes.push(json!({"round":round,"latencyMs":latency_ms,"cpuSeconds":cpu_seconds,
            "uiNodes":view["nodes"].as_array().map(Vec::len),"uiSnapshotTruncated":view["truncated"]}));
    }
    let changed_workload = if options.change_seconds > 0 {
        phase(&host, &nodes, &mut request, 1)?;
        let cpu_before = crate::process_metrics::sample(&[child.id()])?[0];
        let frames_before = host.frames.load(Ordering::Relaxed);
        let started = Instant::now();
        let deadline = started + Duration::from_secs(u64::from(options.change_seconds));
        let mut next = started;
        let mut updates = 0;
        while Instant::now() < deadline {
            if Instant::now() >= next {
                updates += 1;
                for (index, node) in nodes.iter_mut().enumerate() {
                    node["text"] = json!(format!("Stream {updates} item {index}"));
                }
                request += 1;
                snapshot(&host, &nodes, request)?;
                next += Duration::from_millis(500);
            }
            drain(&host)?;
            thread::sleep(Duration::from_millis(10));
        }
        let elapsed = started.elapsed().as_secs_f64();
        let cpu = crate::process_metrics::sample(&[child.id()])?[0] - cpu_before;
        let frames = host.frames.load(Ordering::Relaxed) - frames_before;
        ensure!(frames > 0, "Changed layout stream did not render");
        // Verify only after sampling: inspection acknowledgements are outside the CPU interval.
        verify_text(&host, &format!("Text · Stream {updates} item 0"))?;
        Some(
            json!({"elapsedSeconds":elapsed,"cpuSeconds":cpu,"cpuPercentOfOneCore":cpu/elapsed*100.0,
            "updates":updates,"frames":frames,"lastUpdateVerified":true}),
        )
    } else {
        None
    };
    let tree_checks = if options.exercise_tree {
        ensure!(
            options.nodes >= 100,
            "Tree scrolling checks require at least 100 nodes"
        );
        Some(exercise_tree(&host, &nodes)?)
    } else {
        None
    };
    child.terminate(Duration::from_secs(2))?;
    ensure!(
        child.wait_for_tree_exit(Duration::from_secs(2))?,
        "UI descendants survived shutdown"
    );
    let result = json!({"result":"passed","nodes":options.nodes,"phases":phases,
        "changedLayoutRendered":true,"cpuClockResolutionSeconds":crate::process_metrics::RESOLUTION,
        "snapshotIntervalMs":500,"settleSeconds":options.settle_seconds,
        "initialLayout":{"latencyMs":initial_ms,"cpuSeconds":initial_cpu_seconds,
            "uiNodes":initial_view["nodes"].as_array().map(Vec::len),"uiSnapshotTruncated":initial_view["truncated"]},
        "changedLayouts":changes,"changedWorkload":changed_workload,"treeChecks":tree_checks});
    fs::write(&options.report, serde_json::to_vec_pretty(&result)?)?;
    println!("{result}");
    Ok(())
}
fn check_node_budget(view: &Value, maximum: Option<usize>) -> Result<()> {
    if let Some(maximum) = maximum {
        let count = view["nodes"].as_array().context("Missing UI nodes")?.len();
        ensure!(
            view["truncated"] != true && count <= maximum,
            "Inspector composed {count} UI nodes, budget {maximum}; truncated={}",
            view["truncated"]
        );
    }
    Ok(())
}

fn verify_text(host: &crate::hot_smoke::Host, expected: &str) -> Result<Value> {
    verify_view(host, expected, |view| text_node(view, expected).is_some())
}

fn text_node<'a>(view: &'a Value, expected: &str) -> Option<&'a Value> {
    view["nodes"]
        .as_array()?
        .iter()
        .find(|node| node["text"] == expected)
}

fn verify_view(
    host: &crate::hot_smoke::Host,
    description: &str,
    matches: impl Fn(&Value) -> bool,
) -> Result<Value> {
    static REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let request = REQUEST.fetch_add(1, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut next = Instant::now();
    let mut last = Value::Null;
    while Instant::now() < deadline {
        if Instant::now() >= next {
            message(host, "cranpose.inspector.v2.request", json!(request))?;
            next = Instant::now() + Duration::from_millis(20);
        }
        for response in host.messages.try_iter() {
            let (channel, payload, _) = response?;
            if channel == "cranpose.inspector.v2.snapshot"
                && payload["requestId"].as_u64() == Some(request)
            {
                if matches(&payload) {
                    return Ok(payload);
                }
                last = payload;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    let labels: Vec<_> = last["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["text"].is_string())
        .take(60)
        .map(|node| json!({"text":node["text"],"x":node["x"],"y":node["y"]}))
        .collect();
    anyhow::bail!("Inspector UI did not display {description:?}; last labels: {labels:?}")
}

fn center(node: &Value) -> (f32, f32) {
    let number = |key| node[key].as_f64().unwrap_or_default() as f32;
    (
        number("x") + number("width") / 2.0,
        number("y") + number("height") / 2.0,
    )
}

fn click(host: &crate::hot_smoke::Host, view: &Value, text: &str) -> Result<()> {
    let (x, y) = center(text_node(view, text).context("Missing inspector control")?);
    for kind in [3, 4] {
        host.send(Packet::new(kind).int(0).float(x).float(y))?;
    }
    Ok(())
}

fn exercise_tree(host: &crate::hot_smoke::Host, nodes: &[Value]) -> Result<Value> {
    let label = |index: usize| {
        format!(
            "Text · {}",
            nodes[index]["text"].as_str().unwrap_or_default()
        )
    };
    let first = label(0);
    let last = label(nodes.len() - 1);
    let view = verify_text(host, &first)?;
    let (x, y) = center(text_node(&view, &first).context("First row")?);
    let visible = |view: &Value, text: &str| {
        text_node(view, text).is_some_and(|node| {
            let (_, row_y) = center(node);
            row_y >= y - 14.0 && row_y < 790.0
        })
    };
    let scroll = |delta| {
        host.send(
            Packet::new(6)
                .int(0)
                .float(x)
                .float(y)
                .float(0.0)
                .float(delta)
                .byte(0),
        )
    };
    scroll(-(nodes.len() as f32 * 40.0))?;
    let bottom = verify_view(host, "last row at the bottom", |view| visible(view, &last))?;
    click(host, &bottom, "Collapse all")?;
    let collapsed = verify_view(host, "collapsed tree after scrolling", |view| {
        visible(view, &first) && text_node(view, &last).is_none()
    })?;
    click(host, &collapsed, "Expand all")?;
    let expanded = verify_view(host, "expanded first row", |view| {
        visible(view, &first) && text_node(view, &label(1)).is_some()
    })?;
    scroll(-(nodes.len() as f32 * 40.0))?;
    verify_view(host, "last row after expansion", |view| {
        visible(view, &last)
    })?;
    scroll(nodes.len() as f32 * 40.0)?;
    let top = verify_view(host, "first row after return", |view| visible(view, &first))?;
    // The text field is immediately left of Clear and below its label.
    let (clear_x, field_y) = center(text_node(&top, "Clear").context("Clear control")?);
    let label_x = text_node(&top, "Filter · name, text, source or modifier")
        .context("Filter label")?["x"]
        .as_f64()
        .context("Filter x")? as f32;
    let field_x = (label_x + clear_x) / 2.0;
    for kind in [3, 4] {
        host.send(Packet::new(kind).int(0).float(field_x).float(field_y))?;
    }
    host.send(
        Packet::new(8)
            .int(0)
            .text(&format!("node-{}", nodes.len() - 1)),
    )?;
    let filtered = verify_view(host, "last row filter", |view| {
        visible(view, &last) && text_node(view, "1 matches").is_some()
    })?;
    click(host, &filtered, &last)?;
    let details = verify_text(
        host,
        nodes[nodes.len() - 1]["text"]
            .as_str()
            .context("Last label")?,
    )?;
    ensure!(
        text_node(&details, "Item :42 ↗").is_some(),
        "Selected row lost source navigation"
    );
    click(host, &details, "Layout")?;
    let filtered = verify_text(host, "1 matches")?;
    click(host, &filtered, "Clear")?;
    verify_view(host, "first row after clearing filter", |view| {
        visible(view, &first) && text_node(view, &format!("{} nodes", nodes.len())).is_some()
    })?;
    Ok(
        json!({"lastRowReached":true,"collapseAtBottom":true,"expandedLastRowReached":true,
        "returnedToFirstRow":true,"filterLastRow":true,"selectedDetailsAndSource":true,
        "clearFilter":true,"expandedUiNodes":expanded["nodes"].as_array().map(Vec::len)}),
    )
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
