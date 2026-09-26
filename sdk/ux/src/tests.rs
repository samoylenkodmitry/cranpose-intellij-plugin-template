use super::*;

const DARK_THEME: &str = r##"{"dark":true,"background":"#2b2d30","surface":"#393b40","text":"#dfe1e5","muted":"#868a91","accent":"#3574f0"}"##;

#[test]
fn hex_colors_parse_to_unit_channels() {
    assert_eq!(
        parse_hex_color("#ff0080"),
        Some(Rgba(1.0, 0.0, 128.0 / 255.0, 1.0))
    );
    assert_eq!(parse_hex_color("ff0080"), None);
    assert_eq!(parse_hex_color("#ff00"), None);
    assert_eq!(parse_hex_color("#gg0000"), None);
    assert_eq!(parse_hex_color("#ff00\u{e9}"), None);
}

#[test]
fn a_theme_payload_becomes_a_palette() {
    let theme = IdeTheme::parse(DARK_THEME).expect("a theme payload");
    assert!(theme.dark);

    let palette = theme.palette();
    assert_eq!(parse_hex_color("#2b2d30"), Some(palette.background));
    assert_eq!(parse_hex_color("#3574f0"), Some(palette.accent));
    assert_eq!(palette.on_accent, Rgba(0.0, 0.0, 0.0, 1.0));
}

#[test]
fn malformed_theme_colors_keep_their_standalone_values() {
    let theme = IdeTheme {
        accent: "blue".to_string(),
        ..IdeTheme::parse(DARK_THEME).expect("a theme payload")
    };
    assert_eq!(theme.palette().accent, STANDALONE_PALETTE.accent);
    assert_eq!(IdeTheme::parse("{\"dark\":true}"), None);
}

#[test]
fn text_on_the_accent_stays_readable() {
    assert_eq!(
        readable_on(Rgba(1.0, 1.0, 0.9, 1.0)),
        Rgba(0.0, 0.0, 0.0, 1.0)
    );
    assert_eq!(
        readable_on(Rgba(0.1, 0.2, 0.6, 1.0)),
        Rgba(1.0, 1.0, 1.0, 1.0)
    );
}

#[test]
fn saturated_accents_choose_the_higher_contrast_foreground() {
    assert_eq!(
        readable_on(Rgba(0.0, 1.0, 0.0, 1.0)),
        Rgba(0.0, 0.0, 0.0, 1.0)
    );
    assert_eq!(
        readable_on(Rgba(1.0, 0.0, 0.0, 1.0)),
        Rgba(0.0, 0.0, 0.0, 1.0)
    );
    assert_eq!(
        readable_on(Rgba(0.0, 0.0, 1.0, 1.0)),
        Rgba(1.0, 1.0, 1.0, 1.0)
    );
}
