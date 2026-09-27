# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase at 287ceafe513523b0fdc9bd8998aa724cfe738596](https://github.com/samoylenkodmitry/cranpose-showcase/tree/287ceafe513523b0fdc9bd8998aa724cfe738596).
It uses Cranpose 0.1.169 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/287ceafe513523b0fdc9bd8998aa724cfe738596

SHA-256: `19b69244adcaa0b5c4e18506cda48b695e574d01d94ffc03ea9550121459c230`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
