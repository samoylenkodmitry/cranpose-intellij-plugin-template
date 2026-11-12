# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase v0.1.33 at 119a3f862fac9f4ddc92eba94e311337b35d19f8](https://github.com/samoylenkodmitry/cranpose-showcase/tree/119a3f862fac9f4ddc92eba94e311337b35d19f8).
It uses Cranpose 0.22.4 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/119a3f862fac9f4ddc92eba94e311337b35d19f8

SHA-256: `18e99f1c1e556363ef7351c17872cf9b66dc26c824c7b0ae8cfa5102356ded78`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. Studio instead offers each
project the newest Cranpose on crates.io once it is open. This does not bundle
a Rust toolchain or Cargo dependencies, which are needed to build the generated
app.
