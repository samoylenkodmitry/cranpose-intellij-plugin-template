//! The tool window: a shader card, the IDE's focused file with a history of
//! recent ones, and controls that talk back to the IDE.

use cranpose::{
    BasicTextField, Box, BoxSpec, Brush, Button, ButtonSpec, Color, Column, ColumnSpec,
    LinearArrangement, Modifier, MutableState, Row, RowSpec, ScrollState, SpanStyle, Text,
    TextFieldState, TextStyle, VerticalAlignment, composable, remember, rememberHostMessages,
    rememberMutableStateOf,
    text::{FontWeight, TextUnit},
};
use cranpose_core::CollectEvents;

use crate::{
    aurora::AuroraCard,
    ide::{EDITOR_CHANNEL, EditorFile, Palette, notify_ide, open_in_ide, rememberPalette},
};

/// How many files the history keeps.
pub const RECENT_FILES: usize = 6;

/// Puts `file` at the front of `recent`, dropping an older entry for the same
/// path and anything past [`RECENT_FILES`].
pub fn remember_file(recent: &[EditorFile], file: EditorFile) -> Vec<EditorFile> {
    std::iter::once(file.clone())
        .chain(
            recent
                .iter()
                .filter(|entry| entry.path != file.path)
                .cloned(),
        )
        .take(RECENT_FILES)
        .collect()
}

/// The root of the tool window.
#[composable]
pub fn ToolWindow() {
    let palette = rememberPalette();
    let dark = cranpose::isSystemInDarkTheme();
    let scroll = remember(|| ScrollState::new(0.0)).with(|state| *state);
    let animate = rememberMutableStateOf(|| true);
    let clicks = rememberMutableStateOf(|| 0u32);
    let current = rememberMutableStateOf(|| None::<EditorFile>);
    let recent = rememberMutableStateOf(Vec::<EditorFile>::new);
    let draft = remember(|| TextFieldState::new("")).with(|state| *state);

    CollectEvents(
        rememberHostMessages(EDITOR_CHANNEL),
        (),
        move |payload: String| {
            let file = EditorFile::parse(&payload);
            if let Some(file) = file.clone() {
                recent.set(remember_file(&recent.get(), file));
            }
            current.set(file);
        },
    );

    Column(
        Modifier::empty()
            .fill_max_size()
            .background(palette.background)
            .vertical_scroll(scroll, false)
            .padding(12.0),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(12.0)),
        move || {
            Header(palette);
            AuroraCard(170.0, palette.accent, dark, animate, move || {
                Column(Modifier::empty(), ColumnSpec::default(), move || {
                    Text(
                        "WGSL RuntimeShader",
                        Modifier::empty(),
                        heading(Color(1.0, 1.0, 1.0, 0.95)),
                    );
                    Text(
                        "move the pointer over the card",
                        Modifier::empty(),
                        caption(Color(1.0, 1.0, 1.0, 0.75)),
                    );
                });
            });
            Row(
                Modifier::empty().fill_max_width(),
                RowSpec::default().horizontal_arrangement(LinearArrangement::spaced_by(8.0)),
                move || {
                    let label = if animate.get() { "Pause" } else { "Animate" };
                    Action(palette, label, move || animate.set(!animate.get()));
                    Action(palette, "Clicked", move || clicks.set(clicks.get() + 1));
                    Text(
                        format!("{} times", clicks.get()),
                        Modifier::empty().padding(8.0),
                        body(palette.text),
                    );
                },
            );
            EditorCard(palette, current, recent);
            MessageCard(palette, draft);
        },
    );
}

#[composable]
fn Header(palette: Palette) {
    Column(Modifier::empty(), ColumnSpec::default(), move || {
        Text(
            "Cranpose tool window",
            Modifier::empty(),
            heading(palette.text),
        );
        Text(
            "Rendered off screen by wgpu, streamed into the IDE",
            Modifier::empty(),
            caption(palette.muted),
        );
    });
}

