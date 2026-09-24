use std::{collections::HashSet, time::Duration};

use cranpose_ui_graphics::RenderEffect;

use super::*;

fn caret(kind: &str, y: f32, from_y: f32) -> CaretEvent {
    CaretEvent::parse(&format!(
        r#"{{"kind":"{kind}","x":40,"y":{y},"fromX":10,"fromY":{from_y},"lineHeight":16}}"#
    ))
    .expect("a caret payload")
}

fn effect_at(kind: EffectKind, born: Instant, seed: f32) -> Effect {
    Effect::new(
        kind,
        Point::new(40.0, 8.0),
        Point::new(10.0, 8.0),
        born,
        seed,
    )
}

#[test]
fn caret_payloads_parse() {
    let event = caret("delete", 32.0, 16.0);
    assert_eq!(event.kind, CaretKind::Delete);
    assert_eq!(
        (event.x, event.y, event.from_x, event.from_y),
        (40.0, 32.0, 10.0, 16.0)
    );
    assert_eq!(event.line_height, 16.0);
    assert_eq!(CaretEvent::parse(r#"{"kind":"type"}"#), None);
    assert_eq!(CaretEvent::parse("not json"), None);
}

#[test]
fn each_change_sets_off_its_styles_effect() {
    let born = Instant::now();
    for style in EffectStyle::ALL {
        let kinds = style.kinds();
        let kind = |event: CaretEvent| event.effect(style, born, 0.0).kind;
        assert_eq!(kind(caret("type", 0.0, 0.0)), kinds.typed, "{style:?}");
        assert_eq!(kind(caret("delete", 0.0, 0.0)), kinds.deleted, "{style:?}");
        assert_eq!(kind(caret("move", 16.0, 0.0)), kinds.moved, "{style:?}");
        assert_eq!(kind(caret("move", 160.0, 0.0)), kinds.jumped, "{style:?}");
    }
}

#[test]
fn every_style_has_its_own_effects_and_name() {
    let mut kinds = HashSet::new();
    let mut labels = HashSet::new();
    for style in EffectStyle::ALL {
        let own = style.kinds();
        kinds.extend([own.typed, own.deleted, own.jumped, own.moved]);
        labels.insert(style.label());
        assert!(own.jumped.follows_path(), "{style:?} jumps along a path");
        assert!(!own.typed.follows_path() && !own.moved.follows_path());
    }
    assert_eq!(kinds.len(), EffectKind::ALL.len());
    assert_eq!(labels.len(), EffectStyle::ALL.len());
}

#[test]
fn kind_codes_follow_the_shader_order() {
    for (index, kind) in EffectKind::ALL.iter().enumerate() {
        assert_eq!(kind.code(), index as f32);
        assert!(kind.lifetime() > 0.0 && kind.lifetime() < 1.5, "{kind:?}");
        assert!(kind.reach() > 0.0, "{kind:?}");
    }
}

#[test]
fn effects_sit_on_the_middle_of_the_line() {
    let effect = caret("move", 160.0, 16.0).effect(EffectStyle::Sparks, Instant::now(), 2.0);
    assert_eq!((effect.x, effect.y), (40.0, 168.0));
    assert_eq!((effect.from_x, effect.from_y), (10.0, 24.0));
    assert_eq!(effect.seed, 2.0);
}

#[test]
fn only_path_effects_keep_where_they_started() {
    let born = Instant::now();
    let point = effect_at(EffectKind::Frost, born, 0.0);
    assert_eq!((point.from_x, point.from_y), (point.x, point.y));
    let path = effect_at(EffectKind::Comet, born, 0.0);
    assert_eq!((path.from_x, path.from_y), (10.0, 8.0));
}

#[test]
fn a_rainbow_reaches_over_half_its_path() {
    let born = Instant::now();
    let rainbow = Effect::new(
        EffectKind::Rainbow,
        Point::new(100.0, 0.0),
        Point::new(0.0, 0.0),
        born,
        0.0,
    );
    assert_eq!(rainbow.reach(), EffectKind::Rainbow.reach() + 50.0);
    let comet = effect_at(EffectKind::Comet, born, 0.0);
    assert_eq!(comet.reach(), EffectKind::Comet.reach());
}

#[test]
fn an_effect_ages_until_its_lifetime_is_over() {
    let born = Instant::now();
    let effect = effect_at(EffectKind::Burst, born, 0.0);
    let lifetime = Duration::from_secs_f32(EffectKind::Burst.lifetime());
    assert_eq!(effect.age(born), Some(0.0));
    let halfway = effect.age(born + lifetime / 2).expect("alive halfway");
    assert!((halfway - 0.5).abs() < 1e-3);
    assert_eq!(effect.age(born + lifetime), None);
}

#[test]
fn the_oldest_effects_make_room() {
    let born = Instant::now();
    let full: Vec<Effect> = (0..MAX_EFFECTS)
        .map(|seed| effect_at(EffectKind::Burst, born, seed as f32))
        .collect();
    let next = with_effect(&full, effect_at(EffectKind::Burst, born, 99.0));
    assert_eq!(next.len(), MAX_EFFECTS);
    assert_eq!(next[0].seed, 1.0);
    assert_eq!(next[MAX_EFFECTS - 1].seed, 99.0);
    assert_eq!(
        with_effect(&[], effect_at(EffectKind::Burst, born, 5.0)).len(),
        1
    );
}

#[test]
fn expired_effects_are_dropped() {
    let born = Instant::now();
    let later = born + Duration::from_millis(500);
    let effects = [
        effect_at(EffectKind::Burst, born, 0.0),
        effect_at(EffectKind::Shockwave, born, 1.0),
        effect_at(EffectKind::Burst, later, 2.0),
    ];
    let kept = alive(&effects, later + Duration::from_millis(200));
    assert_eq!(
        kept.iter().map(|effect| effect.seed).collect::<Vec<_>>(),
        vec![0.0, 2.0]
    );
}

#[test]
fn the_shader_carries_every_live_effect() {
    let born = Instant::now();
    let now = born + Duration::from_millis(300);
    let effects = [
        effect_at(EffectKind::Burst, born, 3.0),
        effect_at(EffectKind::Shockwave, born - Duration::from_secs(5), 4.0),
        effect_at(EffectKind::Rainbow, born, 5.0),
    ];
    let RenderEffect::Shader { shader } =
        effects_shader(&effects, now, Color(0.1, 0.2, 0.3, 1.0), 2.0)
    else {
        panic!("a runtime shader");
    };
    let u = shader.uniforms();
    assert_eq!(&u[..2], &[2.0, 2.0]);
    assert_eq!(&u[TINT_SLOT..TINT_SLOT + 3], &[0.1, 0.2, 0.3]);
    let first = FIRST_EFFECT_SLOT;
    let second = FIRST_EFFECT_SLOT + SLOTS_PER_EFFECT;
    assert_eq!(&u[first..first + 4], &[40.0, 8.0, 40.0, 8.0]);
    assert_eq!(
        &u[first + 5..first + 8],
        &[0.0, 3.0, EffectKind::Burst.reach()]
    );
    assert_eq!(&u[second..second + 4], &[40.0, 8.0, 10.0, 8.0]);
    assert_eq!(u[second + 5], EffectKind::Rainbow.code());
    assert_eq!(u[second + 7], effects[2].reach());
}
