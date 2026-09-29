//! Small, event-driven Cranpose surfaces shared by native IDE plugins.
use cranpose::{
    BasicTextField, Box as UiBox, BoxSpec, Color, Column, ColumnSpec, GraphicsLayer,
    LinearArrangement, Modifier, Row, RowSpec, SpanStyle, Text, TextFieldState, TextStyle,
    VerticalAlignment, composable, remember, rememberHostMessages, rememberMutableStateOf,
    send_to_host, text::TextUnit,
};
use cranpose_core::CollectEvents;
use cranpose_ui_graphics::{
    CompositingStrategy, RUNTIME_SHADER_PRELUDE_WGSL, RenderEffect, RuntimeShader,
};
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
mod color;
mod design;
mod lightning;
pub use design::{ActionChip, ArrivalAccent};

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

/// Transient editor effects that may cross text lines. Persistent glyphs,
/// badges and underlines are painted by the IDE editor itself.
#[composable]
pub fn EditorDecorations() {
    lightning::LiveEditLightning();
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
                for item in items.get().into_iter().filter(|i| i["kind"] == "arrival") {
                    let n = |key: &str| item[key].as_f64().unwrap_or_default() as f32;
                    let (x, y, w, h) = (n("x"), n("y"), n("width"), n("height"));
                    cranpose::key(item["request"].as_u64().unwrap_or_default(), move || {
                        ArrivalAccent(
                            Modifier::empty()
                                .offset(x, y)
                                .width(w)
                                .height(h)
                                .rounded_corners(4.0),
                            Color(colors.accent.0, colors.accent.1, colors.accent.2, 0.60),
                        );
                    });
                }
            },
        );
    });
}

fn submit(text: &str, request: u64) {
    let _ = send_to_host(
        "ide.authoring.edit",
        &json!({"value":text,"request":request}).to_string(),
    );
}
fn seek(text: &str, gesture: u64, request: u64) {
    let _ = send_to_host(
        "ide.authoring.edit",
        &json!({"value":text,"gesture":gesture,"request":request}).to_string(),
    );
}
/// Typing shares one Undo group per control session; each drag gets its own.
const TYPING: u64 = 0;

