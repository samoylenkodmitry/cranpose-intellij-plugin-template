//! Bounded task output and controls shared by plugin background operations.
use crate::{
    controls::{ActionButton, label_style},
    ide::Palette,
};
use cranpose::{
    Column, ColumnSpec, LinearArrangement, Modifier, Text, composable, rememberMutableStateOf,
};
use serde_json::Value;

pub const MAX_LOG_LINES: usize = 80;
pub const MAX_LINE_CHARS: usize = 600;

/// Presentation state for the host's stage, log and completion events.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct TaskState {
    pub busy: bool,
    pub status: String,
    pub lines: Vec<String>,
    pub result: String,
}
impl TaskState {
    /// Begin a new operation and discard the previous operation's output.
    pub fn begin(&mut self) {
        self.busy = true;
        self.status = "Working…".into();
        self.lines.clear();
        self.result.clear();
    }

    /// Apply common host events; return false for a plugin-specific event.
    pub fn apply(&mut self, event: &Value) -> bool {
        match event["type"].as_str().unwrap_or_default() {
            "stage" => self.status = event["message"].as_str().unwrap_or_default().into(),
            "log" => self.append(event["text"].as_str().unwrap_or_default()),
            "result" => self.result = event["text"].as_str().unwrap_or_default().into(),
            "job_finished" => {
                self.busy = false;
                if event["cancelled"] == true {
                    self.status = "Stopped".into();
                } else if let Some(error) = event["error"].as_str() {
                    self.status = error.into();
                } else if self.status == "Working…" {
                    self.status = "Done".into();
                }
            }
            _ => return false,
        }
        true
    }

    /// Keep the tail of output, bounded in lines and Unicode characters.
    pub fn append(&mut self, text: &str) {
        let first = self.lines.len();
        // Only the last MAX_LOG_LINES incoming lines can survive. This also
        // bounds temporary allocations for a single very large output event.
        self.lines.extend(
            text.lines()
                .rev()
                .take(MAX_LOG_LINES)
                .map(|line| line.chars().take(MAX_LINE_CHARS).collect::<String>()),
        );
        self.lines[first..].reverse();
        if self.lines.len() > MAX_LOG_LINES {
            self.lines.drain(..self.lines.len() - MAX_LOG_LINES);
        }
    }
}

/// Show status, an optional result and expandable bounded output. Stop remains
/// available while work is running; expanded details do not create a timer.
#[composable]
pub fn TaskOutput(
    palette: Palette,
    task: TaskState,
    cancel_label: &'static str,
    cancel: impl FnMut() + 'static,
) {
    let cancel = std::rc::Rc::new(cancel);
    let expanded = rememberMutableStateOf(|| false);
    Column(
        Modifier::empty().fill_max_width(),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(8.0)),
        move || {
            if task.busy {
                let cancel = cancel.clone();
                ActionButton(palette, cancel_label, true, false, move || cancel());
            }
            Text(
                task.status.clone(),
                Modifier::empty().fill_max_width(),
                label_style(palette, false),
            );
            if !task.result.is_empty() {
                Text(
                    task.result.clone(),
                    Modifier::empty().fill_max_width(),
                    label_style(palette, true),
                );
            }
            if !task.lines.is_empty() {
                ActionButton(
                    palette,
                    if expanded.get() {
                        "Hide details"
                    } else {
                        "Show details"
                    },
                    true,
                    false,
                    move || expanded.set(!expanded.get()),
                );
                if expanded.get() {
                    for line in &task.lines {
                        Text(
                            line.clone(),
                            Modifier::empty().fill_max_width(),
                            label_style(palette, true),
                        );
                    }
                }
            }
        },
    );
}

#[cfg(test)]
#[path = "tests/tasks_tests.rs"]
mod tests;
