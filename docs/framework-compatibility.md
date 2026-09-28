# Testing against Cranpose main

The scheduled `Cranpose main` workflow tests a disposable checkout against one
snapshot of the framework. Run the same preparation locally in a disposable
plugin checkout:

```sh
cargo run --locked -p cranpose-template-tools -- use-framework-main --report framework-compatibility.json
cargo test --locked --no-default-features --workspace
```

Consumers use their own tools package (Studio uses `-p xtask`). The implementation
is shared in `cranpose-plugin-tools`; normal builds never call it.

The tool clones main once into an OS temporary directory outside Cargo's target
directory. It discovers Cranpose and Coroflow packages from that checkout and
adds workspace-root Cargo patches for both Git and registry dependencies. This
also overrides framework dependencies inside pinned SDK revisions, including
aliases, optional features and platform dependencies. Updating only the root UI
dependencies leaves incompatible types from multiple framework revisions.

Only the framework package IDs are unlocked. Other dependencies stay locked
unless Cargo needs to change them to resolve the new framework. An all-features
metadata check then requires every resolved framework package to come from the
selected checkout. A package removed from main, an incompatible version
constraint or an existing conflicting patch causes an error. Failed preparation
restores the root manifest and lockfile. It never rewrites dependency checkouts.

The JSON report records the snapshot commit and resolved package paths/versions.
CI retains it with the generated manifest and lockfile. It is dependency
evidence; the following Rust and actual IDEA tests establish compatibility.

To repeat a particular snapshot, supply `--checkout /absolute/path/to/Cranpose`.
That directory is not changed by the tool. Keep it available for every test
command. An automatically cloned directory is printed and retained for later
Cargo invocations; remove it after testing. Discard the disposable plugin
checkout when finished. Do not commit generated patches or the compatibility
lockfile into a release branch.

The Rust regression builds a local Git SDK and application pinned to different
framework commits. It first reproduces a type mismatch, then requires a locked,
offline Cargo check to succeed after the override. It also checks aliased,
optional and Windows-specific dependencies, repeat preparation, conflicting
patches, removed packages and rollback after incompatible versions. CI runs it
on macOS, Linux and Windows without an external fixture repository.
