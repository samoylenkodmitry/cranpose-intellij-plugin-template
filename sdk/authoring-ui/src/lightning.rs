//! Finite edit-to-preview lightning, drawn entirely by Cranpose.
use cranpose::{
    Box as UiBox, BoxSpec, GraphicsLayer, Modifier, composable, rememberHostMessages,
    rememberMutableStateOf,
};
use cranpose_animation::{Easing, animate_float_as_state_with_initial, tween};
use cranpose_core::CollectEvents;
use cranpose_ui_graphics::{
    CompositingStrategy, RUNTIME_SHADER_PRELUDE_WGSL, RenderEffect, RuntimeShader,
};
use serde_json::Value;
use std::sync::{Arc, OnceLock};

const LIGHTNING: &str = r"
fn hash(x:f32)->f32 { return fract(sin(x*127.1+311.7)*43758.5453); }
fn jagged(s:f32, seed:f32)->f32 {
    let p=s*19.0;
    return mix(hash(floor(p)+seed),hash(floor(p)+1.0+seed),fract(p))*2.0-1.0;
}
@fragment fn effect_fs(input:VertexOutput)->@location(0) vec4<f32> {
    let p=(input.uv*vec2<f32>(textureDimensions(input_texture))-u[62u].xy)*u[3u].xy/max(u[62u].zw,vec2<f32>(1.0));
    let start=u[0u].xy;
    let t=u[2u].x;
    if u[2u].z>0.5 {
        // A small charging constellation beside the edit. This uses a finite
        // ten-second animation, so a disconnected host cannot leave an idle loop.
        let seconds=t*10.0;
        let d=p-start;
        let radius=length(d);
        let angle=atan2(d.y,d.x);
        let envelope=smoothstep(0.0,0.015,t)*(1.0-smoothstep(0.90,1.0,t));
        let sweep=pow(0.5+0.5*cos(angle-seconds*5.0),5.0);
        var energy=exp(-abs(radius-14.0)*1.1)*(0.18+sweep*0.65);
        energy+=exp(-abs(radius-23.0)*1.6)*(0.5+0.5*sin(angle*3.0+seconds*3.0))*0.22;
        for(var i=0;i<5;i=i+1){
            let a=f32(i)*1.25664+seconds*2.8;
            let at=vec2<f32>(cos(a),sin(a))*(11.0+3.0*sin(seconds*2.0+f32(i)));
            energy+=exp(-length(d-at)*1.3)*0.65;
        }
        let rgb=mix(vec3<f32>(0.22,0.80,1.0),vec3<f32>(0.78,0.48,1.0),0.5+0.5*sin(angle+seconds));
        let alpha=clamp(energy*envelope,0.0,0.75);
        return vec4<f32>(rgb*alpha,alpha);
    }
    let bounds=u[1u];
    let end=vec2<f32>(select(bounds.x,bounds.x+bounds.z,start.x>bounds.x+bounds.z*0.5),bounds.y+bounds.w*0.5);
    let delta=end-start;
    let span=max(length(delta),1.0);
    let axis=delta/span;
    let normal=vec2<f32>(-axis.y,axis.x);
    let s=dot(p-start,axis)/span;
    let seed=u[2u].y;
    let envelope=smoothstep(0.0,0.04,t)*(1.0-smoothstep(0.45,1.0,t));
    let head=min(t*5.0,1.02);
    let reach=(1.0-smoothstep(head-0.035,head+0.02,s))*step(0.0,s)*step(s,1.0);
    // Two rapid, bounded discharge changes, then a continuous fade; no idle clock.
    let burst=floor(min(t,0.23)*13.0);
    let bend=jagged(clamp(s,0.0,1.0),seed+burst*41.0)*15.0*sin(clamp(s,0.0,1.0)*3.14159);
    let d=abs(dot(p-start,normal)-bend);
    let core=exp(-d*d/1.4);
    let halo=exp(-d*0.24)*0.66;
    let echo=exp(-abs(dot(p-start,normal)-bend*1.7-3.0)*0.65)*0.22;
    let tail=0.5+0.5*exp(-abs(s-head)*9.0);
    var energy=(core+halo+echo)*reach*envelope*tail;
    let fork_s=clamp((s-0.48)/0.3,0.0,1.0);
    let fork=abs(dot(p-start,normal)-bend-26.0*sin(fork_s*3.14159));
    energy+=exp(-fork*0.7)*sin(fork_s*3.14159)*reach*envelope*0.45;
    let origin=length(p-start);
    energy+=exp(-origin*0.3)*envelope*(1.0-t)*0.8;
    let q=abs(p-(bounds.xy+bounds.zw*0.5))-bounds.zw*0.5;
    let border=abs(length(max(q,vec2<f32>(0.0)))+min(max(q.x,q.y),0.0));
    let impact=smoothstep(0.13,0.23,t)*(1.0-smoothstep(0.3,1.0,t));
    let ring=exp(-border*0.45)*impact*0.65;
    let spark=exp(-length(p-end)*0.17)*impact;
    energy+=ring+spark;
    // Arrival releases a soft expanding contour and a handful of sparks.
    let outward=max(t-0.18,0.0);
    let distance=length(max(q,vec2<f32>(0.0)))+min(max(q.x,q.y),0.0);
    energy+=exp(-abs(distance-outward*34.0)*1.0)*impact*0.36;
    for(var i=0;i<7;i=i+1){
        let a=f32(i)*0.8976+seed;
        let at=end+vec2<f32>(cos(a),sin(a))*outward*70.0;
        energy+=exp(-length(p-at)*1.3)*impact*0.38;
    }
    let spectrum=mix(vec3<f32>(0.30,0.80,1.0),vec3<f32>(0.70,0.43,1.0),clamp(s,0.0,1.0));
    let white=clamp(core*reach*envelope+spark,0.0,1.0);
    let rgb=mix(spectrum,vec3<f32>(0.93,0.98,1.0),white);
    let alpha=clamp(energy,0.0,0.96);
    return vec4<f32>(rgb*alpha,alpha);
}";

