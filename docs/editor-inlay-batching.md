# Editor inlay batches

`cranpose_plugin_host::inlays::execute` runs a Rust closure inside IntelliJ's
synchronous `InlayModel.execute` boundary. Other plugins can use it for large
sets of inlay additions, removals or updates. The closure's result and errors
return to Rust, and its callback and captures are released before return.

Batching has a setup cost that grows with the document. It can make sparse
files slower. It can also change the visual position of a caret at an affected
offset. The closure must not edit the document, access caret or folding state,
change soft wraps, or transform coordinates. See the
[IntelliJ API contract](https://github.com/JetBrains/intellij-community/blob/master/platform/editor-ui-api/src/com/intellij/openapi/editor/InlayModel.java).

The shared authoring host batches new glyphs only when there are at least 256
of them, the document has at most 64 UTF-16 code units per glyph, and no caret
is at a glyph's offset. The existing 1,024-glyph limit still applies. Gutter
creation and disposal run outside this batch. Literal-only edits continue to
reuse their existing anchors.

## Validation and measurement

Run the actual IDE suite against a locally installed IntelliJ distribution:

```sh
cargo run --locked -p cranpose-template-tools -- ide-test --profile --ide /path/to/idea
```

The Rust fixture writes `authoring-placement.json` alongside `results.json` in
the reported evidence directory. It compares ordinary insertion, forced
batching, automatic selection, and retained anchors. It covers dense files,
files near the automatic size boundary, and files with 30,000 extra comment
lines. Optimized profiling uses three rounds, reversing fixture order in the
middle round to expose IDE warm-up effects; each fixture discards two warm-up
iterations and records seven. Checked-debug CI uses one round.

These intervals include disposal and placement but exclude document writes,
source parsing, preview transport, Swing painting and physical display. They
are not editor-to-preview timings. `clear_ms` records the disposal portion.
Do not compare early measurements from a fresh IDE directly with later warmed
measurements or with different machines and JVM checking modes.

The suite verifies Unicode offsets and glyph widths, anchor reuse, primary
and secondary caret handling, selection positions, and callback cleanup after
successful operations, Rust errors and failed JNI calls. Sparse files must
avoid the automatic batch path.
