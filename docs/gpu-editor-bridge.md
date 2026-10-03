# GPU editor presentation

The current embedded protocol sends BGRA pixels. `Surface::frame` copies them
into a Java `BufferedImage`, including the lightning overlay. The editor's
counter badges currently draw through Java2D. Neither path imports Cranpose
textures into the editor.

## Verified capability

Run the installed runtime probe:

```sh
python3 sdk/tools/probes/shared_textures.py --ide /path/to/ide --output target/gpu-probe
```

The report distinguishes an unavailable service from a successful presentation
test. On macOS the test creates a private Metal texture, clears it on the GPU,
wraps it with `JBR.getSharedTextures().wrapTexture`, and draws into an accelerated
Java2D image. Readback is used only to assert the resulting alpha, clipping and
translated placement. Production presentation must not perform that readback.

The tested RustRover runtime is `25.0.4+1-b508.27`. It supports shared Metal
textures. At display scale 2, a 64 by 32 texture becomes a Java image reporting
32 by 16 logical points. The host must not apply that density conversion twice.

## Platform boundary

JetBrains' public SharedTextures service currently implements Metal only.
The Vulkan service reports devices and capabilities but exposes no texture
import or completion primitives. Windows and Linux need either a pixel
presentation path on stock runtimes, native presentation with separate window
integration, or an extension to the runtime's texture import API. Those are
different deployment and support commitments; a missing service must never be
reported as successful GPU sharing.

## Implementation requirements

- Preserve IntelliJ inlay anchors and paint-time rectangles for scrolling,
  folding, soft wraps, split editors and HiDPI changes.
- Render counter visuals and lightning with one Cranpose presentation bridge.
- Carry geometry and state updates rather than rendered pixels on a GPU path.
- Share a renderer across visible decorations; bound retained texture memory.
- Retain textures until both producer writes and consumer reads complete.
  A frame acknowledgment or elapsed delay alone does not establish GPU completion.
- Keep producer waits off the editor thread. Present the latest completed frame.
- Reuse wrappers while their texture and graphics configuration remain valid.
- Stop animation work when hidden, idle, disposed or reduced motion is enabled.
- Preserve animation time when scroll updates move the lightning endpoints.
- Measure GPU readback bytes, uploads, retained memory, idle frames and editor
  responsiveness, and validate actual platform presentation before enabling it.

This probe validates the macOS interop primitive. It does not implement the
production bridge, the Windows/Linux presenter, or the animated inlay migration.

References:

- [JetBrains SharedTextures API](https://jetbrains.github.io/JetBrainsRuntimeApi/jetbrains.runtime.api/com/jetbrains/SharedTextures.html)
- [JetBrains Vulkan API](https://jetbrains.github.io/JetBrainsRuntimeApi/jetbrains.runtime.api/com/jetbrains/Vulkan.html)
- [IntelliJ inlay renderer contract](https://github.com/JetBrains/intellij-community/blob/master/platform/editor-ui-api/src/com/intellij/openapi/editor/EditorCustomElementRenderer.java)
