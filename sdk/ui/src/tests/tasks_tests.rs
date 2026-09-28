use super::*;
use serde_json::json;

#[test]
fn output_retains_a_bounded_unicode_tail_in_order() {
    let mut task = TaskState::default();
    task.append("old\nolder");
    let text = (0..1000)
        .map(|i| format!("{i}:{}\n", "🦀".repeat(700)))
        .collect::<String>();
    assert!(task.apply(&json!({"type":"log", "text":text})));
    assert_eq!(task.lines.len(), MAX_LOG_LINES);
    assert!(task.lines[0].starts_with("920:"));
    assert!(task.lines[79].starts_with("999:"));
    assert!(
        task.lines
            .iter()
            .all(|line| line.chars().count() == MAX_LINE_CHARS)
    );
    task.append("tail\n");
    assert_eq!(task.lines.len(), MAX_LOG_LINES);
    assert!(task.lines[0].starts_with("921:"));
    assert_eq!(task.lines.last().expect("retained final line"), "tail");
}

#[test]
fn completion_preserves_results_and_handles_cancel_and_failure() {
    let mut task = TaskState::default();
    task.begin();
    task.apply(&json!({"type":"result", "text":"archive.zip"}));
    task.apply(&json!({"type":"job_finished"}));
    assert!(!task.busy);
    assert_eq!(task.status, "Done");
    assert_eq!(task.result, "archive.zip");
    task.begin();
    assert!(task.result.is_empty());
    task.apply(&json!({"type":"stage", "message":"Package ready"}));
    task.apply(&json!({"type":"job_finished"}));
    assert_eq!(task.status, "Package ready");
    task.begin();
    task.apply(&json!({"type":"job_finished", "error":"Missing SDK"}));
    assert_eq!(task.status, "Missing SDK");
    task.begin();
    task.apply(&json!({"type":"job_finished", "cancelled":true, "error":"terminated"}));
    assert_eq!(task.status, "Stopped");
    assert!(!task.apply(&json!({"type":"plugin_specific"})));
}
