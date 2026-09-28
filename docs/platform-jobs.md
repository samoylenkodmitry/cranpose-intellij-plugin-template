# Platform build infrastructure

`cranpose-plugin-process::execute` streams both output pipes in bounded 8 KiB
chunks through a bounded queue. It owns the command's process scope and supports
`Cancellation` plus a deadline. Success requires an exited process tree. Timeout,
cancellation and nonzero exits return errors. Run it on a worker thread; output
callbacks should return promptly. The existing capture API remains available for
small command responses.

`cranpose_plugin_host::jobs::start` runs one background operation per trusted
IntelliJ project. Its context supplies the root, a cancellation token and a native
surface event channel. `Context::send` drops results after cancellation or disposal.
The host sends a final `job_finished` event with cancellation/error details. Cancel
requests retain the slot until the worker exits, preventing overlapping compilers.
Project disposal cancels the job without waiting on the IDE event thread.

This is shared infrastructure for Cranpose Build and other long-running native
tools. It does not select a toolchain or alter application build configuration.

Validation: Rust subprocess tests exercise stdout/stderr backpressure, long lines,
nonzero exits, pre-cancellation, deadlines and descendant exit. The actual IDEA
suite checks exclusive operation ownership, explicit cancellation, slot release,
project disposal and rejection of late work.
