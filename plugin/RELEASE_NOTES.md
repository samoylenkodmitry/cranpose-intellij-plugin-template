# Cranpose plugin template 0.9.0

- Shared native GPU presentation for animated editor counters and edit-to-preview
  lightning: Metal on macOS, DirectComposition/DX12 on Windows, and Vulkan on
  Linux X11 and JetBrains Runtime Wayland.
- One renderer per IDE window follows scrolling, folding and display scaling.
  Animations stop when settled, with no rendered frame copies through the CPU.
  Unsupported graphics environments retain static editor counter text.
- Preview recomposition counts link inspection data to composable definitions.
  Overlapping Pick targets offer a chooser before source navigation.
- Native probes cover transparency, input passthrough, text updates, movement,
  lightning and idle behavior. Hardware checks passed on Apple M5, Windows
  RTX 2070 and Linux Intel UHD 730, including Wayland at 125% scaling.

These facilities are reusable template SDK components. Preview tracking and
decoration rendering do not affect application release builds.

## Included since 0.8.2

- The preview controller reconnection test turns off automatic preview start,
  so controllers that start a preview on their own, like Cranpose Studio
  0.14.4, can run it. No behavior change in the SDK.

## Included since 0.8.1

- Live values for literals that have not run yet apply immediately again. 0.8.0
  compiled every value edit in a file that had such a literal, for example an
  animation duration or a number in a click handler, so most edits waited for
  the compiler.
- Strings and characters get no glyph or popup; they are edited in place and
  their edits still apply live. Numbers, booleans and colors keep their controls.
- Plainer wording in the New Project card, the popup hint, project statuses and
  messages.
- The hot-smoke `--structural-state` step checks that an edit to a literal that
  has not run applies without compiling.

## Included since 0.8.0

The first tagged release of the template and its Rust SDK. The SDK crates
share the template version, and Cranpose Studio and Cranpose Build pin this
release.

## Editor and live values

- Value knobs, color swatches, stability badges and composable-call underlines
  are painted by the editor itself, so they stay attached to the text while
  scrolling. The transparent overlay only draws transient effects.
- Live-value popups apply every complete value as you edit. Typing shares one
  Undo step per popup, drags and boolean choices get their own, and incomplete
  input shows its reason inline.
- Live values for a literal that has not run since its file was compiled are
  compiled, so the compiler reports values that do not fit their type. Values
  for literals that have run still apply without compilation.
- Clicking a glyph gives its open hover control keyboard focus. Dense files
  insert glyphs in guarded editor batches without moving carets.

## Studio building blocks

- Native IDE list popups with speed search, checkmarks and separators, and
  native tooltips, for plugin toolbars.
- Previews stay mounted across source navigation. Pick works without the
  inspector. Compiled source paths resolve to editable files, including
  generated launchers.
- Inspection discards observations that arrive after visibility changes.

## Cranpose

- The SDK, **Create Cranpose App** manifests and the bundled Showcase starter
  use Cranpose 0.1.176. New projects offer Cranpose's development-only
  `hot-reload` feature, which keeps remembered state across structural hot
  patches.
- The hot-smoke tool's `--structural-state` step checks that structural edits
  keep state and that literals that have not run are compiled.
- The framework compatibility check builds the complete Cranpose main graph.
