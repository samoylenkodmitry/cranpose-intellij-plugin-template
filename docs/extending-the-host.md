# Extending the IntelliJ host

`IdeBridge` owns the transport callbacks and forwards the theme and focused
editor. Supply its `onReady` and `onMessage` callbacks to add your application's
message contract:

```kotlin
IdeBridge(
    project, panel, content,
    onReady = { panel.send("my.project", projectSnapshot()) },
    onMessage = { channel, payload ->
        if (channel == "my.action") {
            handleAction(payload)
            true
        } else {
            false
        }
    },
)
```

Both callbacks run on the IDE event dispatch thread. Return `true` only when your
handler consumes a channel. Other messages continue to the built-in notification
and file-opening handlers. Custom payloads can be nested JSON or another format;
the bridge dispatches them before its own flat-JSON parser.

Use project services for background work and bind listeners and running processes
to the tool window's content disposer. Project commands must check IntelliJ's
project-trust state before starting user code.

[Cranpose for IntelliJ IDEA](https://github.com/samoylenkodmitry/cranpose-idea)
is a complete example built from this template.

## Hosting previews and other processes

`CranposePanel` accepts an `environment` supplier, evaluated when a process is
launched. Use it for fixture selectors or application settings:

```kotlin
val panel = CranposePanel(
    command = { listOf(executable.toString()) },
    workingDirectory = projectDirectory,
    environment = { mapOf("CRANPOSE_PREVIEW" to selectedDescriptor.id) },
)
```

The session always supplies its own authenticated loopback address and token,
overriding values with those reserved names in the supplied map.

- `panel.contentScale` magnifies application coordinates from 0.25 to 4.0.
  Set the component's preferred size to the logical viewport multiplied by this
  scale. Pointer positions are converted back to application coordinates.
- `panel.onPointerPress` can return false to select an inspected element without
  activating the application's click handler.
- `panel.paintOverlay` draws host selection bounds after the live frame.
- `panel.onStopped` reports launch failures and process exits on the Swing thread.
- `panel.canvas.snapshot()` returns an independent image copy for export. Save
  it off the UI thread.

Bind each running panel and its editor-overlay listeners to a session disposer.
Dispose that session when replacing a preview, then dispose the whole workspace
through IntelliJ's `Disposer` when its editor or tool-window tab closes.

## Inspecting the embedded UI

Framework revisions after 0.1.164 accept an empty message on
`cranpose.inspector.v1.request` and reply on
`cranpose.inspector.v1.snapshot` with a text report of the primary surface's
layout, render scene and screen summary. Request a snapshot from an explicit
user action; keep reports local. The report is for display, not machine parsing.
Older framework versions can still render and do not answer this request.
