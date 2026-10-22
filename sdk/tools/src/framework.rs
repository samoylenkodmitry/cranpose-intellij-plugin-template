//! Development-only Cargo overrides. Never used by packaging or the runtime.
use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const REPOSITORY: &str = "https://github.com/samoylenkodmitry/Cranpose";
/// The registry source Cargo patches as `crates-io`.
const CRATES_IO: [&str; 2] = [
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
];

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
    let sources = tempfile::Builder::new()
        .prefix("cranpose-main-")
        .tempdir()?;
    let checkout = match options.checkout {
        None => {
            let path = sources.path().join("framework");
            output(
                Command::new("git")
                    .args(["clone", "--depth", "1", "--branch", "main", REPOSITORY])
                    .arg(&path),
            )?;
            path
        }
        Some(checkout) => root.join(checkout),
    }
    .canonicalize()?;
    let revision = output(
        Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["rev-parse", "HEAD"]),
    )?;
    let report = apply(
        root,
        &checkout,
        REPOSITORY,
        &sources.path().join("dependents"),
    )?;
    // Paths must remain available to later cargo test/IDE invocations.
    println!(
        "Compatibility sources retained at {}",
        sources.keep().display()
    );
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

/// A framework package of the selected checkout.
struct Crate {
    manifest: PathBuf,
    version: String,
}

fn catalog(checkout: &Path) -> Result<BTreeMap<String, Crate>> {
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
        let version = package["version"]
            .as_str()
            .context("Missing package version")?
            .to_owned();
        result.insert(name.to_owned(), Crate { manifest, version });
    }
    ensure!(
        !result.is_empty(),
        "No Cranpose framework packages in {}",
        checkout.display()
    );
    Ok(result)
}

/// A package outside the workspace served from a copy whose framework
/// requirements name the checkout's versions.
struct Dependent {
    /// The `[patch]` table key of the package's source.
    source: String,
    name: String,
    path: PathBuf,
    id: String,
}

fn add_patches(
    document: &mut toml_edit::DocumentMut,
    source: &str,
    crates: &BTreeMap<String, Crate>,
    dependents: &[Dependent],
) -> Result<()> {
    // A workspace-root patch also reaches pinned Git SDKs, aliases, and target /
    // optional dependencies. Registry consumers must use the same local types.
    let framework = [source, "crates-io"].into_iter().flat_map(|repository| {
        crates
            .iter()
            .map(move |(name, krate)| (repository, name.as_str(), krate.manifest.parent()))
    });
    let copies = dependents.iter().map(|dependent| {
        (
            dependent.source.as_str(),
            dependent.name.as_str(),
            Some(dependent.path.as_path()),
        )
    });
    for (repository, name, directory) in framework.chain(copies) {
        let path = directory
            .context("Patched manifest parent")?
            .to_str()
            .context("Non-UTF-8 patch path")?;
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
    Ok(())
}

/// Points the framework requirements of a manifest at the checkout's
/// versions. Before 1.0 a minor release is a breaking one, so a requirement
/// such as `0.13` excludes a checkout at 0.17 and Cargo ignores its patch.
fn require_checkout_versions(
    document: &mut toml_edit::DocumentMut,
    crates: &BTreeMap<String, Crate>,
) {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let root = document.as_table_mut();
    for kind in KINDS {
        if let Some(table) = root
            .get_mut(kind)
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            require_in(table, crates);
        }
    }
    if let Some(table) = root
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("dependencies"))
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        require_in(table, crates);
    }
    if let Some(targets) = root
        .get_mut("target")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        for (_, target) in targets.iter_mut() {
            for kind in KINDS {
                if let Some(table) = target
                    .get_mut(kind)
                    .and_then(toml_edit::Item::as_table_like_mut)
                {
                    require_in(table, crates);
                }
            }
        }
    }
}

fn require_in(table: &mut dyn toml_edit::TableLike, crates: &BTreeMap<String, Crate>) {
    for (key, item) in table.iter_mut() {
        let name = item
            .as_table_like()
            .and_then(|dependency| dependency.get("package"))
            .and_then(toml_edit::Item::as_str)
            .unwrap_or(key.get());
        let Some(krate) = crates.get(name) else {
            continue;
        };
        if item.is_str() {
            *item = toml_edit::value(krate.version.as_str());
        } else if let Some(dependency) = item.as_table_like_mut()
            && dependency.contains_key("version")
        {
            dependency.insert("version", toml_edit::value(krate.version.as_str()));
        }
    }
}

