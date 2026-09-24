//! What every WGSL shader in the UI shares: Cranpose's RuntimeShader prelude
//! plus a few helpers, joined to the shader's own code once.

use std::sync::{Arc, OnceLock};

use cranpose_ui_graphics::{RUNTIME_SHADER_PRELUDE_WGSL, RuntimeShader};

/// Helpers on top of the prelude, for every shader here:
///
/// - `get_float(i)` reads uniform `i`.
/// - `effect_size()` is the drawn node's size in physical pixels.
/// - `effect_local(uv)` turns a texture coordinate into physical pixels from
///   the node's top-left corner.
pub const HELPERS_WGSL: &str = r"
fn get_float(index: u32) -> f32 {
    return u[index / 4u][index % 4u];
}

fn effect_size() -> vec2<f32> {
    return max(vec2<f32>(get_float(250u), get_float(251u)), vec2<f32>(1.0));
}

fn effect_local(uv: vec2<f32>) -> vec2<f32> {
    let texture_size = vec2<f32>(textureDimensions(input_texture));
    return uv * texture_size - vec2<f32>(get_float(248u), get_float(249u));
}
";

/// A shader's own WGSL, joined to the prelude and [`HELPERS_WGSL`] the first
/// time it is drawn.
pub struct ShaderSource {
    code: &'static str,
    joined: OnceLock<Arc<str>>,
}

impl ShaderSource {
    /// A shader whose `effect_fs` fragment entry point is in `code`.
    pub const fn new(code: &'static str) -> Self {
        Self {
            code,
            joined: OnceLock::new(),
        }
    }

    /// The complete WGSL module.
    pub fn source(&self) -> Arc<str> {
        self.joined
            .get_or_init(|| {
                Arc::from(format!(
                    "{RUNTIME_SHADER_PRELUDE_WGSL}{HELPERS_WGSL}{}",
                    self.code
                ))
            })
            .clone()
    }

    /// A fresh shader to set uniforms on.
    pub fn shader(&self) -> RuntimeShader {
        RuntimeShader::from_shared_source(self.source())
    }
}

#[cfg(test)]
#[path = "tests/shader_tests.rs"]
mod tests;
