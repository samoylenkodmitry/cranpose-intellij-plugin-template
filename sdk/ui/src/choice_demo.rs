//! Small standalone fixture for exercising the shared choice control.
use crate::{
    choice::{ChoiceItem, CompactChoice},
    controls::label_style,
    ide::rememberPalette,
};
use cranpose::{
    Column, ColumnSpec, LinearArrangement, Modifier, Text, composable, rememberHostMessages,
    rememberMutableStateOf, send_to_host,
};
use cranpose_core::CollectEvents;

/// Native fixture used by `xtask choice-smoke`; also useful while styling controls.
#[composable]
pub fn ChoiceDemo() {
    let palette = rememberPalette();
    let count = rememberMutableStateOf(|| 7usize);
    let selected = rememberMutableStateOf(|| "item-0".to_owned());
    CollectEvents(
        rememberHostMessages("demo.choices"),
        (),
        move |payload: String| {
            if let Ok(value) = payload.parse::<usize>() {
                count.set(value.min(10_001));
            }
        },
    );
    Column(
        Modifier::empty()
            .fill_max_size()
            .background(palette.background)
            .padding(12.0),
        ColumnSpec::default().vertical_arrangement(LinearArrangement::spaced_by(6.0)),
        move || {
            let items = (0..count.get())
                .map(|index| ChoiceItem {
                    id: format!("item-{index}"),
                    label: if index == 1 || index == 6 {
                        "Same name".into()
                    } else {
                        format!("Target {index}")
                    },
                    detail: format!("Package {index} · bin"),
                })
                .collect();
            CompactChoice(palette, "target", items, selected.get(), move |id| {
                selected.set(id.to_owned());
                send_to_host("demo.selected", &serde_json::json!({"id":id}).to_string());
            });
            Text(
                format!("Selected: {}", selected.get()),
                Modifier::empty(),
                label_style(palette, false),
            );
        },
    );
}
