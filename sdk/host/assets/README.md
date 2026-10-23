# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase v0.1.31 at 885abb5b625bfdd25745f7092db05a6d28964a95](https://github.com/samoylenkodmitry/cranpose-showcase/tree/885abb5b625bfdd25745f7092db05a6d28964a95).
It uses Cranpose 0.17.1 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/885abb5b625bfdd25745f7092db05a6d28964a95

SHA-256: `d0d5892f7ed7cb30c3b56119b81842969de40e76503ed06f0e7ca322994f6dc3`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. Studio instead offers each
project the newest Cranpose on crates.io once it is open. This does not bundle
a Rust toolchain or Cargo dependencies, which are needed to build the generated
app.
