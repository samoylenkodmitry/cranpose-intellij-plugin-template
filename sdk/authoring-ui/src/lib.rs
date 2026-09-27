//! Small, event-driven Cranpose surfaces shared by native IDE plugins.
use cranpose::{
    BasicTextField, Box as UiBox, BoxSpec, Color, Column, ColumnSpec, GraphicsLayer,
    LinearArrangement, Modifier, Row, RowSpec, SpanStyle, Text, TextFieldState, TextStyle,
    composable, remember, rememberHostMessages, rememberMutableStateOf, send_to_host,
    text::TextUnit,
};
use cranpose_core::CollectEvents;
use cranpose_ui_graphics::{
    CompositingStrategy, RUNTIME_SHADER_PRELUDE_WGSL, RenderEffect, RuntimeShader,
};
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, PartialEq)]
struct Colors {
    background: Color,
    surface: Color,
    text: Color,
    muted: Color,
    accent: Color,
}
#[expect(non_snake_case)]
fn rememberColors() -> Colors {
    let theme = cranpose_core::collectAsState(rememberHostMessages("ide.theme"), (), String::new());
    let p = cranpose_plugin_ux::IdeTheme::parse(&theme.get())
        .map_or(cranpose_plugin_ux::STANDALONE_PALETTE, |t| t.palette());
    let c = |v: cranpose_plugin_ux::Rgba| Color(v.0, v.1, v.2, v.3);
    Colors {
        background: c(p.background),
        surface: c(p.surface),
        text: c(p.text),
        muted: c(p.muted),
        accent: c(p.accent),
    }
}
fn style(color: Color, size: f32) -> TextStyle {
    TextStyle {
        span_style: SpanStyle {
            color: Some(color),
            font_size: TextUnit::Sp(size),
            ..Default::default()
        },
        ..Default::default()
    }
}

const ACCENT: &str = r"
@fragment fn effect_fs(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = max(vec2<f32>(u[62u].z, u[62u].w), vec2<f32>(1.0));
    let p = (input.uv * vec2<f32>(textureDimensions(input_texture)) - u[62u].xy) / size;
    let fade = smoothstep(0.0, 0.08, p.x) * (1.0 - smoothstep(0.90, 1.0, p.x));
    let alpha = u[0u].w * fade;
    let rgb = mix(u[0u].xyz, vec3<f32>(0.52, 0.63, 0.98), clamp(p.x, 0.0, 1.0) * 0.4);
    return vec4<f32>(rgb * alpha, alpha);
}";
/// Static shader accents redraw only when geometry or theme changes.
pub fn accent_effect(color: Color) -> RenderEffect {
    static SOURCE: OnceLock<Arc<str>> = OnceLock::new();
    let mut shader = RuntimeShader::from_shared_source(
        SOURCE
            .get_or_init(|| Arc::from(format!("{RUNTIME_SHADER_PRELUDE_WGSL}{ACCENT}")))
            .clone(),
    );
    shader.set_float4(0, color.0, color.1, color.2, color.3);
    RenderEffect::runtime_shader(shader)
}

