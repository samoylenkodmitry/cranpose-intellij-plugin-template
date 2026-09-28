//! Development-only Cargo overrides. Never used by packaging or the runtime.
use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const REPOSITORY: &str = "https://github.com/samoylenkodmitry/Cranpose";

#[derive(Args)]
pub(crate) struct Options {
    /// Use an existing framework checkout instead of cloning main.
    #[arg(long)]
    checkout: Option<PathBuf>,
    /// Save the tested revision and resolved framework packages as JSON.
    #[arg(long)]
    report: Option<PathBuf>,
}

pub(crate) fn run(root: &Path, options: Options) -> Result<()> {
    // Keep generated sources outside Cargo's target directory: CI caches prune it.
    let owned = if options.checkout.is_none() {
        Some(
            tempfile::Builder::new()
                .prefix("cranpose-main-")
                .tempdir()?,
        )
    } else {
        None
    };
    let checkout = match &owned {
        Some(dir) => {
            let path = dir.path().join("framework");
            output(
                Command::new("git")
                    .args(["clone", "--depth", "1", "--branch", "main", REPOSITORY])
                    .arg(&path),
            )?;
            path
        }
        None => root.join(options.checkout.context("Missing framework checkout")?),
    }
    .canonicalize()?;
    let revision = output(
        Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["rev-parse", "HEAD"]),
    )?;
    let report = apply(root, &checkout, REPOSITORY)?;
    // Paths must remain available to later cargo test/IDE invocations.
    if let Some(dir) = owned {
        println!("Compatibility sources retained at {}", dir.keep().display());
    }
    println!("Cranpose compatibility revision: {}", revision.trim());
    println!(
        "Verified {} framework packages from {}",
        report.len(),
        checkout.display()
    );
    if let Some(path) = options.report {
        fs::write(
            root.join(path),
            serde_json::to_vec_pretty(&json!({
                "revision": revision.trim(), "checkout": checkout, "packages": report,
            }))?,
        )?;
    }
    Ok(())
}

