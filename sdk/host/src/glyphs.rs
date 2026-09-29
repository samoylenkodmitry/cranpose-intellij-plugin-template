//! Editor decorations painted by IntelliJ inside the editor's own paint pass.
//!
//! Inlays and range highlighters scroll, fold and wrap with the text in the
//! same frame. Painting stays in Java2D through a scoped `Graphics2D` copy, so
//! it follows the editor's scale, font and theme without another surface.
use crate::jvm::{A, J, O};
use anyhow::Result;

/// sRGB channels 0..=255 and alpha 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub f32);

/// Colors that read on the current editor background.
#[derive(Clone, Copy, Debug)]
pub struct Tones {
    pub dark: bool,
    pub accent: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub fill: Rgba,
}
impl Tones {
    pub fn current(j: &mut J<'_>) -> Result<Self> {
        let dark = !j
            .static_call("com/intellij/ui/JBColor", "isBright", "()Z", &[])?
            .z()?;
        let focus = j.static_obj(
            "com/intellij/util/ui/JBUI$CurrentTheme$Focus",
            "focusColor",
            "()Ljava/awt/Color;",
            &[],
        )?;
        let accent = Rgba(
            j.int(&focus, "getRed")? as u8,
            j.int(&focus, "getGreen")? as u8,
            j.int(&focus, "getBlue")? as u8,
            1.0,
        );
        Ok(if dark {
            Self {
                dark,
                accent,
                text: Rgba(0xdf, 0xe1, 0xe5, 1.0),
                muted: Rgba(0x9a, 0xa0, 0xaa, 1.0),
                fill: Rgba(255, 255, 255, 0.07),
            }
        } else {
            Self {
                dark,
                accent,
                text: Rgba(0x1e, 0x1f, 0x22, 1.0),
                muted: Rgba(0x6a, 0x6e, 0x78, 1.0),
                fill: Rgba(0, 0, 0, 0.055),
            }
        })
    }
    /// Status colors tuned for the editor background rather than panels.
    pub fn tone(&self, name: &str) -> Rgba {
        match (name, self.dark) {
            ("stable", true) => Rgba(0x5f, 0xb8, 0x8a, 1.0),
            ("stable", false) => Rgba(0x3b, 0x8f, 0x63, 1.0),
            ("warning", true) => Rgba(0xd9, 0xa8, 0x4e, 1.0),
            ("warning", false) => Rgba(0xb0, 0x7d, 0x14, 1.0),
            ("danger", true) => Rgba(0xe5, 0x77, 0x72, 1.0),
            ("danger", false) => Rgba(0xc2, 0x44, 0x40, 1.0),
            _ => self.muted,
        }
    }
}
impl Rgba {
    pub fn alpha(self, alpha: f32) -> Self {
        Self(self.0, self.1, self.2, self.3 * alpha)
    }
}

