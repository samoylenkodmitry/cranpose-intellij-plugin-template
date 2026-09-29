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
    // Popup geometry at 1x: 14px padding, 22px header, 10px gaps, 36px field.
    const FIELD_Y: f32 = 64.0;
    const TRACK_START: f32 = 22.0;
    const TRACK_SPAN: f32 = 336.0;
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
        resize(&host, scale, 340, 124)?;
        host.send(Packet::new(13).int(0).byte(1))?;
        host.send(Packet::message(
            "ide.authoring.control",
            &json!({"literal":{"kind":"string","value":"Original value"}}).to_string(),
        ))?;
        verify_text(
            &host,
            "Applied as you change it. Undo in the editor reverts it.",
        )?;
        let cold_ready_ms = cold_started.elapsed().as_secs_f64() * 1000.0;
        // Typing applies each complete value into one Undo group.
        replace_text(&host, 100.0, FIELD_Y, "Pointer", select_all_modifier)?;
        let first = expect_value(&host, "Pointer")?;
        host.send(Packet::new(8).int(0).text(" edit"))?;
        let second = expect_value(&host, "Pointer edit")?;
        ensure!(
            first["gesture"] == 0 && second["gesture"] == 0,
            "Typing must share one Undo group: {first} {second}"
        );
        let mut typed = Vec::new();
        for (kind, initial, entered, incomplete, hint) in [
            (
                "int",
                "41",
                "42",
                "-",
                "Enter a whole number in range for this type",
            ),
            ("float", "1.5", "0.5", "0.", "Enter a finite number"),
        ] {
            resize(&host, scale, 380, 196)?;
            host.send(Packet::message(
                "ide.authoring.control",
                &json!({"literal":{"kind":kind,"value":initial}}).to_string(),
            ))?;
            // Both numeric kinds share a title; wait for this control's value.
            verify_text(&host, initial)?;
            replace_text(&host, 100.0, FIELD_Y, incomplete, select_all_modifier)?;
            verify_text(&host, hint)?;
            expect_silence(&host)?;
            replace_text(&host, 100.0, FIELD_Y, entered, select_all_modifier)?;
            expect_edit(&host, entered)?;
            typed.push(json!({"kind":kind,"entered":entered,"incompleteHeld":incomplete}));
        }
        resize(&host, scale, 300, 124)?;
        host.send(Packet::message(
            "ide.authoring.control",
            &json!({"literal":{"kind":"bool","value":"true"}}).to_string(),
        ))?;
        let view = verify_text(&host, "false")?;
        click(&host, &view, "false")?;
        let toggled = expect_value(&host, "false")?;
        ensure!(
            toggled.get("gesture").is_none(),
            "Each toggle is its own Undo step"
        );
        typed.push(json!({"kind":"bool","entered":"false"}));
        resize(&host, scale, 380, 196)?;
        host.send(Packet::message(
            "ide.authoring.control",
            &json!({"literal":{"kind":"int","value":"41"}}).to_string(),
        ))?;
        let view = verify_text(&host, "step")?;
        // Range fields apply as soon as they form a valid range.
        for (current, value) in [("0", "-20"), ("100", "40"), ("1", "5")] {
            let node = text_node(&view, current).context("range field")?;
            let (x, y) = center(node);
            replace_text(&host, x, y, value, select_all_modifier)?;
        }
        expect_silence(&host)?;
        let step = text_node(&verify_text(&host, "step")?, "step")
            .context("step label")?
            .clone();
        let (_, row) = center(&step);
        let y = row - 13.0 - 4.0 - 14.0;
        thread::sleep(Duration::from_millis(200));
        capture
            .lock()
            .expect("capture")
            .save(&output.join(format!("authoring-controls-number-{scale}.png")))?;
        host.send(Packet::new(3).int(0).float(TRACK_START).float(y))?;
        let first = expect_value(&host, "-20")?;
        for fraction in [0.5f32, 1.0] {
            host.send(
                Packet::new(2)
                    .int(0)
                    .float(TRACK_START + TRACK_SPAN * fraction)
                    .float(y),
            )?;
            let changed = expect_value(&host, if fraction == 0.5 { "10" } else { "40" })?;
            ensure!(
                changed["gesture"] == first["gesture"] && first["gesture"] != 0,
                "One drag must retain its own Undo group"
            );
        }
        host.send(
            Packet::new(4)
                .int(0)
                .float(TRACK_START + TRACK_SPAN)
                .float(y),
        )?;
        resize(&host, scale, 380, 330)?;
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
        let track = |view: &Value, label: &str| -> Result<f32> {
            let node = text_node(view, label).context("color track label")?;
            Ok(node["y"].as_f64().context("y")? as f32
                + node["height"].as_f64().context("height")? as f32
                + 2.0
                + 14.0)
        };
        let y = track(&view, "Hue")?;
        let x = TRACK_START + TRACK_SPAN * 0.5;
        host.send(Packet::new(3).int(0).float(x).float(y))?;
        expect_edit(&host, "0.0,1.0,1.0,1.0")?;
        host.send(Packet::new(4).int(0).float(x).float(y))?;
        expect_edit(&host, "0.0,1.0,1.0,1.0")?;
        // The hex label is only a display representation. Opening or changing
        // opacity must not quantize the source RGB.
        let precise = "0.19,0.42,0.31,0.123456789";
        host.send(Packet::message(
            "ide.authoring.control",
            &json!({"literal":{"kind":"color","value":precise}}).to_string(),
        ))?;
        let view = verify_text(&host, "#306B4F1F")?;
        expect_silence(&host)?;
        for (label, fraction, expected) in [
            ("Opacity", 0.5, "0.19,0.42,0.31,0.5"),
            ("Hue", 0.75, "0.305,0.19,0.42,0.5"),
            ("Opacity", 0.25, "0.305,0.19,0.42,0.25"),
        ] {
            let y = track(&view, label)?;
            let x = TRACK_START + TRACK_SPAN * fraction;
            for kind in [3, 4] {
                host.send(Packet::new(kind).int(0).float(x).float(y))?;
                expect_edit(&host, expected)?;
            }
        }
        replace_text(&host, 200.0, FIELD_Y, "#12", select_all_modifier)?;
        verify_text(&host, "Enter #RRGGBB or #RRGGBBAA")?;
        expect_silence(&host)?;
        replace_text(&host, 200.0, FIELD_Y, "#12345678", select_all_modifier)?;
        expect_edit(&host, "0.070588,0.203922,0.337255,0.470588")?;
        // Exclude inspector requests and active text-caret blinking from the
        // settled observation. Typing retains the field's keyboard focus.
        host.send(Packet::new(13).int(0).byte(0))?;
        thread::sleep(Duration::from_secs(2));
        let before = host.frames.load(std::sync::atomic::Ordering::Relaxed);
        thread::sleep(Duration::from_secs(2));
        let idle = host.frames.load(std::sync::atomic::Ordering::Relaxed) - before;
        ensure!(idle == 0, "Settled color controls rendered {idle} frames");
        let mut warm_ready_ms = Vec::new();
        for request in 1..=5u64 {
            resize(&host, scale, 380, 196)?;
            let started = Instant::now();
            host.send(Packet::new(13).int(0).byte(1))?;
            host.send(Packet::message("ide.authoring.control", &json!({"request":request,"binding":format!("spacing{request}"),"literal":{"kind":"int","value":"41","range":{"line":7}}}).to_string()))?;
            let view = verify_text(&host, &format!("spacing{request}"))?;
            warm_ready_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            ensure!(
                text_node(&view, "Number · L7").is_some()
                    && text_node(&view, "100").is_some()
                    && text_node(&view, "40").is_none(),
                "Reused control retained an old custom range"
            );
            replace_text(&host, 100.0, FIELD_Y, "42", select_all_modifier)?;
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
            json!({"scale":scale,"liveTyping":true,"typedControls":typed,"customRangeDrag":true,"colorDrag":true,"colorPrecision":true,"settledColorFrames":idle,"coldProcessToControlCompositionMs":cold_ready_ms,"warmControlCompositionMs":warm_ready_ms,"reusedSessionIsolation":true}),
        );
    }
    Ok(json!({"pointerControls":checks}))
}

/// Focus a field, select its text and type a replacement.
fn replace_text(host: &Host, x: f32, y: f32, text: &str, select_all: u8) -> Result<()> {
    pointer(host, x, y)?;
    key(host, "KeyA", select_all)?;
    host.send(Packet::new(8).int(0).text(text))
}
/// Incomplete input and freshly opened controls must leave source untouched.
fn expect_silence(host: &Host) -> Result<()> {
    let deadline = Instant::now() + Duration::from_millis(400);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if let Ok(event) = host.messages.recv_timeout(left) {
            let (channel, payload, _) = event.context("Control connection")?;
            ensure!(
                channel != "ide.authoring.edit",
                "Unexpected source edit {payload}"
            );
        }
    }
    Ok(())
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
    anyhow::bail!("Control did not submit {expected:?}")
}
