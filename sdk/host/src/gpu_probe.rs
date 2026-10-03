use crate::jvm::{self, A, J, O, Scope};
use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

fn edt<T: Send + 'static>(
    j: &mut J<'_>,
    action: impl Fn(&mut J<'_>) -> Result<T> + Send + Sync + 'static,
) -> Result<T> {
    let scope = Scope::default();
    let (sent, received) = mpsc::sync_channel(1);
    let id = scope.register(move |j, _, _| {
        let result = action(j).map_err(|error| format!("{error:#}"));
        if j.env.exception_check()? {
            j.env.exception_describe()?;
            j.env.exception_clear()?;
        }
        sent.send(result)
            .map_err(|_| anyhow::anyhow!("Probe receiver closed"))?;
        j.null()
    });
    let callback = jvm::callback(j, id)?;
    j.static_void(
        "java/awt/EventQueue",
        "invokeAndWait",
        "(Ljava/lang/Runnable;)V",
        &[A::O(&callback)],
    )?;
    received.recv()?.map_err(anyhow::Error::msg)
}

pub fn entry(j: &mut J<'_>) -> Result<()> {
    let result = run(j);
    if let Err(error) = &result {
        eprintln!("GPU presentation probe failed: {error:#}");
        if j.env.exception_check()? {
            j.env.exception_describe()?;
            j.env.exception_clear()?;
        }
    }
    j.static_void(
        "java/lang/System",
        "exit",
        "(I)V",
        &[A::I(i32::from(result.is_err()))],
    )
}

fn run(j: &mut J<'_>) -> Result<()> {
    let library = std::env::var("CRANPOSE_PROBE_LIBRARY")?;
    j.static_void(
        "java/lang/System",
        "load",
        "(Ljava/lang/String;)V",
        &[A::S(&library)],
    )?;
    let runtime = j.static_obj(
        "java/lang/System",
        "getProperty",
        "(Ljava/lang/String;)Ljava/lang/String;",
        &[A::S("java.runtime.version")],
    )?;
    println!("runtime={}", j.read_string(&runtime)?);
    if std::env::var("CRANPOSE_PROBE_KIND").as_deref() == Ok("metal") {
        return metal(j);
    }
    let scope = Scope::default();
    let clicks = Arc::new(AtomicUsize::new(0));
    let counter = clicks.clone();
    let listener = scope.register(move |j, _, _| {
        counter.fetch_add(1, Ordering::Relaxed);
        j.null()
    });
    let frame = edt(j, move |j| {
        let frame = j.new(
            "javax/swing/JFrame",
            "(Ljava/lang/String;)V",
            &[A::S("Cranpose GPU presentation probe")],
        )?;
        j.void(&frame, "setUndecorated", "(Z)V", &[A::Z(true)])?;
        let content = j.new(
            "javax/swing/JPanel",
            "(Ljava/awt/LayoutManager;)V",
            &[A::Null],
        )?;
        let white = j.new("java/awt/Color", "(I)V", &[A::I(0xffffff)])?;
        j.void(
            &content,
            "setBackground",
            "(Ljava/awt/Color;)V",
            &[A::O(&white)],
        )?;
        let button = j.new(
            "javax/swing/JButton",
            "(Ljava/lang/String;)V",
            &[A::S("Input under the GPU layer")],
        )?;
        j.void(
            &button,
            "setBounds",
            "(IIII)V",
            &[A::I(32), A::I(120), A::I(256), A::I(48)],
        )?;
        let callback = jvm::callback(j, listener)?;
        j.void(
            &button,
            "addActionListener",
            "(Ljava/awt/event/ActionListener;)V",
            &[A::O(&callback)],
        )?;
        j.obj(
            &content,
            "add",
            "(Ljava/awt/Component;)Ljava/awt/Component;",
            &[A::O(&button)],
        )?;
        j.void(
            &frame,
            "setContentPane",
            "(Ljava/awt/Container;)V",
            &[A::O(&content)],
        )?;
        j.void(
            &frame,
            "setBounds",
            "(IIII)V",
            &[A::I(100), A::I(100), A::I(400), A::I(240)],
        )?;
        j.void(&frame, "setVisible", "(Z)V", &[A::Z(true)])?;
        Ok(frame)
    })?;
    let toolkit = j.static_obj(
        "java/awt/Toolkit",
        "getDefaultToolkit",
        "()Ljava/awt/Toolkit;",
        &[],
    )?;
    let class = j.obj(&toolkit, "getClass", "()Ljava/lang/Class;", &[])?;
    let toolkit = j.text(&class, "getName")?;
    println!("toolkit={toolkit}");
    let wayland = toolkit.contains("WLToolkit");
    if let Ok(expected) = std::env::var("CRANPOSE_PROBE_TOOLKIT") {
        ensure!(
            (expected == "wayland" && wayland)
                || (expected == "x11" && toolkit.contains("XToolkit")),
            "Requested {expected} toolkit, but the runtime selected {toolkit}"
        );
    }
    let gc = j.obj(
        &frame,
        "getGraphicsConfiguration",
        "()Ljava/awt/GraphicsConfiguration;",
        &[],
    )?;
    let transform = j.obj(
        &gc,
        "getDefaultTransform",
        "()Ljava/awt/geom/AffineTransform;",
        &[],
    )?;
    let scale = j.call(&transform, "getScaleX", "()D", &[])?.d()?;
    println!("displayScale={scale}");
    let target = frame.clone();
    let layer = edt(j, move |j| {
        let handle = j
            .static_call(
                "NativeLayerProbe",
                "create",
                "(Ljava/awt/Window;Z)J",
                &[A::O(&target), A::Z(wayland)],
            )?
            .j()?;
        ensure!(handle != 0, "Native layer returned no handle");
        let name = j.static_obj(
            "NativeLayerProbe",
            "backend",
            "(J)Ljava/lang/String;",
            &[A::J(handle)],
        )?;
        println!("backend={}", j.read_string(&name)?);
        Ok(handle)
    });
    let result = match layer {
        Ok(layer) => {
            let result = check(j, &frame, layer, scale, wayland, &clicks);
            if j.env.exception_check()? {
                j.env.exception_describe()?;
                j.env.exception_clear()?;
            }
            let disposal = edt(j, move |j| {
                j.static_void("NativeLayerProbe", "destroy", "(J)V", &[A::J(layer)])
            });
            result.and(disposal)
        }
        Err(error) => Err(error),
    };
    if j.env.exception_check()? {
        j.env.exception_describe()?;
        j.env.exception_clear()?;
    }
    let disposal = edt(j, move |j| j.void(&frame, "dispose", "()V", &[]));
    result.and(disposal)
}

