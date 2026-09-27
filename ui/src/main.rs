#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use cranpose::{AppLauncher, embed::EmbedEndpoint};
use cranpose_intellij_ui::ToolWindow;

fn main() {
    let content: fn() = match std::env::var("CRANPOSE_AUTHORING").as_deref() {
        Ok("overlay") => cranpose_plugin_authoring_ui::EditorDecorations,
        Ok("value") => cranpose_plugin_authoring_ui::ValueControl,
        Ok("wizard") => cranpose_plugin_authoring_ui::ShowcaseCard,
        _ => ToolWindow,
    };
    let launcher = AppLauncher::new()
        .with_title("Cranpose Tool Window")
        .with_size(420, 760);
    match EmbedEndpoint::from_env() {
        Some(endpoint) => launcher.run_embedded(endpoint, content),
        #[cfg(feature = "desktop")]
        None => launcher.run(content),
        #[cfg(not(feature = "desktop"))]
        None => {
            eprintln!(
                "start this binary from the IDE plugin, or build it with the `desktop` feature"
            );
            std::process::exit(2)
        }
    }
}
