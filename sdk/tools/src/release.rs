//! Release checks and vendor tools. No credentials are printed in command errors.
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
#[derive(Subcommand)]
pub enum Task {
    ValidateTag {
        tag: String,
    },
    CheckZip {
        archive: PathBuf,
        #[arg(long)]
        host_only: bool,
    },
    AuditSource,
    SuccessfulBuild,
    Verify {
        archive: PathBuf,
        #[arg(long, required = true)]
        ide: Vec<String>,
        #[arg(long)]
        verifier: Option<PathBuf>,
        #[arg(long)]
        java: Option<PathBuf>,
    },
    Sign {
        archive: PathBuf,
        output: PathBuf,
        #[arg(long)]
        signer: Option<PathBuf>,
        #[arg(long)]
        java: Option<PathBuf>,
    },
    /// Attach the signed, verified package to its matching GitHub version tag.
    GithubRelease {
        archive: PathBuf,
        tag: String,
    },
    Publish {
        archive: PathBuf,
        #[arg(long)]
        plugin_id: u64,
        #[arg(long, default_value = "")]
        channel: String,
    },
}
pub fn version() -> Result<String> {
    let version = fs::read_to_string(crate::root().join("plugin/VERSION"))?
        .trim()
        .to_owned();
    semver::Version::parse(&version)?;
    Ok(version)
}
pub fn run(task: Task) -> Result<()> {
    match task {
        Task::ValidateTag { tag } => {
            validate_tag(&tag, &version()?)?;
            println!("{}", version()?);
        }
        Task::CheckZip { archive, host_only } => check_zip(&archive, host_only)?,
        Task::AuditSource => audit()?,
        Task::SuccessfulBuild => {
            let commit = output(Command::new("git").args(["rev-parse", "HEAD"]))?;
            let data = output(Command::new("gh").args([
                "run",
                "list",
                "--workflow",
                "build.yml",
                "--commit",
                commit.trim(),
                "--status",
                "success",
                "--json",
                "databaseId,headSha",
            ]))?;
            let runs: serde_json::Value = serde_json::from_str(&data)?;
            let run = runs
                .as_array()
                .context("Build runs")?
                .iter()
                .find(|r| r["headSha"].as_str() == Some(commit.trim()))
                .context("No successful Build for this exact commit")?;
            let id = run["databaseId"].as_u64().context("Build ID")?;
            if let Some(path) = std::env::var_os("GITHUB_OUTPUT") {
                writeln!(
                    fs::OpenOptions::new().append(true).open(path)?,
                    "run_id={id}"
                )?;
            }
            println!("Verified Build {id} for {}", commit.trim());
        }
        Task::Verify {
            archive,
            ide,
            verifier,
            java,
        } => {
            let jar = match verifier {
                Some(path) => path,
                None => download(
                    "https://packages.jetbrains.team/maven/p/intellij-plugin-verifier/intellij-plugin-verifier/org/jetbrains/intellij/plugins/verifier-cli/1.410/verifier-cli-1.410-all.jar",
                    "verifier-1.410.jar",
                )?,
            };
            let reports = crate::target_dir().join("verification");
            fs::create_dir_all(&reports)?;
            let run = tempfile::Builder::new()
                .prefix("run-")
                .tempdir_in(&reports)?
                .keep();
            crate::run(
                Command::new(java.unwrap_or_else(|| "java".into()))
                    .arg("-jar")
                    .arg(jar)
                    .arg("check-plugin")
                    .arg(&archive)
                    .args(&ide)
                    .arg("-verification-reports-dir")
                    .arg(&run),
            )?;
            let mut verdicts = 0;
            inspect_reports(&run, &mut verdicts)?;
            ensure!(
                verdicts == ide.len(),
                "Missing verifier result: got {verdicts} for {} IDEs",
                ide.len()
            );
            println!("Verified: {}", run.display());
        }
        Task::Sign {
            archive,
            output,
            signer,
            java,
        } => {
            check_zip(&archive, false)?;
            let secrets = tempfile::tempdir()?;
            let cert = secrets.path().join("chain.crt");
            let key = secrets.path().join("key.pem");
            fs::write(
                &cert,
                std::env::var("CERTIFICATE_CHAIN").context("CERTIFICATE_CHAIN")?,
            )?;
            fs::write(&key, std::env::var("PRIVATE_KEY").context("PRIVATE_KEY")?)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&key, fs::Permissions::from_mode(0o600))?;
            }
            let signer = match signer {
                Some(path) => path,
                None => download(
                    "https://github.com/JetBrains/marketplace-zip-signer/releases/download/0.1.43/marketplace-zip-signer-cli-0.1.43.jar",
                    "marketplace-zip-signer-0.1.43.jar",
                )?,
            };
            let mut command = Command::new(java.unwrap_or_else(|| "java".into()));
            command
                .arg("-jar")
                .arg(signer)
                .arg("sign")
                .arg("-in")
                .arg(&archive)
                .arg("-out")
                .arg(&output)
                .arg("-cert-file")
                .arg(&cert)
                .arg("-key-file")
                .arg(key);
            if let Ok(password) = std::env::var("PRIVATE_KEY_PASSWORD") {
                command.arg("-key-pass").arg(password);
            }
            let status = command.status().context("Start JetBrains ZIP signer")?;
            ensure!(status.success(), "JetBrains ZIP signer failed ({status})");
        }
        Task::GithubRelease { archive, tag } => {
            validate_tag(&tag, &version()?)?;
            check_zip(&archive, false)?;
            let stage = tempfile::tempdir()?;
            let asset =
                stage
                    .path()
                    .join(format!("{}-{}.zip", crate::config().directory, version()?));
            fs::copy(&archive, &asset)?;
            let exists = Command::new("gh")
                .args(["release", "view", &tag])
                .output()
                .context("Read GitHub release")?
                .status
                .success();
            if !exists {
                let notes = crate::root().join("plugin/RELEASE_NOTES.md");
                ensure!(notes.is_file(), "Missing plugin/RELEASE_NOTES.md");
                output(
                    Command::new("gh")
                        .args([
                            "release",
                            "create",
                            &tag,
                            "--verify-tag",
                            "--title",
                            &tag,
                            "--notes-file",
                        ])
                        .arg(notes),
                )?;
            }
            output(
                Command::new("gh")
                    .args(["release", "upload", &tag, "--clobber"])
                    .arg(&asset),
            )?;
            println!(
                "Attached signed {} package to GitHub release {tag}",
                version()?
            );
        }
        Task::Publish {
            archive,
            plugin_id,
            channel,
        } => {
            check_zip(&archive, false)?;
            let form = reqwest::blocking::multipart::Form::new()
                .text("pluginId", plugin_id.to_string())
                .text("channel", channel)
                .file("file", archive)?;
            let response = client()?
                .post("https://plugins.jetbrains.com/api/updates/upload")
                .bearer_auth(std::env::var("PUBLISH_TOKEN").context("PUBLISH_TOKEN")?)
                .multipart(form)
                .send()?;
            ensure!(
                response.status().is_success(),
                "Marketplace upload failed ({})",
                response.status()
            );
            println!("Uploaded plugin {plugin_id} for Marketplace review");
        }
    }
    Ok(())
}
fn inspect_reports(path: &Path, verdicts: &mut usize) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            inspect_reports(&path, verdicts)?;
        } else {
            let name = path.file_name().context("report name")?.to_string_lossy();
            if name == "verification-verdict.txt" {
                let verdict = fs::read_to_string(&path)?;
                ensure!(
                    verdict.contains("Compatible"),
                    "Plugin verification failed: {verdict}"
                );
                *verdicts += 1;
            }
            if [
                "compatibility-problems.txt",
                "invalid-plugin.txt",
                "plugin-structure-warnings.txt",
                "missing-dependencies.txt",
            ]
            .contains(&name.as_ref())
            {
                let text = fs::read_to_string(&path)?;
                ensure!(text.trim().is_empty(), "{}: {text}", path.display());
            }
        }
    }
    Ok(())
}
pub fn check_zip(path: &Path, host_only: bool) -> Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
    for platform in crate::package::PLATFORMS
        .iter()
        .filter(|p| !host_only || **p == crate::package::host_platform())
    {
        let ui = format!(
            "{}/lib/native/{platform}/{}",
            crate::config().directory,
            crate::package::executable(platform)
        );
        ensure!(zip.by_name(&ui)?.size() > 0, "Empty native UI: {platform}");
        let arch = platform.split_once('-').context("architecture")?.1;
        for alias in crate::package::arch_aliases(arch) {
            let lib = format!(
                "{}/lib/native/{alias}/{}",
                crate::config().directory,
                crate::package::library(platform)
            );
            ensure!(
                zip.by_name(&lib)?.size() > 0,
                "Empty native host: {platform}/{alias}"
            );
        }
    }
    let mut bytes = vec![];
    zip.by_name(&format!("{}/lib/cranpose.jar", crate::config().directory))?
        .read_to_end(&mut bytes)?;
    let mut jar = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut xml = String::new();
    jar.by_name("META-INF/plugin.xml")?
        .read_to_string(&mut xml)?;
    validate_metadata(&xml, &crate::config().plugin_id, &version()?)?;
    ensure!(
        xml.contains("require-restart=\"true\""),
        "Native host updates require an IDE restart"
    );
    for (name, _) in crate::bridge::classes() {
        ensure!(
            jar.by_name(&format!("{name}.class"))?.size() > 0,
            "Missing {name}"
        );
    }
    ensure!(
        !jar.file_names()
            .any(|p| p.starts_with("kotlin/") || p.ends_with(".kotlin_module")),
        "Authored Kotlin remains in package"
    );
    Ok(())
}
fn validate_tag(tag: &str, version: &str) -> Result<()> {
    semver::Version::parse(version)?;
    ensure!(
        tag == format!("v{version}"),
        "Release tag must match plugin/VERSION"
    );
    Ok(())
}
fn validate_metadata(xml: &str, id: &str, version: &str) -> Result<()> {
    ensure!(
        xml.contains(&format!("<id>{id}</id>")),
        "Unexpected plugin ID"
    );
    ensure!(
        xml.contains(&format!("<version>{version}</version>")),
        "Archive version must match plugin/VERSION"
    );
    Ok(())
}
pub fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("cranpose-rust-build")
        .timeout(Duration::from_secs(600))
        .build()?)
}
pub fn download(url: &str, name: &str) -> Result<PathBuf> {
    let cache = crate::target_dir().join("vendor-tools");
    fs::create_dir_all(&cache)?;
    let dest = cache.join(name);
    if dest.metadata().is_ok_and(|m| m.len() > 0) {
        return Ok(dest);
    }
    let mut temp = tempfile::NamedTempFile::new_in(&cache)?;
    client()?
        .get(url)
        .send()?
        .error_for_status()?
        .copy_to(temp.as_file_mut())?;
    temp.persist(&dest)?;
    Ok(dest)
}
fn output(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    ensure!(
        output.status.success(),
        "Command failed ({})",
        output.status
    );
    Ok(String::from_utf8(output.stdout)?)
}
fn audit() -> Result<()> {
    let files = output(Command::new("git").current_dir(crate::root()).args([
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
    ]))?;
    let prohibited: Vec<_> = files
        .lines()
        .filter(|p| {
            Path::new(p).extension().is_some_and(|e| {
                ["kt", "kts", "java", "py", "js", "ts", "sh", "bat"]
                    .iter()
                    .any(|x| e == *x)
            })
        })
        .collect();
    ensure!(
        prohibited.is_empty(),
        "Non-Rust authored code remains: {}",
        prohibited.join(", ")
    );
    println!("All authored plugin, analyzer integration, build and CI tools are Rust");
    Ok(())
}

