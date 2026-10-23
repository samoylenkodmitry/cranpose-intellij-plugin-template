# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase v0.1.32 at ac5f13715a1ad28aaf1e956532dfec1f33c40458](https://github.com/samoylenkodmitry/cranpose-showcase/tree/ac5f13715a1ad28aaf1e956532dfec1f33c40458).
It uses Cranpose 0.17.2 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/ac5f13715a1ad28aaf1e956532dfec1f33c40458

SHA-256: `447c55708c0ae331209d1044edb07be662bee3c1513fc09c7006733922cf73a8`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. Studio instead offers each
project the newest Cranpose on crates.io once it is open. This does not bundle
a Rust toolchain or Cargo dependencies, which are needed to build the generated
app.
