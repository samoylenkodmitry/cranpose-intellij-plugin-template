use anyhow::Result;
use cranpose_plugin_cache::WorkspaceLease;
use std::fs;

#[test]
fn concurrent_cleanup_preserves_other_artifacts_and_clean_reuse_keeps_cache() -> Result<()> {
    let root = tempfile::tempdir()?;
    let first = WorkspaceLease::acquire(root.path(), b"project")?;
    let second = WorkspaceLease::acquire(root.path(), b"project")?;
    let first_artifacts = first.artifacts_path();
    fs::create_dir_all(&first_artifacts)?;
    fs::create_dir_all(second.artifacts_path())?;
    fs::write(first_artifacts.join("invoked.timestamp"), "building")?;
    fs::write(first.path().join("old-source.rs"), "old source")?;
    fs::remove_dir_all(second.artifacts_path())?;
    assert_eq!(
        fs::read_to_string(first_artifacts.join("invoked.timestamp"))?,
        "building"
    );
    first.complete()?;
    let reused = WorkspaceLease::acquire(root.path(), b"project")?;
    assert_eq!(reused.artifacts_path(), first_artifacts);
    assert!(!reused.path().join("old-source.rs").exists());
    assert_eq!(
        fs::read_to_string(reused.artifacts_path().join("invoked.timestamp"))?,
        "building"
    );
    Ok(())
}

#[test]
fn abandoned_artifacts_are_not_reused_or_removed() -> Result<()> {
    let root = tempfile::tempdir()?;
    let abandoned = WorkspaceLease::acquire(root.path(), b"project")?;
    let artifacts = abandoned.artifacts_path();
    fs::create_dir_all(&artifacts)?;
    fs::write(artifacts.join("active"), "descendant may still be building")?;
    drop(abandoned);
    let next = WorkspaceLease::acquire(root.path(), b"project")?;
    assert_ne!(next.artifacts_path(), artifacts);
    assert_eq!(
        fs::read_to_string(artifacts.join("active"))?,
        "descendant may still be building"
    );
    Ok(())
}
