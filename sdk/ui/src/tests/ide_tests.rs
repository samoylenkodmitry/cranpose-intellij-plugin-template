use super::*;

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
