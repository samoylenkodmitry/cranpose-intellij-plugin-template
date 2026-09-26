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

## Inspecting the embedded UI

Framework revisions after 0.1.164 accept an empty message on
`cranpose.inspector.v1.request` and reply on
`cranpose.inspector.v1.snapshot` with a text report of the primary surface's
layout, render scene and screen summary. Request a snapshot from an explicit
user action; keep reports local. The report is for display, not machine parsing.
Older framework versions can still render and do not answer this request.
