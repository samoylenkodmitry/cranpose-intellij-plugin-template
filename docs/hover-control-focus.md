# Activating hover controls

Hover controls open without requesting keyboard focus. An explicit press on the
live-value glyph promotes the existing popup to a keyboard control, retaining
its renderer and edit session. Tab enters its first field. Clicking ordinary
source dismisses the popup and leaves the click available to the editor.

`popup::activate` sets the popup's focus request and defers the native-window
activation until the editor finishes dispatching the press. It restores the
window's focus flags, raises it, and requests focus through `IdeFocusManager`.
Disposed or no-longer-visible popups are ignored by the deferred callback.
This helper is shared Rust host infrastructure; hover never calls it.

The IDEA integration suite invokes the generated `EditorMouseListener` bridge.
It verifies that hover does not request focus, a glyph press does request it and
is consumed, activation retains the popup/renderer, and a source press remains
unconsumed. Existing dismissal and stale-session checks still run.

Headless tests cannot establish native desktop focus. In the macOS RustRover
sandbox, 0.13.5 left keyboard input in the source editor after clicking a hovered
glyph. With the corrected native-window activation, one click followed by Tab,
typing, Tab and Enter updates the source. One Undo restores it. Clicking directly
back into the source permits typing on that first click. These are correctness
checks, not latency or cross-platform desktop focus measurements.
