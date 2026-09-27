# From New Project to the first preview

Consumers that enable `HostFeatures::cargo` get a complete desktop handoff after
the bundled Showcase is generated. The Rust host saves and selects a persistent
Cranpose Cargo configuration, publishes the pinned starter's desktop target,
opens `main.rs` beside its preview, and sends one `showTarget` command. The normal
preview runner owns compilation, progress, cancellation and process cleanup.

This handoff is triggered only by successful wizard generation. Reopening a
project or refreshing metadata does not automatically run it. The IDE trust gate
still applies. Repeated configuration creation reuses the matching configuration
and preserves customized arguments and environment variables.

The desktop target is available before Cargo discovery. Archive tests verify its
package, binary, source and required features against the bundled manifest.
Missing Rust is reported as a `studio.child` stopped event with `setup: "rust"`.
Consumers should offer `setupRust` (opens the official Rust installation page)
and a normal start/retry action. The host checks Cargo again on every attempt,
including the rustup home directory and `CARGO_HOME`, and passes its directory
to the preview runner's PATH. No IDE restart is needed for a standard rustup
installation. Rust's native platform prerequisites and dependency downloads
remain necessary for compilation.

The integration suite verifies persistent selection, duplicate prevention and
preservation of edited run options using the real IntelliJ RunManager. Generation
continues to work without Git, Rust or network access.