fn metal(j: &mut J<'_>) -> Result<()> {
    ensure!(
        j.static_call("com/jetbrains/JBR", "isSharedTexturesSupported", "()Z", &[])?
            .z()?,
        "Shared textures unavailable"
    );
    let service = j.static_obj(
        "com/jetbrains/JBR",
        "getSharedTextures",
        "()Lcom/jetbrains/SharedTextures;",
        &[],
    )?;
    let environment = j.static_obj(
        "java/awt/GraphicsEnvironment",
        "getLocalGraphicsEnvironment",
        "()Ljava/awt/GraphicsEnvironment;",
        &[],
    )?;
    let device = j.obj(
        &environment,
        "getDefaultScreenDevice",
        "()Ljava/awt/GraphicsDevice;",
        &[],
    )?;
    let config = j.obj(
        &device,
        "getDefaultConfiguration",
        "()Ljava/awt/GraphicsConfiguration;",
        &[],
    )?;
    let target = j.obj(
        &config,
        "createCompatibleVolatileImage",
        "(III)Ljava/awt/image/VolatileImage;",
        &[A::I(128), A::I(64), A::I(3)],
    )?;
    let texture = j
        .static_call("SharedTextureProbe", "create", "()J", &[])?
        .j()?;
    ensure!(texture != 0, "Metal rendering failed");
    let image = j.obj(
        &service,
        "wrapTexture",
        "(Ljava/awt/GraphicsConfiguration;J)Ljava/awt/Image;",
        &[A::O(&config), A::J(texture)],
    );
    let result = match &image {
        Ok(image) => (|| -> Result<()> {
            let transform = j.obj(
                &config,
                "getDefaultTransform",
                "()Ljava/awt/geom/AffineTransform;",
                &[],
            )?;
            let scale = j.call(&transform, "getScaleX", "()D", &[])?.d()?;
            ensure!(
                j.call(
                    image,
                    "getWidth",
                    "(Ljava/awt/image/ImageObserver;)I",
                    &[A::Null]
                )?
                .i()?
                    == (64.0 / scale) as i32
                    && j.call(
                        image,
                        "getHeight",
                        "(Ljava/awt/image/ImageObserver;)I",
                        &[A::Null]
                    )?
                    .i()?
                        == (32.0 / scale) as i32,
                "Shared image dimensions"
            );
            let white = j.new("java/awt/Color", "(I)V", &[A::I(0xffffff)])?;
            let src = j.constant(
                "java/awt/AlphaComposite",
                "Src",
                "Ljava/awt/AlphaComposite;",
            )?;
            let over = j.constant(
                "java/awt/AlphaComposite",
                "SrcOver",
                "Ljava/awt/AlphaComposite;",
            )?;
            for y in [12, 4] {
                j.call(
                    &target,
                    "validate",
                    "(Ljava/awt/GraphicsConfiguration;)I",
                    &[A::O(&config)],
                )?;
                let graphics = j.obj(&target, "createGraphics", "()Ljava/awt/Graphics2D;", &[])?;
                let painted = (|| -> Result<()> {
                    j.void(
                        &graphics,
                        "setComposite",
                        "(Ljava/awt/Composite;)V",
                        &[A::O(&src)],
                    )?;
                    j.void(
                        &graphics,
                        "setColor",
                        "(Ljava/awt/Color;)V",
                        &[A::O(&white)],
                    )?;
                    j.void(
                        &graphics,
                        "fillRect",
                        "(IIII)V",
                        &[A::I(0), A::I(0), A::I(128), A::I(64)],
                    )?;
                    j.void(
                        &graphics,
                        "setComposite",
                        "(Ljava/awt/Composite;)V",
                        &[A::O(&over)],
                    )?;
                    j.void(
                        &graphics,
                        "setClip",
                        "(IIII)V",
                        &[A::I(16), A::I(0), A::I(32), A::I(64)],
                    )?;
                    ensure!(
                        j.call(
                            &graphics,
                            "drawImage",
                            "(Ljava/awt/Image;IILjava/awt/image/ImageObserver;)Z",
                            &[A::O(image), A::I(8), A::I(y), A::Null]
                        )?
                        .z()?,
                        "Image was not ready"
                    );
                    Ok(())
                })();
                j.void(&graphics, "dispose", "()V", &[])?;
                painted?;
                let pixels = j.obj(
                    &target,
                    "getSnapshot",
                    "()Ljava/awt/image/BufferedImage;",
                    &[],
                )?;
                let actual = j
                    .call(&pixels, "getRGB", "(II)I", &[A::I(20), A::I(y + 8)])?
                    .i()?;
                ensure!(
                    ((actual >> 16 & 255) - 159).abs() <= 2
                        && ((actual >> 8 & 255) - 191).abs() <= 2
                        && (actual & 255) >= 253,
                    "Shared texture alpha/color: {actual:x}"
                );
                ensure!(
                    j.call(&pixels, "getRGB", "(II)I", &[A::I(12), A::I(y + 8)])?
                        .i()?
                        == -1,
                    "Shared texture clip escaped"
                );
                ensure!(
                    j.call(&pixels, "getRGB", "(II)I", &[A::I(20), A::I(y - 1)])?
                        .i()?
                        == -1,
                    "Shared texture translated bounds escaped"
                );
            }
            let capabilities = j.obj(
                &target,
                "getCapabilities",
                "()Ljava/awt/ImageCapabilities;",
                &[],
            )?;
            ensure!(
                j.bool(&capabilities, "isAccelerated")?,
                "Java2D target is not accelerated"
            );
            println!(
                "PASS: GPU texture import, dimensions, premultiplied alpha, clipping and translated placement"
            );
            println!("acceleratedTarget=true\ndisplayScale={scale}");
            Ok(())
        })(),
        Err(error) => Err(anyhow::anyhow!("{error:#}")),
    };
    if j.env.exception_check()? {
        j.env.exception_describe()?;
        j.env.exception_clear()?;
    }
    j.void(&target, "flush", "()V", &[])?;
    if let Ok(image) = image {
        j.void(&image, "flush", "()V", &[])?;
    }
    j.static_void("SharedTextureProbe", "release", "(J)V", &[A::J(texture)])?;
    result
}

