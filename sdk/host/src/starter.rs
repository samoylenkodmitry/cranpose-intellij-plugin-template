//! Offline, staged generation from the pinned Showcase bundled with the plugin.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Component, Path},
};
pub const SHOWCASE_REPOSITORY: &str = "https://github.com/samoylenkodmitry/cranpose-showcase";
pub const SHOWCASE_REVISION: &str = "287ceafe513523b0fdc9bd8998aa724cfe738596";
const SHOWCASE_ARCHIVE: &[u8] = include_bytes!("../assets/showcase.zip");
const SHOWCASE_SHA256: &str = "19b69244adcaa0b5c4e18506cda48b695e574d01d94ffc03ea9550121459c230";

/// The bundled starter's desktop entry is known before Cargo (or even Rust) is
/// installed. Keep this contract tied to the pinned archive in the tests below.
pub(crate) fn desktop_target(root: &Path) -> crate::model::Target {
    let path = |relative: &str| {
        let value = root.join(relative).to_string_lossy().into_owned();
        if cfg!(windows) {
            value.replace('\\', "/")
        } else {
            value
        }
    };
    let manifest = path("Cargo.toml");
    crate::model::Target {
        id: format!("{manifest}::bin::cranpose-showcase"),
        manifest,
        source: path("src/main.rs"),
        package_name: "cranpose-showcase".into(),
        name: "cranpose-showcase".into(),
        kind: "bin".into(),
        features: vec!["desktop".into()],
        cranpose_dependency: Some("cranpose".into()),
        label: "cranpose-showcase · desktop".into(),
    }
}

pub fn validate_destination(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    ensure!(root.is_dir(), "Project location is not a directory");
    for item in fs::read_dir(root)? {
        let item = item?;
        let name = item.file_name();
        let name = name.to_string_lossy();
        ensure!(
            name == ".idea" || name == ".git" || name.ends_with(".iml"),
            "Project location contains {name}; choose an empty directory"
        );
    }
    Ok(())
}

/// Requires neither external executables nor network access. Never runs template scripts.
pub fn generate(root: &Path, cache: &Path, cancel: impl Fn() -> bool) -> Result<()> {
    validate_destination(root)?;
    ensure!(!cancel(), "Project generation cancelled");
    ensure!(
        format!("{:x}", Sha256::digest(SHOWCASE_ARCHIVE)) == SHOWCASE_SHA256,
        "Bundled Cranpose starter is corrupt; reinstall the plugin"
    );
    fs::create_dir_all(cache)?;
    let stage = tempfile::Builder::new()
        .prefix("cranpose-starter-")
        .tempdir_in(cache)?;
    unpack(SHOWCASE_ARCHIVE, stage.path(), &cancel).context("Unpack bundled Cranpose starter")?;
    ensure!(
        stage.path().join("Cargo.toml").is_file() && stage.path().join("src").is_dir(),
        "Starter has no Cargo application"
    );
    install(stage.path(), root, &cancel)?;
    Ok(())
}