/// A disposable `Graphics2D` copy with antialiasing and float geometry.
pub struct Pen {
    g: O,
}
impl Pen {
    pub fn new(j: &mut J<'_>, graphics: &O) -> Result<Self> {
        let g = j.obj(graphics, "create", "()Ljava/awt/Graphics;", &[])?;
        for (key, value) in [
            ("KEY_ANTIALIASING", "VALUE_ANTIALIAS_ON"),
            ("KEY_STROKE_CONTROL", "VALUE_STROKE_PURE"),
            ("KEY_TEXT_ANTIALIASING", "VALUE_TEXT_ANTIALIAS_ON"),
        ] {
            let key = j.constant(
                "java/awt/RenderingHints",
                key,
                "Ljava/awt/RenderingHints$Key;",
            )?;
            let value = j.constant("java/awt/RenderingHints", value, "Ljava/lang/Object;")?;
            j.void(
                &g,
                "setRenderingHint",
                "(Ljava/awt/RenderingHints$Key;Ljava/lang/Object;)V",
                &[A::O(&key), A::O(&value)],
            )?;
        }
        Ok(Self { g })
    }
    pub fn graphics(&self) -> &O {
        &self.g
    }
    pub fn color(&self, j: &mut J<'_>, c: Rgba) -> Result<()> {
        let color = j.new(
            "java/awt/Color",
            "(IIII)V",
            &[
                A::I(c.0.into()),
                A::I(c.1.into()),
                A::I(c.2.into()),
                A::I((c.3.clamp(0.0, 1.0) * 255.0).round() as i32),
            ],
        )?;
        j.void(&self.g, "setColor", "(Ljava/awt/Color;)V", &[A::O(&color)])
    }
    fn shape(j: &mut J<'_>, rect: [f32; 4], radius: f32) -> Result<O> {
        j.new(
            "java/awt/geom/RoundRectangle2D$Float",
            "(FFFFFF)V",
            &[
                A::F(rect[0]),
                A::F(rect[1]),
                A::F(rect[2]),
                A::F(rect[3]),
                A::F(radius * 2.0),
                A::F(radius * 2.0),
            ],
        )
    }
    pub fn fill_round(&self, j: &mut J<'_>, rect: [f32; 4], radius: f32, c: Rgba) -> Result<()> {
        self.color(j, c)?;
        let shape = Self::shape(j, rect, radius)?;
        j.void(&self.g, "fill", "(Ljava/awt/Shape;)V", &[A::O(&shape)])
    }
    pub fn stroke_round(
        &self,
        j: &mut J<'_>,
        rect: [f32; 4],
        radius: f32,
        width: f32,
        c: Rgba,
    ) -> Result<()> {
        self.color(j, c)?;
        let stroke = j.new("java/awt/BasicStroke", "(F)V", &[A::F(width)])?;
        j.void(
            &self.g,
            "setStroke",
            "(Ljava/awt/Stroke;)V",
            &[A::O(&stroke)],
        )?;
        // Center the stroke on the pixel grid inside the requested bounds.
        let half = width * 0.5;
        let shape = Self::shape(
            j,
            [
                rect[0] + half,
                rect[1] + half,
                rect[2] - width,
                rect[3] - width,
            ],
            (radius - half).max(0.0),
        )?;
        j.void(&self.g, "draw", "(Ljava/awt/Shape;)V", &[A::O(&shape)])
    }
    pub fn fill_circle(&self, j: &mut J<'_>, cx: f32, cy: f32, r: f32, c: Rgba) -> Result<()> {
        self.fill_round(j, [cx - r, cy - r, r * 2.0, r * 2.0], r, c)
    }
    pub fn text(&self, j: &mut J<'_>, text: &str, x: f32, baseline: f32, c: Rgba) -> Result<()> {
        self.color(j, c)?;
        j.void(
            &self.g,
            "drawString",
            "(Ljava/lang/String;FF)V",
            &[A::S(text), A::F(x), A::F(baseline)],
        )
    }
    pub fn dispose(self, j: &mut J<'_>) -> Result<()> {
        j.void(&self.g, "dispose", "()V", &[])
    }
}

/// The inlay's target rectangle as `[x, y, width, height]`.
pub fn bounds(j: &mut J<'_>, rectangle: &O) -> Result<[f32; 4]> {
    Ok([
        j.field_int(rectangle, "x")? as f32,
        j.field_int(rectangle, "y")? as f32,
        j.field_int(rectangle, "width")? as f32,
        j.field_int(rectangle, "height")? as f32,
    ])
}

/// What a live-value glyph shows.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueLook {
    /// A tunable number, string or boolean: a small knob.
    Knob { hovered: bool },
    /// A color literal: its current color, over a checkerboard when translucent.
    Swatch { rgba: [f64; 4], hovered: bool },
}

/// Width reserved in the text for one value glyph.
pub const VALUE_WIDTH: i32 = 16;

/// Paint a value glyph into its inlay rectangle.
pub fn value(j: &mut J<'_>, graphics: &O, rectangle: &O, look: &ValueLook) -> Result<()> {
    let tones = Tones::current(j)?;
    let [x, y, w, h] = bounds(j, rectangle)?;
    let pen = Pen::new(j, graphics)?;
    let result = (|| -> Result<()> {
        let (cx, cy) = (x + w * 0.5, y + h * 0.5);
        match look {
            ValueLook::Knob { hovered } => {
                if *hovered {
                    pen.fill_circle(j, cx, cy, 7.0, tones.accent.alpha(0.18))?;
                }
                let ring = 4.5;
                pen.stroke_round(
                    j,
                    [cx - ring, cy - ring, ring * 2.0, ring * 2.0],
                    ring,
                    1.3,
                    tones.accent.alpha(if *hovered { 0.95 } else { 0.6 }),
                )?;
                pen.fill_circle(j, cx, cy, 1.9, tones.accent.alpha(0.95))?;
            }
            ValueLook::Swatch { rgba, hovered } => {
                let size = (h * 0.62).clamp(9.0, 13.0);
                let rect = [cx - size * 0.5, cy - size * 0.5, size, size];
                if *hovered {
                    pen.fill_round(
                        j,
                        [rect[0] - 2.5, rect[1] - 2.5, size + 5.0, size + 5.0],
                        5.5,
                        tones.accent.alpha(0.22),
                    )?;
                }
                if rgba[3] < 0.999 {
                    let cell = size * 0.5;
                    let (light, dark) = (Rgba(236, 236, 236, 1.0), Rgba(170, 170, 170, 1.0));
                    pen.fill_round(j, rect, 3.0, light)?;
                    pen.fill_round(j, [rect[0] + cell, rect[1], cell, cell], 0.0, dark)?;
                    pen.fill_round(j, [rect[0], rect[1] + cell, cell, cell], 0.0, dark)?;
                }
                let channel = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                pen.fill_round(
                    j,
                    rect,
                    3.0,
                    Rgba(
                        channel(rgba[0]),
                        channel(rgba[1]),
                        channel(rgba[2]),
                        rgba[3] as f32,
                    ),
                )?;
                pen.stroke_round(j, rect, 3.0, 1.0, tones.text.alpha(0.28))?;
            }
        }
        Ok(())
    })();
    let disposed = pen.dispose(j);
    result.and(disposed)
}