pub fn fetch_ide(product: &str, version: &str) -> Result<()> {
    ensure!(
        ["idea", "RustRover"].contains(&product)
            && version.chars().all(|c| c.is_ascii_digit() || c == '.'),
        "Unsupported IDE product/version"
    );
    let name = format!("{product}-{version}");
    let url = format!(
        "https://download.jetbrains.com/{}/{}.tar.gz",
        if product == "idea" {
            "idea"
        } else {
            "rustrover"
        },
        name
    );
    use sha2::{Digest, Sha256};
    let archive = download(&url, &format!("{name}.tar.gz"))?;
    let expected = client()?
        .get(format!("{url}.sha256"))
        .send()?
        .error_for_status()?
        .text()?;
    let expected = expected.split_whitespace().next().context("IDE checksum")?;
    let mut file = fs::File::open(&archive)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == expected,
        "IDE download checksum mismatch"
    );
    let ide = crate::target_dir().join("vendor-tools").join(&name);
    if !ide.join("product-info.json").is_file() {
        fs::create_dir_all(&ide)?;
        crate::run(
            Command::new("tar")
                .arg("-xzf")
                .arg(&archive)
                .arg("-C")
                .arg(&ide)
                .arg("--strip-components=1"),
        )?;
    }
    let ide = ide.canonicalize()?;
    if let Some(path) = std::env::var_os("GITHUB_ENV") {
        writeln!(
            fs::OpenOptions::new().append(true).open(path)?,
            "CRANPOSE_TEST_IDE={}",
            ide.display()
        )?;
    }
    println!("{}", ide.display());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_tag_must_match_exact_version() {
        assert!(validate_tag("v0.12.0", "0.12.0").is_ok());
        for tag in ["0.12.0", "v0.11.2", "v0.12.0-extra", "main"] {
            assert!(validate_tag(tag, "0.12.0").is_err());
        }
    }
    #[test]
    fn archive_cannot_publish_stale_version_or_other_plugin() {
        let xml = "<idea-plugin><id>dev.test</id><version>0.12.0</version></idea-plugin>";
        assert!(validate_metadata(xml, "dev.test", "0.12.0").is_ok());
        assert!(validate_metadata(xml, "dev.other", "0.12.0").is_err());
        assert!(validate_metadata(xml, "dev.test", "0.11.2").is_err());
        assert!(validate_metadata("<id>dev.test</id>", "dev.test", "0.12.0").is_err());
    }
    #[test]
    fn source_version_is_semantic() {
        semver::Version::parse(&version().expect("version")).expect("semver");
    }
    #[test]
    fn empty_archive_cannot_be_published() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("plugin.zip");
        zip::ZipWriter::new(fs::File::create(&path).expect("zip"))
            .finish()
            .expect("finish");
        assert!(check_zip(&path, false).is_err());
    }
    #[test]
    fn incompatible_verdict_fails() {
        let dir = tempfile::tempdir().expect("temp");
        fs::write(dir.path().join("verification-verdict.txt"), "Incompatible").expect("write");
        assert!(inspect_reports(dir.path(), &mut 0).is_err());
    }
}