#[composable]
fn Card(palette: Palette, title: &'static str, content: impl FnMut() + 'static) {
    Column(
        Modifier::empty()
            .fill_max_width()
            .background(palette.surface)
            .rounded_corners(10.0)
            .padding(12.0),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(6.0)),
        move || {
            Text(title, Modifier::empty(), caption(palette.muted));
            content();
        },
    );
}

#[composable]
fn EditorCard(
    palette: Palette,
    current: MutableState<Option<EditorFile>>,
    recent: MutableState<Vec<EditorFile>>,
) {
    Card(palette, "FOCUSED EDITOR", move || {
        match current.get() {
            Some(file) => {
                Text(file.name.clone(), Modifier::empty(), heading(palette.text));
                Text(file.path, Modifier::empty(), caption(palette.muted));
            }
            None => {
                Text("No file is open", Modifier::empty(), body(palette.muted));
            }
        }
        let history = recent.get();
        if history.len() > 1 {
            Text(
                "Recent — click to reopen",
                Modifier::empty().padding(4.0),
                caption(palette.muted),
            );
            for file in history.into_iter().skip(1) {
                let path = file.path.clone();
                Row(
                    Modifier::empty()
                        .fill_max_width()
                        .rounded_corners(6.0)
                        .padding(6.0)
                        .clickable(move |_| {
                            open_in_ide(&path);
                        }),
                    RowSpec::default()
                        .vertical_alignment(VerticalAlignment::CenterVertically)
                        .horizontal_arrangement(LinearArrangement::spaced_by(8.0)),
                    move || {
                        Box(
                            Modifier::empty()
                                .size_points(6.0, 6.0)
                                .draw_behind(move |scope| {
                                    scope.draw_round_rect(
                                        Brush::solid(palette.accent),
                                        cranpose::CornerRadii::uniform(3.0),
                                    );
                                }),
                            BoxSpec::new(),
                            || {},
                        );
                        Text(file.name.clone(), Modifier::empty(), body(palette.text));
                    },
                );
            }
        }
    });
}

#[composable]
fn MessageCard(palette: Palette, draft: TextFieldState) {
    Card(palette, "NOTIFY THE IDE", move || {
        Box(
            Modifier::empty()
                .fill_max_width()
                .background(palette.background)
                .rounded_corners(6.0)
                .padding(8.0),
            BoxSpec::default(),
            move || {
                if draft.text().is_empty() {
                    Text("Type a message…", Modifier::empty(), body(palette.muted));
                }
                BasicTextField(
                    draft,
                    Modifier::empty().fill_max_width(),
                    body(palette.text),
                );
            },
        );
        Action(palette, "Send notification", move || {
            let text = draft.text();
            let content = if text.is_empty() {
                "Hello from Cranpose!".to_string()
            } else {
                text
            };
            notify_ide("Cranpose", &content);
            draft.set_text("");
        });
    });
}

#[composable]
fn Action(palette: Palette, label: &'static str, on_click: impl Fn() + 'static) {
    Button(
        Modifier::empty()
            .background(palette.accent)
            .rounded_corners(6.0)
            .padding(8.0),
        ButtonSpec::default(),
        on_click,
        move || {
            Text(label, Modifier::empty(), body(palette.on_accent));
        },
    );
}

fn body(color: Color) -> TextStyle {
    sized(color, 13.0, None)
}

fn caption(color: Color) -> TextStyle {
    sized(color, 11.0, None)
}

fn heading(color: Color) -> TextStyle {
    sized(color, 16.0, Some(FontWeight::BOLD))
}

fn sized(color: Color, size: f32, weight: Option<FontWeight>) -> TextStyle {
    TextStyle {
        span_style: SpanStyle {
            color: Some(color),
            font_size: TextUnit::Sp(size),
            font_weight: weight,
            ..SpanStyle::default()
        },
        ..TextStyle::default()
    }
}

#[cfg(test)]
#[path = "tests/tool_window_tests.rs"]
mod tests;
