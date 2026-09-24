//! Shader effects drawn on top of the IDE's editor, in the spirit of the Zeus
//! Thunderbolt plugin, in five styles: sparks and lightning, water, ice,
//! cosmic and party.
//!
//! The IDE sends every caret change on [`CARET_CHANNEL`]; [`EditorEffects`]
//! turns each into an effect of the chosen [`EffectStyle`] and draws all live
//! effects with one WGSL shader (`effects.wgsl`) on a [`HostOverlay`], the
//! transparent layer the plugin lays over the focused editor.
//! [`EffectsPlayground`] draws the same effects inside the tool window,
//! wherever it is clicked. When nothing is alive the frame loop stops and no
//! frames are sent.

use std::time::Instant;

use cranpose::{
    Box, BoxSpec, Color, GraphicsLayer, LaunchedEffectAsync, Modifier, MutableState, Point,
    composable, current_density, embed::HostOverlay, rememberHostMessages, rememberMutableStateOf,
};
use cranpose_core::CollectEvents;
use cranpose_ui_graphics::{CompositingStrategy, RenderEffect};
use serde::Deserialize;

use crate::shader::ShaderSource;

/// IDE → UI: a caret change in the focused editor.
pub const CARET_CHANNEL: &str = "ide.caret";

/// How many effects the shader draws at once; older ones make room.
pub const MAX_EFFECTS: usize = 24;

const TINT_SLOT: usize = 4;
const FIRST_EFFECT_SLOT: usize = 8;
const SLOTS_PER_EFFECT: usize = 8;
const JUMP_LINES: f32 = 3.0;

/// What caused a caret change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaretKind {
    /// Text was typed.
    Type,
    /// Text was deleted.
    Delete,
    /// The caret moved without an edit.
    Move,
}

/// A caret change as the IDE reports it, in logical pixels of the overlay.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaretEvent {
    /// What caused the change.
    pub kind: CaretKind,
    /// Where the caret is now.
    pub x: f32,
    /// Where the caret is now: the top of its line.
    pub y: f32,
    /// Where the caret was.
    pub from_x: f32,
    /// Where the caret was: the top of its line.
    pub from_y: f32,
    /// The editor's line height.
    pub line_height: f32,
}

impl CaretEvent {
    /// Reads a [`CARET_CHANNEL`] payload.
    pub fn parse(payload: &str) -> Option<Self> {
        serde_json::from_str(payload).ok()
    }

    /// The effect this change sets off in `style`, born at `born`, centred on
    /// the caret's line.
    pub fn effect(&self, style: EffectStyle, born: Instant, seed: f32) -> Effect {
        let kinds = style.kinds();
        let kind = match self.kind {
            CaretKind::Type => kinds.typed,
            CaretKind::Delete => kinds.deleted,
            CaretKind::Move if (self.y - self.from_y).abs() > JUMP_LINES * self.line_height => {
                kinds.jumped
            }
            CaretKind::Move => kinds.moved,
        };
        let middle = self.line_height * 0.5;
        Effect::new(
            kind,
            Point::new(self.x, self.y + middle),
            Point::new(self.from_x, self.from_y + middle),
            born,
            seed,
        )
    }
}

/// A family of effects, one for each kind of caret change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectStyle {
    /// Fire sparks, shockwaves and lightning.
    Sparks,
    /// Splashes, ripples and bubbles.
    Water,
    /// Frost crystals, shattering ice and snow.
    Ice,
    /// Stars, a black hole and a comet.
    Cosmic,
    /// Confetti, smoke puffs and a rainbow.
    Party,
}

/// What a style draws for each kind of caret change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleKinds {
    /// Where text was typed.
    pub typed: EffectKind,
    /// Where text was deleted.
    pub deleted: EffectKind,
    /// Along a jump of more than a few lines.
    pub jumped: EffectKind,
    /// Where the caret moved a little.
    pub moved: EffectKind,
}

impl EffectStyle {
    /// Every style, in the order the tool window lists them.
    pub const ALL: [Self; 5] = [
        Self::Sparks,
        Self::Water,
        Self::Ice,
        Self::Cosmic,
        Self::Party,
    ];

