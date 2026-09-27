//! Shared owned subprocesses, independent of JNI and the IDE host.
#[cfg(windows)]
pub use cranpose_plugin_process::cpu_time;
pub use cranpose_plugin_process::{Process, capture, is_running};