/// What the source currently holds, as far as this control knows.
#[derive(Clone, PartialEq)]
pub(crate) enum Status {
    Ready,
    Pending,
    Applied,
    Invalid(&'static str),
    Failed(String),
}

/// The value the control last sent, shared by typing and dragging so a
/// slider-driven text update is never sent a second time as typing.
pub(crate) type LastSent = std::rc::Rc<std::cell::RefCell<String>>;

/// Undoable source edits are performed by the host. The control never writes files.
#[composable]
pub fn ValueControl() {
    let control = rememberMutableStateOf(|| None::<String>);
    CollectEvents(
        rememberHostMessages("ide.authoring.control"),
        (),
        move |payload: String| {
            if serde_json::from_str::<Value>(&payload).is_ok() {
                control.set(Some(payload));
            }
        },
    );
    if let Some(payload) = control.get() {
        cranpose::key(payload.clone(), move || {
            if let Ok(init) = serde_json::from_str::<Value>(&payload) {
                ControlForm(init);
            }
        });
    }
}

/// Every complete value applies as it is entered; there is nothing to confirm.
/// Incomplete input (such as `-` or `#12`) waits without touching the source.
#[composable]
fn ControlForm(init: Value) {
    let colors = rememberColors();
    let request = init["request"].as_u64().unwrap_or_default();
    let literal = &init["literal"];
    let text = literal["value"].as_str().unwrap_or_default().to_owned();
    let kind = literal["kind"].as_str().unwrap_or_default().to_owned();
    let suffix = literal["suffix"].as_str().unwrap_or_default().to_owned();
    let draft = color::ColorDraft::new(&text)
        .unwrap_or_else(|| color::ColorDraft::new("0,0,0,1").expect("opaque black"));
    let display = if kind == "color" {
        draft.text.clone()
    } else {
        text.clone()
    };
    let initial = display.clone();
    let field = remember(move || TextFieldState::new(&display)).with(|v| *v);
    let sent = remember(move || LastSent::new(initial.into())).with(Clone::clone);
    let color = rememberMutableStateOf(move || draft);
    let status = rememberMutableStateOf(|| Status::Ready);
    let binding = init["binding"].as_str().map(str::to_owned);
    let line = literal["range"]["line"].as_u64().unwrap_or_default();
    CollectEvents(
        rememberHostMessages("ide.authoring.result"),
        (),
        move |payload: String| {
            if let Ok(v) = serde_json::from_str::<Value>(&payload)
                && v["request"].as_u64().unwrap_or_default() == request
            {
                status.set(if v["ok"] == true {
                    Status::Applied
                } else {
                    Status::Failed(
                        v["error"]
                            .as_str()
                            .unwrap_or("Could not apply value")
                            .into(),
                    )
                });
            }
        },
    );
    let heading = match kind.as_str() {
        "color" => "Color",
        "bool" => "Toggle",
        "int" | "float" => "Number",
        _ => "Text",
    };
    let location = if binding.is_some() {
        format!("{heading} · L{line}")
    } else {
        format!("L{line}")
    };
    let title = binding.unwrap_or_else(|| heading.to_owned());
    UiBox(
        Modifier::empty()
            .fill_max_size()
            .background(colors.background),
        BoxSpec::default(),
        move || {
            design::ControlGlass(colors.accent);
            let kind = kind.clone();
            let suffix = suffix.clone();
            let title = title.clone();
            let location = location.clone();
            let sent = sent.clone();
            Column(
                Modifier::empty().fill_max_size().padding(14.0),
                ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(10.0)),
                move || {
                    design::ControlHeader(title.clone(), location.clone(), status.get(), colors);
                    match kind.as_str() {
                        "bool" => design::BoolSwitch(field, sent.clone(), status, colors, request),
                        "color" => {
                            design::ColorField(field, color, colors);
                            design::ColorControls(
                                field,
                                color,
                                sent.clone(),
                                status,
                                colors,
                                request,
                                suffix.split(',').count() != 3,
                            );
                        }
                        "int" | "float" => {
                            ValueField(field, colors);
                            let integer = kind == "int";
                            let (kind, suffix) = (kind.clone(), suffix.clone());
                            let sent = sent.clone();
                            cranpose::key(kind.clone(), move || {
                                design::NumberControls(
                                    field,
                                    integer,
                                    suffix.clone(),
                                    sent.clone(),
                                    status,
                                    colors,
                                    request,
                                )
                            });
                        }
                        _ => ValueField(field, colors),
                    }
                    if kind != "bool" {
                        LiveCommit(
                            field,
                            kind.clone(),
                            suffix.clone(),
                            color,
                            sent.clone(),
                            status,
                            request,
                        );
                    }
                    design::ControlHint(status.get(), colors);
                },
            );
        },
    );
}

#[composable]
fn ValueField(field: TextFieldState, colors: Colors) {
    BasicTextField(
        field,
        Modifier::empty()
            .fill_max_width()
            .height(36.0)
            .rounded_corners(8.0)
            .background(colors.surface)
            .padding(9.0),
        style(colors.text, 14.0),
    );
}

/// Sends typed text once it forms a complete value of the literal's kind.
#[composable]
fn LiveCommit(
    field: TextFieldState,
    kind: String,
    suffix: String,
    color: cranpose_core::MutableState<color::ColorDraft>,
    sent: LastSent,
    status: cranpose_core::MutableState<Status>,
    request: u64,
) {
    let text = field.text();
    cranpose_core::SideEffect(move || {
        if *sent.borrow() == text {
            return;
        }
        let value = if kind == "color" {
            let mut draft = color.get();
            draft.sync_text(&text).map(|_| {
                let source = draft.source.clone();
                color.set(draft);
                source
            })
        } else {
            complete_value(&kind, &suffix, &text)
        };
        match value {
            Some(value) => {
                *sent.borrow_mut() = text.clone();
                status.set(Status::Pending);
                seek(&value, TYPING, request);
            }
            None => status.set(Status::Invalid(match kind.as_str() {
                "int" => "Enter a whole number in range for this type",
                "float" => "Enter a finite number",
                "color" => "Enter #RRGGBB or #RRGGBBAA",
                _ => "Enter a value",
            })),
        }
    });
}

