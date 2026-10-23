//! The Cranpose a Cargo project uses, the newest one on crates.io, and moving
//! a project's requirements to it.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Where each Cranpose release describes its changes.
pub const RELEASES: &str = "https://github.com/samoylenkodmitry/Cranpose/releases";
/// How long a crates.io answer stands before the next refresh asks again.
const LATEST_TTL: Duration = Duration::from_secs(6 * 60 * 60);
/// How long a failed lookup, offline for example, waits before another try.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
/// The Cargo registries crates.io serves.
const CRATES_IO: [&str; 2] = [
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
];

/// The version of Cranpose a project resolves and the newest published one.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    pub used: Option<String>,
    pub latest: Option<String>,
    /// `latest` is a newer release than `used`.
    pub newer: bool,
}

impl Versions {
    pub fn new(used: Option<String>, latest: Option<String>) -> Self {
        let newer = match (&used, &latest) {
            (Some(used), Some(latest)) => is_newer(latest, used),
            _ => false,
        };
        Self {
            used,
            latest,
            newer,
        }
    }
}

/// Whether release `candidate` comes after `current`; a version that is not
/// `major.minor.patch` compares as not newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (release(candidate), release(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn release(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split(['-', '+']).next()?.split('.');
    let mut release = [0; 3];
    for part in &mut release {
        *part = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(release)
}

/// The newest `cranpose` version in the lockfile `text` that a registry
/// serves, `None` when it locks none.
pub fn locked_version(text: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    document
        .get("package")?
        .as_array_of_tables()?
        .iter()
        .filter(|package| package.get("name").and_then(toml_edit::Item::as_str) == Some("cranpose"))
        .filter(|package| {
            package
                .get("source")
                .and_then(toml_edit::Item::as_str)
                .is_some_and(|source| {
                    source.starts_with("registry+") || source.starts_with("sparse+")
                })
        })
        .filter_map(|package| package.get("version").and_then(toml_edit::Item::as_str))
        .max_by_key(|version| release(version))
        .map(str::to_owned)
}

/// The version `cargo search` reports for `cranpose`, from output such as
/// `cranpose = "0.17.1"    # A Kotlin Compose-like...`.
pub fn searched_version(output: &str) -> Option<String> {
    let rest = output
        .lines()
        .find_map(|line| line.strip_prefix("cranpose = \""))?;
    let version = rest.split('"').next()?;
    release(version).map(|_| version.to_owned())
}

struct Lookup {
    latest: Option<String>,
    at: Option<Instant>,
}

static LOOKUP: Mutex<Lookup> = Mutex::new(Lookup {
    latest: None,
    at: None,
});
static LOOKING: AtomicBool = AtomicBool::new(false);

/// The newest Cranpose on crates.io from the last lookup, without waiting.
pub fn latest() -> Option<String> {
    LOOKUP.lock().expect("Cranpose lookup").latest.clone()
}

/// The newest Cranpose from the last lookup when it differs from `known`.
pub fn latest_if_changed(known: Option<&str>) -> Option<Option<String>> {
    let lookup = LOOKUP.lock().expect("Cranpose lookup");
    (lookup.latest.as_deref() != known).then(|| lookup.latest.clone())
}

/// Asks crates.io for the newest Cranpose in the background, through Cargo
/// so its proxy and registry settings apply, unless a recent answer stands.
pub fn look_up_latest() {
    {
        let lookup = LOOKUP.lock().expect("Cranpose lookup");
        let fresh = lookup.at.is_some_and(|at| {
            at.elapsed()
                < if lookup.latest.is_some() {
                    LATEST_TTL
                } else {
                    RETRY_AFTER
                }
        });
        if fresh || LOOKING.swap(true, Ordering::AcqRel) {
            return;
        }
    }
    std::thread::spawn(|| {
        let found = search().ok().flatten();
        let mut lookup = LOOKUP.lock().expect("Cranpose lookup");
        lookup.at = Some(Instant::now());
        if found.is_some() {
            lookup.latest = found;
        }
        LOOKING.store(false, Ordering::Release);
    });
}

fn search() -> Result<Option<String>> {
    let mut command = Command::new(crate::model::cargo());
    command.args(["search", "cranpose", "--limit", "1"]);
    let output = crate::process::capture(command, None, Duration::from_secs(20), || false)?;
    ensure!(output.status.success(), "cargo search failed");
    Ok(searched_version(&String::from_utf8_lossy(&output.stdout)))
}

/// Sets the requirement of every dependency in `document` that `version`
/// names a version for: plain, renamed, workspace-wide, target-specific, dev
/// and build dependencies. Dependencies without a version requirement, such
/// as path or Git ones, keep theirs.
pub fn require_versions<'v>(
    document: &mut toml_edit::DocumentMut,
    version: &impl Fn(&str) -> Option<&'v str>,
) {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let root = document.as_table_mut();
    for kind in KINDS {
        if let Some(table) = root
            .get_mut(kind)
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            require_in(table, version);
        }
    }
    if let Some(table) = root
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("dependencies"))
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        require_in(table, version);
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
                    require_in(table, version);
                }
            }
        }
    }
}

