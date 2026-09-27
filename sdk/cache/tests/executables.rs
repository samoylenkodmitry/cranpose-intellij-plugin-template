//! Keep subprocess launches separate from unit lease tests: Unix fork briefly
//! inherits unrelated OS locks until exec closes their descriptors.
use anyhow::Result;
use cranpose_plugin_cache::WorkspaceLease;

#[test]
fn staged_tool_keeps_its_distinct_executable_path() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lease = WorkspaceLease::acquire(root.path(), b"executable")?;
    let source = std::env::current_exe()?;
    let alias = lease.stage_executable(&source)?;
    assert_ne!(alias.canonicalize()?, source.canonicalize()?);
    let status = std::process::Command::new(&alias)
        .args(["--exact", "alias_child", "--ignored", "--nocapture"])
        .env("CRANPOSE_ALIAS_TEST_PATH", &alias)
        .status()?;
    assert!(status.success());
    lease.complete()?;
    Ok(())
}
#[test]
#[ignore = "subprocess fixture"]
fn alias_child() -> Result<()> {
    let expected =
        std::path::PathBuf::from(std::env::var_os("CRANPOSE_ALIAS_TEST_PATH").expect("alias"));
    assert_eq!(
        std::env::current_exe()?.canonicalize()?,
        expected.canonicalize()?
    );
    Ok(())
}
