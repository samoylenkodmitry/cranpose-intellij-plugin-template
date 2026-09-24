//! The messages this tool window and its IDE exchange.
//!
//! Every channel carries JSON. The IDE side is the plugin's `IdeBridge.kt`;
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

/// The palette used until the IDE reports its own, and when running alone.
pub const STANDALONE_PALETTE: Palette = Palette {
    background: Color(0.117, 0.122, 0.133, 1.0),
    surface: Color(0.169, 0.176, 0.188, 1.0),
    text: Color(0.875, 0.882, 0.898, 1.0),
    muted: Color(0.525, 0.541, 0.569, 1.0),
    accent: Color(0.208, 0.455, 0.941, 1.0),
    on_accent: Color(1.0, 1.0, 1.0, 1.0),
};

/// The IDE theme as the plugin sends it on [`THEME_CHANNEL`]; colors are `#rrggbb`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct IdeTheme {
    pub dark: bool,
    pub background: String,
    pub surface: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
}

impl IdeTheme {
    /// Reads a [`THEME_CHANNEL`] payload.
    pub fn parse(payload: &str) -> Option<Self> {
        serde_json::from_str(payload).ok()
    }

    /// The palette this theme paints with; a malformed color keeps its standalone value.
    pub fn palette(&self) -> Palette {
        let pick = |hex: &str, fallback: Color| parse_hex_color(hex).unwrap_or(fallback);
        let accent = pick(&self.accent, STANDALONE_PALETTE.accent);
        Palette {
            background: pick(&self.background, STANDALONE_PALETTE.background),
            surface: pick(&self.surface, STANDALONE_PALETTE.surface),
            text: pick(&self.text, STANDALONE_PALETTE.text),
            muted: pick(&self.muted, STANDALONE_PALETTE.muted),
            accent,
            on_accent: readable_on(accent),
        }
    }
}

/// A `#rrggbb` color, or `None` for anything else.
pub fn parse_hex_color(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| {
        digits
            .get(range)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map(|value| f32::from(value) / 255.0)
    };
    Some(Color(channel(0..2)?, channel(2..4)?, channel(4..6)?, 1.0))
}

/// Black or white, whichever reads better on `background`.
pub fn readable_on(background: Color) -> Color {
    let luminance = 0.2126 * background.0 + 0.7152 * background.1 + 0.0722 * background.2;
    if luminance > 0.55 {
        Color(0.0, 0.0, 0.0, 1.0)
    } else {
        Color(1.0, 1.0, 1.0, 1.0)
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
    IdeTheme::parse(&theme.get()).map_or(STANDALONE_PALETTE, |theme| theme.palette())
}

#[cfg(test)]
#[path = "tests/ide_tests.rs"]
mod tests;