fn require_in<'v>(
    table: &mut dyn toml_edit::TableLike,
    version: &impl Fn(&str) -> Option<&'v str>,
) {
    for (key, item) in table.iter_mut() {
        let name = item
            .as_table_like()
            .and_then(|dependency| dependency.get("package"))
            .and_then(toml_edit::Item::as_str)
            .unwrap_or(key.get());
        let Some(required) = version(name) else {
            continue;
        };
        if item.is_str() {
            *item = toml_edit::value(required);
        } else if let Some(dependency) = item.as_table_like_mut()
            && dependency.contains_key("version")
        {
            dependency.insert("version", toml_edit::value(required));
        }
    }
}

/// `cargo metadata` of the workspace at `root`, resolving its dependencies
/// unless `no_deps`. Resolving writes the lockfile Cargo needs for it.
pub fn metadata(root: &Path, no_deps: bool) -> Result<Value> {
    let mut command = Command::new(crate::model::cargo());
    command
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(root.join("Cargo.toml"));
    if no_deps {
        command.arg("--no-deps");
    }
    let output = crate::process::capture(command, None, Duration::from_secs(300), || false)?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn packages(graph: &Value) -> Result<&Vec<Value>> {
    graph["packages"]
        .as_array()
        .context("Cargo metadata has no package list")
}

/// The manifests of the workspace in `graph`: its root manifest and every
/// member's, each once.
pub fn workspace_manifests(graph: &Value) -> Result<BTreeSet<PathBuf>> {
    let root = graph["workspace_root"]
        .as_str()
        .context("Cargo metadata has no workspace root")?;
    let members: BTreeSet<&str> = graph["workspace_members"]
        .as_array()
        .context("Cargo metadata has no workspace members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut manifests = BTreeSet::from([Path::new(root).join("Cargo.toml").canonicalize()?]);
    for package in packages(graph)? {
        if package["id"]
            .as_str()
            .is_some_and(|id| members.contains(id))
        {
            manifests.insert(
                PathBuf::from(
                    package["manifest_path"]
                        .as_str()
                        .context("Missing member manifest path")?,
                )
                .canonicalize()?,
            );
        }
    }
    Ok(manifests)
}

/// The Cranpose crates a resolved `graph` uses: crates.io packages named like
/// Cranpose's at the version its `cranpose` resolves to, since Cranpose
/// publishes all of them under one version. Crates named alike from Git or a
/// path, such as the plugin's own, stay out.
pub fn framework_crates(graph: &Value) -> Result<BTreeSet<String>> {
    let registry = |package: &&Value| {
        package["source"]
            .as_str()
            .is_some_and(|source| CRATES_IO.contains(&source))
    };
    let versions: BTreeSet<&str> = packages(graph)?
        .iter()
        .filter(registry)
        .filter(|package| package["name"] == "cranpose")
        .filter_map(|package| package["version"].as_str())
        .collect();
    Ok(packages(graph)?
        .iter()
        .filter(registry)
        .filter(|package| {
            package["version"]
                .as_str()
                .is_some_and(|version| versions.contains(version))
        })
        .filter_map(|package| package["name"].as_str())
        .filter(|name| *name == "coroflow" || name.starts_with("cranpose"))
        .map(str::to_owned)
        .collect())
}

/// Moves the Cranpose requirements of the workspace at `root` to `version`
/// and resolves it again, which updates the lockfile. Restores the manifests
/// and the lockfile when Cargo cannot resolve the new version.
pub fn update(root: &Path, version: &str) -> Result<()> {
    let graph = metadata(root, false)?;
    let crates = framework_crates(&graph)?;
    ensure!(
        !crates.is_empty(),
        "The project resolves no Cranpose crate from crates.io"
    );
    let lock = root.join("Cargo.lock");
    let original_lock = std::fs::read(&lock).ok();
    let mut originals = Vec::new();
    for manifest in workspace_manifests(&graph)? {
        let text = std::fs::read_to_string(&manifest)?;
        let mut document = text.parse::<toml_edit::DocumentMut>()?;
        require_versions(&mut document, &|name| {
            crates.contains(name).then_some(version)
        });
        let updated = document.to_string();
        if updated != text {
            std::fs::write(&manifest, updated)?;
            originals.push((manifest, text));
        }
    }
    let resolved = metadata(root, false).and_then(|graph| {
        let crates = framework_crates(&graph)?;
        let locked = std::fs::read_to_string(&lock).ok();
        ensure!(
            locked.as_deref().and_then(locked_version).as_deref() == Some(version)
                && !crates.is_empty(),
            "Cargo did not resolve Cranpose {version}"
        );
        Ok(())
    });
    if let Err(error) = resolved {
        for (manifest, text) in &originals {
            std::fs::write(manifest, text)?;
        }
        if let Some(bytes) = original_lock {
            std::fs::write(&lock, bytes)?;
        }
        return Err(error.context(format!("Cranpose was not updated to {version}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_version_cargo_search_prints() {
        let output = "cranpose = \"0.17.1\"    # A Kotlin Compose-like UI framework\n... and 41 crates more (use --limit N to see more)\n";
        assert_eq!(searched_version(output).as_deref(), Some("0.17.1"));
        assert_eq!(searched_version("cranpose-core = \"0.17.1\"\n"), None);
        assert_eq!(searched_version("cranpose = \"latest\"\n"), None);
        assert_eq!(searched_version(""), None);
    }

    #[test]
    fn reads_the_registry_cranpose_from_a_lockfile() {
        let lock = r#"version = 4

[[package]]
name = "cranpose"
version = "0.13.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "cranpose"
version = "0.9.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "cranpose"
version = "9.0.0"
source = "git+https://example.invalid/fork#abc"

[[package]]
name = "cranpose-core"
version = "0.13.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        assert_eq!(locked_version(lock).as_deref(), Some("0.13.0"));
        assert_eq!(locked_version("version = 4\n"), None);
    }

    #[test]
    fn compares_release_numbers() {
        assert!(is_newer("0.17.1", "0.13.0"));
        assert!(is_newer("0.13.10", "0.13.9"));
        assert!(!is_newer("0.13.0", "0.13.0"));
        assert!(!is_newer("0.13.0", "0.17.1"));
        assert!(!is_newer("0.18", "0.17.1"));
        assert!(Versions::new(Some("0.13.0".into()), Some("0.17.1".into())).newer);
        assert!(!Versions::new(None, Some("0.17.1".into())).newer);
    }

    #[test]
    fn updates_only_the_crates_cranpose_publishes() -> Result<()> {
        let registry = "registry+https://github.com/rust-lang/crates.io-index";
        let graph = json!({"packages": [
            {"name": "cranpose", "version": "0.13.0", "source": registry},
            {"name": "cranpose-core", "version": "0.13.0", "source": registry},
            {"name": "coroflow", "version": "0.13.0", "source": registry},
            {"name": "cranpose-wgpu", "version": "30.0.24", "source": registry},
            {"name": "cranpose-plugin-host", "version": "0.13.0", "source": "git+https://example.invalid/template#abc"},
            {"name": "cranpose-mine", "version": "0.13.0", "source": null},
            {"name": "serde", "version": "0.13.0", "source": registry},
        ]});
        let crates = framework_crates(&graph)?;
        assert_eq!(
            crates.iter().map(String::as_str).collect::<Vec<_>>(),
            ["coroflow", "cranpose", "cranpose-core"]
        );
        let mut manifest = r#"[package]
name = "app"

[dependencies]
cranpose = { version = "0.13.0", features = ["desktop"] }
ui = { package = "cranpose-core", version = "0.13" }
coroflow = "0.13.0"
cranpose-mine = { path = "mine", version = "0.13.0" }
cranpose-plugin-host = { git = "https://example.invalid/template", rev = "abc" }
serde = "0.13.0"

[target.'cfg(windows)'.dependencies]
cranpose = { version = "0.13.0", features = ["windows"] }

[workspace.dependencies]
cranpose-core = "0.13.0"
"#
        .parse::<toml_edit::DocumentMut>()?;
        require_versions(&mut manifest, &|name| {
            crates.contains(name).then_some("0.17.1")
        });
        let manifest = manifest.to_string();
        assert_eq!(manifest.matches("\"0.17.1\"").count(), 5, "{manifest}");
        assert!(manifest.contains("features = [\"desktop\"]"));
        assert!(manifest.contains("cranpose-mine = { path = \"mine\", version = \"0.13.0\" }"));
        assert!(manifest.contains("serde = \"0.13.0\""));
        Ok(())
    }

    #[test]
    #[ignore = "asks crates.io; run with --ignored"]
    fn updates_a_project_to_the_newest_release() -> Result<()> {
        let latest = search()?.context("crates.io lists cranpose")?;
        let project = tempfile::tempdir()?;
        std::fs::create_dir_all(project.path().join("src"))?;
        std::fs::write(project.path().join("src/main.rs"), "fn main() {}\n")?;
        std::fs::write(
            project.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ncranpose = { version = \"0.13.0\", features = [\"desktop\"] }\ncoroflow = \"0.13.0\"\n",
        )?;
        let root = project.path().canonicalize()?;
        metadata(&root, false)?;
        let lock = || std::fs::read_to_string(root.join("Cargo.lock"));
        assert_eq!(locked_version(&lock()?).as_deref(), Some("0.13.0"));
        update(&root, &latest)?;
        assert_eq!(locked_version(&lock()?), Some(latest.clone()));
        let manifest = std::fs::read_to_string(root.join("Cargo.toml"))?;
        assert_eq!(
            manifest.matches(&format!("\"{latest}\"")).count(),
            2,
            "{manifest}"
        );
        Ok(())
    }
}
