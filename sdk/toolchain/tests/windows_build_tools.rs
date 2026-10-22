use cranpose_plugin_toolchain::{
    ATL_X64, Arch, MSVC_ARM64, MSVC_X64, WINDOWS_SDK, WindowsBuildTools, missing_for_link_error,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

const TOOLSET: &str = "14.51.36231";

fn touch(path: PathBuf) -> PathBuf {
    fs::create_dir_all(path.parent().expect("parent")).expect("directories");
    fs::write(&path, b"").expect("file");
    path
}

/// A Visual Studio installation with the MSVC linker for `arch`, and ATL
/// when `atl`.
fn installation(root: &Path, name: &str, arch: &str, atl: bool) -> PathBuf {
    let installation = root.join(name);
    let build = installation.join("VC").join("Auxiliary").join("Build");
    fs::create_dir_all(&build).expect("build directory");
    fs::write(
        build.join("Microsoft.VCToolsVersion.default.txt"),
        format!("{TOOLSET}\r\n"),
    )
    .expect("default toolset");
    let toolset = installation
        .join("VC")
        .join("Tools")
        .join("MSVC")
        .join(TOOLSET);
    let host = if arch == "arm64" {
        "Hostarm64"
    } else {
        "Hostx64"
    };
    touch(toolset.join("bin").join(host).join(arch).join("link.exe"));
    if atl {
        touch(
            toolset
                .join("atlmfc")
                .join("lib")
                .join(arch)
                .join("atls.lib"),
        );
    }
    installation
}

/// The Visual Studio Installer of a PC that never added the C++ workload.
fn installation_without_msvc(root: &Path, name: &str) -> PathBuf {
    let installation = root.join(name);
    fs::create_dir_all(&installation).expect("installation");
    installation
}

fn windows_kits(root: &Path, arch: &str) -> PathBuf {
    let kits = root.join("Windows Kits").join("10");
    touch(
        kits.join("Lib")
            .join("10.0.26100.0")
            .join("um")
            .join(arch)
            .join("kernel32.lib"),
    );
    kits
}

#[test]
fn a_complete_installation_builds_and_installs_nothing() {
    let root = tempfile::tempdir().expect("root");
    let tools = WindowsBuildTools::inspect(
        &[installation(root.path(), "BuildTools", "x64", true)],
        Some(&windows_kits(root.path(), "x64")),
        Arch::X64,
    );
    assert!(tools.ready(), "{tools:?}");
    assert_eq!(tools.install(), None);
}

#[test]
fn build_tools_without_atl_add_atl_to_that_installation() {
    let root = tempfile::tempdir().expect("root");
    let build_tools = installation(root.path(), "Build Tools", "x64", false);
    let tools = WindowsBuildTools::inspect(
        std::slice::from_ref(&build_tools),
        Some(&windows_kits(root.path(), "x64")),
        Arch::X64,
    );
    assert_eq!(tools.missing, vec![ATL_X64]);
    assert_eq!(tools.installation.as_deref(), Some(build_tools.as_path()));
    let install = tools.install().expect("an install command");
    assert_eq!(
        install.program.file_name().and_then(|name| name.to_str()),
        Some("setup.exe")
    );
    assert_eq!(
        install.args,
        [
            "modify",
            "--installPath",
            &build_tools.display().to_string(),
            "--add",
            ATL_X64.id,
            "--norestart",
        ]
    );
    let line = install.powershell();
    assert!(
        line.contains(&format!("\"{}\"", build_tools.display())),
        "a path with a space is quoted: {line}"
    );
}

#[test]
fn a_pc_without_visual_studio_installs_the_build_tools_with_atl() {
    let tools = WindowsBuildTools::inspect(&[], None, Arch::X64);
    assert_eq!(tools.missing, vec![MSVC_X64, WINDOWS_SDK, ATL_X64]);
    let install = tools.install().expect("an install command");
    assert_eq!(install.program, PathBuf::from("winget"));
    assert_eq!(
        install.args[..4],
        [
            "install",
            "--id",
            "Microsoft.VisualStudio.BuildTools",
            "--override"
        ]
    );
    assert!(install.args[4].contains("Microsoft.VisualStudio.Workload.VCTools"));
    assert!(install.args[4].contains(ATL_X64.id));
}

#[test]
fn rust_links_with_the_first_installation_that_has_a_linker() {
    let root = tempfile::tempdir().expect("root");
    let empty = installation_without_msvc(root.path(), "Community");
    let complete = installation(root.path(), "BuildTools", "x64", true);
    let tools = WindowsBuildTools::inspect(
        &[empty, complete.clone()],
        Some(&windows_kits(root.path(), "x64")),
        Arch::X64,
    );
    assert!(tools.ready(), "{tools:?}");
    assert_eq!(tools.installation, Some(complete));
}

#[test]
fn atl_in_a_later_installation_does_not_help_the_one_that_links() {
    let root = tempfile::tempdir().expect("root");
    let linking = installation(root.path(), "Enterprise", "x64", false);
    let other = installation(root.path(), "BuildTools", "x64", true);
    let tools = WindowsBuildTools::inspect(
        &[linking.clone(), other],
        Some(&windows_kits(root.path(), "x64")),
        Arch::X64,
    );
    assert_eq!(tools.missing, vec![ATL_X64]);
    assert_eq!(tools.installation, Some(linking));
}

#[test]
fn an_installation_without_the_cpp_workload_gets_msvc_and_atl() {
    let root = tempfile::tempdir().expect("root");
    let community = installation_without_msvc(root.path(), "Community");
    let tools = WindowsBuildTools::inspect(
        std::slice::from_ref(&community),
        Some(&windows_kits(root.path(), "x64")),
        Arch::X64,
    );
    assert_eq!(tools.missing, vec![MSVC_X64, ATL_X64]);
    let install = tools.install().expect("an install command");
    assert_eq!(install.args[0], "modify");
    assert!(install.args.contains(&MSVC_X64.id.to_owned()));
    assert!(install.args.contains(&ATL_X64.id.to_owned()));
}

#[test]
fn a_missing_windows_sdk_is_added() {
    let root = tempfile::tempdir().expect("root");
    let tools = WindowsBuildTools::inspect(
        &[installation(root.path(), "BuildTools", "x64", true)],
        Some(&root.path().join("no kits")),
        Arch::X64,
    );
    assert_eq!(tools.missing, vec![WINDOWS_SDK]);
}

#[test]
fn arm64_builds_need_no_atl() {
    let root = tempfile::tempdir().expect("root");
    let tools = WindowsBuildTools::inspect(
        &[installation(root.path(), "BuildTools", "arm64", false)],
        Some(&windows_kits(root.path(), "arm64")),
        Arch::Arm64,
    );
    assert!(tools.ready(), "{tools:?}");
    let bare = WindowsBuildTools::inspect(&[], None, Arch::Arm64);
    assert_eq!(bare.missing, vec![MSVC_ARM64, WINDOWS_SDK]);
}

#[test]
fn detection_finishes_on_every_host() {
    let tools = WindowsBuildTools::detect();
    if !cfg!(windows) {
        assert!(tools.ready(), "only Windows needs Visual Studio: {tools:?}");
    }
}

#[test]
fn link_errors_name_the_missing_component() {
    assert_eq!(
        missing_for_link_error(
            "          = note: LINK : fatal error LNK1104: cannot open file 'atls.lib'"
        ),
        Some(ATL_X64)
    );
    assert_eq!(
        missing_for_link_error("error: linker `link.exe` not found"),
        Some(Arch::host().components()[0])
    );
    assert_eq!(
        missing_for_link_error("LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'"),
        Some(WINDOWS_SDK)
    );
    assert_eq!(
        missing_for_link_error("error[E0425]: cannot find value `atls` in this scope"),
        None
    );
}
