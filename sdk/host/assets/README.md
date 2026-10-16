# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase v0.1.29 at 178fd4ba77a90908691f161607f69fe85cef9677](https://github.com/samoylenkodmitry/cranpose-showcase/tree/178fd4ba77a90908691f161607f69fe85cef9677).
It uses Cranpose 0.13.0 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/178fd4ba77a90908691f161607f69fe85cef9677

SHA-256: `7465a55733fbd93194e742718fffbe7b8e4cb40ad7dae78e14cadce0ac02cdd3`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
