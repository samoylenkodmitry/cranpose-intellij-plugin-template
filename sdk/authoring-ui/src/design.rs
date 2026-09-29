use super::*;
use cranpose::{BoxWithConstraintsScope as _, Slider, SliderSpec};
use cranpose_animation::{Easing, animate_float_as_state_with_initial, animateFloatAsState, tween};

/// Compact theme-aware action with a finite press transition.
#[composable]
pub fn ActionChip(label: String, selected: bool, action: impl Fn() + 'static) {
    let colors = rememberColors();
    let interaction = cranpose::rememberMutableInteractionSource();
    let pressed = interaction.collectIsPressedAsState();
    let scale = animateFloatAsState(
        if pressed.value() { 0.96 } else { 1.0 },
        tween(140, Easing::FastOutSlowInEasing),
        "action press",
    );
    cranpose::Button(
        Modifier::empty()
            .height(30.0)
            .rounded_corners(7.0)
            .background(if selected {
                Color(colors.accent.0, colors.accent.1, colors.accent.2, 0.15)
            } else {
                Color(colors.surface.0, colors.surface.1, colors.surface.2, 0.45)
            })
            .graphics_layer_value(GraphicsLayer {
                scale_x: scale.value(),
                scale_y: scale.value(),
                ..Default::default()
            }),
        cranpose::ButtonSpec::new().interaction_source(interaction),
        action,
        move || {
            Text(
                label.clone(),
                Modifier::empty().padding(7.0),
                style(if selected { colors.accent } else { colors.text }, 11.0),
            );
        },
    );
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NumberRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}
impl NumberRange {
    pub fn parse(min: &str, max: &str, step: &str, integer: bool) -> Option<Self> {
        let r = Self {
            min: min.parse().ok()?,
            max: max.parse().ok()?,
            step: step.parse().ok()?,
        };
        (r.min.is_finite()
            && r.max.is_finite()
            && r.step.is_finite()
            && r.min < r.max
            && r.step > 0.0
            && (r.max - r.min).is_finite()
            && ((r.max - r.min) / r.step).is_finite()
            && r.min.abs().max(r.max.abs()) <= 9_007_199_254_740_991.0
            && (!integer || [r.min, r.max, r.step].iter().all(|v| v.fract() == 0.0)))
        .then_some(r)
    }
    pub fn around(value: &str, integer: bool, suffix: &str) -> Self {
        let v = value.parse::<f64>().unwrap_or(0.0);
        let extent = if v.abs() <= 100.0 {
            100.0
        } else {
            (v.abs() * 2.0).min(9_007_199_254_740_991.0)
        };
        let (low, high) = Self::bounds(suffix);
        Self {
            min: if v < 0.0 { (-extent).max(low) } else { 0.0 },
            max: extent.min(high),
            step: if integer { 1.0 } else { 0.01 },
        }
    }
    pub fn bounds(suffix: &str) -> (f64, f64) {
        match suffix {
            "u8" => (0.0, 255.0),
            "u16" => (0.0, 65535.0),
            "i8" => (-128.0, 127.0),
            "i16" => (-32768.0, 32767.0),
            "u32" => (0.0, u32::MAX as f64),
            "i32" => (i32::MIN as f64, i32::MAX as f64),
            s if s.starts_with('u') => (0.0, 9_007_199_254_740_991.0),
            _ => (-9_007_199_254_740_991.0, 9_007_199_254_740_991.0),
        }
    }
    pub fn at(self, fraction: f32, integer: bool) -> String {
        let raw = self.min + (self.max - self.min) * f64::from(fraction.clamp(0.0, 1.0));
        let v = if fraction <= 0.0 {
            self.min
        } else if fraction >= 1.0 {
            self.max
        } else {
            (self.min + ((raw - self.min) / self.step).round() * self.step)
                .clamp(self.min, self.max)
        };
        if integer {
            format!("{v:.0}")
        } else {
            let decimals = |n: f64| {
                n.to_string()
                    .split_once('.')
                    .map_or(0, |(_, fraction)| fraction.len())
            };
            let places = decimals(self.min)
                .max(decimals(self.max))
                .max(decimals(self.step));
            if places <= 17 {
                let text = format!("{v:.places$}");
                if text.contains('.') {
                    let text = text.trim_end_matches('0');
                    if text.ends_with('.') {
                        format!("{text}0")
                    } else {
                        text.into()
                    }
                } else {
                    format!("{text}.0")
                }
            } else {
                format_float(v)
            }
        }
    }
    pub fn fraction(self, value: f64) -> f32 {
        ((value - self.min) / (self.max - self.min)).clamp(0.0, 1.0) as f32
    }
}
fn format_float(value: f64) -> String {
    let text = value.to_string();
    if !text.contains('.') && !text.contains('e') {
        format!("{text}.0")
    } else {
        text
    }
}
pub(crate) fn parse_color(text: &str) -> Option<[f64; 4]> {
    if let Some(hex) = text.trim().strip_prefix('#') {
        if !matches!(hex.len(), 6 | 8) || !hex.is_ascii() {
            return None;
        }
        let mut channels = [1.0; 4];
        for (index, channel) in channels.iter_mut().enumerate().take(hex.len() / 2) {
            *channel =
                f64::from(u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?) / 255.0;
        }
        Some(channels)
    } else {
        cranpose_plugin_authoring::runtime::color_channels(text)
    }
}
pub(crate) fn hex(c: [f64; 4]) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        (c[0] * 255.0).round() as u8,
        (c[1] * 255.0).round() as u8,
        (c[2] * 255.0).round() as u8,
        (c[3] * 255.0).round() as u8
    )
}
pub(crate) fn wire(c: [f64; 4]) -> String {
    c.map(format_float).join(",")
}
pub(crate) fn hsv(c: [f64; 4]) -> [f64; 4] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0) / 6.0
    } else if max == c[1] {
        ((c[2] - c[0]) / d + 2.0) / 6.0
    } else {
        ((c[0] - c[1]) / d + 4.0) / 6.0
    };
    [h, if max == 0.0 { 0.0 } else { d / max }, max, c[3]]
}
pub(crate) fn rgb(h: [f64; 4]) -> [f64; 4] {
    let k = |n: f64| (n + h[0] * 6.0).rem_euclid(6.0);
    let f = |n: f64| h[2] * (1.0 - h[1] * (k(n).min(4.0 - k(n))).clamp(0.0, 1.0));
    [f(5.0), f(3.0), f(1.0), h[3]]
}

