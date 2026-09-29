# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase v0.1.25 at ea34010a116cb95557dcf531195befc7f8339fd9](https://github.com/samoylenkodmitry/cranpose-showcase/tree/ea34010a116cb95557dcf531195befc7f8339fd9).
It uses Cranpose 0.1.176 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/ea34010a116cb95557dcf531195befc7f8339fd9

SHA-256: `55001d55fa1e1e3e1d99c1f31632c98ca393b3d97d0c07fbfa0b042a1bc4b503`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
