//! Exercise exactly the generator used by New Project, without a Rust toolchain.
use anyhow::{Result, ensure};
use clap::Args;
use cranpose_ide_host::starter;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Instant};

#[derive(Args)]
pub struct Options {
    /// Empty project directory. Existing source files are never overwritten.
    #[arg(long)]
    output: PathBuf,
}

pub fn run(options: Options) -> Result<()> {
    let cache = tempfile::tempdir()?;
    let started = Instant::now();
    starter::generate(&options.output, cache.path(), || false)?;
    let elapsed = started.elapsed();
    let manifest = fs::read_to_string(options.output.join("Cargo.toml"))?;
    ensure!(
        manifest.contains("cranpose = \"0.1.169\""),
        "Pinned framework"
    );
    ensure!(
        options.output.join("src/main.rs").is_file(),
        "Application entry"
    );
    ensure!(options.output.join("LICENSE").is_file(), "Upstream license");
    ensure!(
        !options.output.join(".git").exists(),
        "No upstream Git metadata"
    );
    ensure!(
        fs::read_dir(cache.path())?.next().is_none(),
        "Stage removed"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "passed": true,
            "repository": starter::SHOWCASE_REPOSITORY,
            "revision": starter::SHOWCASE_REVISION,
            "output": options.output,
            "generation_ms": elapsed.as_secs_f64() * 1000.0,
            "manifest_sha256": format!("{:x}", Sha256::digest(manifest.as_bytes())),
            "empty_path": std::env::var_os("PATH").is_none_or(|path| path.is_empty()),
        }))?
    );
    Ok(())
}