const DESIGN: &str = r"
fn hsv_rgb(h: vec3<f32>) -> vec3<f32> {
    let p = abs(fract(h.xxx + vec3<f32>(0.0, 0.6666667, 0.3333333))*6.0-3.0);
    return h.z * mix(vec3<f32>(1.0), clamp(p-1.0, vec3<f32>(0.0), vec3<f32>(1.0)), h.y);
}
@fragment fn effect_fs(input: VertexOutput) -> @location(0) vec4<f32> {
    let size=max(u[62u].zw,vec2<f32>(1.0));
    let pixel=input.uv*vec2<f32>(textureDimensions(input_texture))-u[62u].xy;
    let p=clamp(pixel/size,vec2<f32>(0.0),vec2<f32>(1.0));
    let mode=u[1u].x;
    var rgb=u[0u].xyz;
    var alpha=u[0u].w;
    if mode == 1.0 { rgb=hsv_rgb(vec3<f32>(p.x,1.0,1.0)); alpha=1.0; }
    if mode == 2.0 { rgb=hsv_rgb(vec3<f32>(u[2u].x,p.x,u[2u].z)); alpha=1.0; }
    if mode == 3.0 { rgb=hsv_rgb(vec3<f32>(u[2u].xy,p.x)); alpha=1.0; }
    if mode == 4.0 { alpha=p.x; }
    if mode <= 4.0 {
        let checker=mix(0.20,0.34,step(1.0,(floor(pixel.x/7.0)+floor(pixel.y/7.0))%2.0));
        let radius=select(size.y*0.5,size.y*0.18,mode==0.0);
        let q=abs(pixel-size*0.5)-(size*0.5-vec2<f32>(radius));
        let distance=length(max(q,vec2<f32>(0.0)))+min(max(q.x,q.y),0.0)-radius;
        let mask=1.0-smoothstep(-0.5,0.5,distance);
        return vec4<f32>(mix(vec3<f32>(checker),rgb,alpha)*mask,mask);
    }
    let t=u[1u].y;
    let sweep=p.x-t*1.55+0.25;
    let beam=exp(-pow(sweep*9.0,2.0));
    let spectrum=0.55+0.45*cos(vec3<f32>(0.0,2.1,4.2)+p.x*5.0+p.y*1.4-t*4.0);
    let ray=pow(max(0.0,cos(p.y*15.0+sweep*28.0)),12.0)*beam;
    let edge=smoothstep(0.0,0.02,p.x)*(1.0-smoothstep(0.92,1.0,p.x));
    let rails=exp(-min(p.y,1.0-p.y)*35.0);
    let trail=exp(-pow((sweep+0.16)*4.0,2.0));
    if mode == 6.0 {
        // Quiet glass at rest; a spectral border blooms on entrance and press.
        let border=exp(-min(min(pixel.x,size.x-pixel.x),min(pixel.y,size.y-pixel.y))*0.8);
        alpha=alpha*(0.10+border*0.65+(beam+ray)*0.16*(1.0-t));
        rgb=mix(rgb,spectrum,0.32+border*0.3);
    } else {
        alpha=alpha*(1.0-t)*edge*(0.16*trail+beam*0.7+ray*0.4+rails*(beam+trail)*0.9);
        rgb=mix(rgb,spectrum,0.65);
        rgb=mix(rgb,vec3<f32>(0.9,0.98,1.0),beam*rails*0.85);
    }
    return vec4<f32>(rgb*alpha,alpha);
}";
fn effect(color: Color, mode: f32, progress: f32, hsv: [f64; 4]) -> RenderEffect {
    static SOURCE: OnceLock<Arc<str>> = OnceLock::new();
    let mut shader = RuntimeShader::from_shared_source(
        SOURCE
            .get_or_init(|| Arc::from(format!("{RUNTIME_SHADER_PRELUDE_WGSL}{DESIGN}")))
            .clone(),
    );
    shader.set_float4(0, color.0, color.1, color.2, color.3);
    shader.set_float4(4, mode, progress, 0.0, 0.0);
    shader.set_float4(
        8,
        hsv[0] as f32,
        hsv[1] as f32,
        hsv[2] as f32,
        hsv[3] as f32,
    );
    RenderEffect::runtime_shader(shader)
}
fn layer(effect: RenderEffect) -> GraphicsLayer {
    GraphicsLayer {
        render_effect: Some(effect),
        compositing_strategy: CompositingStrategy::Offscreen,
        ..Default::default()
    }
}