#[composable]
fn Bolt(from: [f32; 2], target: [f32; 4], size: [f32; 2], request: u64, pending: bool) {
    let progress = animate_float_as_state_with_initial(
        0.0,
        1.0,
        tween(if pending { 10_000 } else { 850 }, Easing::LinearEasing),
        "live edit lightning",
    );
    static SOURCE: OnceLock<Arc<str>> = OnceLock::new();
    let mut shader = RuntimeShader::from_shared_source(
        SOURCE
            .get_or_init(|| Arc::from(format!("{RUNTIME_SHADER_PRELUDE_WGSL}{LIGHTNING}")))
            .clone(),
    );
    shader.set_float4(0, from[0], from[1], 0.0, 0.0);
    shader.set_float4(4, target[0], target[1], target[2], target[3]);
    shader.set_float4(
        8,
        progress.value(),
        (request % 1024) as f32,
        if pending { 1.0 } else { 0.0 },
        0.0,
    );
    shader.set_float4(12, size[0], size[1], 0.0, 0.0);
    UiBox(
        Modifier::empty()
            .fill_max_size()
            .graphics_layer_value(GraphicsLayer {
                render_effect: Some(RenderEffect::runtime_shader(shader)),
                compositing_strategy: CompositingStrategy::Offscreen,
                ..Default::default()
            }),
        BoxSpec::default(),
        || {},
    );
}
#[composable]
pub(crate) fn LiveEditLightning() {
    let state = rememberMutableStateOf(|| None::<(u64, [f32; 2], [f32; 4], [f32; 2], bool)>);
    CollectEvents(
        rememberHostMessages("ide.authoring.bolt"),
        (),
        move |payload: String| {
            let Ok(value) = serde_json::from_str::<Value>(&payload) else {
                return;
            };
            if value["clear"] == true {
                state.set(None);
                return;
            }
            let Some(request) = value["request"].as_u64() else {
                return;
            };
            let numbers = |key: &str, n: usize| -> Option<Vec<f32>> {
                let v = value[key].as_array()?;
                if v.len() != n {
                    return None;
                }
                v.iter()
                    .map(|v| {
                        v.as_f64()
                            .filter(|f| f.is_finite() && f.abs() < 32768.0)
                            .map(|f| f as f32)
                    })
                    .collect()
            };
            let (Some(from), Some(target)) = (numbers("from", 2), numbers("target", 4)) else {
                return;
            };
            let Some(size) = numbers("size", 2) else {
                return;
            };
            if target[2] <= 0.0 || target[3] <= 0.0 {
                return;
            }
            state.set(Some((
                request,
                [from[0], from[1]],
                [target[0], target[1], target[2], target[3]],
                [size[0], size[1]],
                value["phase"] == "pending",
            )));
        },
    );
    cranpose::embed::HostOverlay("window", move || {
        if let Some((request, from, target, size, pending)) = state.get() {
            cranpose::key(request, move || Bolt(from, target, size, request, pending));
        }
    });
}
#[cfg(test)]
mod tests {
    #[test]
    fn lightning_shader_validates() {
        let source = format!("{}{}", super::RUNTIME_SHADER_PRELUDE_WGSL, super::LIGHTNING);
        let module = naga::front::wgsl::parse_str(&source).expect("WGSL");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("valid shader");
    }
}
