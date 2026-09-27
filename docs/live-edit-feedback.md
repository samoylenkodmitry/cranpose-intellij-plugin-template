# Edit-to-preview lightning

The shared Rust host and Cranpose authoring UI can draw a short lightning arc
from an edited string to its updated `Text` view. This is enabled by the normal
authoring integration; no application release changes are needed.

The host records the selected document's edit offset and modification stamp.
After source analysis, it tracks the matching string initializer and its `Text`
call sites, including resolved local variables and `format!` text. It requests
one layout snapshot after a preview frame arrives. The new rendered text and
source call must both match before the arc starts. Duplicate views, stale source,
offscreen endpoints, other IDE windows, invalid bounds and truncated snapshots
produce no arc. Compiler-owned format fields remain wildcards between the
verified text segments.

An 850 ms Cranpose shader draws a white core, cyan/violet halo, a small fork and
an outline around the destination view. Its separate `window` overlay is placed
in the common Swing layered pane and passes pointer input through. The host
renders only the arc's bounding rectangle, hides it after settling and cancels
it on a subsequent edit. Display scale, preview zoom and viewport pan are mapped
through the actual component hierarchy.

Pending edits replace previous ones. Each workspace has at most one snapshot
request in flight, at most eight attempts, and a ten-second expiry. Requests
only follow received preview frames; there is no additional idle capture loop.
Reserved request IDs are consumed by the host, so they cannot advance the
inspector's response ordering. Closing a preview clears pending work.

## Verification and timing

Rust tests cover source/text identity, aliases, Unicode, format fields, duplicate
views, malformed bounds, request isolation and capture budgets. The real IDE
suite checks Swing source/preview coordinate conversion and pointer passthrough.
`authoring-smoke` verifies pixels at both ends, animation, final transparency and
zero settled frames at 1× and 2×. Its report includes transient native CPU time;
PNG capture and host/IDE costs are outside that CPU sample.
It records first painted and final transparent frame receipt separately. Cold
software GPU delivery has a ten-second ceiling; once visible, the effect must
settle within three seconds. These are test timeouts, not latency promises.

For an actual IDE trace, add `-Dcranpose.trace.edits=true` to the sandbox's VM
options and restart it. Successful matches log `CRANPOSE_EDIT_PRESENTED` with
`editToMatchedFrameMs` and the snapshot request count. This interval starts in
the document-change callback and ends after the host has received a frame,
verified its text/layout and queued the shader. It includes source polling and
snapshot confirmation, and excludes final Swing/display presentation. Disable
the option after measuring. This feature does not change the 100 ms authoring
timer or claim to reduce editor-to-display latency.
