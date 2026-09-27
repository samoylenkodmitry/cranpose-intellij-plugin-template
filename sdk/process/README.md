# Owned plugin processes

`Process::spawn(command)` owns a preview root and its descendants. Call
`terminate(grace)` from a worker thread to send termination immediately, allow
bounded cleanup, and force-stop remaining children. Reaping is capped at another
500 ms. Drop provides forced cleanup on error paths and after normal root exit.

A plugin development runner starts its compiler with `Process::spawn_worker`.
On Unix an SDK-owned runner is a process-group leader; its compiler inherits that
group. The host can therefore reach the compiler and application even when the
runner is killed before Rust destructors execute. A standalone runner instead
owns a separate compiler group. Workers remove the private ownership environment
marker before launching the compiler. Descendants must not detach into new
sessions or groups; process groups do not contain deliberately detached daemons.

On Windows each owner uses a Job Object with kill-on-close. Children start
suspended, are assigned to the Job, then resume, so they cannot spawn outside it.
The small Windows FFI module is isolated behind the safe `Process` interface.
Stable Rust does not expose a primary thread handle, so it locates the suspended
thread through Toolhelp. A host crash between process creation and Job assignment
can leave a suspended child; normal cancellation and every returned spawn error
clean it up. See Microsoft's [Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
and [thread snapshot API](https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot).
Windows cancellation force-stops the Job; it does not emulate Unix TERM handlers.

`capture` uses the same ownership for Cargo metadata, stability analysis and other
bounded commands. Descendant cleanup closes inherited pipes before readers join.
`is_running` is a diagnostic check, not a way to establish ownership of a PID.

## Regression tests and latency measurements

All fixtures are Rust subprocesses: runner → compiler → application. Tests verify
process exit and stopped application heartbeats, with graceful and ignored signals,
forced runner exit, standalone workers, inherited pipes, repeated replacement,
and an independent preview that must stay alive. Host Session tests additionally
cover connection timeout, failed authentication, EOF and cancellation during hello.
CI runs the suite on macOS, Linux and Windows.

```text
cargo test -p cranpose-plugin-process -p cranpose-plugin-host --test lifecycle --test session_lifecycle
cargo test -p cranpose-plugin-process --test lifecycle shutdown_latency_benchmark -- --ignored --nocapture
```

The manual Unix benchmark emits five samples each for the old two-second wait
and escaped compiler group, graceful owned shutdown and a 200 ms forced fallback.
It cleans the deliberately orphaned legacy fixtures before asserting results.
These are lifecycle overhead measurements, not compiler or UI startup timings.
The shared `hot-smoke` tool separately records real preview `shutdownMs` and the
runner, compiler and application PIDs it verified had exited.

`wait_for_tree_exit(timeout)` observes completion after shutdown before reusing
resources such as a private compiler workspace. It checks process-group membership
on macOS/Linux and the Job's active process count on Windows. A shared Unix worker
excludes its still-running owner. The wait is bounded and returns false if another
process remains; callers must abandon the workspace on false or observation error.
A regression fixture exits its parent while descendants remain alive to prove that
parent exit alone cannot authorize reuse.