/// The `[patch]` key of a resolved package source, `None` for a source
/// Cargo cannot patch by URL.
fn patch_key(source: &str) -> Option<String> {
    if CRATES_IO.contains(&source) {
        return Some("crates-io".into());
    }
    let url = source.strip_prefix("git+")?;
    Some(url.split(['?', '#']).next().unwrap_or(url).to_owned())
}

/// The directory Cargo unpacked a dependency into: a Git checkout keeps the
/// whole repository there, so the package's path and workspace dependencies
/// resolve inside a copy of it.
fn unpacked_root(manifest: &Path) -> Result<PathBuf> {
    let directory = manifest.parent().context("Dependency manifest parent")?;
    directory
        .ancestors()
        .find(|ancestor| ancestor.join(".cargo-ok").exists())
        .map(Path::to_path_buf)
        .with_context(|| format!("{} is not in a Cargo source checkout", directory.display()))
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == "target" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn require_checkout_versions_under(
    directory: &Path,
    crates: &BTreeMap<String, Crate>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            require_checkout_versions_under(&path, crates)?;
        } else if path.file_name().is_some_and(|name| name == "Cargo.toml") {
            let mut document = fs::read_to_string(&path)?.parse::<toml_edit::DocumentMut>()?;
            require_checkout_versions(&mut document, crates);
            fs::write(&path, document.to_string())?;
        }
    }
    Ok(())
}

