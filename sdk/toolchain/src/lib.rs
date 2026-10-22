//! The C++ build tools a native Windows build of a Cranpose app links with,
//! and the command that installs the parts a PC lacks.
//!
//! Rust's MSVC targets link with the `link.exe` of a Visual Studio
//! installation against a Windows SDK. Cranpose's x64 renderer also links
//! Microsoft's DirectX shader compiler, which references ATL, so the toolset
//! that links must carry `atls.lib`. A PC with the Build Tools of a Rust
//! install but no ATL fails its first Cranpose build with
//! `LNK1104: cannot open file 'atls.lib'`.
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// A Visual Studio component a Windows build needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Component {
    /// The installer's ID, as `--add` takes it.
    pub id: &'static str,
    /// The name the Visual Studio Installer shows.
    pub title: &'static str,
}

/// MSVC compilers and linker for x64 programs.
pub const MSVC_X64: Component = Component {
    id: "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
    title: "MSVC Build Tools for x64/x86 (Latest)",
};
/// MSVC compilers and linker for ARM64 programs.
pub const MSVC_ARM64: Component = Component {
    id: "Microsoft.VisualStudio.Component.VC.Tools.ARM64",
    title: "MSVC Build Tools for ARM64/ARM64EC (Latest)",
};
/// The Windows SDK, whose import libraries such as `kernel32.lib` every
/// program links.
pub const WINDOWS_SDK: Component = Component {
    id: "Microsoft.VisualStudio.Component.Windows11SDK.26100",
    title: "Windows 11 SDK (10.0.26100)",
};
/// ATL, whose `atls.lib` the x64 shader compiler links.
pub const ATL_X64: Component = Component {
    id: "Microsoft.VisualStudio.Component.VC.ATL",
    title: "C++ ATL for x64/x86 (Latest MSVC)",
};

/// The page that offers the Build Tools when no installer command can run.
pub const BUILD_TOOLS_PAGE: &str = "https://visualstudio.microsoft.com/visual-cpp-build-tools/";

/// The architecture a build produces programs for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
}

impl Arch {
    /// This host's architecture, the one Cargo builds for by default.
    pub fn host() -> Self {
        if cfg!(target_arch = "aarch64") {
            Self::Arm64
        } else {
            Self::X64
        }
    }
    fn directory(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::Arm64 => "arm64",
        }
    }
    fn host_tools(self) -> &'static str {
        match self {
            Self::X64 => "Hostx64",
            Self::Arm64 => "Hostarm64",
        }
    }
    fn msvc(self) -> Component {
        match self {
            Self::X64 => MSVC_X64,
            Self::Arm64 => MSVC_ARM64,
        }
    }
}

/// What a PC's Visual Studio lacks for a native build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsBuildTools {
    /// The installation whose linker Rust uses, or else the first one found.
    pub installation: Option<PathBuf>,
    /// The components to add. Empty when the PC builds.
    pub missing: Vec<Component>,
    arch: Arch,
}

impl WindowsBuildTools {
    /// Inspects this PC for a build for [`Arch::host`]. Other systems take
    /// none of these tools, so nothing is missing there.
    pub fn detect() -> Self {
        if !cfg!(windows) {
            return Self {
                installation: None,
                missing: Vec::new(),
                arch: Arch::host(),
            };
        }
        Self::inspect(&installations(), windows_kits().as_deref(), Arch::host())
    }

    /// Inspects `installations`, in the order `vswhere` lists them, which is
    /// the order in which Rust looks for a linker, and the Windows SDK under
    /// `windows_kits` (`...\Windows Kits\10`).
    pub fn inspect(installations: &[PathBuf], windows_kits: Option<&Path>, arch: Arch) -> Self {
        let (installation, toolset) = match installations
            .iter()
            .find_map(|installation| Some((installation, toolset(installation, arch)?)))
        {
            Some((installation, toolset)) => (Some(installation.clone()), Some(toolset)),
            None => (installations.first().cloned(), None),
        };
        let mut missing = Vec::new();
        if toolset.is_none() {
            missing.push(arch.msvc());
        }
        if !windows_kits.is_some_and(|kits| has_sdk(kits, arch)) {
            missing.push(WINDOWS_SDK);
        }
        let atl = toolset.is_some_and(|toolset| {
            toolset
                .join("atlmfc")
                .join("lib")
                .join(arch.directory())
                .join("atls.lib")
                .is_file()
        });
        // ARM64 builds do not link the static shader compiler.
        if arch == Arch::X64 && !atl {
            missing.push(ATL_X64);
        }
        Self {
            installation,
            missing,
            arch,
        }
    }

