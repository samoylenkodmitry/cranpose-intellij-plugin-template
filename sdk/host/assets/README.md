# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase at dc439faf9fd019191ccbf1c6fa70f2c523ae1163](https://github.com/samoylenkodmitry/cranpose-showcase/tree/dc439faf9fd019191ccbf1c6fa70f2c523ae1163).
It uses Cranpose 0.1.167 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/dc439faf9fd019191ccbf1c6fa70f2c523ae1163

SHA-256: `6fd9c29bd04d83e1e3b9309809cf279d1d23452e7336d506eedaba1a385bc60e`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