    /// The style's name in the tool window.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sparks => "Sparks",
            Self::Water => "Water",
            Self::Ice => "Ice",
            Self::Cosmic => "Cosmic",
            Self::Party => "Party",
        }
    }

    /// The effects the style draws.
    pub fn kinds(self) -> StyleKinds {
        use EffectKind::*;
        let [typed, deleted, jumped, moved] = match self {
            Self::Sparks => [Burst, Shockwave, Lightning, Sparkle],
            Self::Water => [Splash, Ripple, BubbleTrail, Bubbles],
            Self::Ice => [Frost, Shatter, FrostTrail, Snow],
            Self::Cosmic => [Stars, BlackHole, Comet, Twinkle],
            Self::Party => [Confetti, Poof, Rainbow, Sprinkle],
        };
        StyleKinds {
            typed,
            deleted,
            jumped,
            moved,
        }
    }
}

/// The shapes the shader draws. The order is the shader's kind code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EffectKind {
    /// Sparks flying out of a point.
    Burst,
    /// A ring expanding from a point.
    Shockwave,
    /// A jagged bolt between two points.
    Lightning,
    /// A small, quick burst.
    Sparkle,
    /// Droplets thrown up from a point, and a small ring.
    Splash,
    /// Rings spreading on water.
    Ripple,
    /// Bubbles rising along a path.
    BubbleTrail,
    /// A few bubbles rising from a point.
    Bubbles,
    /// A six-armed frost crystal growing.
    Frost,
    /// Ice shards flying apart.
    Shatter,
    /// A line of frost growing along a path.
    FrostTrail,
    /// Snowflakes drifting down.
    Snow,
    /// Twinkling stars spiralling out.
    Stars,
    /// A black hole with a swirling disk.
    BlackHole,
    /// A comet flying along a path.
    Comet,
    /// A few twinkling stars.
    Twinkle,
    /// Confetti thrown up and fluttering down.
    Confetti,
    /// A puff of smoke.
    Poof,
    /// A rainbow arching over a path.
    Rainbow,
    /// A little confetti.
    Sprinkle,
}

impl EffectKind {
    /// Every kind, in shader code order.
    pub const ALL: [Self; 20] = [
        Self::Burst,
        Self::Shockwave,
        Self::Lightning,
        Self::Sparkle,
        Self::Splash,
        Self::Ripple,
        Self::BubbleTrail,
        Self::Bubbles,
        Self::Frost,
        Self::Shatter,
        Self::FrostTrail,
        Self::Snow,
        Self::Stars,
        Self::BlackHole,
        Self::Comet,
        Self::Twinkle,
        Self::Confetti,
        Self::Poof,
        Self::Rainbow,
        Self::Sprinkle,
    ];

    /// How long the effect stays on screen, in seconds.
    pub fn lifetime(self) -> f32 {
        match self {
            Self::Sparkle => 0.45,
            Self::Lightning => 0.55,
            Self::Shockwave => 0.6,
            Self::Twinkle | Self::Poof => 0.7,
            Self::Burst | Self::Shatter => 0.75,
            Self::Splash | Self::FrostTrail | Self::Comet | Self::Sprinkle => 0.8,
            Self::Ripple | Self::Bubbles | Self::Frost | Self::Stars | Self::BlackHole => 0.9,
            Self::BubbleTrail | Self::Snow => 1.0,
            Self::Confetti | Self::Rainbow => 1.1,
        }
    }

    /// How far from its point, or its path, the effect ever draws, in
    /// logical pixels. A rainbow also reaches half its path's length.
    pub fn reach(self) -> f32 {
        match self {
            Self::FrostTrail | Self::Rainbow => 16.0,
            Self::Comet | Self::Frost => 30.0,
            Self::Lightning => 32.0,
            Self::Twinkle => 36.0,
            Self::BlackHole => 42.0,
            Self::Sparkle | Self::Snow | Self::Poof => 48.0,
            Self::Shockwave => 52.0,
            Self::BubbleTrail | Self::Bubbles => 64.0,
            Self::Shatter | Self::Sprinkle => 72.0,
            Self::Ripple => 78.0,
            Self::Stars => 84.0,
            Self::Splash => 90.0,
            Self::Burst => 110.0,
            Self::Confetti => 150.0,
        }
    }