    pub fn ready(&self) -> bool {
        self.missing.is_empty()
    }

    /// The command that installs what is missing: the Visual Studio
    /// Installer adds the components to the installation found, and winget
    /// installs the Build Tools with them on a PC without one. `None` when
    /// nothing is missing.
    pub fn install(&self) -> Option<Install> {
        if self.ready() {
            return None;
        }
        Some(match &self.installation {
            Some(installation) => {
                let mut args = vec![
                    "modify".to_owned(),
                    "--installPath".to_owned(),
                    installation.display().to_string(),
                ];
                for component in &self.missing {
                    args.extend(["--add".to_owned(), component.id.to_owned()]);
                }
                args.extend(["--passive".to_owned(), "--norestart".to_owned()]);
                Install {
                    program: installer_directory().join("setup.exe"),
                    args,
                }
            }
            None => {
                let mut components = vec![
                    "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
                        .to_owned(),
                ];
                if self.arch == Arch::Arm64 {
                    components.push(format!("--add {}", MSVC_ARM64.id));
                } else {
                    components.push(format!("--add {}", ATL_X64.id));
                }
                Install {
                    program: PathBuf::from("winget"),
                    args: vec![
                        "install".to_owned(),
                        "--id".to_owned(),
                        "Microsoft.VisualStudio.BuildTools".to_owned(),
                        "--override".to_owned(),
                        components.join(" "),
                    ],
                }
            }
        })
    }
}

/// A command that installs Visual Studio components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Install {
    /// The command as a PowerShell line, for a person to read or paste.
    pub fn powershell(&self) -> String {
        std::iter::once(format!("& {}", quote(&self.program.display().to_string())))
            .chain(self.args.iter().map(|arg| quote(arg)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Starts the command without waiting. The installer shows its own
    /// window and asks for administrator rights.
    pub fn spawn(&self) -> std::io::Result<()> {
        Command::new(&self.program)
            .args(&self.args)
            .spawn()
            .map(drop)
    }
}

fn quote(text: &str) -> String {
    if text.contains([' ', '(', ')', '\'', '"', '&', ';']) {
        format!("\"{}\"", text.replace('"', "`\""))
    } else {
        text.to_owned()
    }
}

/// The MSVC toolset of `installation` that links for `arch`: the default
/// version Rust picks, when its linker is there.
fn toolset(installation: &Path, arch: Arch) -> Option<PathBuf> {
    let version = std::fs::read_to_string(
        installation
            .join("VC")
            .join("Auxiliary")
            .join("Build")
            .join("Microsoft.VCToolsVersion.default.txt"),
    )
    .ok()?;
    let toolset = installation
        .join("VC")
        .join("Tools")
        .join("MSVC")
        .join(version.trim());
    toolset
        .join("bin")
        .join(arch.host_tools())
        .join(arch.directory())
        .join("link.exe")
        .is_file()
        .then_some(toolset)
}

fn has_sdk(kits: &Path, arch: Arch) -> bool {
    std::fs::read_dir(kits.join("Lib")).is_ok_and(|versions| {
        versions.flatten().any(|version| {
            version
                .path()
                .join("um")
                .join(arch.directory())
                .join("kernel32.lib")
                .is_file()
        })
    })
}

fn program_files_x86() -> PathBuf {
    std::env::var_os("ProgramFiles(x86)")
        .map_or_else(|| PathBuf::from(r"C:\Program Files (x86)"), PathBuf::from)
}

fn installer_directory() -> PathBuf {
    program_files_x86()
        .join("Microsoft Visual Studio")
        .join("Installer")
}

fn output(command: Command) -> Option<String> {
    cranpose_plugin_process::capture(command, None, Duration::from_secs(20), || false)
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Installation directories of Visual Studio and its Build Tools.
fn installations() -> Vec<PathBuf> {
    let vswhere = installer_directory().join("vswhere.exe");
    if !vswhere.is_file() {
        return Vec::new();
    }
    let mut command = Command::new(vswhere);
    command.args(["-products", "*", "-property", "installationPath", "-utf8"]);
    output(command).map_or_else(Vec::new, |text| {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect()
    })
}

/// The Windows SDK root the registry names, or its default place.
fn windows_kits() -> Option<PathBuf> {
    let mut command = Command::new("reg");
    command.args([
        "query",
        r"HKLM\SOFTWARE\Microsoft\Windows Kits\Installed Roots",
        "/v",
        "KitsRoot10",
    ]);
    output(command)
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.split_once("REG_SZ"))
                .map(|(_, root)| PathBuf::from(root.trim()))
        })
        .or_else(|| Some(program_files_x86().join("Windows Kits").join("10")))
}
