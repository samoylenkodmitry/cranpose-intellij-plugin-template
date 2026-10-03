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

## Reusable private workspaces

`WorkspaceLease::acquire(root, identity)` obtains a private directory at a reusable
path. Identify the project with its canonical path and an application format tag.
An OS file lock provides exclusive access. Concurrent callers skip busy slots and
receive different directories immediately. Acquiring a clean slot clears its old
contents before returning it, while its path stays stable for compiler caches.

Use `artifacts_path()` for compiler output that must survive clean restarts.
It belongs to the same exclusive lease but sits outside the copied sources that
`acquire` clears. Concurrent or abandoned slots keep separate artifacts, so a
compiler that clears its own output cannot erase another active build. The caller
creates the directory when needed and only accesses it while holding the lease.

Call `complete(self)` only after every process using the directory has exited.
Ordinary Drop leaves a busy marker. Failed or killed owners are never automatically
reused, because descendants might still be alive after the owner's lock closes.
Abandoned directories remain until the caller clears its cache with all users
stopped. The cache is private infrastructure and must be outside application source.

Tests exercise separate OS processes, forced owner exit, concurrent access,
project separation and removal of old files after clean shutdown. CI runs them on
macOS, Linux and Windows.

`hot-smoke --fixture counter --reuse-fixture` uses the same mechanism to keep a
fixture's source path stable across completed runs. Add `--profile-startup
--require-cached-workspace` on restart to assert that the runner also reused its
private path. A new fixture or concurrent run has a separate directory; a failed
run leaves its fixture abandoned. Application and test process cleanup completes
before either directory becomes available again.
