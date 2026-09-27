//! Exercise shader decorations through the production native surface protocol.
use anyhow::{Context, Result, ensure};
use clap::Args;
use cranpose_ide_host::{
    process::Process,
    protocol::{self, Event, Packet},
};
use serde_json::json;
use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
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
}
pub fn run(options: Options) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let token = format!("authoring-{:032x}", rand::random::<u128>());
    let log = fs::File::create(&options.log)?;
    let mut command = Command::new(options.binary.canonicalize()?);
    command
        .env("CRANPOSE_AUTHORING", "overlay")
        .env("CRANPOSE_EMBED_ADDRESS", listener.local_addr()?.to_string())
        .env("CRANPOSE_EMBED_TOKEN", &token)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    let mut child = Process::spawn(command)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stream = loop {
        if let Ok((stream, _)) = listener.accept() {
            break stream;
        }
        ensure!(
            child.try_wait()?.is_none() && Instant::now() < deadline,
            "Overlay did not connect"
        );
        thread::sleep(Duration::from_millis(10));
    };
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    ensure!(
        matches!(protocol::read(&mut stream)?, Some(Event::Hello(2, ref received)) if received == &token),
        "Overlay authentication"
    );
    stream.set_read_timeout(None)?;
    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let (send, receive) = mpsc::sync_channel(32);
    let output = writer.clone();
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            while let Some(event) = protocol::read(&mut stream)? {
                if let Event::Frame(frame) = &event {
                    Packet::new(11)
                        .int(frame.surface)
                        .int(frame.id)
                        .send(&mut *output.lock().expect("socket"))?;
                }
                if send.send(Ok(event)).is_err() {
                    break;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            let _ = send.send(Err(e));
        }
    });
    let send = |p: Packet| p.send(&mut *writer.lock().expect("socket"));
    // An unattached primary must compose once to declare its host overlay.
    send(size(0, 1, 1))?;
    send(Packet::new(13).int(0).byte(1))?;
    let surface = loop {
        let event = receive
            .recv_timeout(Duration::from_secs(10))
            .context("Overlay declaration timed out")??;
        if let Event::Overlay(id, anchor) = event {
            ensure!(anchor == "editor", "Editor anchor");
            break id;
        }
    };
    send(size(surface, 640, 400))?;
    send(Packet::new(13).int(surface).byte(1))?;
    let geometry = json!({"items":[
        {"kind":"value","x":24,"y":24,"width":14,"height":22,"label":"◆"},
        {"kind":"call","x":48,"y":46,"width":120,"height":3},
        {"kind":"badge","x":200,"y":24,"width":100,"height":22,"label":"stable","tone":"stable"}
    ]});
    send(Packet::message(
        "ide.authoring.geometry",
        &geometry.to_string(),
    ))?;
    let mut painted = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(event) = receive.recv_timeout(Duration::from_millis(100))
            && let Event::Frame(frame) = event?
            && frame.surface == surface
        {
            painted = frame
                .pixels
                .iter()
                .filter(|p| (**p as u32 >> 24) != 0)
                .count();
            if painted > 100 {
                break;
            }
        }
    }
    ensure!(
        painted > 100 && painted < 10_000,
        "Editor decorations must paint small accents on a transparent surface: {painted} pixels"
    );
    let mut animated_geometry = geometry.clone();
    animated_geometry["items"]
        .as_array_mut()
        .context("items")?
        .insert(
            0,
            json!({"kind":"arrival","request":1,"x":8,"y":72,"width":580,"height":24}),
        );
    send(Packet::message(
        "ide.authoring.geometry",
        &animated_geometry.to_string(),
    ))?;
    let transition = Instant::now();
    let mut arrival_frames = 0;
    while transition.elapsed() < Duration::from_secs(1) {
        if let Ok(event) = receive.recv_timeout(Duration::from_millis(50))
            && let Event::Frame(frame) = event?
            && frame.surface == surface
        {
            arrival_frames += 1;
        }
    }
    ensure!(
        arrival_frames >= 3,
        "Source arrival shader did not animate: {arrival_frames} frames"
    );
    // Wait for settling, then demand event-driven idle rendering.
    let settle = Instant::now() + Duration::from_secs(2);
    while Instant::now() < settle {
        let _ = receive.recv_timeout(Duration::from_millis(100));
    }
    let cpu = crate::process_metrics::sample(&[child.id()])?[0];
    let start = Instant::now();
    let mut idle_frames = 0;
    while start.elapsed() < Duration::from_secs(3) {
        if let Ok(event) = receive.recv_timeout(Duration::from_millis(100))
            && matches!(event?, Event::Frame(_))
        {
            idle_frames += 1;
        }
    }
    let cpu = crate::process_metrics::sample(&[child.id()])?[0] - cpu;
    ensure!(
        idle_frames == 0,
        "Settled decorations emitted {idle_frames} frames"
    );
    let mut report = json!({"result":"passed","paintedPixels":painted,"arrivalFrames":arrival_frames,"idleFrames":idle_frames,
        "idleSeconds":start.elapsed().as_secs_f64(),"idleCpuSeconds":cpu});
    let _ = writer
        .lock()
        .expect("socket")
        .shutdown(std::net::Shutdown::Both);
    child.terminate(Duration::from_secs(2))?;
    report["controls"] = crate::control_smoke::run(&options.binary, &options.log)?;
    fs::write(options.report, serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}
fn size(id: u32, w: u32, h: u32) -> Packet {
    Packet::new(1).int(id).int(w).int(h).float(1.0).float(60.0)
}
