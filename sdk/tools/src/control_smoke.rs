//! Pointer regression for the live-value popup over the native surface protocol.
use crate::{
    hot_smoke::Host,
    ui_probe::{center, click, text_node, verify_text},
};
use anyhow::{Context, Result, ensure};
use cranpose_ide_host::{process::Process, protocol::Packet};
use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(crate) fn run(binary: &Path, log: &Path) -> Result<Value> {
    let mut checks = Vec::new();
    let select_all_modifier = if cfg!(target_os = "macos") { 8 } else { 2 };
    for scale in [1, 2] {
        let output = log.parent().unwrap_or(Path::new("."));
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let token = format!("control-{:032x}", rand::random::<u128>());
        let log = fs::OpenOptions::new().create(true).append(true).open(log)?;
        let mut command = Command::new(binary.canonicalize()?);
        command
            .env("CRANPOSE_AUTHORING", "value")
            .env("CRANPOSE_EMBED_ADDRESS", listener.local_addr()?.to_string())
            .env("CRANPOSE_EMBED_TOKEN", &token)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        let cold_started = Instant::now();
        let mut child = Process::spawn(command)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let stream = loop {
            if let Ok((stream, _)) = listener.accept() {
                break stream;
            }
            ensure!(
                child.try_wait()?.is_none() && Instant::now() < deadline,
                "Control connection timed out"
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
                .int(340 * scale)
                .int(240 * scale)
                .float(scale as f32)
                .float(60.0),
        )?;
        host.send(Packet::new(13).int(0).byte(1))?;
        host.send(Packet::message(
            "ide.authoring.control",
            &json!({"literal":{"kind":"string","value":"Original value"}}).to_string(),
        ))?;
        verify_text(&host, "Apply")?;
        let cold_ready_ms = cold_started.elapsed().as_secs_f64() * 1000.0;
        pointer(&host, 70.0, 66.0)?;
        key(&host, "KeyA", select_all_modifier)?;
        host.send(Packet::new(8).int(0).text("Pointer edit"))?;
        // Selecting text opens the floating selection menu, which used to cover
        // the action row. Test the actual hit targets, not just their bounds.
        key(&host, "KeyA", select_all_modifier)?;
        let menu = verify_text(&host, "Copy")?;
        click(&host, &menu, "Apply")?;
        expect_edit(&host, "Pointer edit")?;
        // The first host acknowledgment changes the footer text and can wrap it
        // on another platform's fonts. Wait for that layout before targeting Reset.
        verify_text(&host, "Source updated · Undo in the editor")?;
        let menu = verify_text(&host, "Copy")?;
        click(&host, &menu, "Reset")?;
        if let Err(error) = expect_edit(&host, "Original value") {
            capture
                .lock()
                .expect("capture")
                .save(&output.join(format!("authoring-controls-reset-failure-{scale}.png")))?;
            return Err(error);
        }
        let mut typed = Vec::new();
        for (kind, initial, action, changed) in [
            ("int", "41", "+", "42"),
            ("float", "1.5", "−", "0.5"),
            ("bool", "true", "Toggle", "false"),
        ] {
            resize(
                &host,
                scale,
                if kind == "bool" { 340 } else { 400 },
                if kind == "bool" { 240 } else { 380 },
            )?;
            host.send(Packet::message(
                "ide.authoring.control",
                &json!({"literal":{"kind":kind,"value":initial}}).to_string(),
            ))?;
            verify_text(&host, kind)?;
            pointer(&host, 70.0, 66.0)?;
            key(&host, "KeyA", select_all_modifier)?;
            let menu = verify_text(&host, "Copy")?;
            click(&host, &menu, action)?;
            expect_edit(&host, changed)?;
            key(&host, "KeyA", select_all_modifier)?;
            let menu = verify_text(&host, "Copy")?;
            click(&host, &menu, "Reset")?;
            expect_edit(&host, initial)?;
            typed.push(json!({"kind":kind,"action":action,"reset":true}));
        }
        resize(&host, scale, 400, 380)?;
        host.send(Packet::message(
            "ide.authoring.control",
            &json!({"literal":{"kind":"int","value":"41"}}).to_string(),
        ))?;
        let view = verify_text(&host, "Set range")?;
        for (label, value) in [("Min", "-20"), ("Max", "40"), ("Step", "5")] {
            let node = text_node(&view, label).context("range field label")?;
            let (x, _) = center(node);
            pointer(
                &host,
                x,
                node["y"].as_f64().context("label y")? as f32
                    + node["height"].as_f64().context("height")? as f32
                    + 14.0,
            )?;
            key(&host, "KeyA", select_all_modifier)?;
            host.send(Packet::new(8).int(0).text(value))?;
        }
        let view = verify_text(&host, "Set range")?;
        click(&host, &view, "Set range")?;
        let view = verify_text(&host, "-20 → 40")?;
        thread::sleep(Duration::from_millis(200));
        capture
            .lock()
            .expect("capture")
            .save(&output.join(format!("authoring-controls-number-{scale}.png")))?;
        let label = text_node(&view, "SEEK TO PREVIEW").context("seek label")?;
        let y = label["y"].as_f64().context("y")? as f32
            + label["height"].as_f64().context("height")? as f32
            + 6.0
            + 14.0;
        host.send(Packet::new(3).int(0).float(24.0).float(y))?;
        let first = expect_value(&host, "-20")?;
        for fraction in [0.5f32, 1.0] {
            host.send(
                Packet::new(2)
                    .int(0)
                    .float(24.0 + 352.0 * fraction)
                    .float(y),
            )?;
            let changed = expect_value(&host, if fraction == 0.5 { "10" } else { "40" })?;
            ensure!(
                changed["gesture"] == first["gesture"],
                "One drag must retain its Undo group"
            );
        }
        host.send(Packet::new(4).int(0).float(376.0).float(y))?;
        resize(&host, scale, 400, 490)?;
        host.send(Packet::message(
            "ide.authoring.control",
            &json!({"literal":{"kind":"color","value":"1,0,0,1"}}).to_string(),
        ))?;
        let view = verify_text(&host, "Hue")?;
        thread::sleep(Duration::from_millis(200));
        capture
            .lock()
            .expect("capture")
            .save(&output.join(format!("authoring-controls-color-{scale}.png")))?;
        let label = text_node(&view, "Hue").context("hue label")?;
        let y = label["y"].as_f64().context("y")? as f32
            + label["height"].as_f64().context("height")? as f32
            + 4.0
            + 14.0;
        host.send(Packet::new(3).int(0).float(200.0).float(y))?;
        expect_edit(&host, "0.0,1.0,1.0,1.0")?;
        host.send(Packet::new(4).int(0).float(200.0).float(y))?;
        expect_edit(&host, "0.0,1.0,1.0,1.0")?;
        let view = verify_text(&host, "Reset")?;
        click(&host, &view, "Reset")?;
        expect_edit(&host, "1,0,0,1")?;
        // The hex label is only a display representation. Opening, applying,
        // resetting or changing opacity must not quantize the source RGB.
        let precise = "0.19,0.42,0.31,0.123456789";
        host.send(Packet::message(
            "ide.authoring.control",
            &json!({"literal":{"kind":"color","value":precise}}).to_string(),
        ))?;
        let view = verify_text(&host, "#306B4F1F")?;
        click(&host, &view, "Apply")?;
        expect_edit(&host, precise)?;
        for (label, fraction, expected) in [
            ("Opacity", 0.5, "0.19,0.42,0.31,0.5"),
            ("Hue", 0.75, "0.305,0.19,0.42,0.5"),
            ("Opacity", 0.25, "0.305,0.19,0.42,0.25"),
        ] {
            let view = verify_text(&host, label)?;
            let node = text_node(&view, label).context("color track label")?;
            let y = node["y"].as_f64().context("y")? as f32
                + node["height"].as_f64().context("height")? as f32
                + 18.0;
            let x = 24.0 + 352.0 * fraction;
            for kind in [3, 4] {
                host.send(Packet::new(kind).int(0).float(x).float(y))?;
                expect_edit(&host, expected)?;
            }
            let view = verify_text(&host, "Apply")?;
            click(&host, &view, "Apply")?;
            expect_edit(&host, expected)?;
        }
        let view = verify_text(&host, "Reset")?;
        click(&host, &view, "Reset")?;
        expect_edit(&host, precise)?;
        click(&host, &view, "Apply")?;
        expect_edit(&host, precise)?;
        pointer(&host, 70.0, 66.0)?;
        key(&host, "KeyA", select_all_modifier)?;
        host.send(Packet::new(8).int(0).text("#12"))?;
        let view = verify_text(&host, "Apply")?;
        click(&host, &view, "Apply")?;
        let view = verify_text(&host, "Enter #RRGGBB or #RRGGBBAA")?;
        click(&host, &view, "Reset")?;
        expect_edit(&host, precise)?;
        pointer(&host, 70.0, 66.0)?;
        key(&host, "KeyA", select_all_modifier)?;
        host.send(Packet::new(8).int(0).text("#12345678"))?;
        let view = verify_text(&host, "Apply")?;
        click(&host, &view, "Apply")?;
        expect_edit(&host, "0.070588,0.203922,0.337255,0.470588")?;
        // Exclude inspector requests and active text-caret blinking from the
        // settled observation. Apply can retain the field's keyboard focus.
        host.send(Packet::new(13).int(0).byte(0))?;
        thread::sleep(Duration::from_secs(2));
        let before = host.frames.load(std::sync::atomic::Ordering::Relaxed);
        thread::sleep(Duration::from_secs(2));
        let idle = host.frames.load(std::sync::atomic::Ordering::Relaxed) - before;
        ensure!(idle == 0, "Settled color controls rendered {idle} frames");
        let mut warm_ready_ms = Vec::new();
        for request in 1..=5u64 {
            resize(&host, scale, 400, 380)?;
            let started = Instant::now();
            host.send(Packet::new(13).int(0).byte(1))?;
            host.send(Packet::message("ide.authoring.control", &json!({"request":request,"binding":format!("spacing{request}"),"literal":{"kind":"int","value":"41","range":{"line":7}}}).to_string()))?;
            let view = verify_text(&host, &format!("spacing{request} · L7"))?;
            warm_ready_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            ensure!(
                text_node(&view, "0 → 100").is_some(),
                "Reused control retained an old custom range"
            );
            click(&host, &view, "+")?;
            let edit = expect_value(&host, "42")?;
            ensure!(
                edit["request"] == request,
                "Reused control sent stale session identity"
            );
            host.send(Packet::new(13).int(0).byte(0))?;
        }
        child.terminate(Duration::from_secs(2))?;
        ensure!(
            child.wait_for_tree_exit(Duration::from_secs(2))?,
            "Control descendants survived shutdown"
        );
        checks.push(
            json!({"scale":scale,"applyWithSelectionMenu":true,"resetWithSelectionMenu":true,"typedControls":typed,"customRangeDrag":true,"colorDrag":true,"colorPrecision":true,"settledColorFrames":idle,"coldProcessToControlCompositionMs":cold_ready_ms,"warmControlCompositionMs":warm_ready_ms,"reusedSessionIsolation":true}),
        );
    }
    Ok(json!({"pointerControls":checks}))
}

fn pointer(host: &Host, x: f32, y: f32) -> Result<()> {
    for kind in [3, 4] {
        host.send(Packet::new(kind).int(0).float(x).float(y))?;
    }
    Ok(())
}
fn key(host: &Host, code: &str, modifiers: u8) -> Result<()> {
    for down in [1, 0] {
        host.send(Packet::new(7).int(0).byte(down).byte(modifiers).text(code))?;
    }
    Ok(())
}
fn expect_edit(host: &Host, expected: &str) -> Result<()> {
    expect_value(host, expected).map(|_| ())
}
fn resize(host: &Host, scale: u32, width: u32, height: u32) -> Result<()> {
    host.send(
        Packet::new(1)
            .int(0)
            .int(width * scale)
            .int(height * scale)
            .float(scale as f32)
            .float(60.0),
    )
}
fn expect_value(host: &Host, expected: &str) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let Ok(event) = host.messages.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        let (channel, payload, _) = event.context("Control connection")?;
        if channel == "ide.authoring.edit" {
            ensure!(
                payload["value"] == expected,
                "Expected {expected:?}, got {payload}"
            );
            host.send(Packet::message(
                "ide.authoring.result",
                &json!({"ok":true,"request":payload["request"]}).to_string(),
            ))?;
            return Ok(payload);
        }
    }
    anyhow::bail!("Pointer control did not submit {expected:?} while selection menu was open")
}