#[composable]
pub fn EditorDecorations() {
    let items = rememberMutableStateOf(Vec::<Value>::new);
    let colors = rememberColors();
    CollectEvents(
        rememberHostMessages("ide.authoring.geometry"),
        (),
        move |payload: String| {
            if let Ok(v) = serde_json::from_str::<Value>(&payload) {
                items.set(v["items"].as_array().cloned().unwrap_or_default());
            }
        },
    );
    cranpose::embed::HostOverlay("editor", move || {
        UiBox(
            Modifier::empty().fill_max_size(),
            BoxSpec::default(),
            move || {
                for (index, item) in items.get().into_iter().take(256).enumerate() {
                    cranpose::key(index, move || {
                        let n = |key: &str| item[key].as_f64().unwrap_or_default() as f32;
                        let (x, y, w, h) = (n("x"), n("y"), n("width"), n("height"));
                        if item["kind"] == "call" {
                            let color =
                                Color(colors.accent.0, colors.accent.1, colors.accent.2, 0.55);
                            UiBox(
                                Modifier::empty()
                                    .offset(x, y)
                                    .width(w)
                                    .height(h)
                                    .graphics_layer(move || GraphicsLayer {
                                        render_effect: Some(accent_effect(color)),
                                        compositing_strategy: CompositingStrategy::Offscreen,
                                        ..Default::default()
                                    }),
                                BoxSpec::default(),
                                || {},
                            );
                        } else {
                            let label = item["label"].as_str().unwrap_or("◆").to_owned();
                            let tone = match item["tone"].as_str() {
                                Some("warning") => Color(0.85, 0.64, 0.28, 1.0),
                                Some("danger") => Color(0.87, 0.40, 0.40, 1.0),
                                Some("stable") => Color(0.33, 0.72, 0.56, 1.0),
                                _ => colors.accent,
                            };
                            UiBox(
                                Modifier::empty().offset(x, y).width(w).height(h),
                                BoxSpec::default().content_alignment(cranpose::Alignment::CENTER),
                                move || {
                                    if item["kind"] == "badge" {
                                        UiBox(
                                            Modifier::empty()
                                                .fill_max_size()
                                                .rounded_corners(5.0)
                                                .graphics_layer(move || GraphicsLayer {
                                                    render_effect: Some(accent_effect(Color(
                                                        tone.0, tone.1, tone.2, 0.14,
                                                    ))),
                                                    compositing_strategy:
                                                        CompositingStrategy::Offscreen,
                                                    ..Default::default()
                                                }),
                                            BoxSpec::default(),
                                            || {},
                                        );
                                    }
                                    Text(
                                        label.clone(),
                                        Modifier::empty(),
                                        style(
                                            tone,
                                            if item["kind"] == "badge" { 11.0 } else { 9.0 },
                                        ),
                                    );
                                },
                            );
                        }
                    });
                }
            },
        );
    });
}