/// A bounded source-arrival sweep. Key the composition by navigation request
/// to restart it; no frame clock runs after its 900 ms transition completes.
#[composable]
pub fn ArrivalAccent(modifier: Modifier, color: Color) {
    let progress = animate_float_as_state_with_initial(
        0.0,
        1.0,
        tween(900, Easing::FastOutSlowInEasing),
        "source arrival",
    );
    UiBox(
        modifier.graphics_layer_value(layer(effect(color, 5.0, progress.value(), [0.0; 4]))),
        BoxSpec::default(),
        || {},
    );
}

/// Entrance glass is keyed to the control session, never to scrolling geometry.
#[composable]
pub(crate) fn ControlGlass(color: Color) {
    let progress = animate_float_as_state_with_initial(
        0.0,
        1.0,
        tween(500, Easing::FastOutSlowInEasing),
        "control entrance",
    );
    UiBox(
        Modifier::empty()
            .fill_max_size()
            .rounded_corners(10.0)
            .graphics_layer_value(layer(effect(
                Color(color.0, color.1, color.2, 0.32),
                6.0,
                progress.value(),
                [0.0; 4],
            ))),
        BoxSpec::default(),
        || {},
    );
}

/// Title row: what is being tuned, where it lives, and whether source has it.
#[composable]
pub(crate) fn ControlHeader(
    title: String,
    location: String,
    status: super::Status,
    colors: Colors,
) {
    Row(
        Modifier::empty().fill_max_width().height(22.0),
        RowSpec::default()
            .horizontal_arrangement(LinearArrangement::spaced_by(8.0))
            .vertical_alignment(VerticalAlignment::CenterVertically),
        move || {
            StatusDot(status.clone(), colors);
            Text(
                title.clone(),
                Modifier::empty().weight(1.0),
                super::style(colors.text, 13.0),
            );
            Text(
                location.clone(),
                Modifier::empty(),
                super::style(colors.muted, 11.0),
            );
        },
    );
}