fn check(
    j: &mut J<'_>,
    frame: &O,
    layer: i64,
    scale: f64,
    wayland: bool,
    clicks: &AtomicUsize,
) -> Result<()> {
    let robot = j.new("java/awt/Robot", "()V", &[])?;
    j.void(&robot, "setAutoDelay", "(I)V", &[A::I(40)])?;
    j.void(&robot, "waitForIdle", "()V", &[])?;
    let captures = if wayland {
        Some(
            tempfile::Builder::new()
                .prefix("captures-")
                .tempdir_in(std::env::var("CRANPOSE_PROBE_OUTPUT")?)?
                .keep(),
        )
    } else {
        None
    };
    await_pixel(j, frame, &robot, &captures, 64, 40, [255; 3])?;
    let input = click(j, frame, &robot, wayland, clicks)?;
    ensure!(
        input || wayland && std::env::var_os("CRANPOSE_PROBE_PARENT_DISPLAY").is_none(),
        "Baseline native input unavailable"
    );
    clicks.store(0, Ordering::Relaxed);
    for y in [32, 72, 32] {
        present(j, layer, scale, y, 32)?;
        await_pixel(j, frame, &robot, &captures, 64, y + 8, [159, 191, 255])?;
        await_pixel(j, frame, &robot, &captures, 24, y + 8, [255; 3])?;
        await_pixel(j, frame, &robot, &captures, 64, y - 8, [255; 3])?;
    }
    present(j, layer, scale, 120, 48)?;
    ensure!(
        !input || click(j, frame, &robot, wayland, clicks)?,
        "GPU layer intercepted input"
    );
    println!("PASS: native GPU alpha, translated placement and bounds");
    println!(
        "input={}",
        if input {
            "native click passthrough passed"
        } else {
            "unverified: baseline input unavailable"
        }
    );
    println!("pixelTransport=none (screen readback is test-only)");
    Ok(())
}