/// Editor font at badge size, and the component whose metrics measure it.
pub fn badge_font(j: &mut J<'_>, inlay: &O) -> Result<(O, O)> {
    let editor = j.obj(
        inlay,
        "getEditor",
        "()Lcom/intellij/openapi/editor/Editor;",
        &[],
    )?;
    let scheme = j.obj(
        &editor,
        "getColorsScheme",
        "()Lcom/intellij/openapi/editor/colors/EditorColorsScheme;",
        &[],
    )?;
    let plain = j.constant(
        "com/intellij/openapi/editor/colors/EditorFontType",
        "PLAIN",
        "Lcom/intellij/openapi/editor/colors/EditorFontType;",
    )?;
    let font = j.obj(
        &scheme,
        "getFont",
        "(Lcom/intellij/openapi/editor/colors/EditorFontType;)Ljava/awt/Font;",
        &[A::O(&plain)],
    )?;
    let size = (j.int(&scheme, "getEditorFontSize")? as f32 * 0.8).max(9.0);
    let font = j.obj(&font, "deriveFont", "(F)Ljava/awt/Font;", &[A::F(size)])?;
    let component = j.obj(
        &editor,
        "getContentComponent",
        "()Ljavax/swing/JComponent;",
        &[],
    )?;
    Ok((font, component))
}

const BADGE_DOT: f32 = 5.0;
const BADGE_PAD: f32 = 6.0;
const BADGE_MARGIN: f32 = 4.0;

/// Width of a badge inlay for `label`, including its outer margin.
pub fn badge_width(j: &mut J<'_>, inlay: &O, label: &str) -> Result<i32> {
    let (font, component) = badge_font(j, inlay)?;
    let metrics = j.obj(
        &component,
        "getFontMetrics",
        "(Ljava/awt/Font;)Ljava/awt/FontMetrics;",
        &[A::O(&font)],
    )?;
    let text = j
        .call(
            &metrics,
            "stringWidth",
            "(Ljava/lang/String;)I",
            &[A::S(label)],
        )?
        .i()? as f32;
    Ok((BADGE_MARGIN * 2.0 + BADGE_PAD * 2.0 + BADGE_DOT + 5.0 + text).ceil() as i32)
}

/// A quiet pill: a status dot and a muted label on a faint fill.
pub fn badge(j: &mut J<'_>, args: &[O], label: &str, tone: &str) -> Result<()> {
    let tones = Tones::current(j)?;
    let (font, _) = badge_font(j, &args[0])?;
    let [x, y, w, h] = bounds(j, &args[2])?;
    let pen = Pen::new(j, &args[1])?;
    let result = (|| -> Result<()> {
        j.void(
            pen.graphics(),
            "setFont",
            "(Ljava/awt/Font;)V",
            &[A::O(&font)],
        )?;
        let metrics = j.obj(
            pen.graphics(),
            "getFontMetrics",
            "()Ljava/awt/FontMetrics;",
            &[],
        )?;
        let ascent = j.int(&metrics, "getAscent")? as f32;
        let descent = j.int(&metrics, "getDescent")? as f32;
        let height = (ascent + descent + 3.0).min(h - 2.0);
        let top = y + (h - height) * 0.5;
        let pill = [x + BADGE_MARGIN, top, w - BADGE_MARGIN * 2.0, height];
        pen.fill_round(j, pill, height * 0.5, tones.fill)?;
        let color = tones.tone(tone);
        let cy = top + height * 0.5;
        pen.fill_circle(
            j,
            pill[0] + BADGE_PAD + BADGE_DOT * 0.5,
            cy,
            BADGE_DOT * 0.5,
            color,
        )?;
        let baseline = cy + (ascent - descent) * 0.5;
        pen.text(
            j,
            label,
            pill[0] + BADGE_PAD + BADGE_DOT + 5.0,
            baseline,
            if tone == "muted" { tones.muted } else { color },
        )
    })();
    let disposed = pen.dispose(j);
    result.and(disposed)
}

