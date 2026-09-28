//! Compact controls that follow the IDE palette.
use crate::ide::Palette;
use cranpose::{Button, ButtonSpec, Modifier, SpanStyle, Text, TextStyle, text::TextUnit};

/// Render a compact action, with selected and disabled appearances.
#[expect(non_snake_case)]
pub fn ActionButton(
    palette: Palette,
    label: &str,
    enabled: bool,
    selected: bool,
    action: impl FnMut() + 'static,
) {
    let label = label.to_owned();
    let mut action = action;
    Button(
        Modifier::empty()
            .rounded_corners(6.0)
            .background(if selected {
                palette.selection()
            } else {
                palette.background
            })
            .padding(7.0),
        ButtonSpec::default(),
        move || {
            if enabled {
                action();
            }
        },
        move || {
            Text(
                label.clone(),
                Modifier::empty(),
                label_style(palette, !enabled),
            );
        },
    );
}

/// The compact body or secondary text used in task controls.
pub fn label_style(palette: Palette, muted: bool) -> TextStyle {
    TextStyle {
        span_style: SpanStyle {
            color: Some(if muted { palette.muted } else { palette.text }),
            font_size: TextUnit::Sp(12.0),
            ..Default::default()
        },
        ..Default::default()
    }
}
