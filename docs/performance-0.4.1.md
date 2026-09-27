# Source catalog indexing

The authoring catalog previously rescanned source prefixes for every function,
call and literal span: finding its line, converting the character column to bytes,
then counting UTF-16 units from the start of the file. Source with many marked
expressions therefore accumulated roughly quadratic range-conversion work.

The Rust authoring SDK now builds one source index per parse. It records line
starts in character positions and cumulative encoding differences only at
non-ASCII characters. Span lookups use those entries without rescanning text.
ASCII files need only the line-start table. Parser columns follow
[proc_macro2's character-position contract](https://docs.rs/proc-macro2/latest/proc_macro2/struct.LineColumn.html).

## Measurements

Apple M5, macOS, rustc 1.98.1, optimized release executables. Three pairs, seven
measured parses per case in each run, with one untimed warmup per case. The order
was before/after, after/before, before/after. The executables were built before
these runs; no compilation was requested between samples. The same IDE/demo
remained open. This was an interactive workstation: unrelated Java and Rust
builds were active during the series, and timings varied. These are local samples,
not a quiet-machine or cross-machine guarantee.

| Synthetic source | Bytes | Calls / literals | Before median | After median |
|---|---:|---:|---:|---:|
| 32 small ASCII functions | 4,118 | 64 / 64 | 1.014 ms | 1.000 ms |
| 256 ASCII functions | 33,170 | 512 / 512 | 37.09 ms | 6.38 ms |
| 1,024 ASCII functions | 133,034 | 2,048 / 2,048 | 495.70 ms | 22.43 ms |
| 2,048 ASCII functions | 267,178 | 4,096 / 4,096 | 1,852.03 ms | 44.06 ms |
| 1,024 Unicode functions | 138,154 | 2,048 / 2,048 | 491.75 ms | 24.25 ms |
| 32 long Unicode function bodies | 213,942 | 4,096 / 4,096 | 996.73 ms | 39.30 ms |

Each median combines 21 samples. The largest ASCII case fell 97.6%; the tiny-file
case showed no meaningful gain. The preliminary investigation measured the large
case at 1,304.91 → 32.05 ms under different background load. Its samples are kept
separately and excluded from the table.

Only `Catalog::parse` is timed: Rust syntax parsing, range conversion, live-value
catalog construction and schema hashing. This excludes IDE text extraction,
decoration placement, event dispatch, compilation, runtime application and display
presentation. No end-to-end edit latency, startup, idle CPU or release-build
speedup is claimed. The 0.7.0 live-value latency measurements remain separate.

## Correctness and reproduction

Every measured catalog matches the pre-index catalog's SHA-256, covering the
schema, all byte and UTF-16 ranges, call ends, literal identities, values and types.
The retained baseline is also an ordinary Rust integration test on CI. Unit tests
check every character boundary through ASCII, combining marks, supplementary
characters, CRLF, empty lines, end-of-file and long lines. Timing is not a CI
assertion.

```sh
cargo test --locked -p cranpose-plugin-authoring
cargo run --locked --release -p cranpose-plugin-authoring --example catalog_profile -- 7
```

Raw results are in [measurements/catalog-0.4.1](measurements/catalog-0.4.1).
`baseline.json` and `preliminary-after.json` are the initial investigation;
`before-2` through `before-4` and `after-2` through `after-4` form the table.
The baseline executable used the authoring implementation at template commit
`b3b4064f199ffc8df50e5bd74509a278b1c6ffd7` with the same benchmark fixture.