    /// Whether the effect runs from where the caret was to where it is.
    pub fn follows_path(self) -> bool {
        matches!(
            self,
            Self::Lightning | Self::BubbleTrail | Self::FrostTrail | Self::Comet | Self::Rainbow
        )
    }

    fn code(self) -> f32 {
        f32::from(self as u8)
    }
}

/// One live effect, in logical pixels of the surface it is drawn on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effect {
    /// What to draw.
    pub kind: EffectKind,
    /// Where it happens.
    pub x: f32,
    /// Where it happens.
    pub y: f32,
    /// Where a path starts; the same point as `x`, `y` for other effects.
    pub from_x: f32,
    /// Where a path starts; the same point as `x`, `y` for other effects.
    pub from_y: f32,
    /// When it started.
    pub born: Instant,
    /// Varies the shape between effects of one kind.
    pub seed: f32,
}

impl Effect {
    /// A `kind` effect at `at`, running from `from` if it follows a path.
    pub fn new(kind: EffectKind, at: Point, from: Point, born: Instant, seed: f32) -> Self {
        let from = if kind.follows_path() { from } else { at };
        Self {
            kind,
            x: at.x,
            y: at.y,
            from_x: from.x,
            from_y: from.y,
            born,
            seed,
        }
    }

    /// How far through its life the effect is at `now`, `0..1`; `None` once
    /// it is over.
    pub fn age(&self, now: Instant) -> Option<f32> {
        let age = now.saturating_duration_since(self.born).as_secs_f32() / self.kind.lifetime();
        (age < 1.0).then_some(age)
    }

    /// How far from its point or path the effect draws, in logical pixels.
    pub fn reach(&self) -> f32 {
        let reach = self.kind.reach();
        if self.kind != EffectKind::Rainbow {
            return reach;
        }
        let chord = (self.x - self.from_x).hypot(self.y - self.from_y);
        reach + (chord * 0.5).max(6.0)
    }
}

/// `effects` with `effect` added, dropping the oldest past [`MAX_EFFECTS`].
pub fn with_effect(effects: &[Effect], effect: Effect) -> Vec<Effect> {
    let skip = (effects.len() + 1).saturating_sub(MAX_EFFECTS);
    effects
        .iter()
        .skip(skip)
        .copied()
        .chain(std::iter::once(effect))
        .collect()
}

/// The effects of `effects` still alive at `now`.
pub fn alive(effects: &[Effect], now: Instant) -> Vec<Effect> {
    effects
        .iter()
        .filter(|effect| effect.age(now).is_some())
        .copied()
        .collect()
}

pub(crate) static EFFECTS_SHADER: ShaderSource = ShaderSource::new(include_str!("effects.wgsl"));

/// The shader drawing `effects` as they are at `now`, with bolts, rings and
/// comets in `tint`, at `density` physical pixels per logical one.
pub fn effects_shader(effects: &[Effect], now: Instant, tint: Color, density: f32) -> RenderEffect {
    let mut shader = EFFECTS_SHADER.shader();
    let live: Vec<(Effect, f32)> = effects
        .iter()
        .filter_map(|effect| effect.age(now).map(|age| (*effect, age)))
        .take(MAX_EFFECTS)
        .collect();
    shader.set_float(0, live.len() as f32);
    shader.set_float(1, density);
    shader.set_float4(TINT_SLOT, tint.0, tint.1, tint.2, 1.0);
    for (index, (effect, age)) in live.iter().enumerate() {
        let slot = FIRST_EFFECT_SLOT + index * SLOTS_PER_EFFECT;
        shader.set_float4(slot, effect.x, effect.y, effect.from_x, effect.from_y);
        shader.set_float4(
            slot + 4,
            *age,
            effect.kind.code(),
            effect.seed,
            effect.reach(),
        );
    }
    RenderEffect::runtime_shader(shader)
}

/// Live effects and the clock they are drawn at, from [`rememberEffects`].
#[derive(Clone, Copy)]
pub struct EffectsState {
    effects: MutableState<Vec<Effect>>,
    now: MutableState<Instant>,
    spawned: MutableState<u32>,
}

