//! Exercise real Cargo `file!()` and manifest locations, including a generated
//! binary crate that compiles modules also present in its library dependency.
use cranpose_plugin_ux::source_paths::resolve;
use std::{fs, path::Path, process::Command};

fn write(root: &Path, file: &str, text: &str) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    fs::write(path, text).expect("fixture source");
}

#[test]
fn cargo_workspace_library_and_generated_binary_resolve_to_editable_files() {
    let temp = tempfile::tempdir().expect("fixture");
    let root = temp.path().canonicalize().expect("root");
    let compiled = root.join("compiled");
    let original = root.join("original");
    let member = "packages/app";
    let launcher = ".generated/launcher";
    write(
        &compiled,
        "Cargo.toml",
        "[workspace]\nmembers=['packages/app','.generated/launcher']\nresolver='2'\n",
    );
    write(
        &compiled,
        "packages/app/Cargo.toml",
        "[package]\nname='path-fixture'\nversion='0.1.0'\nedition='2024'\n",
    );
    write(
        &compiled,
        ".generated/launcher/Cargo.toml",
        "[package]\nname='path-launcher'\nversion='0.1.0'\nedition='2024'\n[dependencies]\npath-fixture={path='../../packages/app'}\n",
    );
    write(
        &compiled,
        "packages/app/src/lib.rs",
        "pub fn location() -> (&'static str, &'static str) { (file!(), env!(\"CARGO_MANIFEST_DIR\")) }\n",
    );
    write(
        &compiled,
        ".generated/launcher/src/screens/detail.rs",
        "pub fn location() -> (&'static str, &'static str) { (file!(), env!(\"CARGO_MANIFEST_DIR\")) }\n",
    );
    write(
        &compiled,
        ".generated/launcher/src/main.rs",
        "mod screens { pub mod detail; }\nfn main() { for (file, manifest) in [path_fixture::location(), screens::detail::location()] { println!(\"{file}|{manifest}\"); } }\n",
    );
    for file in ["src/lib.rs", "src/screens/detail.rs"] {
        write(&original.join(member), file, "editable source\n");
    }
    let result = Command::new(env!("CARGO"))
        .current_dir(&compiled)
        .args(["run", "--offline", "--quiet", "-p", "path-launcher"])
        .env("CARGO_TARGET_DIR", root.join("artifacts"))
        .output()
        .expect("run Cargo fixture");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).expect("Cargo output");
    let mappings = [(compiled.join(launcher), original.join(member))];
    let locations: Vec<_> = stdout
        .lines()
        .map(|line| {
            let (file, manifest) = line.split_once('|').expect("source location");
            resolve(
                Path::new(file),
                Path::new(manifest),
                &compiled,
                &original,
                &mappings,
            )
        })
        .collect();
    assert_eq!(
        locations,
        [
            original.join(member).join("src/lib.rs"),
            original.join(member).join("src/screens/detail.rs")
        ]
    );
    assert!(locations.iter().all(|path| path.is_file()));
}
