use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use sha2::{Digest, Sha512};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Subcommand)]
pub enum Task {
    Metal {
        #[arg(long)]
        ide: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    FetchRuntime {
        #[arg(long)]
        output: PathBuf,
    },
    Native {
        #[arg(long)]
        java_home: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, value_parser = ["x11", "wayland"])]
        toolkit: Option<String>,
        #[arg(long)]
        isolated: bool,
        #[arg(long)]
        weston_root: Option<PathBuf>,
    },
}

pub fn run(task: Task) -> Result<()> {
    match task {
        Task::Metal { ide, output } => metal(&ide, &output),
        Task::FetchRuntime { output } => fetch_runtime(&output),
        Task::Native {
            java_home,
            output,
            toolkit,
            isolated,
            weston_root,
        } => native(
            &java_home,
            &output,
            toolkit.as_deref(),
            isolated,
            weston_root.as_deref(),
        ),
    }
}

fn metal(ide: &Path, output: &Path) -> Result<()> {
    ensure!(cfg!(target_os = "macos"), "Metal probe requires macOS");
    let ide = if ide.join("Contents").is_dir() {
        ide.join("Contents")
    } else {
        ide.to_path_buf()
    };
    let java_home = ide.join("jbr/Contents/Home");
    fs::create_dir_all(output)?;
    let output = output.canonicalize()?;
    let library = output.join("libshared_texture_probe.dylib");
    crate::run(
        Command::new("clang")
            .args([
                "-fobjc-arc",
                "-dynamiclib",
                "-framework",
                "Metal",
                "-framework",
                "Foundation",
            ])
            .arg("-I")
            .arg(java_home.join("include"))
            .arg("-I")
            .arg(java_home.join("include/darwin"))
            .arg(crate::root().join("sdk/tools/probes/SharedTextureProbe.m"))
            .arg("-o")
            .arg(&library),
    )?;
    let mut command = crate::probe_command(Some(java_home.join("bin/java")), Some(ide), true, &[])?;
    command
        .env("CRANPOSE_PROBE_LIBRARY", library)
        .env("CRANPOSE_PROBE_KIND", "metal");
    execute(&mut command, &output, &java_home, Some("metal"))
}

fn hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha512::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn fetch_runtime(output: &Path) -> Result<()> {
    let platform = match std::env::consts::OS {
        "macos" => "osx",
        "windows" => "windows",
        "linux" => "linux",
        other => anyhow::bail!("Unsupported platform {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x64",
        other => anyhow::bail!("Unsupported architecture {other}"),
    };
    let name = format!("jbrsdk-25.0.4.1-{platform}-{arch}-b623.69.tar.gz");
    let url = format!("https://cache-redirector.jetbrains.com/intellij-jbr/{name}");
    let client = crate::release::client()?;
    let checksum = client
        .get(format!("{url}.checksum"))
        .send()?
        .error_for_status()?
        .text()?;
    let checksum = checksum
        .split_whitespace()
        .next()
        .context("Missing runtime checksum")?;
    let archive = crate::release::download(&url, &name)?;
    ensure!(
        hash(&archive)? == checksum,
        "JetBrains SDK checksum mismatch"
    );
    fs::create_dir_all(output)?;
    crate::run(
        Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(output),
    )?;
    let mut runtime = output.join(name.trim_end_matches(".tar.gz"));
    if platform == "osx" {
        runtime = runtime.join("Contents/Home");
    }
    let runtime = runtime.canonicalize()?;
    ensure!(
        runtime.join("bin").join(java()).is_file(),
        "Extracted runtime missing java"
    );
    println!("{}", runtime.display());
    if let Some(env) = std::env::var_os("GITHUB_ENV") {
        writeln!(
            fs::OpenOptions::new().append(true).open(env)?,
            "CRANPOSE_PROBE_JAVA_HOME={}",
            runtime.display()
        )?;
    }
    Ok(())
}

fn java() -> &'static str {
    if cfg!(windows) { "java.exe" } else { "java" }
}

struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(command: &mut Command, log: &Path) -> Result<Owned> {
    let log = fs::File::create(log)?;
    Ok(Owned(
        command
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()?,
    ))
}

fn display(output: &Path) -> Result<(Owned, String)> {
    let mut process = Owned(
        Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "800x600x24",
                "-nolisten",
                "tcp",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::from(fs::File::create(output.join("xvfb.log"))?))
            .spawn()?,
    );
    let stdout = process.0.stdout.take().context("Xvfb stdout")?;
    let (sent, received) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        use std::io::BufRead;
        let mut line = String::new();
        let result = std::io::BufReader::new(stdout)
            .read_line(&mut line)
            .map(|_| line);
        let _ = sent.send(result);
    });
    let number = received.recv_timeout(Duration::from_secs(5))??;
    ensure!(
        !number.trim().is_empty() && number.trim().bytes().all(|byte| byte.is_ascii_digit()),
        "Invalid Xvfb display: {number}"
    );
    Ok((process, format!(":{}", number.trim())))
}

fn native(
    java_home: &Path,
    output: &Path,
    toolkit: Option<&str>,
    isolated: bool,
    weston_root: Option<&Path>,
) -> Result<()> {
    fs::create_dir_all(output)?;
    let output = compiler_path(output)?;
    let java_home = compiler_path(java_home)?;
    let source = crate::root().join("sdk/tools/probes/NativeLayerProbe.cpp");
    let library = output.join(format!(
        "{}native_layer_probe{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    if cfg!(windows) {
        crate::run(
            Command::new("cl")
                .args(["/nologo", "/std:c++17", "/EHsc", "/LD"])
                .arg(format!("/I{}", java_home.join("include").display()))
                .arg(format!("/I{}", java_home.join("include/win32").display()))
                .arg(&source)
                .arg(format!(
                    "/Fo{}",
                    output.join("native_layer_probe.obj").display()
                ))
                .arg("/link")
                .arg(format!("/OUT:{}", library.display()))
                .arg(format!("/LIBPATH:{}", java_home.join("lib").display()))
                .args(["jawt.lib", "d3d11.lib", "dxgi.lib", "dcomp.lib"]),
        )?;
    } else {
        ensure!(
            cfg!(target_os = "linux"),
            "Native layer probe currently supports Windows and Linux; Metal texture probe is separate"
        );
        let flags = Command::new("pkg-config")
            .args([
                "--cflags",
                "--libs",
                "x11",
                "xtst",
                "xfixes",
                "xrender",
                "egl",
                "glesv2",
                "wayland-client",
                "wayland-egl",
            ])
            .output()?;
        ensure!(
            flags.status.success(),
            "Native probe dependencies: {}",
            String::from_utf8_lossy(&flags.stderr)
        );
        crate::run(
            Command::new("c++")
                .args([
                    "-std=c++17",
                    "-shared",
                    "-fPIC",
                    "-Wall",
                    "-Wextra",
                    "-Wno-unused-parameter",
                ])
                .arg(format!("-I{}", java_home.join("include").display()))
                .arg(format!("-I{}", java_home.join("include/linux").display()))
                .arg(&source)
                .arg(format!("-L{}", java_home.join("lib").display()))
                .arg(format!("-Wl,-rpath,{}", java_home.join("lib").display()))
                .arg("-ljawt")
                .args(String::from_utf8(flags.stdout)?.split_whitespace())
                .arg("-o")
                .arg(&library),
        )?;
    }
    let vm_options = match toolkit {
        Some("wayland") => vec!["-Dawt.toolkit.name=WLToolkit"],
        Some("x11") => vec!["-Dawt.toolkit.name=XToolkit"],
        _ => Vec::new(),
    };
    let mut command = crate::probe_command(
        Some(java_home.join("bin").join(java())),
        None,
        true,
        &vm_options,
    )?;
    command
        .env("CRANPOSE_PROBE_LIBRARY", &library)
        .env("CRANPOSE_PROBE_OUTPUT", &output);
    if let Some(toolkit) = toolkit {
        command.env("CRANPOSE_PROBE_TOOLKIT", toolkit);
    }
    let mut children = Vec::new();
    if isolated {
        let (server, name) = display(&output)?;
        children.push(server);
        if toolkit == Some("wayland") {
            let weston = weston_root
                .context("Isolated Wayland requires --weston-root")?
                .canonicalize()?;
            let runtime = output.join("runtime");
            fs::create_dir_all(&runtime)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
            }
            let library_path =
                std::env::join_paths([weston.join("usr/lib"), weston.join("usr/lib/weston")])?;
            let modules = format!(
                "x11-backend.so={0}/usr/lib/libweston-15/x11-backend.so;gl-renderer.so={0}/usr/lib/libweston-15/gl-renderer.so;kiosk-shell.so={0}/usr/lib/weston/kiosk-shell.so",
                weston.display()
            );
            let mut compositor = Command::new(weston.join("usr/bin/weston"));
            compositor
                .args([
                    "--backend=x11",
                    "--renderer=gl",
                    "--shell=kiosk-shell.so",
                    "--no-config",
                    "--socket=cranpose-gpu-probe",
                    "--idle-time=0",
                    "--width=800",
                    "--height=600",
                    "--debug",
                ])
                .env("DISPLAY", &name)
                .env("XDG_RUNTIME_DIR", &runtime)
                .env("LD_LIBRARY_PATH", &library_path)
                .env("WESTON_MODULE_MAP", modules);
            children.push(spawn(&mut compositor, &output.join("weston.log"))?);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !runtime.join("cranpose-gpu-probe").exists() {
                ensure!(
                    Instant::now() < deadline,
                    "Wayland compositor failed: {}",
                    fs::read_to_string(output.join("weston.log"))?
                );
                thread::sleep(Duration::from_millis(50));
            }
            command
                .env("DISPLAY", &name)
                .env("XDG_RUNTIME_DIR", runtime)
                .env("WAYLAND_DISPLAY", "cranpose-gpu-probe")
                .env("LD_LIBRARY_PATH", library_path)
                .env("CRANPOSE_PROBE_PARENT_DISPLAY", name)
                .env(
                    "CRANPOSE_PROBE_CAPTURE",
                    weston.join("usr/bin/weston-screenshooter"),
                );
        } else {
            children.push(spawn(
                Command::new("picom")
                    .args(["--backend", "xrender", "--config", "/dev/null"])
                    .env("DISPLAY", &name),
                &output.join("compositor.log"),
            )?);
            thread::sleep(Duration::from_millis(300));
            command.env("DISPLAY", name);
        }
    }
    let result = execute(&mut command, &output, &java_home, toolkit);
    while let Some(child) = children.pop() {
        drop(child);
    }
    result
}

// MSVC and the Java launcher do not consistently accept Win32 verbatim paths.
fn compiler_path(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    #[cfg(windows)]
    {
        let path = path.to_string_lossy();
        if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{unc}")));
        }
        return Ok(PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(&path)));
    }
    #[cfg(not(windows))]
    Ok(path)
}

fn execute(
    command: &mut Command,
    output: &Path,
    java_home: &Path,
    toolkit: Option<&str>,
) -> Result<()> {
    let mut process = spawn(command, &output.join("probe.log"))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = process.0.try_wait()? {
            break status;
        }
        ensure!(
            Instant::now() < deadline,
            "GPU probe timed out: {}",
            output.join("probe.log").display()
        );
        thread::sleep(Duration::from_millis(50));
    };
    let log = fs::read_to_string(output.join("probe.log"))?;
    print!("{log}");
    fs::write(
        output.join("native-layer.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"platform":std::env::consts::OS,"javaHome":java_home,"toolkit":toolkit,"result":log,"passed":status.success()}),
        )?,
    )?;
    ensure!(status.success(), "GPU presentation probe failed");
    Ok(())
}
