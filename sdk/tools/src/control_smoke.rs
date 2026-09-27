//! Pointer regression for the live-value popup over the native surface protocol.
use crate::{
    hot_smoke::Host,
    ui_probe::{click, verify_text},
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
        let host = Host::new(stream, &token)?;
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
        pointer(&host, 70.0, 66.0)?;
        key(&host, "KeyA", select_all_modifier)?;
        host.send(Packet::new(8).int(0).text("Pointer edit"))?;
        // Selecting text opens the floating selection menu, which used to cover
        // the action row. Test the actual hit targets, not just their bounds.
        key(&host, "KeyA", select_all_modifier)?;
        let menu = verify_text(&host, "Copy")?;
        click(&host, &menu, "Apply")?;
        expect_edit(&host, "Pointer edit")?;
        let menu = verify_text(&host, "Copy")?;
        click(&host, &menu, "Reset")?;
        expect_edit(&host, "Original value")?;
        let mut typed = Vec::new();
        for (kind, initial, action, changed) in [
            ("int", "41", "+", "42"),
            ("float", "1.5", "−", "0.5"),
            ("bool", "true", "Toggle", "false"),
        ] {
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
        child.terminate(Duration::from_secs(2))?;
        ensure!(
            child.wait_for_tree_exit(Duration::from_secs(2))?,
            "Control descendants survived shutdown"
        );
        checks.push(
            json!({"scale":scale,"applyWithSelectionMenu":true,"resetWithSelectionMenu":true,"typedControls":typed}),
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
            host.send(Packet::message("ide.authoring.result", r#"{"ok":true}"#))?;
            return Ok(());
        }
    }
    anyhow::bail!("Pointer control did not submit {expected:?} while selection menu was open")
}
