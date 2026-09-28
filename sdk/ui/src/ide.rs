//! The messages this tool window and its IDE exchange.
//!
//! Every channel carries JSON. The IDE side is the shared Rust host;
//! the two files are the whole contract. Send with
//! [`send_to_host`](cranpose::send_to_host) and receive with
//! [`rememberHostMessages`](cranpose::rememberHostMessages).

use cranpose::{Color, rememberHostMessages, send_to_host};
use cranpose_core::collectAsState;
use serde::Deserialize;

/// IDE → UI: the IDE's current look, sent on connect and on every theme change.
pub const THEME_CHANNEL: &str = "ide.theme";
/// IDE → UI: the file in the focused editor, `{}` when none is open.
pub const EDITOR_CHANNEL: &str = "ide.editor";
/// UI → IDE: show a balloon notification.
pub const NOTIFY_CHANNEL: &str = "ide.notify";
/// UI → IDE: open a file in the editor.
pub const OPEN_CHANNEL: &str = "ide.open";

/// The colors the tool window draws with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub surface: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub on_accent: Color,
}

impl Palette {
    /// A subdued selection fill derived from the current IDE colors.
    pub fn selection(self) -> Color {
        Color(
            self.accent.0 * 0.18 + self.background.0 * 0.82,
            self.accent.1 * 0.18 + self.background.1 * 0.82,
            self.accent.2 * 0.18 + self.background.2 * 0.82,
            1.0,
        )
    }
}

pub use cranpose_plugin_ux::IdeTheme;

impl From<cranpose_plugin_ux::Palette> for Palette {
    fn from(value: cranpose_plugin_ux::Palette) -> Self {
        let color = |c: cranpose_plugin_ux::Rgba| Color(c.0, c.1, c.2, c.3);
        Self {
            background: color(value.background),
            surface: color(value.surface),
            text: color(value.text),
            muted: color(value.muted),
            accent: color(value.accent),
            on_accent: color(value.on_accent),
        }
    }
}

/// The file in the IDE's focused editor, as sent on [`EDITOR_CHANNEL`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct EditorFile {
    pub path: String,
    pub name: String,
}

impl EditorFile {
    /// Reads an [`EDITOR_CHANNEL`] payload; `None` when no editor is open.
    pub fn parse(payload: &str) -> Option<Self> {
        serde_json::from_str(payload).ok()
    }
}

/// Asks the IDE to show a notification; `false` when running without an IDE.
pub fn notify_ide(title: &str, content: &str) -> bool {
    let payload = serde_json::json!({ "title": title, "content": content });
    send_to_host(NOTIFY_CHANNEL, &payload.to_string())
}

/// Asks the IDE to open `path`; `false` when running without an IDE.
pub fn open_in_ide(path: &str) -> bool {
    let payload = serde_json::json!({ "path": path });
    send_to_host(OPEN_CHANNEL, &payload.to_string())
}

/// The palette of the IDE's current theme, following every change.
#[expect(non_snake_case)]
#[track_caller]
pub fn rememberPalette() -> Palette {
    let theme = collectAsState(rememberHostMessages(THEME_CHANNEL), (), String::new());
    IdeTheme::parse(&theme.get())
        .map_or(cranpose_plugin_ux::STANDALONE_PALETTE, |theme| {
            theme.palette()
        })
        .into()
}

#[cfg(test)]
#[path = "tests/ide_tests.rs"]
mod tests;
