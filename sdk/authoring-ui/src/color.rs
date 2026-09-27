//! Exact source channels and HSV editing state, independent of the hex label.
use super::design::{hex, hsv, parse_color, rgb, wire};

#[derive(Clone, PartialEq)]
pub(crate) struct ColorDraft {
    pub rgba: [f64; 4],
    pub hsva: [f64; 4],
    pub text: String,
    pub source: String,
}

impl ColorDraft {
    pub fn new(source: &str) -> Option<Self> {
        let mut rgba = parse_color(source)?;
        // New hex input has eight-bit precision. Six decimal places retain it
        // within half a millionth while keeping generated source readable.
        let source = if source.trim().starts_with('#') {
            rgba = rgba.map(compact);
            wire(rgba)
        } else {
            source.into()
        };
        Some(Self {
            rgba,
            hsva: hsv(rgba),
            text: hex(rgba),
            source,
        })
    }

    /// An untouched or slider-updated hex label never becomes a new RGB value.
    /// Invalid drafts leave the last valid preview intact and cannot be applied.
    pub fn sync_text(&mut self, text: &str) -> Option<bool> {
        if self.text == text {
            return Some(false);
        }
        let mut next = Self::new(text)?;
        next.text = text.into();
        *self = next;
        Some(true)
    }

    pub fn seek(&mut self, axis: usize, fraction: f32) {
        self.hsva[axis] = f64::from(fraction.clamp(0.0, 1.0));
        if axis == 3 {
            // Opacity has no reason to convert or round the existing RGB.
            self.rgba[3] = compact(self.hsva[3]);
        } else {
            let generated = rgb(self.hsva);
            for (channel, value) in self.rgba[..3].iter_mut().zip(generated) {
                *channel = compact(value);
            }
        }
        // Keep the unsimplified HSV state during dragging, including hue when
        // saturation/value is zero; never feed rounded RGB back into the axes.
        self.text = hex(self.rgba);
        self.source = wire(self.rgba);
    }
}

fn compact(value: f64) -> f64 {
    (value.clamp(0.0, 1.0) * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_apply_and_opacity_preserve_precise_rgb() {
        let original = "0.123456789012345,0.42,0.000000000001,0.123456789";
        let mut draft = ColorDraft::new(original).expect("valid color");
        let rgb = draft.rgba[..3].to_vec();
        assert_eq!(draft.sync_text(&draft.text.clone()), Some(false));
        assert_eq!(draft.source, original);
        for n in 0..1000 {
            draft.seek(3, (n % 101) as f32 / 100.0);
            assert_eq!(draft.rgba[..3], rgb);
            assert_eq!(draft.sync_text(&draft.text.clone()), Some(false));
            // Closing and reopening the popup must not accumulate hex drift.
            draft = ColorDraft::new(&draft.source).expect("valid color");
            assert_eq!(draft.rgba[..3], rgb);
        }
    }

    #[test]
    fn hsv_edits_remain_compact_without_hex_round_trips() {
        let mut draft = ColorDraft::new("0.19,0.42,0.31,0.123456789").expect("valid color");
        draft.seek(0, 0.75);
        assert_eq!(draft.source, "0.305,0.19,0.42,0.123456789");
        for _ in 0..1000 {
            draft.seek(0, 0.25);
            assert_eq!(draft.source, "0.305,0.42,0.19,0.123456789");
            draft = ColorDraft::new(&draft.source).expect("valid color");
            draft.seek(0, 0.75);
            assert_eq!(draft.source, "0.305,0.19,0.42,0.123456789");
            draft = ColorDraft::new(&draft.source).expect("valid color");
        }
        draft.seek(1, 0.0);
        draft.seek(0, 0.5);
        draft.seek(1, 1.0);
        assert_eq!(draft.source, "0.0,0.42,0.42,0.123456789");
        draft.seek(2, 0.0);
        draft.seek(0, 0.0);
        draft.seek(2, 0.42);
        assert_eq!(draft.source, "0.42,0.0,0.0,0.123456789");
    }

    #[test]
    fn edited_hex_is_explicit_and_invalid_drafts_do_not_change_color() {
        let mut draft = ColorDraft::new("0.19,0.42,0.31,1").expect("valid color");
        let before = draft.clone();
        assert_eq!(draft.sync_text("#12"), None);
        assert!(draft == before);
        assert_eq!(draft.sync_text("#12345678"), Some(true));
        assert_eq!(draft.source, "0.070588,0.203922,0.337255,0.470588");
        let before = draft.clone();
        assert_eq!(draft.sync_text("#12345678"), Some(false));
        assert!(draft == before);
        for byte in 0..=255 {
            let value = f64::from(byte) / 255.0;
            assert!((compact(value) - value).abs() <= 0.0000005);
            assert_eq!((compact(value) * 255.0).round() as u8, byte);
        }
    }
}
