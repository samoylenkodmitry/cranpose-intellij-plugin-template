//! Replace this sample operation with your plugin's own background work.
use anyhow::{Result, bail};
use cranpose_host::{
    jobs::{self, Operation},
    jvm::{J, O},
};
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub fn handle(j: &mut J<'_>, project: &O, channel: &str, payload: &str) -> Result<bool> {
    jobs::handle_message(j, project, "template.task", channel, payload, prepare)
}

fn prepare(payload: &str) -> Result<Operation> {
    let value: serde_json::Value = serde_json::from_str(payload)?;
    match value["action"].as_str() {
        Some("cancel") => Ok(Operation::Cancel),
        Some("scan") => Ok(Operation::run(false, |context| {
            context.send(&json!({"type":"stage", "message":"Scanning project…"}));
            let summary = scan(
                &context.root,
                || context.cancel.is_cancelled(),
                |path| {
                    context.send(&json!({"type":"log", "text":path.display().to_string()}));
                },
            )?;
            context.send(&json!({"type":"result", "text":summary}));
            context.send(&json!({"type":"stage", "message":"Scan complete"}));
            Ok(())
        })),
        _ => bail!("Unknown project task"),
    }
}

/// A bounded, read-only scan. Never follows links or enters build/VCS folders.
fn scan(
    root: &Path,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(&Path),
) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut pending = vec![(root.to_owned(), 0)];
    let mut entries = 0;
    let mut files = 0;
    let mut limited = false;
    'scan: while let Some((directory, depth)) = pending.pop() {
        if cancelled() {
            return Ok("Scan stopped".into());
        }
        for entry in std::fs::read_dir(&directory)? {
            if cancelled() {
                return Ok("Scan stopped".into());
            }
            if entries >= 20_000 || Instant::now() >= deadline {
                limited = true;
                break 'scan;
            }
            entries += 1;
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_file() {
                files += 1;
            }
            if entries % 1000 == 0 {
                progress(&directory);
            }
            if kind.is_dir()
                && !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | ".idea" | "target" | "node_modules")
                )
            {
                if depth < 32 {
                    pending.push((entry.path(), depth + 1));
                } else {
                    limited = true;
                }
            }
        }
    }
    Ok(format!(
        "{files} files found{}",
        if limited { " (scan limit reached)" } else { "" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_requests_never_start_work() {
        assert!(prepare("{").is_err());
        assert!(prepare(r#"{"action":"unknown"}"#).is_err());
        assert!(matches!(
            prepare(r#"{"action":"cancel"}"#).unwrap(),
            Operation::Cancel
        ));
    }

    #[test]
    fn cancellation_does_not_access_the_filesystem() {
        assert_eq!(
            scan(Path::new("/no/such/project"), || true, |_| {}).unwrap(),
            "Scan stopped"
        );
    }
}
