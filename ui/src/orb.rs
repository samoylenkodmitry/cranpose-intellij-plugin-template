//! A floating window of its own, shaped by the shader that draws it: a
//! glowing plasma orb on a transparent, borderless window. Drag it anywhere;
//! click it to change its colors.
//!
//! It is an ordinary `Modifier::window` subtree. Run standalone it becomes a
//! desktop window; inside the IDE the plugin opens it as a transparent window
//! owned by the IDE frame.

use cranpose::{
    Alignment, Box, BoxSpec, Color, GraphicsLayer, Modifier, WindowConfig, WindowFocus,
    WindowModifierExt, composable, rememberMutableStateOf,
};
use cranpose_animation::{AnimationSpec, RepeatMode, StartOffset, infiniteRepeatable};
use cranpose_ui_graphics::{CompositingStrategy, RenderEffect};

use crate::shader::ShaderSource;

/// The orb window's side, in logical pixels.
pub const ORB_SIZE: f32 = 150.0;

/// Where the orb first appears, relative to the tool window's top-left corner.
pub const ORB_OFFSET: (f32, f32) = (-ORB_SIZE - 24.0, 48.0);

/// The color pairs a click cycles through.
pub const PALETTES: [(Color, Color); 3] = [
    (Color(0.20, 0.45, 1.00, 1.0), Color(0.85, 0.30, 1.00, 1.0)),
    (Color(1.00, 0.45, 0.10, 1.0), Color(1.00, 0.90, 0.30, 1.0)),
    (Color(0.10, 0.85, 0.60, 1.0), Color(0.30, 0.60, 1.00, 1.0)),
];

pub(crate) static ORB_SHADER: ShaderSource = ShaderSource::new(
    r"
@fragment
fn effect_fs(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = effect_size();
    let local = effect_local(input.uv);
    let t = get_float(0u) * 6.2831853;
    let inner = vec3<f32>(get_float(4u), get_float(5u), get_float(6u));
    let outer = vec3<f32>(get_float(8u), get_float(9u), get_float(10u));

    let radius = min(size.x, size.y) * 0.36;
    let offset = (local - size * 0.5) / radius;
    let d = length(offset);

    let swirl = sin(offset.x * 5.0 + t) + sin(offset.y * 6.0 - t * 2.0)
        + sin((offset.x + offset.y) * 4.0 + t * 3.0) + sin(d * 9.0 - t * 2.0);
    let plasma = 0.5 + 0.125 * swirl;
    var color = mix(inner, outer, plasma);
    let shine = pow(max(0.0, 1.0 - length(offset - vec2<f32>(-0.35, -0.4)) * 1.6), 3.0);
    color = color * (0.55 + 0.6 * (1.0 - d * d)) + vec3<f32>(1.0) * shine * 0.8;

    let body = 1.0 - smoothstep(0.97, 1.0, d);
    let glow = max(0.0, 1.0 - (d - 1.0) / 0.35);
    let halo = glow * glow * (1.0 - body) * 0.5;
    let alpha = clamp(body + halo, 0.0, 1.0);
    let rgb = color * body + mix(inner, outer, 0.5) * halo;
    return vec4<f32>(rgb, alpha);
}
",
);

/// The orb shader at `phase` of its cycle, in the colors of `palette`.
pub fn orb_effect(phase: f32, palette: (Color, Color)) -> RenderEffect {
    let (inner, outer) = palette;
    let mut shader = ORB_SHADER.shader();
    shader.set_float(0, phase);
    shader.set_float4(4, inner.0, inner.1, inner.2, 1.0);
    shader.set_float4(8, outer.0, outer.1, outer.2, 1.0);
    RenderEffect::runtime_shader(shader)
}

/// The floating orb window.
#[composable]
pub fn FloatingOrb() {
    let palette = rememberMutableStateOf(|| 0usize);
    let transition = cranpose_animation::rememberInfiniteTransition("orb");
    let phase = transition.animateFloat(
        0.0,
        1.0,
        infiniteRepeatable(
            AnimationSpec::linear(8_000),
            RepeatMode::Restart,
            StartOffset::default(),
        ),
        "orb_phase",
    );
    Box(
        Modifier::empty().window(
            WindowConfig::borderless("Cranpose orb", ORB_SIZE, ORB_SIZE)
                .with_transparent(true)
                .with_shadow(false)
                .with_focus(WindowFocus::Never)
                .with_host_window_position(ORB_OFFSET.0, ORB_OFFSET.1),
        ),
        BoxSpec::default().content_alignment(Alignment::CENTER),
        move || {
            Box(
                Modifier::empty()
                    .fill_max_size()
                    .graphics_layer(move || GraphicsLayer {
                        render_effect: Some(orb_effect(
                            phase.get(),
                            PALETTES[palette.get() % PALETTES.len()],
                        )),
                        compositing_strategy: CompositingStrategy::Offscreen,
                        ..Default::default()
                    })
                    .window_drag_area(|| {}, || {})
                    .clickable(move |_| palette.set(palette.get() + 1)),
                BoxSpec::default(),
                || {},
            );
        },
    );
}

#[cfg(test)]
#[path = "tests/orb_tests.rs"]
mod tests;
