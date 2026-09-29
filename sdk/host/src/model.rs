use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub package_name: String,
    pub name: String,
    pub kind: String,
    pub manifest: String,
    pub source: String,
    pub features: Vec<String>,
    pub cranpose_dependency: Option<String>,
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub status: String,
    pub busy: bool,
    pub targets: Vec<Target>,
    pub selected: String,
    pub root: String,
    pub diagnostics: Vec<Value>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            status: "Open a Cargo project, then refresh.".into(),
            busy: false,
            targets: vec![],
            selected: String::new(),
            root: String::new(),
            diagnostics: vec![],
        }
    }
}
pub fn metadata(text: &str) -> Result<Snapshot> {
    let value: Value = serde_json::from_str(text)?;
    let members = value["workspace_members"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing Cargo workspace members"))?;
    let packages = value["packages"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing Cargo packages"))?
        .iter()
        .filter(|p| members.contains(&p["id"]))
        .collect::<Vec<_>>();
    let mut targets = vec![];
    for package in &packages {
        let dependency = package["dependencies"]
            .as_array()
            .and_then(|ds| ds.iter().find(|d| d["name"] == "cranpose"));
        if dependency.is_none() && package["name"] != "cranpose" {
            continue;
        }
        for target in package["targets"].as_array().into_iter().flatten() {
            let kinds = target["kind"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing Cargo target kinds"))?;
            let kind = if kinds.contains(&json!("bin")) {
                "bin"
            } else if kinds.contains(&json!("example"))
                && target["crate_types"]
                    .as_array()
                    .is_some_and(|ts| ts.contains(&json!("bin")))
            {
                "example"
            } else {
                continue;
            };
            let name = s(target, "name");
            let manifest = s(package, "manifest_path");
            let package_name = s(package, "name");
            targets.push(Target {
                id: format!("{manifest}::{kind}::{name}"),
                label: format!("{package_name} / {name} ({kind})"),
                package_name,
                name,
                manifest,
                kind: kind.into(),
                source: s(target, "src_path"),
                features: target["required-features"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                cranpose_dependency: dependency
                    .map(|d| d["rename"].as_str().unwrap_or("cranpose").to_owned()),
            });
        }
    }
    let status = if targets.is_empty() {
        "No application found. Add a binary or example that depends on cranpose.".into()
    } else {
        format!(
            "{} {} in {} {}",
            targets.len(),
            if targets.len() == 1 {
                "application"
            } else {
                "applications"
            },
            packages.len(),
            if packages.len() == 1 {
                "package"
            } else {
                "packages"
            }
        )
    };
    Ok(Snapshot {
        status,
        selected: targets.first().map(|t| t.id.clone()).unwrap_or_default(),
        targets,
        root: s(&value, "workspace_root"),
        ..Snapshot::default()
    })
}
pub fn s(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
pub fn cargo() -> PathBuf {
    cargo_path().unwrap_or_else(|| PathBuf::from(cargo_name()))
}
fn cargo_name() -> &'static str {
    if cfg!(windows) { "cargo.exe" } else { "cargo" }
}
pub fn cargo_path() -> Option<PathBuf> {
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|p| p.join(".cargo")));
    let path = std::env::var_os("PATH").unwrap_or_default();
    find_cargo(cargo_home.as_deref(), &path)
}
fn find_cargo(cargo_home: Option<&Path>, path: &std::ffi::OsStr) -> Option<PathBuf> {
    cargo_home
        .map(|p| p.join("bin"))
        .into_iter()
        .chain(std::env::split_paths(path).filter(|p| !p.as_os_str().is_empty()))
        .map(|p| p.join(cargo_name()))
        .find(|p| {
            if !p.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                p.metadata()
                    .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
}
pub fn arguments(task: &str, target: &Target) -> Vec<String> {
    let mut args = vec![
        if task == "preview" { "build" } else { task }.into(),
        "--manifest-path".into(),
        target.manifest.clone(),
        "--package".into(),
        target.package_name.clone(),
    ];
    if task != "test" {
        args.extend([format!("--{}", target.kind), target.name.clone()]);
    }
    let mut features = target.features.clone();
    if task == "preview" {
        if let Some(dep) = &target.cranpose_dependency {
            features.push(format!("{dep}/preview"));
        } else if target.package_name == "cranpose" {
            features.push("preview".into());
        }
    }
    features.sort();
    features.dedup();
    if !features.is_empty() {
        args.extend(["--features".into(), features.join(",")]);
    }
    if task == "preview" || task == "check" {
        args.push("--message-format=json".into());
    }
    args
}
pub fn diagnostic(line: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["reason"] != "compiler-message" {
        return None;
    }
    let message = &value["message"];
    let spans = message["spans"].as_array()?;
    let span = spans
        .iter()
        .find(|s| s["is_primary"] == true)
        .or_else(|| spans.first());
    Some(
        json!({"message":message["message"],"level":message["level"],"rendered":message["rendered"].as_str().unwrap_or_default(),
        "file":span.map(|s|s["file_name"].clone()).unwrap_or(json!("")),"line":span.map(|s|s["line_start"].clone()).unwrap_or(json!(1)),
        "column":span.map(|s|s["column_start"].clone()).unwrap_or(json!(1))}),
    )
}
pub fn is_cranpose_source(path: &Path) -> bool {
    if path.extension().is_none_or(|e| e != "rs")
        || path.metadata().map(|m| m.len() > 2_000_000).unwrap_or(true)
    {
        return false;
    }
    for parent in path.ancestors().skip(1) {
        let manifest = parent.join("Cargo.toml");
        if manifest.metadata().is_ok_and(|m| m.len() < 1_000_000)
            && std::fs::read_to_string(manifest).is_ok_and(|s| s.contains("cranpose"))
        {
            return true;
        }
    }
    false
}
pub fn starter_manifest(name: &str) -> Result<String> {
    ensure!(
        name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'),
        "Use lowercase letters, digits, hyphens or underscores, starting with a letter."
    );
    Ok(format!(
        "[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ncranpose = {{ git = \"https://github.com/samoylenkodmitry/Cranpose\", rev = \"283736c61e85486214ba0ef8b9ac813000b5055d\", features = [\"desktop\", \"preview\"] }}\n"
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cargo_discovery_supports_fresh_install_and_custom_home_without_path() {
        let temp = tempfile::tempdir().expect("temp");
        let home = temp.path().join("Rust tools 🦀");
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).expect("bin");
        let path = std::ffi::OsStr::new("");
        assert!(find_cargo(Some(&home), path).is_none());
        let cargo = bin.join(cargo_name());
        std::fs::write(&cargo, "test executable").expect("file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(find_cargo(Some(&home), path).is_none());
            std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).expect("mode");
        }
        assert_eq!(find_cargo(Some(&home), path), Some(cargo.clone()));
        let path = std::env::join_paths([&bin]).expect("path");
        assert_eq!(find_cargo(None, &path), Some(cargo));
    }
    #[test]
    fn starter_rejects_injected_manifest() {
        assert!(starter_manifest("app\"\n[evil]").is_err());
        assert!(
            starter_manifest("field-notes")
                .expect("manifest")
                .contains("283736c61e85486214ba0ef8b9ac813000b5055d")
        );
    }
    #[test]
    fn cargo_metadata_keeps_alias_and_workspace_boundary() {
        let value = json!({"workspace_root":"/app","workspace_members":["app"],"packages":[{"id":"app","name":"app","manifest_path":"/app/Cargo.toml","dependencies":[{"name":"cranpose","rename":"cp"}],"targets":[{"name":"app","kind":["bin"],"crate_types":["bin"],"src_path":"/app/src/main.rs","required-features":["desktop"]}]},{"id":"foreign","name":"cranpose","targets":[]}]});
        let snapshot = metadata(&value.to_string()).expect("metadata");
        assert_eq!(snapshot.targets.len(), 1);
        let args = arguments("preview", &snapshot.targets[0]);
        assert!(args.contains(&"cp/preview,desktop".into()));
        assert!(
            !arguments("run", &snapshot.targets[0])
                .iter()
                .any(|s| s.contains("/preview"))
        );
    }
}
