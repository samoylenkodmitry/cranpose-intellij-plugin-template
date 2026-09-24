use super::*;
use crate::{aurora::AURORA_SHADER, effects::EFFECTS_SHADER, orb::ORB_SHADER};

fn validate(shader: &ShaderSource) -> Result<(), String> {
    let module =
        naga::front::wgsl::parse_str(&shader.source()).map_err(|error| error.to_string())?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|error| format!("{error:?}"))
}

#[test]
fn every_shader_compiles() {
    for (name, shader) in [
        ("aurora", &AURORA_SHADER),
        ("effects", &EFFECTS_SHADER),
        ("orb", &ORB_SHADER),
    ] {
        assert_eq!(validate(shader), Ok(()), "{name}");
    }
}

#[test]
fn a_broken_shader_is_caught() {
    static BROKEN: ShaderSource = ShaderSource::new("fn effect_fs() -> f32 { return nope; }");
    assert!(validate(&BROKEN).is_err());
}

#[test]
fn the_source_is_joined_once_and_shared() {
    static PLAIN: ShaderSource = ShaderSource::new("// plain");
    let first = PLAIN.source();
    assert!(Arc::ptr_eq(&first, &PLAIN.source()));
    assert!(first.starts_with(RUNTIME_SHADER_PRELUDE_WGSL));
    assert!(first.contains(HELPERS_WGSL));
    assert!(first.ends_with("// plain"));
    assert_eq!(PLAIN.shader().source(), &*first);
}
