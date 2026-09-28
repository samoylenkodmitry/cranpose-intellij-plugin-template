//! A complete cancellable operation using the shared UI and host job APIs.
use cranpose::{
    Column, ColumnSpec, LinearArrangement, Modifier, Text, composable, rememberHostMessages,
    rememberMutableStateOf,
};
use cranpose_core::CollectEvents;
use cranpose_plugin_ui::{
    controls::{ActionButton, label_style},
    ide::Palette,
    tasks::{TaskOutput, TaskState},
};

pub const CHANNEL: &str = "template.task";

#[composable]
pub fn BackgroundTask(palette: Palette) {
    let task = rememberMutableStateOf(TaskState::default);
    CollectEvents(rememberHostMessages(CHANNEL), (), move |payload: String| {
        if let Ok(event) = serde_json::from_str(&payload) {
            task.update(|state| {
                state.apply(&event);
            });
        }
    });
    Column(
        Modifier::empty()
            .fill_max_width()
            .background(palette.surface)
            .rounded_corners(10.0)
            .padding(12.0),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(8.0)),
        move || {
            Text(
                "PROJECT TASK",
                Modifier::empty(),
                label_style(palette, true),
            );
            Text(
                "Count project files in a cancellable Rust worker.",
                Modifier::empty(),
                label_style(palette, false),
            );
            ActionButton(
                palette,
                "Scan project",
                !task.get().busy,
                false,
                move || {
                    task.update(TaskState::begin);
                    if !cranpose::send_to_host(CHANNEL, r#"{"action":"scan"}"#) {
                        task.update(|state| {
                            state.busy = false;
                            state.status = "Open this panel in the IDE to scan a project".into();
                        });
                    }
                },
            );
            TaskOutput(palette, task.get(), "Stop scan", move || {
                cranpose::send_to_host(CHANNEL, r#"{"action":"cancel"}"#);
            });
        },
    );
}