/// Green once the host confirmed the source edit, amber while one is in
/// flight, red when the last input could not be applied.
#[composable]
fn StatusDot(status: super::Status, colors: Colors) {
    let tone = match status {
        super::Status::Ready => Color(colors.accent.0, colors.accent.1, colors.accent.2, 0.9),
        super::Status::Pending => Color(0.92, 0.70, 0.30, 1.0),
        super::Status::Applied => Color(0.36, 0.78, 0.55, 1.0),
        super::Status::Invalid(_) | super::Status::Failed(_) => Color(0.92, 0.42, 0.40, 1.0),
    };
    let glow = animateFloatAsState(
        if matches!(status, super::Status::Applied) {
            1.0
        } else {
            0.0
        },
        tween(420, Easing::FastOutSlowInEasing),
        "status glow",
    );
    UiBox(
        Modifier::empty().width(14.0).height(14.0),
        BoxSpec::default().content_alignment(cranpose::Alignment::CENTER),
        move || {
            UiBox(
                Modifier::empty()
                    .width(8.0 + 6.0 * glow.value())
                    .height(8.0 + 6.0 * glow.value())
                    .rounded_corners(7.0)
                    .background(Color(tone.0, tone.1, tone.2, 0.25 * glow.value())),
                BoxSpec::default(),
                || {},
            );
            UiBox(
                Modifier::empty()
                    .width(8.0)
                    .height(8.0)
                    .rounded_corners(4.0)
                    .background(tone),
                BoxSpec::default(),
                || {},
            );
        },
    );
}

/// One quiet line: how edits behave, or why the last input waits.
#[composable]
pub(crate) fn ControlHint(status: super::Status, colors: Colors) {
    let (text, color) = match status {
        super::Status::Invalid(hint) => (hint.to_owned(), Color(0.92, 0.62, 0.40, 1.0)),
        super::Status::Failed(error) => (error, Color(0.92, 0.48, 0.45, 1.0)),
        _ => (
            "Applied as you change it. Undo in the editor reverts it.".to_owned(),
            colors.muted,
        ),
    };
    Text(
        text,
        Modifier::empty().fill_max_width(),
        super::style(color, 11.0),
    );
}