fn unpack(bytes: &[u8], destination: &Path, cancel: &impl Fn() -> bool) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(archive.len() <= 2048, "Starter contains too many entries");
    let prefix = format!("cranpose-showcase-{SHOWCASE_REVISION}/");
    let mut total = 0_u64;
    for index in 0..archive.len() {
        ensure!(!cancel(), "Project generation cancelled");
        let mut entry = archive.by_index(index)?;
        let name = entry
            .name()
            .strip_prefix(&prefix)
            .context("Unexpected starter root")?;
        if name.is_empty() && entry.is_dir() {
            continue;
        }
        // Check portable names on every host, including Windows drive/UNC syntax.
        ensure!(
            !name.is_empty()
                && !name.contains(['\\', ':'])
                && name
                    .split('/')
                    .filter(|part| !part.is_empty())
                    .all(|part| part != "." && part != "..")
                && Path::new(name)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "Invalid starter path: {name}"
        );
        let mode = entry.unix_mode().unwrap_or(0);
        let kind = mode & 0o170000;
        ensure!(
            matches!(kind, 0 | 0o100000 | 0o040000),
            "Starter contains a link or special file"
        );
        ensure!(
            name.split('/').next() != Some(".git"),
            "Starter contains Git metadata"
        );
        ensure!(
            entry.size() <= 16 * 1024 * 1024,
            "Starter file is too large"
        );
        total = total
            .checked_add(entry.size())
            .context("Starter size overflow")?;
        ensure!(total <= 64 * 1024 * 1024, "Starter is too large");
        let target = destination.join(name);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
        } else {
            fs::create_dir_all(target.parent().context("Starter parent")?)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            // Reading to EOF also verifies the ZIP CRC before anything is installed.
            std::io::copy(&mut entry, &mut output)?;
            output.flush()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    &target,
                    fs::Permissions::from_mode(if mode & 0o111 != 0 { 0o755 } else { 0o644 }),
                )?;
            }
        }
    }
    Ok(())
}
fn install(source: &Path, destination: &Path, cancel: &impl Fn() -> bool) -> Result<()> {
    validate_destination(destination)?;
    fs::create_dir_all(destination)?;
    let mut created = Vec::new();
    fn copy(
        source: &Path,
        destination: &Path,
        created: &mut Vec<std::path::PathBuf>,
        cancel: &impl Fn() -> bool,
    ) -> Result<()> {
        for entry in fs::read_dir(source)? {
            ensure!(!cancel(), "Project generation cancelled");
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "Starter contains a symlink: {}",
                entry.path().display()
            );
            let target = destination.join(entry.file_name());
            if kind.is_dir() {
                fs::create_dir(&target)?;
                created.push(target.clone());
                copy(&entry.path(), &target, created, cancel)?;
            } else if kind.is_file() {
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                created.push(target.clone());
                std::io::copy(&mut fs::File::open(entry.path())?, &mut output)?;
                output.flush()?;
                fs::set_permissions(&target, entry.metadata()?.permissions())?;
            } else {
                anyhow::bail!("Unsupported starter file");
            }
        }
        Ok(())
    }
    if let Err(error) = copy(source, destination, &mut created, cancel) {
        for path in created.into_iter().rev() {
            if path.is_dir() {
                let _ = fs::remove_dir(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
        return Err(error);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_starter_without_external_tools() {
        // A child isolates PATH from other concurrently running process tests.
        if std::env::var_os("CRANPOSE_STARTER_TEST_CHILD").is_none() {
            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .args([
                    "--exact",
                    "starter::tests::bundled_starter_without_external_tools",
                    "--nocapture",
                ])
                .env("CRANPOSE_STARTER_TEST_CHILD", "1")
                .env("PATH", "")
                .env("HTTP_PROXY", "http://127.0.0.1:1")
                .env("HTTPS_PROXY", "http://127.0.0.1:1")
                .output()
                .expect("child");
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("My project 🦀");
        let cache = temp.path().join("cache");
        fs::create_dir_all(root.join(".idea")).expect("IDE metadata");
        fs::write(root.join(".idea/keep.xml"), "existing").expect("metadata");
        generate(&root, &cache, || false).expect("offline generation");
        let target = desktop_target(&root);
        let manifest = fs::read_to_string(&target.manifest).expect("manifest");
        let manifest = manifest.parse::<toml_edit::DocumentMut>().expect("TOML");
        assert_eq!(
            manifest["package"]["name"].as_str(),
            Some(target.package_name.as_str())
        );
        let binary = manifest["bin"]
            .as_array_of_tables()
            .expect("binaries")
            .iter()
            .find(|bin| bin["name"].as_str() == Some(target.name.as_str()))
            .expect("desktop bin");
        assert_eq!(
            root.join(binary["path"].as_str().expect("entry")),
            Path::new(&target.source)
        );
        let features = binary["required-features"]
            .as_array()
            .expect("features")
            .iter()
            .map(|v| v.as_str().expect("feature").to_owned())
            .collect::<Vec<_>>();
        assert_eq!(features, target.features);
        assert!(manifest["dependencies"].get("cranpose").is_some());
        let mut archive = zip::ZipArchive::new(Cursor::new(SHOWCASE_ARCHIVE)).expect("bundle");
        let prefix = format!("cranpose-showcase-{SHOWCASE_REVISION}/");
        let mut files = 0;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("entry");
            if entry.is_dir() {
                continue;
            }
            let target = root.join(entry.name().strip_prefix(&prefix).expect("prefix"));
            let mut expected = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut expected).expect("read");
            assert_eq!(
                fs::read(&target).expect("generated file"),
                expected,
                "{}",
                target.display()
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&target).expect("mode").permissions().mode() & 0o111,
                    entry.unix_mode().unwrap_or(0) & 0o111
                );
            }
            files += 1;
        }
        assert_eq!(files, 66);
        assert_eq!(
            fs::read_to_string(root.join(".idea/keep.xml")).expect("metadata"),
            "existing"
        );
        assert!(!root.join(".git").exists());
        assert_eq!(fs::read_dir(&cache).expect("cache").count(), 0);
        assert!(
            generate(&root, &cache, || false).is_err(),
            "must not overwrite a project"
        );
    }

    #[test]
    fn cancelled_unpack_never_touches_project() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("project");
        let cache = temp.path().join("cache");
        let count = std::cell::Cell::new(0);
        assert!(
            generate(&root, &cache, || {
                count.set(count.get() + 1);
                count.get() > 10
            })
            .is_err()
        );
        assert!(!root.exists());
        assert_eq!(fs::read_dir(cache).expect("cache").count(), 0);
    }

    #[test]
    fn archive_rejects_escape_links_and_file_directory_collisions() {
        for names in [
            vec!["../escape"],
            vec!["/absolute"],
            vec!["C:/drive"],
            vec!["a\\..\\escape"],
            vec![".git/config"],
            vec!["same", "same/child"],
        ] {
            let temp = tempfile::tempdir().expect("temp");
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for name in names {
                zip.start_file(
                    format!("cranpose-showcase-{SHOWCASE_REVISION}/{name}"),
                    zip::write::SimpleFileOptions::default(),
                )
                .expect("entry");
                zip.write_all(b"data").expect("data");
            }
            let bytes = zip.finish().expect("finish").into_inner();
            assert!(unpack(&bytes, temp.path(), &|| false).is_err());
        }
        let temp = tempfile::tempdir().expect("temp");
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.add_symlink(
            format!("cranpose-showcase-{SHOWCASE_REVISION}/link"),
            "../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("link");
        assert!(
            unpack(
                &zip.finish().expect("finish").into_inner(),
                temp.path(),
                &|| false
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_existing_code_and_preserves_ide_metadata() {
        let temp = tempfile::tempdir().expect("temp");
        let src = temp.path().join("source");
        let dst = temp.path().join("destination");
        fs::create_dir_all(&src).expect("source");
        fs::write(src.join("Cargo.toml"), "starter").expect("file");
        fs::create_dir_all(dst.join(".idea")).expect("idea");
        install(&src, &dst, &|| false).expect("install");
        assert_eq!(
            fs::read_to_string(dst.join("Cargo.toml")).expect("manifest"),
            "starter"
        );
        assert!(install(&src, &dst, &|| false).is_err());
        assert!(dst.join(".idea").is_dir());
    }
    #[test]
    fn cancellation_does_not_leave_partial_files() {
        let temp = tempfile::tempdir().expect("temp");
        let src = temp.path().join("source");
        let dst = temp.path().join("destination");
        fs::create_dir(&src).expect("source");
        for n in 0..3 {
            fs::write(src.join(n.to_string()), "fixture").expect("file");
        }
        let count = std::cell::Cell::new(0);
        assert!(
            install(&src, &dst, &|| {
                count.set(count.get() + 1);
                count.get() > 1
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(dst).expect("destination").count(), 0);
    }
}
