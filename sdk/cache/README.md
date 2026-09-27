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

## Derived files

`DerivedFile::new(root, inputs)` keys a generated file by sorted input names and
bytes. `store` atomically replaces the record; `load` verifies its payload checksum
and returns a miss for absent, truncated or damaged records. Concurrent readers
see a complete previous or replacement record. I/O failures remain errors so the
caller can report them and continue without the optimization.

Consumers copy the bytes into their private workspace and let the generating tool
validate them. This cache is a starting point for normal resolution, not proof that
the result is current. Studio uses it for the private development Cargo.lock after
manifest instrumentation. Source manifests, original lockfiles and toolchain/config
inputs determine reuse; Cargo can still resolve changed external dependencies.

Tests cover input invalidation, order independence, corruption and simultaneous
readers/writers. The cross-platform cache job runs these alongside asset tests.

## Startup and settled idle measurements

The shared Rust harness accepts `--profile-startup` to record runner preparation,
spawn-to-connection, and connection-to-first-snapshot timings. Dioxus's serving and
build-completed timestamps are reported separately because their clock starts in
the compiler process. Unsupported or missing log evidence stays null. Host polling
adds up to 100 ms for connection and about 200 ms for a matching snapshot.

Use `--idle-seconds 20 --idle-settle-seconds 5` to measure visible, inspecting and
hidden phases after settling each one. CPU is process CPU time divided by elapsed
wall time, expressed as a percentage of one core; short samples can hide timer
resolution and startup effects.

On a second Studio launch, `--profile-startup --require-cached-dependencies`
asserts that a private lockfile was restored. Combine it with
`--build-diagnostics --require-cached-support` to check helper crate reuse. These
are regression assertions; elapsed-time comparisons need repeated controlled runs.
