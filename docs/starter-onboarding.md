# From New Project to the first preview

Consumers that enable `HostFeatures::cargo` get a complete desktop handoff after
the bundled Showcase is generated. The Rust host saves and selects a persistent
Cranpose Cargo configuration, publishes the pinned starter's desktop target,
opens `main.rs` beside its preview, and sends one `showTarget` command. The normal
preview runner owns compilation, progress, cancellation and process cleanup.

This handoff is triggered only by successful wizard generation. Reopening a
project or refreshing metadata does not automatically run it. The IDE trust gate
still applies. Repeated configuration creation reuses the matching configuration
and preserves customized arguments and environment variables.

The desktop target is available before Cargo discovery. Archive tests verify its
package, binary, source and required features against the bundled manifest.
Missing Rust is reported as a `studio.child` stopped event with `setup: "rust"`.
Consumers should offer `setupRust` (opens the official Rust installation page)
and a normal start/retry action. The host checks Cargo again on every attempt,
including the rustup home directory and `CARGO_HOME`, and passes its directory
to the preview runner's PATH. No IDE restart is needed for a standard rustup
installation. Rust's native platform prerequisites and dependency downloads
remain necessary for compilation.

On Windows the host then checks the Visual Studio installation Rust links
with: the MSVC build tools, a Windows SDK and, for x64 builds, ATL, whose
`atls.lib` the static DirectX shader compiler of Cranpose's x64 renderer
links. A missing part is reported as a stopped event with `setup: "windows"`,
the Visual Studio Installer's names of the missing components in `missing` and
the install command as a PowerShell line in `command`. Consumers should offer
`setupWindowsTools` and a retry. The action starts the Visual Studio Installer
for that installation, or winget on a PC without Visual Studio, and opens the
Build Tools page when neither starts. A PC found complete is not checked again
while the IDE runs. `cranpose-plugin-toolchain` holds the check and the
command.

On Windows the development cache is `%LOCALAPPDATA%\cranpose-dev`. Under the
IDE's system directory, a preview's object files passed the 260 characters
MSVC's linker accepts.

The integration suite verifies persistent selection, duplicate prevention and
preservation of edited run options using the real IntelliJ RunManager. Generation
continues to work without Git, Rust or network access.