/// Two explicit states; choosing one writes it immediately as its own Undo step.
#[composable]
pub(crate) fn BoolSwitch(
    field: TextFieldState,
    sent: super::LastSent,
    status: cranpose_core::MutableState<super::Status>,
    colors: Colors,
    request: u64,
) {
    let value = field.text() == "true";
    let position = animateFloatAsState(
        if value { 1.0 } else { 0.0 },
        tween(180, Easing::FastOutSlowInEasing),
        "bool switch",
    );
    let sent = sent.clone();
    Row(
        Modifier::empty()
            .fill_max_width()
            .height(36.0)
            .rounded_corners(9.0)
            .background(colors.surface)
            .padding(3.0),
        RowSpec::default().vertical_alignment(VerticalAlignment::CenterVertically),
        move || {
            let sent = sent.clone();
            UiBox(
                Modifier::empty().fill_max_size(),
                BoxSpec::default(),
                move || {
                    let sent = sent.clone();
                    cranpose::BoxWithConstraints(Modifier::empty().fill_max_size(), move |scope| {
                        let half = scope.constraints().max_width * 0.5;
                        UiBox(
                            Modifier::empty()
                                .offset(half * position.value(), 0.0)
                                .width(half)
                                .fill_max_height()
                                .rounded_corners(7.0)
                                .background(Color(
                                    colors.accent.0,
                                    colors.accent.1,
                                    colors.accent.2,
                                    0.22,
                                )),
                            BoxSpec::default(),
                            || {},
                        );
                    });
                    Row(
                        Modifier::empty().fill_max_size(),
                        RowSpec::default().vertical_alignment(VerticalAlignment::CenterVertically),
                        move || {
                            for option in ["false", "true"] {
                                let sent = sent.clone();
                                let selected = (option == "true") == value;
                                UiBox(
                                    Modifier::empty().weight(1.0).fill_max_height().clickable(
                                        move |_| {
                                            if field.text() != option {
                                                *sent.borrow_mut() = option.to_owned();
                                                field.set_text(option);
                                                status.set(super::Status::Pending);
                                                submit(option, request);
                                            }
                                        },
                                    ),
                                    BoxSpec::default()
                                        .content_alignment(cranpose::Alignment::CENTER),
                                    move || {
                                        Text(
                                            option,
                                            Modifier::empty(),
                                            super::style(
                                                if selected { colors.text } else { colors.muted },
                                                13.0,
                                            ),
                                        );
                                    },
                                );
                            }
                        },
                    );
                },
            );
        },
    );
}

/// The swatch and its editable hex form, side by side.
#[composable]
pub(crate) fn ColorField(
    field: TextFieldState,
    draft: cranpose_core::MutableState<super::color::ColorDraft>,
    colors: Colors,
) {
    Row(
        Modifier::empty().fill_max_width().height(36.0),
        RowSpec::default()
            .horizontal_arrangement(LinearArrangement::spaced_by(8.0))
            .vertical_alignment(VerticalAlignment::CenterVertically),
        move || {
            let c = draft.get().rgba;
            UiBox(
                Modifier::empty()
                    .width(36.0)
                    .height(36.0)
                    .graphics_layer_value(layer(effect(
                        Color(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32),
                        0.0,
                        0.0,
                        [0.0; 4],
                    ))),
                BoxSpec::default(),
                || {},
            );
            BasicTextField(
                field,
                Modifier::empty()
                    .weight(1.0)
                    .height(36.0)
                    .rounded_corners(8.0)
                    .background(colors.surface)
                    .padding(9.0),
                super::style(colors.text, 14.0),
            );
        },
    );
}
#[composable]
pub(crate) fn Seekbar(
    value: f32,
    colors: Colors,
    mode: usize,
    hsva: [f64; 4],
    change: impl Fn(f32) + 'static,
    finish: impl Fn() + 'static,
) {
    Slider(
        Modifier::empty().fill_max_width().height(28.0),
        value,
        change,
        finish,
        SliderSpec::new().thumb_extent(16.0),
        move |scope| {
            let pressed = animateFloatAsState(
                if scope.is_dragging() { 1.2 } else { 1.0 },
                tween(140, Easing::FastOutSlowInEasing),
                "seek thumb",
            );
            let rgba = rgb(hsva);
            let color = Color(
                rgba[0] as f32,
                rgba[1] as f32,
                rgba[2] as f32,
                rgba[3] as f32,
            );
            let track = Modifier::empty()
                .offset(8.0, 10.0)
                .width(scope.track_extent())
                .height(8.0)
                .rounded_corners(4.0);
            UiBox(
                if mode > 0 {
                    track.graphics_layer_value(layer(effect(color, mode as f32, 0.0, hsva)))
                } else {
                    track.background(colors.surface)
                },
                BoxSpec::default(),
                || {},
            );
            if mode == 0 {
                UiBox(
                    Modifier::empty()
                        .offset(8.0, 10.0)
                        .width(scope.track_extent() * value)
                        .height(8.0)
                        .rounded_corners(4.0)
                        .graphics_layer_value(layer(accent_effect(colors.accent))),
                    BoxSpec::default(),
                    || {},
                );
            }
            UiBox(
                Modifier::empty()
                    .offset(scope.thumb_offset(), 6.0)
                    .width(16.0)
                    .height(16.0)
                    .rounded_corners(8.0)
                    .background(colors.text)
                    .graphics_layer_value(GraphicsLayer {
                        scale_x: pressed.value(),
                        scale_y: pressed.value(),
                        ..Default::default()
                    }),
                BoxSpec::default().content_alignment(cranpose::Alignment::CENTER),
                move || {
                    UiBox(
                        Modifier::empty()
                            .width(8.0)
                            .height(8.0)
                            .rounded_corners(4.0)
                            .background(colors.accent),
                        BoxSpec::default(),
                        || {},
                    );
                },
            );
        },
    );
}