fn output(command: &mut Command) -> Result<String> {
    let result = command
        .output()
        .context("Start framework compatibility tool")?;
    ensure!(
        result.status.success(),
        "Compatibility command failed ({command:?}):\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(String::from_utf8(result.stdout)?)
}

fn metadata(root: &Path, no_deps: bool) -> Result<Value> {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .args(["metadata", "--format-version", "1"]);
    if no_deps {
        command.arg("--no-deps");
    } else {
        command.arg("--all-features");
    }
    Ok(serde_json::from_str(&output(&mut command)?)?)
}

fn framework_name(name: &str) -> bool {
    name == "coroflow" || name == "cranpose" || name.starts_with("cranpose-")
}

fn packages(value: &Value) -> Result<&Vec<Value>> {
    value["packages"]
        .as_array()
        .context("Cargo metadata has no package list")
}

fn catalog(checkout: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut result = BTreeMap::new();
    for package in packages(&metadata(checkout, true)?)? {
        let name = package["name"].as_str().context("Missing package name")?;
        if !framework_name(name) {
            continue;
        }
        let manifest = PathBuf::from(
            package["manifest_path"]
                .as_str()
                .context("Missing manifest path")?,
        )
        .canonicalize()?;
        ensure!(
            manifest.starts_with(checkout),
            "Framework package {name} is outside the selected checkout"
        );
        result.insert(name.to_owned(), manifest);
    }
    ensure!(
        !result.is_empty(),
        "No Cranpose framework packages in {}",
        checkout.display()
    );
    Ok(result)
}

fn patched_manifest(
    original: &str,
    source: &str,
    crates: &BTreeMap<String, PathBuf>,
) -> Result<String> {
    let mut document = original.parse::<toml_edit::DocumentMut>()?;
    // A workspace-root patch also reaches pinned Git SDKs, aliases, and target /
    // optional dependencies. Registry consumers must use the same local types.
    for repository in [source, "crates-io"] {
        for (name, manifest) in crates {
            let path = manifest
                .parent()
                .context("Framework manifest parent")?
                .to_str()
                .context("Non-UTF-8 framework path")?;
            if let Some(existing) = document
                .get("patch")
                .and_then(|p| p.get(repository))
                .and_then(|p| p.get(name))
            {
                ensure!(
                    existing.get("path").and_then(toml_edit::Item::as_str) == Some(path),
                    "Existing patch for {repository}/{name}; use a clean compatibility checkout"
                );
            }
            let mut table = toml_edit::InlineTable::new();
            table.insert("path", path.into());
            document["patch"][repository][name] = toml_edit::value(table);
        }
    }
    Ok(document.to_string())
}

fn same_repository(source: &str, repository: &str) -> bool {
    let source = source
        .strip_prefix("git+")
        .unwrap_or(source)
        .split(['?', '#'])
        .next()
        .unwrap_or(source);
    source.trim_end_matches('/').trim_end_matches(".git")
        == repository.trim_end_matches('/').trim_end_matches(".git")
}

fn verify(
    value: &Value,
    repository: &str,
    crates: &BTreeMap<String, PathBuf>,
) -> Result<Vec<Value>> {
    let mut seen = BTreeMap::new();
    let mut report = Vec::new();
    for package in packages(value)? {
        let name = package["name"]
            .as_str()
            .context("Missing resolved package name")?;
        let source = package["source"].as_str();
        // Also catch a package removed from main that still resolves from an old revision.
        ensure!(
            !source.is_some_and(|s| same_repository(s, repository)),
            "Mixed framework graph: {name} still resolves from {}. Check version constraints or removed packages before running compatibility tests.",
            source.unwrap_or_default()
        );
        if let Some(expected) = crates.get(name) {
            let manifest = PathBuf::from(
                package["manifest_path"]
                    .as_str()
                    .context("Missing resolved manifest path")?,
            )
            .canonicalize()?;
            ensure!(
                source.is_none() && &manifest == expected,
                "Mixed framework graph: {name} resolved outside the selected checkout. Check version constraints before running compatibility tests."
            );
            ensure!(
                seen.insert(name, ()).is_none(),
                "Multiple resolved versions of framework package {name}"
            );
            report.push(
                json!({"name": name, "version": package["version"], "manifest_path": manifest}),
            );
        }
    }
    ensure!(
        !report.is_empty(),
        "The project did not resolve any packages from the selected framework"
    );
    report.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(report)
}

fn apply(root: &Path, checkout: &Path, source: &str) -> Result<Vec<Value>> {
    let crates = catalog(checkout)?;
    let manifest = root.join("Cargo.toml");
    let lock = root.join("Cargo.lock");
    let original = fs::read_to_string(&manifest)?;
    let original_lock = match fs::read(&lock) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let patched = patched_manifest(&original, source, &crates)?;
    let result = (|| {
        let before = metadata(root, false)?;
        let mut update = Command::new("cargo");
        update.current_dir(root).arg("update");
        let mut count = 0;
        for package in packages(&before)? {
            if package["name"]
                .as_str()
                .is_some_and(|name| crates.contains_key(name))
                || package["source"]
                    .as_str()
                    .is_some_and(|s| same_repository(s, source))
            {
                update
                    .arg("--package")
                    .arg(package["id"].as_str().context("Missing Cargo package ID")?);
                count += 1;
            }
        }
        ensure!(
            count > 0,
            "The project does not depend on the selected framework"
        );
        fs::write(&manifest, patched)?;
        // A newer patch can be ignored while an older version remains locked.
        // Unlock only framework packages, including their distinct source IDs.
        output(&mut update)?;
        verify(&metadata(root, false)?, source, &crates)
    })();
    if let Err(error) = result {
        fs::write(&manifest, original).context("Restore compatibility manifest after failure")?;
        match original_lock {
            Some(bytes) => {
                fs::write(lock, bytes).context("Restore compatibility lockfile after failure")?
            }
            None if lock.exists() => fs::remove_file(lock)
                .context("Remove generated compatibility lockfile after failure")?,
            None => (),
        }
        bail!("Framework override rolled back: {error:#}");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, text: &str) -> Result<()> {
        let path = root.join(path);
        fs::create_dir_all(path.parent().context("File parent")?)?;
        fs::write(path, text)?;
        Ok(())
    }

    fn commit(root: &Path) -> Result<String> {
        output(Command::new("git").current_dir(root).args(["add", "."]))?;
        output(Command::new("git").current_dir(root).args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "fixture",
        ]))?;
        Ok(output(
            Command::new("git")
                .current_dir(root)
                .args(["rev-parse", "HEAD"]),
        )?
        .trim()
        .to_owned())
    }

    fn framework(root: &Path, version: &str) -> Result<()> {
        write(
            root,
            "Cargo.toml",
            "[workspace]\nmembers=['graphics','optional','platform']\nresolver='3'\n",
        )?;
        for name in ["graphics", "optional", "platform"] {
            write(
                root,
                &format!("{name}/Cargo.toml"),
                &format!(
                    "[package]\nname='cranpose-{name}'\nversion='{version}'\nedition='2024'\n"
                ),
            )?;
            write(root, &format!("{name}/src/lib.rs"), "pub struct Color;\n")?;
        }
        Ok(())
    }

    // An actual Cargo graph: the SDK and app pin different Git revisions. The
    // alias is optional, and a target dependency adds another source path.
    #[test]
    fn overrides_transitive_git_revisions_aliases_and_optional_targets() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let old = dir.path().join("old");
        framework(&old, "0.1.0")?;
        output(Command::new("git").current_dir(&old).arg("init"))?;
        let first = commit(&old)?;
        write(&old, "change", "second revision")?;
        let second = commit(&old)?;
        let source = reqwest::Url::from_directory_path(old.canonicalize()?)
            .map_err(|()| anyhow::anyhow!("Fixture repository URL"))?
            .to_string();
        let sdk = dir.path().join("sdk");
        write(
            &sdk,
            "Cargo.toml",
            &format!(
                "[package]\nname='fixture-sdk'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ngraphics={{package='cranpose-graphics',version='0.1',git='{source}',rev='{first}'}}\n"
            ),
        )?;
        write(
            &sdk,
            "src/lib.rs",
            "pub fn color() -> graphics::Color { graphics::Color }\n",
        )?;
        output(Command::new("git").current_dir(&sdk).arg("init"))?;
        let sdk_rev = commit(&sdk)?;
        let sdk_source = reqwest::Url::from_directory_path(sdk.canonicalize()?)
            .map_err(|()| anyhow::anyhow!("Fixture SDK URL"))?
            .to_string();
        let app = dir.path().join("app");
        write(
            &app,
            "Cargo.toml",
            &format!(
                "[package]\nname='fixture-app'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nfixture-sdk={{git='{sdk_source}',rev='{sdk_rev}'}}\ngraphics={{package='cranpose-graphics',version='0.1',git='{source}',rev='{second}'}}\nmaybe={{package='cranpose-optional',version='0.1',git='{source}',rev='{first}',optional=true}}\n[target.'cfg(windows)'.dependencies]\nwindows-colors={{package='cranpose-platform',version='0.1',git='{source}',rev='{first}'}}\n"
            ),
        )?;
        write(
            &app,
            "src/lib.rs",
            "pub fn color() -> graphics::Color { fixture_sdk::color() }\n#[cfg(feature=\"maybe\")] pub fn optional() -> maybe::Color { maybe::Color }\n",
        )?;
        let before = Command::new("cargo")
            .current_dir(&app)
            .args(["check", "--all-features"])
            .output()?;
        ensure!(
            !before.status.success()
                && String::from_utf8_lossy(&before.stderr).contains("mismatched types"),
            "Fixture must reproduce the original mixed-type failure"
        );
        let snapshot = dir.path().join("main");
        framework(&snapshot, "0.1.1")?;
        let original_sdk = fs::read(sdk.join("Cargo.toml"))?;
        let report = apply(&app, &snapshot.canonicalize()?, &source)?;
        assert_eq!(report.len(), 3);
        output(Command::new("cargo").current_dir(&app).args([
            "check",
            "--locked",
            "--all-features",
            "--offline",
        ]))?;
        assert_eq!(fs::read(sdk.join("Cargo.toml"))?, original_sdk);
        let once = fs::read(app.join("Cargo.toml"))?;
        apply(&app, &snapshot.canonicalize()?, &source)?;
        assert_eq!(fs::read(app.join("Cargo.toml"))?, once);
        // An incompatible patch must fail closed and restore both files exactly.
        let original_lock = fs::read(app.join("Cargo.lock"))?;
        framework(&snapshot, "0.2.0")?;
        let error =
            apply(&app, &snapshot.canonicalize()?, &source).expect_err("incompatible versions");
        assert!(error.to_string().contains("rolled back"));
        assert_eq!(fs::read(app.join("Cargo.toml"))?, once);
        assert_eq!(fs::read(app.join("Cargo.lock"))?, original_lock);
        Ok(())
    }

    #[test]
    fn refuses_existing_overrides_without_rewriting_the_manifest() -> Result<()> {
        let crates = BTreeMap::from([(
            "cranpose-core".into(),
            PathBuf::from("/test/core/Cargo.toml"),
        )]);
        let original = "[workspace]\n[patch.crates-io]\ncranpose-core = { git = 'https://example.invalid/custom' }\n";
        assert!(patched_manifest(original, REPOSITORY, &crates).is_err());
        Ok(())
    }

    #[test]
    fn rejects_removed_framework_package_from_old_git_revision() {
        let value = json!({"packages":[{"name":"cranpose-removed","source":format!("git+{REPOSITORY}.git?rev=old#abc")}]});
        let error = verify(&value, REPOSITORY, &BTreeMap::new()).expect_err("removed package");
        assert!(error.to_string().contains("Mixed framework graph"));
    }
}
