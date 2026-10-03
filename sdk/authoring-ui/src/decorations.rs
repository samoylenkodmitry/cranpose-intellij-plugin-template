//! Editor decorations drawn by Cranpose on a native GPU surface.
use super::*;

/// One visible counter, positioned in logical surface coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct CounterFrame {
    pub id: u64,
    pub bounds: [f32; 4],
    /// Editor viewport, in the same surface coordinates.
    pub clip: [f32; 4],
    pub label: String,
    /// Finite pulse progress, 0..1. One is the idle appearance.
    pub progress: f32,
    pub foreground: [f32; 4],
}
/// Lightning with a host-owned clock, preserved when its anchors move.
#[derive(Clone, Debug, PartialEq)]
pub struct LightningFrame {
    pub from: [f32; 2],
    pub target: [f32; 4],
    pub size: [f32; 2],
    pub request: u64,
    pub pending: bool,
    pub progress: f32,
}
/// A batch containing every visible decoration in one window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub counters: Vec<CounterFrame>,
    pub lightning: Option<LightningFrame>,
}

const COUNTER: &str = r"
@fragment fn effect_fs(input:VertexOutput)->@location(0) vec4<f32> {
    let size=u[0u].yz;
    let p=(input.uv*vec2<f32>(textureDimensions(input_texture))-u[62u].xy)*size/max(u[62u].zw,vec2<f32>(1.0));
    let t=clamp(u[0u].x,0.0,1.0);
    let pulse=sin(t*3.14159265)*(1.0-t);
    let q=abs(p-size*0.5)-(size*0.5-vec2<f32>(5.0));
    let d=length(max(q,vec2<f32>(0.0)))+min(max(q.x,q.y),0.0)-5.0;
    let inside=1.0-smoothstep(-0.8,0.8,d);
    let edge=exp(-abs(d+1.0)*1.6);
    let sweep=exp(-pow((p.x-size.x*t)/14.0,2.0))*pulse;
    let dot=exp(-length(p-vec2<f32>(5.0,size.y*0.5))*1.6);
    let energy=inside*(0.055+edge*(0.10+pulse*0.5)+sweep*0.16+dot*0.60);
    let rgb=mix(vec3<f32>(0.21,0.67,0.96),vec3<f32>(0.62,0.43,1.0),clamp(p.x/size.x,0.0,1.0));
    let alpha=clamp(energy,0.0,0.88);
    return vec4<f32>(rgb*alpha,alpha);
}";

/// Draws counters and lightning as one Cranpose composition.
#[composable]
pub fn Decorations(frame: Frame) {
    UiBox(
        Modifier::empty().fill_max_size(),
        BoxSpec::default(),
        move || {
            for counter in &frame.counters {
                let counter = counter.clone();
                cranpose::key(counter.id, move || Counter(counter.clone()));
            }
            if let Some(bolt) = &frame.lightning {
                super::lightning::LightningFrame(
                    bolt.from,
                    bolt.target,
                    bolt.size,
                    bolt.request,
                    bolt.pending,
                    bolt.progress,
                );
            }
        },
    );
}
#[composable]
fn Counter(counter: CounterFrame) {
    let [x, y, w, h] = counter.bounds;
    let [cx, cy, cw, ch] = counter.clip;
    let left = x.max(cx);
    let top = y.max(cy);
    let right = (x + w).min(cx + cw);
    let bottom = (y + h).min(cy + ch);
    if right <= left || bottom <= top {
        return;
    }
    UiBox(
        Modifier::empty()
            .absolute_offset(left, top)
            .width(right - left)
            .height(bottom - top)
            .clip_to_bounds(),
        BoxSpec::default(),
        move || {
            let mut inner = counter.clone();
            inner.bounds = [x - left, y - top, w, h];
            CounterContent(inner);
        },
    );
}
#[composable]
fn CounterContent(counter: CounterFrame) {
    let [x, y, w, h] = counter.bounds;
    static SOURCE: OnceLock<Arc<str>> = OnceLock::new();
    let mut shader = RuntimeShader::from_shared_source(
        SOURCE
            .get_or_init(|| Arc::from(format!("{RUNTIME_SHADER_PRELUDE_WGSL}{COUNTER}")))
            .clone(),
    );
    shader.set_float4(0, counter.progress, w, h, 0.0);
    UiBox(
        Modifier::empty()
            .absolute_offset(x, y)
            .width(w)
            .height(h)
            .clip_to_bounds(),
        BoxSpec::default(),
        move || {
            UiBox(
                Modifier::empty()
                    .fill_max_size()
                    .graphics_layer_value(GraphicsLayer {
                        render_effect: Some(RenderEffect::runtime_shader(shader.clone())),
                        compositing_strategy: CompositingStrategy::Offscreen,
                        ..Default::default()
                    }),
                BoxSpec::default(),
                || {},
            );
            let [r, g, b, a] = counter.foreground;
            Text(
                counter.label.clone(),
                Modifier::empty().absolute_offset(11.0, (h - 14.0).max(0.0) * 0.5),
                style(Color(r, g, b, a), 11.0),
            );
        },
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn counter_shader_validates() {
        let source = format!("{}{}", super::RUNTIME_SHADER_PRELUDE_WGSL, super::COUNTER);
        let module = naga::front::wgsl::parse_str(&source).expect("WGSL");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("valid shader");
    }
}