/// Copies the sources of the packages outside the workspace whose framework
/// requirements the checkout does not meet, with those requirements pointed
/// at the checkout, and returns a patch for every package served from such
/// a source. Patching all of a source keeps its packages one copy each.
fn copy_dependents(
    graph: &Value,
    crates: &BTreeMap<String, Crate>,
    directory: &Path,
) -> Result<Vec<Dependent>> {
    let members: BTreeSet<&str> = graph["workspace_members"]
        .as_array()
        .context("Cargo metadata has no workspace members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut outdated = BTreeSet::new();
    for package in packages(graph)? {
        let (Some(id), Some(source)) = (package["id"].as_str(), package["source"].as_str()) else {
            continue;
        };
        let name = package["name"].as_str().unwrap_or_default();
        if members.contains(id) || crates.contains_key(name) {
            continue;
        }
        let requires_other = package["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|dependency| {
                let (Some(name), Some(requirement)) =
                    (dependency["name"].as_str(), dependency["req"].as_str())
                else {
                    return false;
                };
                crates.get(name).is_some_and(|krate| {
                    let version = semver::Version::parse(&krate.version);
                    let requirement = semver::VersionReq::parse(requirement);
                    !matches!((version, requirement), (Ok(version), Ok(requirement)) if requirement.matches(&version))
                })
            });
        if requires_other {
            outdated.insert(source);
        }
    }
    let mut dependents = Vec::new();
    for (index, source) in outdated.into_iter().enumerate() {
        let key = patch_key(source)
            .with_context(|| format!("Cannot patch {source} to the selected framework"))?;
        for package in packages(graph)? {
            if package["source"].as_str() != Some(source) {
                continue;
            }
            let manifest = PathBuf::from(
                package["manifest_path"]
                    .as_str()
                    .context("Missing dependency manifest path")?,
            );
            let original = unpacked_root(&manifest)?;
            let copy = directory.join(format!(
                "{index}-{}",
                original
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("source")
            ));
            if !copy.exists() {
                copy_tree(&original, &copy)?;
                require_checkout_versions_under(&copy, crates)?;
            }
            let path = copy.join(
                manifest
                    .parent()
                    .context("Dependency manifest parent")?
                    .strip_prefix(&original)?,
            );
            let name = package["name"]
                .as_str()
                .context("Missing dependency name")?
                .to_owned();
            ensure!(
                !dependents
                    .iter()
                    .any(|other: &Dependent| other.source == key && other.name == name),
                "Several revisions of {name} from {key} need the selected framework"
            );
            dependents.push(Dependent {
                source: key.clone(),
                name,
                path,
                id: package["id"]
                    .as_str()
                    .context("Missing Cargo package ID")?
                    .to_owned(),
            });
        }
    }
    Ok(dependents)
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

fn verify(value: &Value, repository: &str, crates: &BTreeMap<String, Crate>) -> Result<Vec<Value>> {
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
                source.is_none() && manifest == expected.manifest,
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

/// The manifests of the workspace at `root`: its root manifest and every
/// member's, each once.
fn workspace_manifests(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let workspace = metadata(root, true)?;
    let mut manifests = BTreeSet::from([root.join("Cargo.toml").canonicalize()?]);
    for package in packages(&workspace)? {
        manifests.insert(
            PathBuf::from(
                package["manifest_path"]
                    .as_str()
                    .context("Missing member manifest path")?,
            )
            .canonicalize()?,
        );
    }
    Ok(manifests)
}

fn apply(root: &Path, checkout: &Path, source: &str, copies: &Path) -> Result<Vec<Value>> {
    let crates = catalog(checkout)?;
    let manifest = root.join("Cargo.toml").canonicalize()?;
    let lock = root.join("Cargo.lock");
    let mut originals = BTreeMap::new();
    for path in workspace_manifests(root)? {
        let text = fs::read_to_string(&path)?;
        originals.insert(path, text);
    }
    let original_lock = match fs::read(&lock) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let result = (|| {
        let before = metadata(root, false)?;
        let dependents = copy_dependents(&before, &crates, copies)?;
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
        for dependent in &dependents {
            update.arg("--package").arg(&dependent.id);
        }
        for (path, text) in &originals {
            let mut document = text.parse::<toml_edit::DocumentMut>()?;
            require_checkout_versions(&mut document, &crates);
            if path == &manifest {
                add_patches(&mut document, source, &crates, &dependents)?;
            }
            fs::write(path, document.to_string())?;
        }
        // A newer patch can be ignored while an older version remains locked.
        // Unlock only framework packages, including their distinct source IDs.
        output(&mut update)?;
        verify(&metadata(root, false)?, source, &crates)
    })();
    if let Err(error) = result {
        for (path, text) in &originals {
            fs::write(path, text).context("Restore compatibility manifest after failure")?;
        }
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

    const ALL: [&str; 3] = ["graphics", "optional", "platform"];

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

    fn framework(root: &Path, version: &str, names: &[&str]) -> Result<()> {
        let members = names
            .iter()
            .map(|name| format!("'{name}'"))
            .collect::<Vec<_>>()
            .join(",");
        write(
            root,
            "Cargo.toml",
            &format!("[workspace]\nmembers=[{members}]\nresolver='3'\n"),
        )?;
        for name in names {
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
        framework(&old, "0.1.0", &ALL)?;
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
        let original_app = fs::read(app.join("Cargo.toml"))?;
        let original_lock = fs::read(app.join("Cargo.lock"))?;
        let original_sdk = fs::read(sdk.join("Cargo.toml"))?;
        // A package main removed still resolves from the old revision: the
        // override must fail closed and restore both files exactly.
        let snapshot = dir.path().join("main");
        framework(&snapshot, "0.2.0", &["graphics", "platform"])?;
        let copies = dir.path().join("copies");
        let error =
            apply(&app, &snapshot.canonicalize()?, &source, &copies).expect_err("removed package");
        assert!(error.to_string().contains("rolled back"));
        assert_eq!(fs::read(app.join("Cargo.toml"))?, original_app);
        assert_eq!(fs::read(app.join("Cargo.lock"))?, original_lock);

        // Main is a minor version ahead, which before 1.0 the app's and the
        // Git SDK's `0.1` requirements exclude. Both now resolve main.
        framework(&snapshot, "0.2.0", &ALL)?;
        let report = apply(&app, &snapshot.canonicalize()?, &source, &copies)?;
        assert_eq!(report.len(), 3);
        output(Command::new("cargo").current_dir(&app).args([
            "check",
            "--locked",
            "--all-features",
            "--offline",
        ]))?;
        assert_eq!(fs::read(sdk.join("Cargo.toml"))?, original_sdk);
        let once = fs::read(app.join("Cargo.toml"))?;
        apply(&app, &snapshot.canonicalize()?, &source, &copies)?;
        assert_eq!(fs::read(app.join("Cargo.toml"))?, once);
        Ok(())
    }

    #[test]
    fn refuses_existing_overrides() -> Result<()> {
        let crates = BTreeMap::from([(
            "cranpose-core".into(),
            Crate {
                manifest: PathBuf::from("/test/core/Cargo.toml"),
                version: "0.17.0".into(),
            },
        )]);
        let mut document = "[workspace]\n[patch.crates-io]\ncranpose-core = { git = 'https://example.invalid/custom' }\n"
            .parse::<toml_edit::DocumentMut>()?;
        assert!(add_patches(&mut document, REPOSITORY, &crates, &[]).is_err());
        Ok(())
    }

    #[test]
    fn rejects_removed_framework_package_from_old_git_revision() {
        let value = json!({"packages":[{"name":"cranpose-removed","source":format!("git+{REPOSITORY}.git?rev=old#abc")}]});
        let error = verify(&value, REPOSITORY, &BTreeMap::new()).expect_err("removed package");
        assert!(error.to_string().contains("Mixed framework graph"));
    }
}
