# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase at 52782348528023b6e16f566c724bdb82d2944ef0](https://github.com/samoylenkodmitry/cranpose-showcase/tree/52782348528023b6e16f566c724bdb82d2944ef0).
It uses Cranpose 0.1.172 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/52782348528023b6e16f566c724bdb82d2944ef0

SHA-256: `d72677f59ef6b4e0d28f36c27450b04d116eb7abcbec87ff415a2dc979b102cf`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
