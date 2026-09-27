//! Publish generated files once, preserving Cargo fingerprints on later launches.
mod derived;
mod lease;
use anyhow::{Context, Result, ensure};
pub use derived::DerivedFile;
pub use lease::WorkspaceLease;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Materialize an immutable bundle, addressed by its sorted paths and contents.
/// Existing bundles are verified and reused without writes or timestamp changes.
/// A complete staging directory is renamed into place; concurrent publishers
/// reuse the winner. Different contents never replace an in-use version.
/// The caller owns `root` and must keep it outside application source directories.
pub fn materialize(root: &Path, files: &[(&str, &[u8])]) -> Result<PathBuf> {
    ensure!(!files.is_empty(), "An asset bundle must contain files");
    let mut entries = BTreeMap::new();
    for &(name, contents) in files {
        ensure!(
            !name.contains(['\\', ':'])
                && name
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != ".."),
            "Asset path must be an unambiguous relative file: {name}"
        );
        ensure!(
            entries.insert(name, contents).is_none(),
            "Duplicate asset path: {name}"
        );
    }
    for name in entries.keys() {
        for (offset, _) in name.match_indices('/') {
            ensure!(
                !entries.contains_key(&name[..offset]),
                "Asset is both file and directory: {}",
                &name[..offset]
            );
        }
    }
    let mut hash = Sha256::new();
    hash.update(b"cranpose-plugin-assets-v1\0");
    for (&name, &contents) in &entries {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let destination = root.join(format!("{:x}", hash.finalize()));
    if destination.try_exists()? {
        verify(&destination, &entries)?;
        return Ok(destination);
    }
    let staging = tempfile::Builder::new()
        .prefix(".pending-")
        .tempdir_in(&root)?;
    for (&name, &contents) in &entries {
        let path = staging.path().join(name);
        fs::create_dir_all(path.parent().context("asset parent")?)?;
        fs::write(&path, contents)?;
    }
    // Detect filesystem aliases (for example case-insensitive paths) before publication.
    verify(staging.path(), &entries)?;
    if let Err(error) = fs::rename(staging.path(), &destination) {
        // Another complete publisher may have won the atomic rename race.
        if !destination.try_exists()? {
            return Err(error.into());
        }
    }
    verify(&destination, &entries)?;
    Ok(destination)
}

fn verify(root: &Path, expected: &BTreeMap<&str, &[u8]>) -> Result<()> {
    let mut directories = vec![root.to_owned()];
    let mut seen = 0;
    while let Some(directory) = directories.pop() {
        ensure!(
            fs::symlink_metadata(&directory)?.file_type().is_dir(),
            "Cached asset directory is not a regular directory: {}",
            directory.display()
        );
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                directories.push(path);
            } else {
                ensure!(
                    kind.is_file(),
                    "Cached asset is not a regular file: {}",
                    path.display()
                );
                let relative = path
                    .strip_prefix(root)?
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let contents = expected
                    .get(relative.as_str())
                    .with_context(|| format!("Unexpected cached asset: {}", path.display()))?;
                ensure!(
                    fs::read(&path)?.as_slice() == *contents,
                    "Cached asset contents changed: {}",
                    path.display()
                );
                seen += 1;
            }
        }
    }
    ensure!(
        seen == expected.len(),
        "Cached asset bundle is incomplete: {}",
        root.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests;