/// A thin accent underline for a composable call, painted by the editor.
pub fn call_underline(j: &mut J<'_>, markup: &O, start: i32, end: i32) -> Result<O> {
    let tones = Tones::current(j)?;
    let attributes = j.new(
        "com/intellij/openapi/editor/markup/TextAttributes",
        "()V",
        &[],
    )?;
    let accent = tones.accent.alpha(if tones.dark { 0.55 } else { 0.45 });
    let color = j.new(
        "java/awt/Color",
        "(IIII)V",
        &[
            A::I(accent.0.into()),
            A::I(accent.1.into()),
            A::I(accent.2.into()),
            A::I((accent.3 * 255.0) as i32),
        ],
    )?;
    j.void(
        &attributes,
        "setEffectColor",
        "(Ljava/awt/Color;)V",
        &[A::O(&color)],
    )?;
    let effect = j.constant(
        "com/intellij/openapi/editor/markup/EffectType",
        "LINE_UNDERSCORE",
        "Lcom/intellij/openapi/editor/markup/EffectType;",
    )?;
    j.void(
        &attributes,
        "setEffectType",
        "(Lcom/intellij/openapi/editor/markup/EffectType;)V",
        &[A::O(&effect)],
    )?;
    let area = j.constant(
        "com/intellij/openapi/editor/markup/HighlighterTargetArea",
        "EXACT_RANGE",
        "Lcom/intellij/openapi/editor/markup/HighlighterTargetArea;",
    )?;
    // HighlighterLayer.ADDITIONAL_SYNTAX: above syntax colors, below warnings.
    j.obj(
        markup,
        "addRangeHighlighter",
        "(IIILcom/intellij/openapi/editor/markup/TextAttributes;Lcom/intellij/openapi/editor/markup/HighlighterTargetArea;)Lcom/intellij/openapi/editor/markup/RangeHighlighter;",
        &[
            A::I(start),
            A::I(end),
            A::I(3000),
            A::O(&attributes),
            A::O(&area),
        ],
    )
}

/// Paint an inlay's renderer into an offscreen image and return its opaque
/// pixels as `[r, g, b]`, so tests exercise the real Java2D calls headlessly.
#[cfg(feature = "ide-tests")]
pub(crate) fn rendered(j: &mut J<'_>, inlay: &O) -> Result<Vec<[u8; 3]>> {
    let width = j.int(inlay, "getWidthInPixels")?.max(1);
    let height = 22;
    let image = j.new(
        "java/awt/image/BufferedImage",
        "(III)V",
        &[A::I(width), A::I(height), A::I(2)],
    )?;
    let graphics = j.obj(&image, "createGraphics", "()Ljava/awt/Graphics2D;", &[])?;
    let rectangle = j.new(
        "java/awt/Rectangle",
        "(IIII)V",
        &[A::I(0), A::I(0), A::I(width), A::I(height)],
    )?;
    let renderer = j.obj(
        inlay,
        "getRenderer",
        "()Lcom/intellij/openapi/editor/EditorCustomElementRenderer;",
        &[],
    )?;
    j.void(
        &renderer,
        "paint",
        "(Lcom/intellij/openapi/editor/Inlay;Ljava/awt/Graphics;Ljava/awt/Rectangle;Lcom/intellij/openapi/editor/markup/TextAttributes;)V",
        &[A::O(inlay), A::O(&graphics), A::O(&rectangle), A::Null],
    )?;
    j.void(&graphics, "dispose", "()V", &[])?;
    let mut pixels = vec![];
    for y in 0..height {
        for x in 0..width {
            let argb = j
                .call(&image, "getRGB", "(II)I", &[A::I(x), A::I(y)])?
                .i()? as u32;
            if argb >> 24 > 0x80 {
                pixels.push([(argb >> 16) as u8, (argb >> 8) as u8, argb as u8]);
            }
        }
    }
    Ok(pixels)
}
