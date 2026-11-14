# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase at a91a3ce6eb02d859a07b48d393e8978b89795358](https://github.com/samoylenkodmitry/cranpose-showcase/tree/a91a3ce6eb02d859a07b48d393e8978b89795358),
the head of cranpose-showcase#40 on v0.1.33.
It uses Cranpose 0.22.4 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/a91a3ce6eb02d859a07b48d393e8978b89795358

SHA-256: `52c41bffcb7820ead38f4a80ad162b1708a489478ccccf2cf9fceace43d023e5`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. Studio instead offers each
project the newest Cranpose on crates.io once it is open. This does not bundle
a Rust toolchain or Cargo dependencies, which are needed to build the generated
app.
