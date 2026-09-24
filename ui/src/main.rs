#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use cranpose::{AppLauncher, embed::EmbedEndpoint};
use cranpose_intellij_ui::ToolWindow;

fn main() {
    let launcher = AppLauncher::new()
        .with_title("Cranpose Tool Window")
        .with_size(420, 760);
    match EmbedEndpoint::from_env() {
        Some(endpoint) => launcher.run_embedded(endpoint, ToolWindow),
        #[cfg(feature = "desktop")]
        None => launcher.run(ToolWindow),
        #[cfg(not(feature = "desktop"))]
        None => {
            eprintln!(
                "start this binary from the IDE plugin, or build it with the `desktop` feature"
            );
            std::process::exit(2)
        }
    }
}
