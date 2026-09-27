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
    let (mut editor_surface, mut window_surface) = (None, None);
    while editor_surface.is_none() || window_surface.is_none() {
        let event = receive
            .recv_timeout(Duration::from_secs(10))
            .context("Overlay declaration timed out")??;
        if let Event::Overlay(id, anchor) = event {
            match anchor.as_str() {
                "editor" => editor_surface = Some(id),
                "window" => window_surface = Some(id),
                _ => anyhow::bail!("Unexpected overlay anchor: {anchor}"),
            }
        }
    }
    let surface = editor_surface.context("Editor anchor")?;
    let lightning_surface = window_surface.context("Window anchor")?;
    send(size(surface, 640, 400))?;
    send(Packet::new(13).int(surface).byte(1))?;
    let geometry = json!({"items":[
        {"kind":"value","x":24,"y":24,"width":14,"height":22,"label":"◆"},
        {"kind":"call","x":48,"y":46,"width":120,"height":3},
        {"kind":"badge","x":200,"y":24,"width":100,"height":22,"label":"stable","tone":"stable"},
        {"kind":"color","anchor":1,"value":"0.1,0.8,0.2,1","x":340,"y":24,"width":14,"height":22},
        {"kind":"color","anchor":2,"value":"0.1,0.8,0.2,0","x":368,"y":24,"width":14,"height":22}
    ]});
    send(Packet::message(
        "ide.authoring.geometry",
        &geometry.to_string(),
    ))?;
    let mut painted = 0;
    let mut capture = crate::ui_probe::FrameCapture::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(event) = receive.recv_timeout(Duration::from_millis(100))
            && let Event::Frame(frame) = event?
            && frame.surface == surface
        {
            let mut sample = frame.clone();
            sample.surface = 0;
            capture.update(&sample);
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
    let opaque = capture.pixel(345, 34).context("opaque inline color")?.0;
    let transparent = capture
        .pixel(373, 34)
        .context("transparent inline color")?
        .0;
    ensure!(
        opaque[1] > opaque[0].saturating_add(70),
        "Inline swatch lost its source color: {opaque:?}"
    );
    ensure!(
        transparent[0].abs_diff(transparent[1]) <= 2
            && transparent[1].abs_diff(transparent[2]) <= 2,
        "Transparent swatch ignored alpha: {transparent:?}"
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
    let mut lightning = vec![];
    for scale in [1u32, 2] {
        send(
            Packet::new(1)
                .int(lightning_surface)
                .int(640 * scale)
                .int(300 * scale)
                .float(scale as f32)
                .float(60.0),
        )?;
        send(Packet::new(13).int(lightning_surface).byte(1))?;
        send(Packet::message("ide.authoring.bolt", &json!({"request":100+scale,"phase":"pending","from":[40,80],"target":[520,170,92,24],"size":[640,300]}).to_string()))?;
        let charging = Instant::now();
        let mut pending_frames = 0;
        let mut pending_capture = crate::ui_probe::FrameCapture::default();
        while charging.elapsed() < Duration::from_secs(10) && pending_frames < 3 {
            if let Ok(event) = receive.recv_timeout(Duration::from_millis(50))
                && let Event::Frame(mut frame) = event?
                && frame.surface == lightning_surface
            {
                frame.surface = 0;
                pending_capture.update(&frame);
                let (painted, target) = pending_capture
                    .alpha_counts(30, [516 * scale, 166 * scale, 616 * scale, 198 * scale]);
                ensure!(
                    target == 0,
                    "Pending effect must not claim the target is ready"
                );
                if painted > 50 {
                    pending_frames += 1;
                }
            }
        }
        ensure!(
            pending_frames >= 3,
            "Charging shader did not animate at {scale}x"
        );
        send(Packet::message("ide.authoring.bolt", "{\"clear\":true}"))?;
        let cancelled = Instant::now();
        let mut cleared = false;
        while cancelled.elapsed() < Duration::from_secs(3) {
            if let Ok(event) = receive.recv_timeout(Duration::from_millis(50))
                && let Event::Frame(mut frame) = event?
                && frame.surface == lightning_surface
            {
                frame.surface = 0;
                pending_capture.update(&frame);
                if pending_capture.alpha_counts(0, [0; 4]).0 == 0 {
                    cleared = true;
                    break;
                }
            }
        }
        ensure!(cleared, "Cancelled charging must clear the overlay");
        send(Packet::message(
            "ide.authoring.bolt",
            &json!({"request":scale,"from":[40,80],"target":[520,170,92,24],"size":[640,300]})
                .to_string(),
        ))?;
        let started = Instant::now();
        let animation_cpu = crate::process_metrics::sample(&[child.id()])?[0];
        let mut frames = 0;
        let mut brightest = 0;
        let mut target_pixels = 0;
        let mut capture = crate::ui_probe::FrameCapture::default();
        let mut evidence = None;
        let mut first_paint = None;
        let mut transparent = None;
        // Software GPU pipeline creation is part of cold delivery, not of the
        // 850 ms animation clock. Bound both total delivery and visible settling.
        while started.elapsed() < Duration::from_secs(10) {
            if let Ok(event) = receive.recv_timeout(Duration::from_millis(50))
                && let Event::Frame(mut frame) = event?
                && frame.surface == lightning_surface
            {
                frames += 1;
                frame.surface = 0;
                capture.update(&frame);
                let (painted, impact) =
                    capture.alpha_counts(30, [516 * scale, 166 * scale, 616 * scale, 198 * scale]);
                if painted > 500 {
                    first_paint.get_or_insert_with(|| started.elapsed());
                }
                if first_paint.is_some() && capture.alpha_counts(0, [0; 4]).0 == 0 {
                    transparent = Some(started.elapsed());
                }
                target_pixels = target_pixels.max(impact);
                if painted > brightest {
                    brightest = painted;
                    evidence = Some(capture.clone());
                }
            }
            if transparent.is_some() && started.elapsed() >= Duration::from_millis(1500) {
                break;
            }
            if first_paint.is_some_and(|first| started.elapsed() - first > Duration::from_secs(3)) {
                break;
            }
        }
        eprintln!(
            "Lightning {scale}x: first paint={first_paint:?}, transparent={transparent:?}, received frames={frames}, observation={:?}",
            started.elapsed()
        );
        ensure!(
            frames >= 3 && brightest > 500 && target_pixels > 100,
            "Lightning must animate across source and target at {scale}x: frames={frames} painted={brightest} target={target_pixels}"
        );
        let alpha_left = capture.alpha_counts(0, [0; 4]).0 > 0;
        ensure!(
            transparent.is_some() && !alpha_left,
            "Lightning must finish transparent at {scale}x"
        );
        let animation_cpu = crate::process_metrics::sample(&[child.id()])?[0] - animation_cpu;
        lightning.push(json!({"scale":scale,"pendingFrames":pending_frames,"cancelledPending":cleared,"frames":frames,"paintedPixels":brightest,"impactPixels":target_pixels,"finishedTransparent":true,"firstPaintMs":first_paint.map(|t|t.as_secs_f64()*1000.0),"transparentMs":transparent.map(|t|t.as_secs_f64()*1000.0),"cpuSeconds":animation_cpu,"observationSeconds":started.elapsed().as_secs_f64()}));
        // PNG encoding must not hold up the frame receiver or its acknowledgments.
        evidence.context("Lightning evidence")?.save(
            &options
                .log
                .with_file_name(format!("lightning-{scale}x.png")),
        )?;
    }
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
    let mut report = json!({"result":"passed","paintedPixels":painted,"inlineColorAlpha":true,"arrivalFrames":arrival_frames,"lightning":lightning,"idleFrames":idle_frames,
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