impl EffectsState {
    /// Starts a `kind` effect at `at`, running from `from` if it follows a
    /// path.
    pub fn spawn(&self, kind: EffectKind, at: Point, from: Point) {
        let seed = self.next_seed();
        self.add(Effect::new(kind, at, from, Instant::now(), seed));
    }

    /// Starts the effect `caret` sets off in `style`.
    pub fn spawn_caret(&self, caret: &CaretEvent, style: EffectStyle) {
        let seed = self.next_seed();
        self.add(caret.effect(style, Instant::now(), seed));
    }

    /// The layer that draws the live effects, with bolts and rings in `tint`.
    pub fn layer(&self, tint: Color) -> GraphicsLayer {
        let live = self.effects.get();
        if live.is_empty() {
            return GraphicsLayer::default();
        }
        GraphicsLayer {
            render_effect: Some(effects_shader(
                &live,
                self.now.get(),
                tint,
                current_density(),
            )),
            compositing_strategy: CompositingStrategy::Offscreen,
            ..Default::default()
        }
    }

    fn next_seed(&self) -> f32 {
        let count = self.spawned.get_non_reactive().wrapping_add(1);
        self.spawned.set(count);
        (count % 1000) as f32 * 1.618
    }

    fn add(&self, effect: Effect) {
        self.now.set(effect.born);
        self.effects
            .set(with_effect(&self.effects.get_non_reactive(), effect));
    }
}

/// Effects that last their lifetime, with a frame loop that runs only while
/// one is alive.
#[expect(non_snake_case)]
#[track_caller]
pub fn rememberEffects() -> EffectsState {
    let state = EffectsState {
        effects: rememberMutableStateOf(Vec::<Effect>::new),
        now: rememberMutableStateOf(Instant::now),
        spawned: rememberMutableStateOf(|| 0u32),
    };
    let animating = !state.effects.get().is_empty();
    LaunchedEffectAsync(animating, move |scope| {
        Box::pin(async move {
            if !animating {
                return;
            }
            let clock = scope.runtime().frame_clock();
            while scope.is_active() {
                clock.next_frame().await;
                let frame_time = Instant::now();
                state.now.set(frame_time);
                let current = state.effects.get_non_reactive();
                let still_alive = alive(&current, frame_time);
                if still_alive.len() != current.len() {
                    state.effects.set(still_alive);
                }
            }
        })
    });
    state
}

/// Draws effects in the style `style` holds over the focused editor, with
/// bolts and rings in `tint`.
#[composable]
pub fn EditorEffects(tint: Color, style: MutableState<EffectStyle>) {
    let effects = rememberEffects();
    CollectEvents(
        rememberHostMessages(CARET_CHANNEL),
        (),
        move |payload: String| {
            if let Some(caret) = CaretEvent::parse(&payload) {
                effects.spawn_caret(&caret, style.get_non_reactive());
            }
        },
    );
    HostOverlay("editor", move || {
        Box(
            Modifier::empty()
                .fill_max_size()
                .graphics_layer(move || effects.layer(tint)),
            BoxSpec::default(),
            || {},
        );
    });
}

/// A `height` tall area on `background` that draws effects in the style
/// `style` holds wherever it is clicked: typing, deleting, a jump from the
/// previous click and a caret move, in turn.
#[composable]
pub fn EffectsPlayground(
    height: f32,
    background: Color,
    tint: Color,
    style: MutableState<EffectStyle>,
) {
    let effects = rememberEffects();
    let clicks = rememberMutableStateOf(|| 0usize);
    let last = rememberMutableStateOf(|| Point::new(0.0, 0.0));
    Box(
        Modifier::empty()
            .fill_max_width()
            .height(height)
            .background(background)
            .rounded_corners(8.0)
            .graphics_layer(move || effects.layer(tint))
            .clickable(move |at| {
                let kinds = style.get_non_reactive().kinds();
                let turn = clicks.get_non_reactive();
                let kind = [kinds.typed, kinds.deleted, kinds.jumped, kinds.moved][turn % 4];
                effects.spawn(kind, at, last.get_non_reactive());
                clicks.set(turn + 1);
                last.set(at);
            }),
        BoxSpec::default(),
        || {},
    );
}

#[cfg(test)]
#[path = "tests/effects_tests.rs"]
mod tests;
