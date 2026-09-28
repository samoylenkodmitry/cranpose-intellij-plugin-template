# Bundled Showcase starter

`showcase.zip` is the unmodified GitHub source archive of
[samoylenkodmitry/cranpose-showcase at 8b251cc0d668e078ebcee49fa01710e2fe35547e](https://github.com/samoylenkodmitry/cranpose-showcase/tree/8b251cc0d668e078ebcee49fa01710e2fe35547e).
It uses Cranpose 0.1.171 and includes its Apache-2.0 LICENSE, README, assets,
platform projects and original executable permissions. These are upstream
files; the plugin never executes starter scripts during project creation.

Source: https://codeload.github.com/samoylenkodmitry/cranpose-showcase/zip/8b251cc0d668e078ebcee49fa01710e2fe35547e

SHA-256: `2d0d8970585650bb292623259f121361e9b7edeb4746cf898f9c5a5fa4e59e86`

The SDK embeds this archive in its native host so New Project works without
Git, download utilities or an internet connection. The hash is checked before
extraction. Updating the starter requires reviewing a new pinned archive and
updating both the revision and checksum in `starter.rs`; generation never
silently switches to the latest upstream code. This does not bundle a Rust
toolchain or Cargo dependencies, which are needed to build the generated app.
