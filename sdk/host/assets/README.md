# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase v0.1.30 at 8511f12335a1d56889ba4d8517ed8e44ca3508ce](https://github.com/samoylenkodmitry/cranpose-showcase/tree/8511f12335a1d56889ba4d8517ed8e44ca3508ce).
It uses Cranpose 0.13.0 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/8511f12335a1d56889ba4d8517ed8e44ca3508ce

SHA-256: `8e22bc2b6ba9a32c8160b1146e0a9d1dfde812b9aa89e569ed039c8667f5a144`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