#[composable]
pub(crate) fn NumberControls(
    field: TextFieldState,
    integer: bool,
    suffix: String,
    sent: super::LastSent,
    status: cranpose_core::MutableState<super::Status>,
    colors: Colors,
    request: u64,
) {
    let bounds = NumberRange::bounds(&suffix);
    let range =
        rememberMutableStateOf(move || NumberRange::around(&field.text(), integer, &suffix));
    let minimum = remember(|| TextFieldState::new(range.get().min.to_string())).with(|s| *s);
    let maximum = remember(|| TextFieldState::new(range.get().max.to_string())).with(|s| *s);
    let step = remember(|| TextFieldState::new(range.get().step.to_string())).with(|s| *s);
    let gesture = rememberMutableStateOf(|| 1u64);
    // Range fields are live too: a complete, valid range replaces the slider's.
    let (low, high, grain) = (minimum.text(), maximum.text(), step.text());
    cranpose_core::SideEffect(move || {
        if let Some(next) = NumberRange::parse(&low, &high, &grain, integer)
            .filter(|r| r.min >= bounds.0 && r.max <= bounds.1)
            && next != range.get()
        {
            range.set(next);
        }
    });
    Column(
        Modifier::empty().fill_max_width(),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(4.0)),
        move || {
            // Incomplete typing keeps the slider at the last value sent to source.
            let current = field
                .text()
                .parse::<f64>()
                .or_else(|_| sent.borrow().parse::<f64>());
            if let Ok(current) = current.as_ref().copied()
                && current.abs() <= 9_007_199_254_740_991.0
            {
                let sent = sent.clone();
                Seekbar(
                    range.get().fraction(current),
                    colors,
                    0,
                    [0.0; 4],
                    move |fraction| {
                        let value = range.get().at(fraction, integer);
                        if field.text() != value {
                            *sent.borrow_mut() = value.clone();
                            field.set_text(&value);
                            status.set(super::Status::Pending);
                            seek(&value, gesture.get(), request);
                        }
                    },
                    move || gesture.set(gesture.get().wrapping_add(1)),
                );
            } else {
                Text(
                    "Use exact input outside ±2^53",
                    Modifier::empty(),
                    super::style(colors.muted, 11.0),
                );
            }
            Row(
                Modifier::empty().fill_max_width().height(26.0),
                RowSpec::default()
                    .horizontal_arrangement(LinearArrangement::spaced_by(6.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    RangeField(minimum, colors);
                    UiBox(Modifier::empty().weight(1.0), BoxSpec::default(), || {});
                    Text("step", Modifier::empty(), super::style(colors.muted, 10.0));
                    RangeField(step, colors);
                    UiBox(Modifier::empty().weight(1.0), BoxSpec::default(), || {});
                    RangeField(maximum, colors);
                },
            );
        },
    );
}

#[composable]
fn RangeField(state: TextFieldState, colors: Colors) {
    BasicTextField(
        state,
        Modifier::empty()
            .width(64.0)
            .height(24.0)
            .rounded_corners(6.0)
            .background(Color(
                colors.surface.0,
                colors.surface.1,
                colors.surface.2,
                0.6,
            ))
            .padding(5.0),
        super::style(colors.muted, 11.0),
    );
}