fn present(j: &mut J<'_>, layer: i64, scale: f64, y: i32, height: i32) -> Result<()> {
    edt(j, move |j| {
        j.static_void(
            "NativeLayerProbe",
            "present",
            "(JIIII)V",
            &[
                A::J(layer),
                A::I((32.0 * scale) as i32),
                A::I((f64::from(y) * scale) as i32),
                A::I((256.0 * scale) as i32),
                A::I((f64::from(height) * scale) as i32),
            ],
        )
    })
}

fn origin(j: &mut J<'_>, frame: &O) -> Result<[i32; 2]> {
    let point = j.obj(frame, "getLocationOnScreen", "()Ljava/awt/Point;", &[])?;
    Ok([j.field_int(&point, "x")?, j.field_int(&point, "y")?])
}

fn click(j: &mut J<'_>, frame: &O, robot: &O, wayland: bool, clicks: &AtomicUsize) -> Result<bool> {
    let [x, y] = origin(j, frame)?;
    if wayland {
        if !j
            .static_call(
                "NativeLayerProbe",
                "clickWayland",
                "(II)Z",
                &[A::I(x + 160), A::I(y + 144)],
            )?
            .z()?
        {
            return Ok(false);
        }
    } else {
        j.void(robot, "mouseMove", "(II)V", &[A::I(x + 160), A::I(y + 144)])?;
        j.void(robot, "mousePress", "(I)V", &[A::I(1024)])?;
        j.void(robot, "mouseRelease", "(I)V", &[A::I(1024)])?;
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        j.void(robot, "waitForIdle", "()V", &[])?;
        if clicks.load(Ordering::Relaxed) > 0 {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn await_pixel(
    j: &mut J<'_>,
    frame: &O,
    robot: &O,
    captures: &Option<PathBuf>,
    x: i32,
    y: i32,
    expected: [i32; 3],
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let [ox, oy] = origin(j, frame)?;
        let pixel = if let Some(captures) = captures {
            let executable = std::env::var("CRANPOSE_PROBE_CAPTURE").context(
                "Wayland requires a compositor capture; AWT Robot reads only Java's buffer",
            )?;
            let output = std::fs::File::create(captures.join("capture.log"))?;
            let mut child = Command::new(executable)
                .env("XDG_PICTURES_DIR", captures)
                .stdout(Stdio::from(output.try_clone()?))
                .stderr(Stdio::from(output))
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if Instant::now() >= deadline {
                    child.kill()?;
                    child.wait()?;
                    anyhow::bail!("Compositor capture timed out");
                }
                thread::sleep(Duration::from_millis(20));
            };
            ensure!(
                status.success(),
                "Compositor capture failed: {}",
                std::fs::read_to_string(captures.join("capture.log"))?
            );
            let latest = std::fs::read_dir(captures)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "png"))
                .max_by_key(|path| {
                    path.metadata()
                        .and_then(|metadata| metadata.modified())
                        .ok()
                })
                .context("No compositor capture")?;
            let file = j.new(
                "java/io/File",
                "(Ljava/lang/String;)V",
                &[A::S(&latest.to_string_lossy())],
            )?;
            let image = j.static_obj(
                "javax/imageio/ImageIO",
                "read",
                "(Ljava/io/File;)Ljava/awt/image/BufferedImage;",
                &[A::O(&file)],
            )?;
            j.call(&image, "getRGB", "(II)I", &[A::I(ox + x), A::I(oy + y)])?
                .i()?
        } else {
            let color = j.obj(
                robot,
                "getPixelColor",
                "(II)Ljava/awt/Color;",
                &[A::I(ox + x), A::I(oy + y)],
            )?;
            j.int(&color, "getRGB")?
        };
        let actual = [(pixel >> 16) & 255, (pixel >> 8) & 255, pixel & 255];
        if actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() <= 5)
        {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Pixel {x},{y}: {actual:?}, expected {expected:?}"
        );
        thread::sleep(Duration::from_millis(40));
    }
}
