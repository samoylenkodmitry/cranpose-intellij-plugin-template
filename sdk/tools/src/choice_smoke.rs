//! Exercise compact choices over the production native surface protocol.
use crate::{
    hot_smoke::Host,
    ui_probe::{click, text_node, verify_text, verify_view},
};
use anyhow::{Context, Result, ensure};
use clap::Args;
use cranpose_ide_host::{process::Process, protocol::Packet};
use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Args)]
pub struct Options {
    /// Record the seven-target Studio dashboard geometry instead of the shared fixture.
    #[arg(long)]
    dashboard: bool,
    /// Require the dashboard background to cover the entire viewport.
    #[arg(long, requires = "dashboard")]
    require_background: bool,
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    log: PathBuf,
    #[arg(long)]
    report: PathBuf,
}

pub fn run(options: Options) -> Result<()> {
    if options.dashboard {
        return dashboard(options);
    }
    let mut checks = Vec::new();
    for scale in [1, 2] {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let token = format!("choices-{:032x}", rand::random::<u128>());
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&options.log)?;
        let mut command = Command::new(options.binary.canonicalize()?);
        command
            .env("CRANPOSE_AUTHORING", "choices")
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
                child.try_wait()?.is_none() && Instant::now() < deadline,
                "Choice fixture did not connect"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let host = Host::new(stream, &token)?;
        host.send(
            Packet::new(1)
                .int(0)
                .int(380 * scale)
                .int(650 * scale)
                .float(scale as f32)
                .float(60.0),
        )?;
        host.send(Packet::new(13).int(0).byte(1))?;
        let collapsed = verify_text(&host, "Change target · 7 choices")?;
        ensure!(
            text_node(&collapsed, "Target 2").is_none(),
            "Closed choices still composed"
        );
        let collapsed_y = y(&collapsed, "Selected: item-0")?;
        click(&host, &collapsed, "Change target · 7 choices")?;
        let first = verify_text(&host, "1–5 of 7")?;
        ensure!(text_node(&first, "Target 5").is_none(), "Unbounded page");
        click(&host, &first, "Next")?;
        let last = verify_text(&host, "6–7 of 7")?;
        click(&host, &last, "Same name")?;
        let chosen = verify_text(&host, "Selected: item-6")?;
        ensure!(
            text_node(&chosen, "Search target").is_none(),
            "Selection did not close chooser"
        );
        // Reopening reaches the selected page, including duplicate labels.
        click(&host, &chosen, "Change target · 7 choices")?;
        let last = verify_text(&host, "6–7 of 7")?;
        search(&host, &last, "temporary filter")?;
        let none = verify_text(&host, "No matching choices")?;
        search(&host, &none, "")?;
        let last = verify_text(&host, "1–5 of 7")?;
        search(&host, &last, "package 1")?;
        let filtered = verify_view(&host, "filtered duplicate", |view| {
            text_node(view, "Package 1 · bin").is_some()
                && text_node(view, "Target 0").is_none()
                && text_node(view, "1–5 of 7").is_none()
                && text_node(view, "6–7 of 7").is_none()
        })?;
        ensure!(
            text_node(&filtered, "Target 5").is_none(),
            "Filtering retained stale result"
        );
        // Native Tab from the search field reaches the first result; Enter selects it.
        key(&host, "Tab", 0)?;
        key(&host, "Enter", 0)?;
        verify_text(&host, "Selected: item-1")?;
        let closed = verify_text(&host, "Change target · 7 choices")?;
        // Focus returns to the trigger, so Enter opens it again.
        key(&host, "Enter", 0)?;
        let opened = verify_text(&host, "1–5 of 7")?;
        search(&host, &opened, "no such target")?;
        let empty = verify_text(&host, "No matching choices")?;
        click(&host, &empty, "Close choices")?;
        verify_text(&host, "Selected: item-1")?;
        host.send(Packet::message("demo.choices", "10001"))?;
        let huge = verify_text(&host, "Change target · 10001 choices")?;
        ensure!(
            (y(&huge, "Selected: item-1")? - y(&closed, "Selected: item-1")?).abs() < 1.0,
            "Collapsed height grows with catalog size"
        );
        click(&host, &huge, "Change target · 10001 choices")?;
        let huge = verify_text(&host, "1–5 of 10001")?;
        let nodes = huge["nodes"].as_array().context("UI nodes")?.len();
        ensure!(
            nodes <= 65,
            "Chooser exceeded 65-node composition budget: {nodes}"
        );
        search(&host, &huge, "target 10000")?;
        let end = verify_text(&host, "Target 10000")?;
        click(&host, &end, "Target 10000")?;
        let chosen = verify_text(&host, "Selected: item-10000")?;
        click(&host, &chosen, "Change target · 10001 choices")?;
        verify_text(&host, "10001–10001 of 10001")?;
        host.send(Packet::message("demo.choices", "7"))?;
        let shrunk = verify_text(&host, "6–7 of 7")?;
        ensure!(
            text_node(&shrunk, "Choose target").is_some(),
            "Removed selection misrepresented"
        );
        click(&host, &shrunk, "Same name")?;
        verify_text(&host, "Selected: item-6")?;
        host.send(Packet::message("demo.choices", "0"))?;
        let none = verify_text(&host, "Change target · 0 choices")?;
        click(&host, &none, "Change target · 0 choices")?;
        verify_text(&host, "No matching choices")?;
        key(&host, "Escape", 0)?;
        verify_view(&host, "Escape closes", |view| {
            text_node(view, "Search target").is_none()
        })?;
        child.terminate(Duration::from_secs(2))?;
        ensure!(
            child.wait_for_tree_exit(Duration::from_secs(2))?,
            "Fixture remained alive"
        );
        checks.push(
            json!({"scale":scale,"collapsedFooterY":collapsed_y,"largeCatalogUiNodes":nodes,
            "pointerSelection":true,"keyboardSelection":true,"duplicateIdentity":true,
            "search":true,"catalogShrink":true,"empty":true,"exited":true}),
        );
    }
    fs::write(
        options.report,
        serde_json::to_vec_pretty(&json!({"checks":checks}))?,
    )?;
    Ok(())
}
fn dashboard(options: Options) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let token = format!("dashboard-{:032x}", rand::random::<u128>());
    let log = fs::File::create(&options.log)?;
    let mut command = Command::new(options.binary.canonicalize()?);
    command
        .env_remove("CRANPOSE_AUTHORING")
        .env_remove("CRANPOSE_STUDIO")
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
            child.try_wait()?.is_none() && Instant::now() < deadline,
            "Dashboard did not connect"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let capture = std::sync::Arc::new(std::sync::Mutex::new(
        crate::ui_probe::FrameCapture::default(),
    ));
    let host = Host::capturing(stream, &token, Some(capture.clone()))?;
    host.send(
        Packet::new(1)
            .int(0)
            .int(440)
            .int(1200)
            .float(1.0)
            .float(60.0),
    )?;
    host.send(Packet::new(13).int(0).byte(1))?;
    let names = [
        "cranpose-showcase",
        "cranpose-showcase-ios",
        "robot-app-screens",
        "robot-planet-gallery",
        "robot-saved-removal",
        "robot-tab-switch",
        "robot-wide-layout",
    ];
    let targets: Vec<_> = names
        .into_iter()
        .map(|name| {
            json!({
                "id":format!("/fixture/Cargo.toml::{name}"), "name":name,
                "packageName":"cranpose-showcase", "kind":"bin"
            })
        })
        .collect();
    host.send(Packet::message(
        "cranpose.project",
        &json!({
            "status":"7 applications in 1 package", "targets":targets,
            "selected":"/fixture/Cargo.toml::cranpose-showcase", "root":"/fixture"
        })
        .to_string(),
    ))?;
    let view = verify_view(&host, "loaded dashboard", |view| {
        text_node(view, "Build & run").is_some()
            && text_node(view, "Preview").is_some()
            && text_node(view, "cranpose-showcase").is_some()
    })?;
    // Snapshot delivery can precede the corresponding paint. Give the renderer a
    // bounded opportunity to fill the viewport before evaluating actual pixels.
    let deadline = Instant::now() + Duration::from_secs(2);
    let (top, bottom) = loop {
        let frame = capture.lock().expect("frame capture");
        let top = frame.pixel(5, 5).map(|(rgba, _)| rgba);
        let bottom = frame.pixel(5, 1195).map(|(rgba, _)| rgba);
        if (top.is_some_and(|pixel| pixel[3] == 255) && top == bottom) || Instant::now() >= deadline
        {
            break (top, bottom);
        }
        drop(frame);
        thread::sleep(Duration::from_millis(10));
    };
    if options.require_background {
        ensure!(
            top.is_some_and(|pixel| pixel[3] == 255) && top == bottom,
            "Dashboard background does not cover viewport: top={top:?}, bottom={bottom:?}"
        );
    }
    let report = json!({"logicalWidth":440,"logicalHeight":1200,"targets":7,
        "previewY":y(&view,"Preview")?,"buildRunY":y(&view,"Build & run")?,
        "backgroundTop":top,"backgroundBottom":bottom,
        "uiNodes":view["nodes"].as_array().context("nodes")?.len(),
        "limits":"Isolated native dashboard geometry; excludes IDE chrome, startup and latency."});
    child.terminate(Duration::from_secs(2))?;
    ensure!(
        child.wait_for_tree_exit(Duration::from_secs(2))?,
        "Dashboard remained alive"
    );
    fs::write(options.report, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
fn y(view: &Value, label: &str) -> Result<f64> {
    text_node(view, label).context("label")?["y"]
        .as_f64()
        .context("label y")
}
fn search(host: &Host, view: &Value, text: &str) -> Result<()> {
    let label = text_node(view, "Search target").context("search label")?;
    let x = label["x"].as_f64().context("x")? as f32 + 20.0;
    let y = (label["y"].as_f64().context("y")? + label["height"].as_f64().context("height")?)
        as f32
        + 20.0;
    for kind in [3, 4] {
        host.send(Packet::new(kind).int(0).float(x).float(y))?;
    }
    key(host, "KeyA", if cfg!(target_os = "macos") { 8 } else { 2 })?;
    if text.is_empty() {
        key(host, "Backspace", 0)
    } else {
        host.send(Packet::new(8).int(0).text(text))
    }
}
fn key(host: &Host, code: &str, modifiers: u8) -> Result<()> {
    for down in [1, 0] {
        host.send(Packet::new(7).int(0).byte(down).byte(modifiers).text(code))?;
    }
    Ok(())
}