#[composable]
pub(crate) fn ColorControls(
    field: TextFieldState,
    draft: cranpose_core::MutableState<super::color::ColorDraft>,
    sent: super::LastSent,
    status: cranpose_core::MutableState<super::Status>,
    colors: Colors,
    request: u64,
    alpha: bool,
) {
    let gesture = rememberMutableStateOf(|| 1u64);
    Column(
        Modifier::empty().fill_max_width(),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(2.0)),
        move || {
            for (index, label) in ["Hue", "Saturation", "Brightness", "Opacity"]
                .into_iter()
                .enumerate()
                .take(if alpha { 4 } else { 3 })
            {
                let sent = sent.clone();
                cranpose::key(index, move || {
                    let sent = sent.clone();
                    Row(
                        Modifier::empty().fill_max_width(),
                        RowSpec::default()
                            .horizontal_arrangement(LinearArrangement::SpaceBetween)
                            .vertical_alignment(VerticalAlignment::CenterVertically),
                        move || {
                            Text(label, Modifier::empty(), super::style(colors.muted, 11.0));
                            Text(
                                if index == 0 {
                                    format!("{:.0}°", draft.get().hsva[index] * 360.0)
                                } else {
                                    format!("{:.0}%", draft.get().hsva[index] * 100.0)
                                },
                                Modifier::empty(),
                                super::style(colors.text, 11.0),
                            );
                        },
                    );
                    Seekbar(
                        draft.get().hsva[index] as f32,
                        colors,
                        index + 1,
                        draft.get().hsva,
                        move |fraction| {
                            let mut next = draft.get();
                            next.seek(index, fraction);
                            *sent.borrow_mut() = next.text.clone();
                            field.set_text(&next.text);
                            status.set(super::Status::Pending);
                            seek(&next.source, gesture.get(), request);
                            draft.set(next);
                        },
                        move || gesture.set(gesture.get().wrapping_add(1)),
                    );
                });
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_snap_and_validate_ranges() {
        let range = NumberRange::parse("-20", "40", "5", true).expect("range");
        assert_eq!(range.at(0.51, true), "10");
        assert_eq!(range.at(1.0, true), "40");
        for (min, max, step) in [
            ("2", "1", "1"),
            ("0", "1", "0"),
            ("NaN", "1", "1"),
            ("0", "inf", "1"),
            ("0", "10", "0.5"),
        ] {
            assert!(NumberRange::parse(min, max, step, true).is_none());
        }
        assert!(NumberRange::parse("9007199254740992", "9007199254740994", "1", true).is_none());
        assert_eq!(NumberRange::around("255", true, "u8").max, 255.0);
        assert_eq!(
            NumberRange::parse("0.1", "0.9", "0.1", false)
                .expect("decimal range")
                .at(0.25, false),
            "0.3"
        );
        assert_eq!(
            NumberRange::parse("0", "0.95", "0.1", false)
                .expect("non-grid endpoint")
                .at(1.0, false),
            "0.95"
        );
        assert_eq!(
            NumberRange::parse(
                "0",
                "0.00000000000000000002",
                "0.00000000000000000001",
                false
            )
            .expect("tiny range")
            .at(0.5, false)
            .parse::<f64>()
            .expect("number"),
            1e-20
        );
    }
    #[test]
    fn colors_round_trip_and_reject_incomplete_hex() {
        for c in [
            [0.0, 0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.25, 0.6, 0.9, 0.5],
            [1.0, 0.0, 0.0, 1.0],
        ] {
            let out = rgb(hsv(c));
            for i in 0..4 {
                assert!((c[i] - out[i]).abs() < 1e-8);
            }
        }
        assert_eq!(hex(parse_color("#8ECDB3").expect("hex")), "#8ECDB3FF");
        assert!(parse_color("#ff").is_none());
        assert!(parse_color("#🦀ffff").is_none());
        naga::front::wgsl::parse_str(&format!("{RUNTIME_SHADER_PRELUDE_WGSL}{DESIGN}"))
            .expect("design shader");
    }
}
