//! The Rust side of the plugin: the whole tool window UI.
//!
//! `main.rs` runs [`ToolWindow`] inside the IDE when the plugin starts the
//! binary, and as a desktop window otherwise. [`ide`] is the message contract
//! with the plugin's `IdeBridge.kt`.

pub mod aurora;
pub mod effects;
pub mod ide;
pub mod orb;
pub mod shader;
pub mod tool_window;

pub use tool_window::ToolWindow;
