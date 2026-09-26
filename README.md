# Cranpose IntelliJ plugin template

Build an IntelliJ Platform plugin in Rust, with its UI drawn by
[Cranpose](https://github.com/samoylenkodmitry/Cranpose).
For developing Cranpose applications, use [Cranpose for IntelliJ IDEA](https://github.com/samoylenkodmitry/cranpose-idea).

<img src="docs/effects.gif" width="100%" alt="Template tool window, editor shader effects and draggable floating orb">

The UI, native IDE host, build tools, integration tests and release tools are Rust.
Rust generates the small JVM extension classes that IntelliJ requires. There is no
authored Java or Kotlin and no Gradle build. IntelliJ itself still supplies its JVM
and platform APIs.

## Architecture

The shared Rust host calls the IntelliJ SDK through JNI. A separate Cranpose process
draws the UI and sends BGRA frames over an authenticated loopback connection. The
host displays those pixels and forwards mouse, keyboard, scale and visibility events.

- The UI process is isolated from the IDE. Click a stopped surface to restart it.
- Dirty rectangles, frame acknowledgements and visibility control keep idle work low.
- The same Cranpose UI runs in a tool window or a standalone desktop window.
- Editor overlays and transparent floating windows use Cranpose surfaces.
- A debug UI binary restarts after Cargo replaces it and the file settles.
- CI packages the native host and UI for macOS, Linux and Windows, on both ARM64 and x86-64.

## Develop

Run the UI as a desktop window:

```bash
cargo run -p cranpose-intellij-ui
```

Build an installable plugin for the current machine:

```bash
cargo run -p cranpose-template-tools -- package
```

Install the resulting ZIP from `target/plugin/` using **Settings → Plugins → Install
Plugin from Disk**, then restart the IDE. Supported platform baseline: 2026.1.

For fast UI iteration, add this VM option to your development IDE:

```text
-Dcranpose.ui.binary=/absolute/path/to/target/debug/cranpose-intellij-ui
```

Then run `cargo build -p cranpose-intellij-ui --no-default-features` after UI edits.
The host watches the binary and restarts its UI automatically. Host changes require
repackaging and restarting the IDE.

## Layout

| Path | Purpose |
|---|---|
| `ui/src/tool_window.rs` | Cranpose tool window |
| `ui/src/effects.rs`, `effects.wgsl` | Five editor effect styles |
| `ui/src/orb.rs` | Draggable transparent shader window |
| `ui/src/ide.rs` | JSON channel contract |
| `host/src/lib.rs` | Rust JNI entry point and optional custom host messages |
| `xtask/src/main.rs` | Plugin identity and shared Rust build tools |
| `plugin/src/main/resources/META-INF/plugin.xml` | SDK extension registration |
| `plugin/VERSION` | Plugin release version |

The host and build tools are pinned Rust dependencies from
[cranpose-idea](https://github.com/samoylenkodmitry/cranpose-idea).
Their source is in `ide-host/`, `jvm-bridge/` and `xtask/` there.
This template disables that plugin's Cargo dashboard and stability analysis.

## Make it yours

1. Change the ID, display name, vendor and tool-window name in `plugin.xml`.
2. Set the same plugin ID in `xtask/src/main.rs` and choose your package directory name.
3. Write your composables in `ui/src/`.
4. Add host channels in Rust; see [Extending the host](docs/extending-the-host.md).
5. Set `plugin/VERSION` before creating a release.

Keep the native library and UI executable names unless you also extend the shared
packager and binary locator.

## Test and release

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p cranpose-template-tools -- release audit-source
cargo run -p cranpose-template-tools -- ide-test --ide "/Applications/IntelliJ IDEA.app"
```

The integration suite launches a separate headless IDEA, loads the generated JVM
classes and native host, checks project lifecycle and pixel updates, and renders
the real Cranpose UI. Evidence is saved under `target/ide-tests/`. Linux needs
`mesa-vulkan-drivers`. An isolated RustRover test installation requires its own license;
use IDEA for headless tests.

The **Build** workflow compiles all six platforms, runs Rust and native IDE tests,
assembles the ZIP and runs JetBrains Plugin Verifier against IDEA and RustRover.
The nightly **Cranpose main** workflow runs the same integration suite against the
framework's current main branch.

The **Publish** workflow is manual. It requires a matching version tag and an already
successful Build for that exact commit. It downloads the verified ZIP, signs it,
and uploads it to the numeric Marketplace plugin ID you supply. Configure
`PUBLISH_TOKEN`, `CERTIFICATE_CHAIN`, `PRIVATE_KEY` and `PRIVATE_KEY_PASSWORD`.
For a new Marketplace listing, upload the first ZIP manually and then use its ID.

## Messages and effects

| Channel | Direction | Content |
|---|---|---|
| `ide.theme` | Host → UI | IDE palette and dark mode |
| `ide.editor` | Host → UI | Selected file path and name |
| `ide.caret` | Host → UI | Typing, deletion or movement, with caret coordinates |
| `ide.notify` | UI → host | Notification title and content |
| `ide.open` | UI → host | File path |
| `host.overlay` | UI → host | Cranpose overlay declaration |

Send with `cranpose::send_to_host` and receive with `rememberHostMessages`.
`HostOverlay("editor")` places a transparent, input-free surface over the selected
editor. `Modifier::window` opens an IDE-owned surface; `window_drag_area` makes the
floating orb draggable.

The included styles are sparks, water, ice, cosmic and party. Each has effects for
typing, deletion and caret movement. Add an `EffectKind` and a WGSL case to extend
them. WGSL remains the GPU shader language; application and tooling code is Rust.

[Full-quality effects video](docs/effects.mp4) · [Effect reference image](docs/effects.png)

## Credits

Built with Cranpose and the IntelliJ Platform SDK. Apache-2.0.
