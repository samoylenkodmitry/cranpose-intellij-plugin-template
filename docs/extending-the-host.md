# Extend the Rust host

The template loads the pinned `cranpose-ide-host` Rust library. Configure optional
features before the first JNI dispatch. The template disables the Cargo dashboard
and stability analyzer because its tool window is a general Cranpose application.

## Receive custom UI messages

In `host/src/lib.rs`, call `cranpose_host::set_message_handler(handle_message)` before
forwarding the JNI dispatch. Registration is idempotent; the first handler wins.

```rust
fn handle_message(
    j: &mut cranpose_host::jvm::J<'_>,
    project: &cranpose_host::jvm::O,
    channel: &str,
    payload: &str,
) -> anyhow::Result<bool> {
    if channel != "my-plugin.show-project" {
        return Ok(false);
    }
    let name = j.text(project, "getName")?;
    // Parse payload with serde_json and call the IDE SDK through j.
    eprintln!("{name}: {payload}");
    Ok(true)
}
```

Add `anyhow = "1"` to the host dependencies when using this example. A return value
of `true` consumes the message. Return `false` to let the shared host handle built-in
channels such as `ide.notify`, `ide.open` and `host.overlay`.

The callback runs on the IDE event thread. Keep it short; use workers for I/O and
`cranpose_host::jvm::later` to return to the IDE thread. Keep JVM global references
only while needed and register callbacks in a disposal `Scope`.

## Built-in channel requests

```rust
cranpose::send_to_host("ide.notify", r#"{"title":"Done","content":"Analysis finished"}"#);
cranpose::send_to_host("ide.open", r#"{"path":"/absolute/path/src/main.rs"}"#);
```

The host sends `ide.theme`, `ide.editor` and `ide.caret` state to connected panels.
Use Cranpose's `rememberHostMessages` and `collectAsState` to consume them.

## Extend SDK registrations

The shared `cranpose-jvm-bridge` crate writes JVM classfiles directly from Rust.
Its IntelliJ adapters forward into native Rust; no Java or Kotlin source generation
or compiler is involved. Existing adapters cover tool windows, surfaces, listeners,
editor previews, inlay renderers, actions and run configurations.

For a new SDK interface, add its descriptors and forwarding methods to the generator,
handle its operations in the Rust host, and register the class in `plugin.xml`.
Run the native IDEA integration tests and Plugin Verifier after changing descriptors.

## Inspection and development

The template's binary watcher restarts an ordinary rebuilt UI. Stateful application
hot reload belongs to [Cranpose for IntelliJ IDEA](https://github.com/samoylenkodmitry/cranpose-idea):
its development runner owns instrumentation and Subsecond integration in a separate
Cargo workspace. Application release manifests, profiles and binaries stay unchanged.