/// The source text a complete input produces, or `None` while it is incomplete.
fn complete_value(kind: &str, suffix: &str, text: &str) -> Option<String> {
    let trimmed = text.trim();
    match kind {
        "int" => {
            let value = trimmed.replace('_', "").parse::<i128>().ok()?;
            let (low, high) = integer_bounds(suffix);
            (low..=high).contains(&value).then(|| value.to_string())
        }
        "float" => float_literal(trimmed, !suffix.is_empty()).then(|| trimmed.to_owned()),
        "bool" => matches!(trimmed, "true" | "false").then(|| trimmed.to_owned()),
        _ => Some(text.to_owned()),
    }
}
fn integer_bounds(suffix: &str) -> (i128, i128) {
    match suffix {
        "u8" => (0, u8::MAX.into()),
        "u16" => (0, u16::MAX.into()),
        "u32" => (0, u32::MAX.into()),
        "u64" | "usize" => (0, u64::MAX.into()),
        "u128" => (0, i128::MAX),
        "i8" => (i8::MIN.into(), i8::MAX.into()),
        "i16" => (i16::MIN.into(), i16::MAX.into()),
        "i32" => (i32::MIN.into(), i32::MAX.into()),
        "i64" | "isize" => (i64::MIN.into(), i64::MAX.into()),
        _ => (i128::MIN, i128::MAX),
    }
}
/// A finite decimal Rust float literal. Without a type suffix it needs a
/// fraction or exponent, or the edit would turn the literal into an integer.
fn float_literal(text: &str, suffixed: bool) -> bool {
    let digits = |s: &str| {
        s.starts_with(|c: char| c.is_ascii_digit())
            && s.chars().all(|c| c.is_ascii_digit() || c == '_')
    };
    let body = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e.strip_prefix(['+', '-']).unwrap_or(e))),
        None => (body, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (mantissa, None),
    };
    digits(whole)
        && fraction.is_none_or(digits)
        && exponent.is_none_or(digits)
        && (suffixed || fraction.is_some() || exponent.is_some())
        && text
            .replace('_', "")
            .parse::<f64>()
            .is_ok_and(f64::is_finite)
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
            Text("Showcase", Modifier::empty(), style(c.text, 20.0));
            Text(
                "A sample Cranpose application with a star chart, glass effects and custom shaders. It builds for desktop, web, Android and iOS.",
                Modifier::empty().fill_max_width(),
                style(c.muted, 13.0),
            );
            Text(
                "The sample comes with the plugin, so creating the project needs no network.\nThe project opens with a desktop run configuration and the preview.\nThe first build needs Rust and downloads the dependencies.",
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
    fn only_complete_values_are_sent() {
        assert_eq!(complete_value("int", "", "42").as_deref(), Some("42"));
        assert_eq!(
            complete_value("int", "", " 1_000 ").as_deref(),
            Some("1000")
        );
        assert!(complete_value("int", "", "-").is_none());
        assert!(complete_value("int", "u8", "256").is_none());
        assert!(complete_value("int", "u8", "-1").is_none());
        assert_eq!(
            complete_value("int", "", "9007199254740993").as_deref(),
            Some("9007199254740993")
        );
        assert_eq!(complete_value("float", "", "1.5").as_deref(), Some("1.5"));
        assert!(complete_value("float", "", "1e").is_none());
        assert!(complete_value("float", "", "NaN").is_none());
        assert!(complete_value("float", "", "5.").is_none());
        assert!(complete_value("float", "", ".5").is_none());
        assert!(complete_value("float", "", "2").is_none());
        assert_eq!(complete_value("float", "f32", "2").as_deref(), Some("2"));
        assert_eq!(
            complete_value("float", "", "-2.5e-3").as_deref(),
            Some("-2.5e-3")
        );
        assert!(complete_value("float", "", "1e999").is_none());
        assert!(complete_value("bool", "", "tru").is_none());
        assert_eq!(complete_value("string", "", "").as_deref(), Some(""));
    }
    #[test]
    fn accent_shader_is_valid() {
        naga::front::wgsl::parse_str(&format!("{RUNTIME_SHADER_PRELUDE_WGSL}{ACCENT}"))
            .expect("valid WGSL");
    }
}
