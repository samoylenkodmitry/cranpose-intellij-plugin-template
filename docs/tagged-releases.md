# Publishing a verified version tag

The shared Rust `xtask release` commands can be called by a tag-triggered job
after a successful package and Plugin Verifier job. Download the existing
all-platform archive from that run; do not rebuild between verification and signing.

1. `release validate-tag vX.Y.Z` requires an exact match with `plugin/VERSION`.
2. `release check-zip ARCHIVE` checks native payloads, plugin identity and version.
3. `release sign ARCHIVE signed-plugin.zip` uses JetBrains ZIP Signer and the
   `CERTIFICATE_CHAIN`, `PRIVATE_KEY` and optional `PRIVATE_KEY_PASSWORD` secrets.
4. `release github-release signed-plugin.zip vX.Y.Z` attaches the archive under
   its plugin directory/version name. New releases use `plugin/RELEASE_NOTES.md`.
5. `release publish signed-plugin.zip --plugin-id ID` submits an update using
   `PUBLISH_TOKEN`. The empty default channel publishes to Stable.

The first Marketplace listing requires a manual upload. A consumer can leave its
Marketplace ID unset for the first tag, create the signed GitHub release, then
set the ID for later tags. Studio's Build and Publish workflows demonstrate this.
Manual retries should use `release successful-build` to find a successful Build
for the exact checked-out tagged commit before downloading its artifact.

Signing keys and upload tokens belong in GitHub secrets, never in the repository.
Marketplace review is separate from a successful upload.
