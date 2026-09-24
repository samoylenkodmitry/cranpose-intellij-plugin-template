//! A card drawn by an app-defined WGSL shader: aurora bands that drift with
//! time and lean toward the mouse pointer. The same shader runs on Metal,
//! Vulkan and DirectX 12.

use std::{cell::Cell, rc::Rc};

use cranpose::{
    Alignment, Box, BoxSpec, Color, GraphicsLayer, Modifier, MutableState, Point, PointerEventKind,
    PointerInputScope, composable, current_density, remember, rememberMutableStateOf,
};
use cranpose_animation::{AnimationSpec, RepeatMode, StartOffset, infiniteRepeatable};
use cranpose_ui_graphics::{CompositingStrategy, RenderEffect};

use crate::shader::ShaderSource;

/// How long one full cycle of the animation takes.
pub const CYCLE_MILLIS: u64 = 24_000;

const CORNER_RADIUS: f32 = 14.0;

pub(crate) static AURORA_SHADER: ShaderSource = ShaderSource::new(
    r"
fn band(p: vec2<f32>, pointer: vec2<f32>, lean: f32, t: f32, layer: f32) -> f32 {
    let tau = 6.28318530718;
    let crest = 0.52
        + 0.17 * sin(p.x * (3.0 + layer) + tau * t * (1.0 + layer) + layer * 1.7)
        + 0.06 * sin(p.x * 9.0 - tau * t * 2.0 + layer);
    let offset = p - pointer;
    let pull = lean * 0.22 * exp(-10.0 * dot(offset, offset));
    let distance = abs(p.y - crest + pull);
    return exp(-distance * (20.0 - 5.0 * layer));
}

@fragment
fn effect_fs(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = effect_size();
    let local = effect_local(input.uv);
    let p = local / size;

    let t = get_float(0u);
    let density = get_float(3u);
    let pointer = vec2<f32>(get_float(1u), get_float(2u)) * density / size;
    let lean = get_float(4u);
    let accent = vec3<f32>(get_float(8u), get_float(9u), get_float(10u));
    let dark = get_float(11u);

    var color = mix(vec3<f32>(0.93, 0.95, 0.99), vec3<f32>(0.02, 0.03, 0.08), dark);
    color = color + accent * band(p, pointer, lean, t, 0.0) * 0.75;
    color = color + vec3<f32>(0.25, 0.95, 0.65) * band(p, pointer, lean, t, 1.0) * 0.5;
    color = color + vec3<f32>(0.75, 0.35, 0.95) * band(p, pointer, lean, t, 2.0) * 0.35;
    let glow_offset = p - pointer;
    color = color + accent * lean * 0.3 * exp(-35.0 * dot(glow_offset, glow_offset));

    let radius = get_float(5u) * density;
    let half_size = size * 0.5;
    let q = abs(local - half_size) - half_size + vec2<f32>(radius);
    let edge = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    let coverage = clamp(0.5 - edge, 0.0, 1.0);

    let content = textureSample(input_texture, input_sampler, input.uv);
    let rgb = content.rgb + color * (1.0 - content.a);
    return vec4<f32>(rgb * coverage, coverage);
}
",
);

/// What the aurora shader is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuroraParams {
    pub phase: f32,
    pub pointer: Point,
    pub lean: f32,
    pub accent: Color,
    pub dark: bool,
}

/// The aurora effect for `params`: uniforms 0 phase, 1–2 pointer, 3 density,
/// 4 lean, 5 corner radius, 8–10 accent, 11 dark.
pub fn aurora_effect(params: &AuroraParams) -> RenderEffect {
    let mut shader = AURORA_SHADER.shader();
    shader.set_float(0, params.phase);
    shader.set_float2(1, params.pointer.x, params.pointer.y);
    shader.set_float(3, current_density());
    shader.set_float(4, params.lean);
    shader.set_float(5, CORNER_RADIUS);
    shader.set_float4(8, params.accent.0, params.accent.1, params.accent.2, 1.0);
    shader.set_float(11, if params.dark { 1.0 } else { 0.0 });
    RenderEffect::runtime_shader(shader)
}

/// A card of `height` logical pixels painted by the aurora shader, with
/// `content` on top. The animation runs while `animate` holds `true`.
#[composable]
pub fn AuroraCard(
    height: f32,
    accent: Color,
    dark: bool,
    animate: MutableState<bool>,
    content: impl FnMut() + 'static,
) {
    let transition = cranpose_animation::rememberInfiniteTransition("aurora");
    let phase = transition.animateFloat(
        0.0,
        1.0,
        infiniteRepeatable(
            AnimationSpec::linear(CYCLE_MILLIS),
            RepeatMode::Restart,
            StartOffset::default(),
        ),
        "aurora_phase",
    );
    let shown_phase = remember(|| Rc::new(Cell::new(0.0f32))).with(Rc::clone);
    let pointer = rememberMutableStateOf(|| Point {
        x: -1000.0,
        y: -1000.0,
    });
    let hovered = rememberMutableStateOf(|| false);
    let lean = cranpose_animation::animateFloatAsState(
        if hovered.get() { 1.0 } else { 0.0 },
        cranpose_animation::AnimationType::default(),
        "aurora_lean",
    );

    Box(
        Modifier::empty()
            .fill_max_width()
            .height(height)
            .graphics_layer(move || {
                if animate.get() {
                    shown_phase.set(phase.get());
                }
                let current = shown_phase.get();
                GraphicsLayer {
                    render_effect: Some(aurora_effect(&AuroraParams {
                        phase: current,
                        pointer: pointer.get(),
                        lean: lean.get(),
                        accent,
                        dark,
                    })),
                    compositing_strategy: CompositingStrategy::Offscreen,
                    ..Default::default()
                }
            })
            .pointer_input((), move |scope: PointerInputScope| async move {
                scope
                    .await_pointer_event_scope(|events| async move {
                        loop {
                            let event = events.await_pointer_event().await;
                            match event.kind {
                                PointerEventKind::Enter | PointerEventKind::Move => {
                                    hovered.set(true);
                                    pointer.set(event.position);
                                }
                                PointerEventKind::Exit | PointerEventKind::Cancel => {
                                    hovered.set(false);
                                }
                                _ => {}
                            }
                        }
                    })
                    .await;
            }),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        content,
    );
}
