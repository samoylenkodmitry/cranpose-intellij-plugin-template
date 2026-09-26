use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
use zip::{ZipWriter, write::SimpleFileOptions};
pub const PLATFORMS: &[&str] = &[
    "macos-aarch64",
    "macos-x86_64",
    "linux-aarch64",
    "linux-x86_64",
    "windows-aarch64",
    "windows-x86_64",
];
pub fn arch_aliases(arch: &str) -> &'static [&'static str] {
    match arch {
        "aarch64" => &["aarch64", "arm64"],
        _ => &["amd64", "x86_64", "x64"],
    }
}
pub fn host_platform() -> String {
    format!(
        "{}-{}",
        if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(windows) {
            "windows"
        } else {
            "linux"
        },
        std::env::consts::ARCH
    )
}
pub fn library(platform: &str) -> &'static str {
    if platform.starts_with("macos") {
        "libcranpose_ide_host.dylib"
    } else if platform.starts_with("windows") {
        "cranpose_ide_host.dll"
    } else {
        "libcranpose_ide_host.so"
    }
}
pub fn executable(platform: &str) -> &'static str {
    if platform.starts_with("windows") {
        "cranpose-intellij-ui.exe"
    } else {
        "cranpose-intellij-ui"
    }
}
pub fn build(
    native_dir: Option<&Path>,
    output: Option<&Path>,
    version: &str,
    debug: bool,
) -> Result<PathBuf> {
    semver::Version::parse(version).context("Invalid plugin version")?;
    let root = crate::root();
    let output = output
        .map(Path::to_owned)
        .unwrap_or_else(|| crate::target_dir().join("plugin"));
    fs::create_dir_all(&output)?;
    let destination = output.join(&crate::config().directory);
    ensure!(
        !destination.exists() || destination.join(".cranpose-build").is_file(),
        "Refusing to replace a directory not created by xtask: {}",
        destination.display()
    );
    let staging = tempfile::Builder::new()
        .prefix(".cranpose-build-")
        .tempdir_in(&output)?;
    let directory = staging.path().join(&crate::config().directory);
    fs::create_dir_all(directory.join("lib"))?;
    let mut xml = fs::read_to_string(root.join("plugin/src/main/resources/META-INF/plugin.xml"))?;
    let root_start = xml
        .find("<idea-plugin")
        .context("Missing plugin descriptor root")?;
    let root_end = root_start
        + xml[root_start..]
            .find('>')
            .context("Invalid plugin descriptor root")?
        + 1;
    xml.insert_str(
        root_end,
        &format!("\n    <version>{version}</version>\n    <idea-version since-build=\"261\"/>"),
    );
    let mut jar = ZipWriter::new(fs::File::create(directory.join("lib/cranpose.jar"))?);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for (name, bytes) in crate::bridge::classes() {
        jar.start_file(format!("{name}.class"), options)?;
        jar.write_all(&bytes)?;
    }
    jar.start_file("META-INF/plugin.xml", options)?;
    jar.write_all(xml.as_bytes())?;
    let resources = root.join("plugin/src/main/resources");
    add_directory(&mut jar, &resources.join("icons"), &resources)?;
    jar.finish()?;
    let native = if let Some(native) = native_dir {
        native.to_owned()
    } else {
        let mut command = Command::new("cargo");
        command.current_dir(&root).args([
            "build",
            "--locked",
            "--no-default-features",
            "-p",
            "cranpose-intellij-ui",
            "-p",
            &crate::config().host_package,
        ]);
        if !debug {
            command.arg("--release");
        }
        crate::run(&mut command)?;
        let native = crate::target_dir().join("native-local");
        let platform = host_platform();
        let target = native.join(&platform);
        fs::create_dir_all(&target)?;
        let build = crate::target_dir().join(if debug { "debug" } else { "release" });
        for name in [executable(&platform), library(&platform)] {
            fs::copy(build.join(name), target.join(name))?;
        }
        native
    };
    let platforms = if native_dir.is_some() {
        PLATFORMS.iter().map(|s| s.to_string()).collect()
    } else {
        vec![host_platform()]
    };
    for platform in platforms {
        let source = if native.join(&platform).is_dir() {
            native.join(&platform)
        } else {
            native.join(format!("native-{platform}"))
        };
        let destination = directory.join("lib/native").join(&platform);
        fs::create_dir_all(&destination)?;
        let ui = source.join(executable(&platform));
        ensure!(
            ui.metadata().is_ok_and(|m| m.len() > 0),
            "Missing native UI: {}",
            ui.display()
        );
        fs::copy(&ui, destination.join(executable(&platform)))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                destination.join(executable(&platform)),
                fs::Permissions::from_mode(0o755),
            )?;
        }
        let host = source.join(library(&platform));
        ensure!(
            host.metadata().is_ok_and(|m| m.len() > 0),
            "Missing native host: {}",
            host.display()
        );
        let arch = platform.split_once('-').context("platform")?.1;
        for alias in arch_aliases(arch) {
            let destination = directory.join("lib/native").join(alias);
            fs::create_dir_all(&destination)?;
            fs::copy(&host, destination.join(library(&platform)))?;
        }
    }
    let zip = output.join(format!("{}-{version}.zip", crate::config().directory));
    let mut archive = ZipWriter::new(fs::File::create(&zip)?);
    add_directory(&mut archive, &directory, staging.path())?;
    archive.finish()?;
    fs::write(directory.join(".cranpose-build"), version)?;
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    fs::rename(&directory, &destination)?;
    Ok(zip)
}
fn add_directory(zip: &mut ZipWriter<fs::File>, directory: &Path, base: &Path) -> Result<()> {
    let mut paths = fs::read_dir(directory)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    for path in paths {
        if path.is_dir() {
            add_directory(zip, &path, base)?;
        } else {
            let name = path
                .strip_prefix(base)?
                .to_string_lossy()
                .replace('\\', "/");
            let executable = path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("cranpose-intellij-ui"));
            zip.start_file(
                name,
                SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(if executable { 0o755 } else { 0o644 }),
            )?;
            let mut file = fs::File::open(path)?;
            std::io::copy(&mut file, zip)?;
        }
    }
    Ok(())
}
