mod authoring_smoke;
mod bridge;
mod control_smoke;
mod hot_smoke;
mod ide_test;
mod inspection_profile;
mod package;
mod process_metrics;
mod release;
mod ui_probe;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cranpose_jvm_bridge::{BRIDGE, Class, DISPATCH};
use std::{fs, io::Write, path::PathBuf, process::Command};
#[derive(Parser)]
#[command(about = "Build and verify the Rust/Cranpose IntelliJ plugin")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}
#[derive(Subcommand)]
enum Task {
    /// Update UI framework dependencies in this checkout for the nightly compatibility job.
    UseFrameworkMain,
    FetchIde {
        #[arg(long, default_value = "idea")]
        product: String,
        #[arg(long, default_value = "2026.1")]
        version: String,
    },
    IdeTest {
        #[arg(long)]
        ide: PathBuf,
        /// Use an optimized host without JNI checking for timing experiments.
        /// The default suite retains its checked debug host.
        #[arg(long)]
        profile: bool,
    },

    #[command(subcommand)]
    Release(release::Task),
    HotSmoke(hot_smoke::Options),
    /// Profile the Cranpose Studio inspector with repeatable synthetic layouts.
    InspectionProfile(inspection_profile::Options),
    /// Verify native editor shader decorations and settled idle rendering.
    AuthoringSmoke(authoring_smoke::Options),
    BridgeTest {
        java: Option<PathBuf>,
        #[arg(long)]
        ide: Option<PathBuf>,
    },
    Package {
        #[arg(long)]
        native_dir: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        debug: bool,
    },
}
pub fn run_cli(config: BuildConfig) -> Result<()> {
    CONFIG
        .set(config)
        .map_err(|_| anyhow::anyhow!("Build tools were already configured"))?;
    match Cli::parse().command {
        Task::UseFrameworkMain => use_framework_main(),
        Task::FetchIde { product, version } => release::fetch_ide(&product, &version),
        Task::IdeTest { ide, profile } => ide_test::run(&ide, profile),
        Task::Release(task) => release::run(task),
        Task::HotSmoke(options) => hot_smoke::run(options),
        Task::InspectionProfile(options) => inspection_profile::run(options),
        Task::AuthoringSmoke(options) => authoring_smoke::run(options),
        Task::BridgeTest { java, ide } => bridge_test(java, ide),
        Task::Package {
            native_dir,
            output,
            version,
            debug,
        } => {
            let version = version.unwrap_or(release::version()?);
            let path = package::build(native_dir.as_deref(), output.as_deref(), &version, debug)?;
            println!("{}", path.display());
            Ok(())
        }
    }
}
pub fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"))
}
/// Shared Rust build and release tools for Cranpose IntelliJ plugins.
#[derive(Clone, Debug)]
pub struct BuildConfig {
    pub root: PathBuf,
    pub plugin_id: String,
    pub directory: String,
    pub host_package: String,
}
static CONFIG: std::sync::OnceLock<BuildConfig> = std::sync::OnceLock::new();
pub(crate) fn config() -> &'static BuildConfig {
    #[cfg(test)]
    CONFIG.get_or_init(|| BuildConfig {
        root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("sdk")
            .parent()
            .expect("workspace")
            .to_path_buf(),
        plugin_id: "dev.cranpose.toolwindow".into(),
        directory: "cranpose-template".into(),
        host_package: "cranpose-template-host".into(),
    });
    CONFIG
        .get()
        .expect("run_cli must configure the plugin workspace")
}
pub fn root() -> PathBuf {
    config().root.clone()
}
pub fn run(command: &mut Command) -> Result<()> {
    let status = command.status().context("Start build tool")?;
    anyhow::ensure!(status.success(), "Command failed: {command:?} ({status})");
    Ok(())
}
fn bridge_test(java: Option<PathBuf>, ide: Option<PathBuf>) -> Result<()> {
    let root = root();
    run(Command::new("cargo")
        .args(["build", "-p", &config().host_package])
        .current_dir(&root))?;
    let lib = target_dir().join("bridge-test/lib");
    fs::create_dir_all(&lib)?;
    let mut classes = if ide.is_some() {
        bridge::classes()
    } else {
        vec![(BRIDGE.to_string(), cranpose_jvm_bridge::native_bridge(None))]
    };
    classes[0].1 = cranpose_jvm_bridge::native_bridge(None);
    let name = "dev/cranpose/rust/Probe";
    let mut probe = Class::new(name, "java/lang/Object", &[]);
    probe.constructor("()V", "()V", &[], false);
    probe.forward("echo", "(Ljava/lang/String;)Ljava/lang/String;");
    probe.forward("add", "(JJ)J");
    let names = classes
        .iter()
        .map(|(name, _)| name.replace('/', "."))
        .collect::<Vec<_>>();
    probe.method(9, "main", "([Ljava/lang/String;)V", |c| {
        c.text("Probe.main");
        c.null();
        c.null();
        c.invoke(0xb8, BRIDGE, "call", DISPATCH);
        c.op(0x57);
        for name in &names {
            c.text(name);
            c.integer(0);
            c.class(BRIDGE);
            c.invoke(
                0xb6,
                "java/lang/Class",
                "getClassLoader",
                "()Ljava/lang/ClassLoader;",
            );
            c.invoke(
                0xb8,
                "java/lang/Class",
                "forName",
                "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;",
            );
            c.op(0x57);
        }
        c.op(0xb1);
    });
    classes.push((name.into(), probe.finish()));
    let jar = lib.join("cranpose.jar");
    let mut archive = zip::ZipWriter::new(fs::File::create(&jar)?);
    for (name, bytes) in classes {
        archive.start_file(
            format!("{name}.class"),
            zip::write::SimpleFileOptions::default(),
        )?;
        archive.write_all(&bytes)?;
    }
    archive.finish()?;
    let filename = format!(
        "{}cranpose_ide_host{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    for arch in package::arch_aliases(std::env::consts::ARCH) {
        let dest = lib.join("native").join(arch);
        fs::create_dir_all(&dest)?;
        fs::copy(
            target_dir().join("debug").join(&filename),
            dest.join(&filename),
        )?;
    }
    let mut classpath = vec![jar];
    if let Some(ide) = ide {
        let ide = if ide.join("Contents").is_dir() {
            ide.join("Contents")
        } else {
            ide
        };
        for item in fs::read_dir(ide.join("lib"))? {
            let path = item?.path();
            if path.extension().is_some_and(|s| s == "jar") {
                classpath.push(path);
            }
        }
    }
    run(Command::new(java.unwrap_or_else(|| PathBuf::from("java")))
        .args([
            "--enable-native-access=ALL-UNNAMED",
            "-Xcheck:jni",
            "-Xverify:all",
            "-cp",
        ])
        .arg(std::env::join_paths(classpath)?)
        .arg("dev.cranpose.rust.Probe"))?;
    println!("Verified {} generated bridge classes", names.len());
    Ok(())
}

fn use_framework_main() -> Result<()> {
    let path = root().join("ui/Cargo.toml");
    let mut doc = fs::read_to_string(&path)?.parse::<toml_edit::DocumentMut>()?;
    for name in [
        "cranpose",
        "cranpose-core",
        "cranpose-animation",
        "cranpose-ui-graphics",
    ] {
        if let Some(value) = doc["dependencies"].get_mut(name) {
            let mut table = value.as_inline_table().cloned().unwrap_or_default();
            table.remove("version");
            table.remove("rev");
            table.insert("git", "https://github.com/samoylenkodmitry/Cranpose".into());
            table.insert("branch", "main".into());
            *value = toml_edit::value(table);
        }
    }
    fs::write(path, doc.to_string())?;
    run(Command::new("cargo").current_dir(root()).arg("update"))
}
