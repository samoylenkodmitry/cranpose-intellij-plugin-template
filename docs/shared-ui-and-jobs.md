# Shared Cranpose UI and project jobs

The template and Cranpose Studio use the same `cranpose-plugin-ui` crate. It
contains code extracted from Studio, with no dependency on Studio or its builder.
Use local SDK paths in a generated plugin, or pin all SDK crates to one Git
revision in an existing project. Keep the SDK's Cranpose revision aligned with
your UI's revision; unlike `sdk/ux`, this crate renders Cranpose widgets.

## Reusable pieces

| API | Use |
|---|---|
| `ide::rememberPalette` | Follow the IDE theme, with a standalone fallback |
| `ide::EditorFile` | Decode the currently focused editor |
| `ide::open_in_ide`, `notify_ide` | Navigate to a file or show an IDE notification |
| `controls::ActionButton` | Compact selected/disabled actions in the IDE palette |
| `tasks::TaskState` | Stage, log, result and completion state |
| `tasks::TaskOutput` | Stop control, status, result and expandable output |
| `cranpose_host::jobs::handle_message` | Route a channel through the IDE queue |
| `cranpose_host::jobs::Operation::run` | Run cancellable work outside the IDE thread |

The complete example is in [`ui/src/tasks.rs`](../ui/src/tasks.rs) and
[`host/src/tasks.rs`](../host/src/tasks.rs). **Scan project** runs a bounded,
read-only file count. It skips VCS/build folders and symbolic links and checks
cancellation between entries. Replace the scan with your plugin's own worker.

## Host contract

Register a handler with `cranpose_host::set_message_handler`. Delegate its channel
to `jobs::handle_message`, passing a request parser that returns `Operation::Cancel`
or `Operation::run(save_documents, worker)`. Unrecognized channels return `false`.

Parsing, trust checks and optional document saving run in the IDE's non-modal
queue. This matters for messages received from a Swing timer: saving directly
inside that callback can violate the IDE's write-intent rules. Cancellation uses
the same queue, preserving request order. Work queued before project disposal is
discarded. The worker gets a project root, cancellation token and `send` method;
it never accesses JNI.

The SDK allows one active job per project, catches worker panics and sends a
`job_finished` event with optional error and cancellation fields. The worker must
cooperate with cancellation. For subprocesses, use `cranpose-plugin-process` and
its owned, bounded process-tree shutdown. Project disposal cancels active work
and suppresses subsequent UI delivery.

For a long-running application, use `cranpose_plugin_process::execute_observed`.
It sends `ExecutionEvent::Started { pid }` after successful OS process creation,
before any `Output` event. Forward that notification as a stage such as
"Application running". It does not establish first paint or application readiness.
Failed spawn and pre-cancellation send no start event. The existing `execute`
API keeps its output-only callback; both APIs share cancellation and tree cleanup.

## UI contract

Call `TaskState::begin` before sending a request. Feed received JSON to
`TaskState::apply`. It accepts these events:

```json
{"type":"stage","message":"Preparing…"}
{"type":"log","text":"A line of output"}
{"type":"result","text":"Path or summary"}
{"type":"job_finished","cancelled":false,"error":null}
```

`apply` returns `false` for plugin-specific events. Studio handles build plans,
tool checks and package metadata in its own adapter, then shares the same output
component. Logs retain only the last 80 lines, at most 600 Unicode characters
per line. A large incoming batch is bounded before allocation. Stop remains
available during work. The output component adds no polling or animation timer.

The broader SDK also includes live color/number/string controls, hover sessions,
inline swatches, editor overlays, source-arrival and edit-feedback shaders,
virtualized row windows, process ownership and plugin packaging. See
[authoring](authoring.md), [extending the host](extending-the-host.md), and the
[SDK table](../README.md#layout) for their entry points.

## Validation

Rust tests cover bounded Unicode logs, completion/cancellation, editor payloads
and outbound messages. The actual IDE suite routes run/cancel messages from a
Swing callback, saves documents in the queued context, and verifies worker
cancellation. Existing tests also cover exclusive jobs and project disposal.
Both the template demo and Studio compile this crate on all six native targets.

For a Studio-compatible inspector, `inspection-profile --resize-rounds 3` checks
the disconnected controller and a populated tree at 1024 px and 480 px. It waits
for both the new viewport geometry and a fresh rendered inspection. After scrolling
ten rows, it also verifies that switching between side and bottom placement retains
the visible rows. Use at least 100 fixture nodes. These checks run outside the idle
CPU observation intervals.

Keep scroll and filter state above responsive branches that move or hide an
inspector. The branch's modifier nodes need that state while detaching, and users
should keep their place when changing the panel size. The resize check catches
resetting the tree even when the renderer stays alive; the IDE suite additionally
tests controller restart and real Swing component bounds.
