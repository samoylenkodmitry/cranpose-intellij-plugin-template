//! Starts a separate headless IDE and runs the native integration suite inside its plugin loader.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
pub fn run(ide: &Path, profile: bool) -> Result<()> {
    let ide = ide.canonicalize()?;
    let home = if ide.join("Contents").is_dir() {
        ide.join("Contents")
    } else {
        ide.clone()
    };
    let info_path = if home.join("Resources/product-info.json").is_file() {
        home.join("Resources/product-info.json")
    } else {
        home.join("product-info.json")
    };
    let info: serde_json::Value = serde_json::from_str(&fs::read_to_string(&info_path)?)?;
    let launch = info["launch"]
        .as_array()
        .context("IDE launch metadata")?
        .iter()
        .find(|v| {
            v["arch"]
                .as_str()
                .is_none_or(|s| crate::package::arch_aliases(std::env::consts::ARCH).contains(&s))
        })
        .context("Host IDE architecture")?;
    let sandbox = crate::target_dir().join("ide-tests");
    fs::create_dir_all(&sandbox)?;
    let run = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&sandbox)?
        .keep();
    crate::package::build(
        None,
        Some(&run.join("plugins")),
        &crate::release::version()?,
        false,
    )?;
    let mut build = Command::new("cargo");
    build.current_dir(crate::root()).args([
        "build",
        "--locked",
        "-p",
        &crate::config().host_package,
        "--features",
        "ide-tests",
    ]);
    if profile {
        build.arg("--release");
    }
    crate::run(&mut build)?;
    let lib = run
        .join("plugins")
        .join(&crate::config().directory)
        .join("lib");
    let library = crate::package::library(&crate::package::host_platform());
    for arch in crate::package::arch_aliases(std::env::consts::ARCH) {
        fs::copy(
            crate::target_dir()
                .join(if profile { "release" } else { "debug" })
                .join(library),
            lib.join("native").join(arch).join(library),
        )?;
    }
    let jar = lib.join("cranpose.jar");
    let mut input = zip::ZipArchive::new(fs::File::open(&jar)?)?;
    let mut output = zip::ZipWriter::new(fs::File::create(lib.join("test.jar"))?);
    for index in 0..input.len() {
        let mut entry = input.by_index(index)?;
        let name = entry.name().to_owned();
        let mut bytes = vec![];
        entry.read_to_end(&mut bytes)?;
        if name == "META-INF/plugin.xml" {
            bytes=String::from_utf8(bytes)?.replace("<extensions defaultExtensionNs=\"com.intellij\">","<extensions defaultExtensionNs=\"com.intellij\"><appStarter id=\"cranpose-self-test\" implementation=\"dev.cranpose.rust.SelfTest\"/>").into_bytes();
        }
        output.start_file(name, zip::write::SimpleFileOptions::default())?;
        output.write_all(&bytes)?;
    }
    let mut class = cranpose_jvm_bridge::Class::new(
        "dev/cranpose/rust/SelfTest",
        "java/lang/Object",
        &["com/intellij/openapi/application/ApplicationStarter"],
    );
    class.constructor("()V", "()V", &[], false);
    for (name, sig) in [
        ("main", "(Ljava/util/List;)V"),
        ("isHeadless", "()Z"),
        ("getRequiredModality", "()I"),
    ] {
        class.forward(name, sig);
    }
    output.start_file(
        "dev/cranpose/rust/SelfTest.class",
        zip::write::SimpleFileOptions::default(),
    )?;
    output.write_all(&class.finish())?;
    output.finish()?;
    drop(input);
    fs::rename(lib.join("test.jar"), jar)?;
    let mut command = Command::new(
        info_path.parent().context("metadata directory")?.join(
            launch["javaExecutablePath"]
                .as_str()
                .context("Bundled Java")?,
        ),
    );
    command.current_dir(&run);
    for arg in launch["additionalJvmArguments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
    {
        command.arg(
            arg.replace("$APP_PACKAGE/Contents", &home.to_string_lossy())
                .replace("$APP_PACKAGE", &ide.to_string_lossy())
                .replace("$IDE_HOME", &home.to_string_lossy()),
        );
    }
    let classpath = launch["bootClassPathJarNames"]
        .as_array()
        .context("boot classpath")?
        .iter()
        .filter_map(|v| v.as_str())
        .map(|p| home.join("lib").join(p));
    command.args([
        "-Xmx2g",
        "-Xverify:all",
        "-Djava.awt.headless=true",
        "-Didea.is.internal=true",
        "-Didea.initially.ask.config=false",
        "-Didea.auto.reload.plugins=false",
        "-Dide.show.tips.on.startup.default.value=false",
        "-Dsplash=false",
    ]);
    if !profile {
        command.arg("-Xcheck:jni");
    }
    command.arg(format!(
        "-Didea.required.plugins.id={}",
        crate::config().plugin_id
    ));
    command.arg(format!("-Didea.home.path={}", home.display()));
    for (key, path) in [
        ("idea.config.path", "config"),
        ("idea.system.path", "system"),
        ("idea.plugins.path", "plugins"),
        ("idea.log.path", "log"),
        ("cranpose.test.output", "evidence"),
    ] {
        command.arg(format!("-D{key}={}", run.join(path).display()));
    }
    command.arg(format!(
        "-Dcranpose.test.workspace={}",
        crate::root().display()
    ));
    command
        .arg("-cp")
        .arg(std::env::join_paths(classpath)?)
        .args(["com.intellij.idea.Main", "cranpose-self-test"]);
    let log = fs::File::create(run.join("console.log"))?;
    command.stdout(log.try_clone()?).stderr(log);

    let mut child = cranpose_ide_host::process::Process::spawn(command)?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.terminate(Duration::ZERO);
            anyhow::bail!("IDE integration suite timed out: {}", run.display());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    println!("IDE integration evidence: {}", run.display());
    ensure!(
        status.success(),
        "IDE integration suite failed ({status}); see {}/console.log",
        run.display()
    );
    let report = run.join("evidence/results.json");
    let report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(report).context("IDE did not complete the native test suite")?,
    )?;
    ensure!(report["passed"] == true, "IDE integration failed: {report}");
    println!("{report}");
    Ok(())
}
