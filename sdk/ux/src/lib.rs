//! Theme and inspection support shared by Cranpose IDE plugins.
pub mod source_paths;
pub mod tree;
pub mod viewport;
use serde::Deserialize;

#[cfg(test)]
mod tests;

/// sRGB channels in the inclusive range 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

/// The colors the tool window draws with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Rgba,
    pub surface: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub on_accent: Rgba,
}

/// The palette used until the IDE reports its own, and when running alone.
pub const STANDALONE_PALETTE: Palette = Palette {
    background: Rgba(0.117, 0.122, 0.133, 1.0),
    surface: Rgba(0.169, 0.176, 0.188, 1.0),
    text: Rgba(0.875, 0.882, 0.898, 1.0),
    muted: Rgba(0.525, 0.541, 0.569, 1.0),
    accent: Rgba(0.208, 0.455, 0.941, 1.0),
    on_accent: Rgba(1.0, 1.0, 1.0, 1.0),
};

/// The IDE theme as the plugin sends it on `ide.theme`; colors are `#rrggbb`.
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
    /// Reads a `ide.theme` payload.
    pub fn parse(payload: &str) -> Option<Self> {
        serde_json::from_str(payload).ok()
    }

    /// The palette this theme paints with; a malformed color keeps its standalone value.
    pub fn palette(&self) -> Palette {
        let pick = |hex: &str, fallback: Rgba| parse_hex_color(hex).unwrap_or(fallback);
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
pub fn parse_hex_color(hex: &str) -> Option<Rgba> {
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
    Some(Rgba(channel(0..2)?, channel(2..4)?, channel(4..6)?, 1.0))
}

/// Black or white, whichever reads better on `background`.
pub fn readable_on(background: Rgba) -> Rgba {
    let linear = |channel: f32| {
        let channel = channel.clamp(0.0, 1.0);
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(background.0)
        + 0.7152 * linear(background.1)
        + 0.0722 * linear(background.2);
    let black_contrast = (luminance + 0.05) / 0.05;
    let white_contrast = 1.05 / (luminance + 0.05);
    if black_contrast >= white_contrast {
        Rgba(0.0, 0.0, 0.0, 1.0)
    } else {
        Rgba(1.0, 1.0, 1.0, 1.0)
    }
}

pub mod delivery;
