# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase v0.1.28 at 80b5b06dee46531918bb3f0d39a9cbcd6a736aed](https://github.com/samoylenkodmitry/cranpose-showcase/tree/80b5b06dee46531918bb3f0d39a9cbcd6a736aed).
It uses Cranpose 0.9.32 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/80b5b06dee46531918bb3f0d39a9cbcd6a736aed

SHA-256: `b8fcb85f4d071d30b8284940f8ccbb22955bbeaa38d90fadfc5851937bd22a9d`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
