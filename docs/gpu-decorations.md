# GPU editor decorations

Recomposition counters and edit-to-preview lightning share a Cranpose composition in each IDE window. IntelliJ block inlays reserve space above source definitions and retain the source anchors. Cranpose draws the visible counter text and shaders.

## Presentation

| Desktop | Native presentation | GPU backend |
| --- | --- | --- |
| macOS | Input-transparent NSView with CAMetalLayer | Metal |
| Windows | DirectComposition visual on the existing AWT HWND | DX12 |
| Linux X11 | Owned ARGB window with an empty input region | Vulkan; compositor required |
| Linux Wayland | Input-transparent child wl_surface and subsurface | Vulkan; JetBrains Runtime WLToolkit |

The Wayland bridge checks the runtime accessors before using their handles. It borrows the existing display without disconnecting it, owns its child surface, and uses wp_viewporter when available to map physical buffer pixels into the parent's surface units. Incompatible runtimes, missing compositors and device failures retain static IntelliJ counter text and log the specific backend failure. They do not start an animated CPU-image fallback.

The Metal view is created from the render worker and dispatched to AppKit. This leaves the AWT event thread available for accessibility callbacks. The view and native surface stay retained until the swap chain is released. Native layers have no input region/hit target.

Presentation prefers Mailbox when the adapter supports it, with FIFO as the fallback. The worker supplies the 60 Hz frame limit. This avoids FIFO blocking subsequent frames on JBR's shared Wayland connection.

## Work and lifetime

- One worker and one latest-scene mailbox per window. JNI carries geometry and labels; rendered frame pixels never cross JNI.
- Scrolling, folding, inlay changes, editor/window layout, theme and menu events refresh geometry. Counters are clipped to their editor's viewport. Animation identity survives geometry changes.
- Counters pulse for 700 ms after a label changes. Completion lightning lasts 850 ms. Pending feedback is bounded to 10 seconds. Rendering is capped at 60 Hz and sleeps when settled.
- The surface covers the union of visible decorations. Physical dimensions are checked against device limits and a 16-megapixel budget. Glyph atlases and shader resources are cached by the Cranpose renderer.
- Inactive windows and open Swing menus hide decorations. Stopping all previews releases the window renderer. Project disposal detaches listeners and stops workers without joining them on the EDT.
- This is plugin-host rendering. Application release builds gain no tracking or rendering dependency from this bridge.

## Validation

```console
cargo run -p cranpose-template-tools -- gpu-probe native --direct --java-home <jbr> --output target/gpu-probe/native
cargo run -p cranpose-template-tools -- gpu-probe native --direct --effects --java-home <jbr> --output target/gpu-probe/effects
```

The first probe verifies native transparency, translated placement, bounds and real mouse passthrough. On macOS it compares blending with an independent Core Animation color layer, since native color-managed compositing differs from Java2D's already blended RGB pixels. The second verifies actual text pixels, changed digits, movement without stale pixels, visible lightning, input and zero submitted frames after settling. Screen readback occurs only in these tests.

X11 runs can use `--toolkit x11 --isolated` with Xvfb and picom. Wayland runs use `--toolkit wayland --isolated --weston-root <extracted-weston-15>`, with compositor capture rather than AWT Robot's Java-only buffer. Desktop GPU tests require an interactive session; an SSH service session on Windows cannot create the user's DirectComposition target.

The IDE integration suite also checks the actual editor listener signatures, stable source anchors, split editors and removal of Java2D text when GPU drawing is active. Shader validation and animation-clock tests run in the Rust suite.
