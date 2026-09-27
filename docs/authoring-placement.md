# Retaining live editor glyphs

Literal edits retain existing inlays when the source schema, literal identities,
glyph count and actual IDE offsets still match. Each anchor must remain valid.
IntelliJ moves inlay offsets with document changes, including UTF-16 shifts;
see the [Inlay contract](https://github.com/JetBrains/intellij-community/blob/master/platform/editor-ui-api/src/com/intellij/openapi/editor/Inlay.java).

Structural changes, missing or invalid anchors and unexpected caret-related
offsets use the existing full replacement path. Invalid source clears the
markers. The current catalog still replaces the old one and refreshes Cranpose
shader geometry, including updated color swatches. No extra timer, parser or
application instrumentation is involved.

## Real IDE regression and measurements

The normal `ide-test` suite checks anchor identity across Unicode string edits
and grouped colors with aliases, then checks invalid-anchor recovery, structural
preview-action replacement and disposal. It emits `authoring-placement.json`
beside `results.json` in its evidence directory.

Each fixture has 32, 256 or 1,024 literal glyphs. After two warmup pairs, seven
pairs alternate forced replacement (the previous implementation) and reuse in
the same IDE process. Every operation follows a document command that switches
the first string between ASCII and supplementary Unicode. The test asserts
that reuse keeps the native callback identities and all resulting glyph offsets
and reserved widths are correct. CI checks these invariants, without a brittle
wall-clock threshold.

For timing experiments with an optimized Rust host and without `-Xcheck:jni`:

```sh
cargo run --locked -p cranpose-template-tools -- ide-test --profile --ide /path/to/idea
```

Omit `--profile` for the normal checked debug suite. Both modes run the same
integration assertions in an isolated headless IDE; neither opens an interactive
desktop session. The profiling host contains test-only entry points and is not
a distributable plugin package.

The interval measures glyph validation or replacement only. It excludes source
parsing, document commands, transport, Cranpose painting and display. Results
are not editor-to-display latency, compilation time or idle CPU measurements.
