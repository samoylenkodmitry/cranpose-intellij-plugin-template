use cranpose_ui_graphics::RenderEffect;

use super::*;

#[test]
fn the_orb_shader_takes_the_phase_and_palette() {
    let (inner, outer) = PALETTES[1];
    let RenderEffect::Shader { shader } = orb_effect(0.25, PALETTES[1]) else {
        panic!("a runtime shader");
    };
    let u = shader.uniforms();
    assert_eq!(u[0], 0.25);
    assert_eq!(&u[4..7], &[inner.0, inner.1, inner.2]);
    assert_eq!(&u[8..11], &[outer.0, outer.1, outer.2]);
}

#[test]
fn the_orb_opens_beside_the_tool_window() {
    assert!(ORB_OFFSET.0 + ORB_SIZE < 0.0);
    assert!(ORB_OFFSET.1 >= 0.0);
    assert!(PALETTES.iter().all(|(inner, outer)| inner != outer));
}