fn submit(text: &str) {
    let _ = send_to_host("ide.authoring.edit", &json!({"value":text}).to_string());
}
#[composable]
fn ControlButton(label: &'static str, colors: Colors, action: impl Fn() + Clone + 'static) {
    UiBox(
        Modifier::empty()
            .height(30.0)
            .padding(1.0)
            .rounded_corners(6.0)
            .background(colors.surface)
            .clickable(move |_| action())
            .padding(8.0),
        BoxSpec::default().content_alignment(cranpose::Alignment::CENTER),
        move || {
            Text(label, Modifier::empty(), style(colors.text, 12.0));
        },
    );
}
/// Undoable source edits are performed by the host. The control never writes files.
#[composable]
pub fn ValueControl() {
    let colors = rememberColors();
    let kind = rememberMutableStateOf(String::new);
    let field = remember(|| TextFieldState::new("")).with(|v| *v);
    let original = rememberMutableStateOf(String::new);
    let message = rememberMutableStateOf(|| "Applies to source · Undo in the editor".to_owned());
    let ready = rememberMutableStateOf(|| false);
    CollectEvents(
        rememberHostMessages("ide.authoring.control"),
        (),
        move |payload: String| {
            if let Ok(v) = serde_json::from_str::<Value>(&payload) {
                let literal = &v["literal"];
                let text = literal["value"].as_str().unwrap_or_default();
                field.set_text(text);
                original.set(text.into());
                kind.set(literal["kind"].as_str().unwrap_or_default().into());
                ready.set(true);
            }
        },
    );
    CollectEvents(
        rememberHostMessages("ide.authoring.result"),
        (),
        move |payload: String| {
            if let Ok(v) = serde_json::from_str::<Value>(&payload) {
                message.set(if v["ok"] == true {
                    "Source updated · Undo in the editor".into()
                } else {
                    v["error"]
                        .as_str()
                        .unwrap_or("Could not apply value")
                        .into()
                });
            }
        },
    );
    // Text controls commit explicitly, preventing incomplete numeric input from
    // replacing valid source. Steppers and toggles commit each complete value.
    Column(
        Modifier::empty()
            .fill_max_size()
            .background(colors.background)
            .padding(16.0),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(12.0)),
        move || {
            Row(
                Modifier::empty().fill_max_width(),
                RowSpec::default().horizontal_arrangement(LinearArrangement::SpaceBetween),
                move || {
                    Text("◆  Live value", Modifier::empty(), style(colors.text, 15.0));
                    Text(kind.get(), Modifier::empty(), style(colors.muted, 11.0));
                },
            );
            BasicTextField(
                field,
                Modifier::empty()
                    .fill_max_width()
                    .height(38.0)
                    .rounded_corners(6.0)
                    .background(colors.surface)
                    .padding(9.0),
                style(colors.text, 14.0),
            );
            Row(
                Modifier::empty().fill_max_width(),
                RowSpec::default().horizontal_arrangement(LinearArrangement::spaced_by(8.0)),
                move || {
                    match kind.get().as_str() {
                        "int" | "float" => {
                            for (label, step) in [("−", -1.0), ("+", 1.0)] {
                                ControlButton(label, colors, move || {
                                    if let Some(value) =
                                        step_value(&field.text(), &kind.get(), step)
                                    {
                                        field.set_text(&value);
                                        submit(&value);
                                    }
                                });
                            }
                        }
                        "bool" => ControlButton("Toggle", colors, move || {
                            let value = if field.text() == "true" {
                                "false"
                            } else {
                                "true"
                            };
                            field.set_text(value);
                            submit(value);
                        }),
                        _ => {}
                    }
                    ControlButton("Apply", colors, move || {
                        if ready.get() {
                            submit(&field.text());
                        }
                    });
                    ControlButton("Reset", colors, move || {
                        let text = original.get();
                        field.set_text(&text);
                        submit(&text);
                    });
                },
            );
            Text(
                message.get(),
                Modifier::empty().fill_max_width(),
                style(colors.muted, 11.0),
            );
        },
    );
}
fn step_value(text: &str, kind: &str, step: f64) -> Option<String> {
    if kind == "int" {
        let value = text.parse::<i128>().ok()?;
        value.checked_add(step as i128).map(|v| v.to_string())
    } else {
        let value = text.parse::<f64>().ok()? + step;
        value.is_finite().then(|| {
            let s = value.to_string();
            if s.contains('.') || s.contains('e') {
                s
            } else {
                format!("{s}.0")
            }
        })
    }
}

#[composable]
pub fn ShowcaseCard() {
    let c = rememberColors();
    Column(
        Modifier::empty()
            .fill_max_size()
            .background(c.background)
            .padding(24.0),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(14.0)),
        move || {
            Text("CRANPOSE", Modifier::empty(), style(c.accent, 11.0));
            Text(
                "Start with something beautiful.",
                Modifier::empty(),
                style(c.text, 24.0),
            );
            Text("Showcase", Modifier::empty(), style(c.text, 17.0));
            Text(
                "A native Rust application with a star chart, liquid glass and custom GPU shaders.",
                Modifier::empty().fill_max_width(),
                style(c.muted, 13.0),
            );
            Text(
                "Desktop · Web · Android · iOS",
                Modifier::empty(),
                style(c.accent, 12.0),
            );
            Text(
                "Creates a copy of samoylenkodmitry/cranpose-showcase.\nChoose your project name and location above. Git and an internet connection are required.",
                Modifier::empty().fill_max_width(),
                style(c.muted, 12.0),
            );
        },
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_integer_steps_and_finite_floats() {
        assert_eq!(
            step_value("9007199254740993", "int", 1.0).as_deref(),
            Some("9007199254740994")
        );
        assert!(step_value(&i128::MAX.to_string(), "int", 1.0).is_none());
        assert_eq!(step_value("1.0", "float", 1.0).as_deref(), Some("2.0"));
        assert!(step_value("NaN", "float", 1.0).is_none());
    }
    #[test]
    fn accent_shader_is_valid() {
        naga::front::wgsl::parse_str(&format!("{RUNTIME_SHADER_PRELUDE_WGSL}{ACCENT}"))
            .expect("valid WGSL");
    }
}
