use super::*;

const DARK_THEME: &str = r##"{"dark":true,"background":"#2b2d30","surface":"#393b40","text":"#dfe1e5","muted":"#868a91","accent":"#3574f0"}"##;

#[test]
fn hex_colors_parse_to_unit_channels() {
    assert_eq!(
        parse_hex_color("#ff0080"),
        Some(Color(1.0, 0.0, 128.0 / 255.0, 1.0))
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
    assert_eq!(palette.on_accent, Color(1.0, 1.0, 1.0, 1.0));
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
        readable_on(Color(1.0, 1.0, 0.9, 1.0)),
        Color(0.0, 0.0, 0.0, 1.0)
    );
    assert_eq!(
        readable_on(Color(0.1, 0.2, 0.6, 1.0)),
        Color(1.0, 1.0, 1.0, 1.0)
    );
}

#[test]
fn an_editor_payload_names_the_open_file() {
    assert_eq!(
        EditorFile::parse(r#"{"path":"/src/main.rs","name":"main.rs"}"#),
        Some(EditorFile {
            path: "/src/main.rs".to_string(),
            name: "main.rs".to_string(),
        })
    );
    assert_eq!(EditorFile::parse("{}"), None);
}

#[test]
fn requests_go_to_the_ide_as_json_and_report_a_missing_ide() {
    let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = std::sync::Arc::clone(&sent);
    cranpose::install_host_outbox(move |message| {
        recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(message);
    });
    assert!(notify_ide("Saved", "all files"));
    assert!(open_in_ide("/src/main.rs"));
    cranpose::clear_host_outbox();

    assert!(!notify_ide("title", "content"));
    assert!(!open_in_ide("/src/main.rs"));

    let sent = sent
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        sent,
        vec![
            cranpose::HostMessage::new(
                NOTIFY_CHANNEL,
                r#"{"content":"all files","title":"Saved"}"#
            ),
            cranpose::HostMessage::new(OPEN_CHANNEL, r#"{"path":"/src/main.rs"}"#),
        ]
    );
}
