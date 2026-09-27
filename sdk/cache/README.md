# Generated asset cache

`materialize(root, files)` publishes a complete generated bundle under a SHA-256
of its sorted relative paths and bytes. Reopening the same bundle verifies every
file without rewriting it. Timestamps stay stable, so generating unchanged build
inputs does not invalidate Cargo's existing artifacts.

The cache belongs outside application source. A publisher writes into a private
staging directory and atomically renames the completed directory. Concurrent
publishers reuse the winner. Changed content gets a different directory; an older
running preview retains its original helper sources. Bundles are not removed by
this API. Corrupt, unexpected or symlinked files produce an error instead of
silently changing an active bundle.

```rust,no_run
let files: &[(&str, &[u8])] = &[("helper/src/lib.rs", b"pub fn helper() {}")];
let bundle = cranpose_plugin_cache::materialize(std::path::Path::new("plugin-cache/assets"), files)?;
# Ok::<(), anyhow::Error>(())
```

Tests cover stable timestamps, version separation, concurrent publication, path
validation and corruption. CI runs cache tests on macOS, Linux and Windows.

Studio uses this crate for its generated hot-reload macro and runtime crates.
The shared `hot-smoke` harness can prove reuse in a second launch:

```text
hot-smoke ... --startup-only --build-diagnostics --require-cached-support --report restart.json
```

The assertion checks Cargo's `compiler-artifact` freshness for both support
crates. It is a compiler-level regression check, not a timing threshold.
Cargo's [rebuild diagnostics](https://doc.rust-lang.org/cargo/faq.html#why-is-cargo-rebuilding-my-code)
explain the `CARGO_LOG=cargo::core::compiler::fingerprint=info` setting used by the harness.
