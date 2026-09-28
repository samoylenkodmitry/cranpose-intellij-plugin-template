# Navigate from generated builds to source

`cranpose_plugin_ux::source_paths::resolve` maps Rust source locations from a
private compilation tree back to editable project files. Pass the reported
`file!()` path, `CARGO_MANIFEST_DIR`, compilation root, original workspace root
and any extra `(generated_directory, original_directory)` mappings.

Cargo may report a workspace-relative file such as
`.generated/launcher/src/screens/detail.rs` alongside a manifest directory
ending in `.generated/launcher`. The resolver avoids appending that directory
twice. Package-relative and absolute source paths also work. Nested mappings
take precedence over enclosing mappings regardless of insertion order.

The resolver does no filesystem scanning and does not guess from filenames.
Sources outside the known compilation tree remain unchanged. A native Rust
integration test compiles a real workspace library and generated binary, then
checks that both compiler locations resolve to existing editable files.
