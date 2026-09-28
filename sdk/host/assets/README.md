# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase at 119e3d57d9e7c304f934637d0311d6a54dd4c23c](https://github.com/samoylenkodmitry/cranpose-showcase/tree/119e3d57d9e7c304f934637d0311d6a54dd4c23c).
It uses Cranpose 0.1.173 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/119e3d57d9e7c304f934637d0311d6a54dd4c23c

SHA-256: `3bad29a0e22daaea3b4419e7d6d7a2256612f7db14f0a5bd020f49a127a9fb96`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
