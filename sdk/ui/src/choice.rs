//! A compact, searchable choice list with bounded composition.
use crate::{controls::label_style, ide::Palette};
use cranpose::{
    BasicTextFieldOptions, BasicTextFieldWithOptions, Button, ButtonSpec, Column, ColumnSpec,
    FocusRequester, KeyCode, LinearArrangement, Modifier, Row, RowSpec, Text, TextFieldLineLimits,
    TextFieldState, TextOptions, TextOverflow, TextWithOptions, UnhandledKeyEvents, composable,
    remember, rememberMutableStateOf,
};
use std::rc::Rc;

/// An opaque identity and two display lines. Labels need not be unique.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceItem {
    pub id: String,
    pub label: String,
    pub detail: String,
}

/// At most this many choices are composed, even for very large catalogs.
pub const PAGE_SIZE: usize = 5;

fn matching(items: &[ChoiceItem], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    let terms: Vec<_> = query.split_whitespace().collect();
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let haystack = format!("{} {}", item.label, item.detail).to_lowercase();
            terms
                .iter()
                .all(|term| haystack.contains(term))
                .then_some(index)
        })
        .collect()
}

fn page_start(requested: usize, count: usize) -> usize {
    requested.min(count.saturating_sub(1) / PAGE_SIZE) * PAGE_SIZE
}

/// Display the selected item until the user explicitly opens the chooser.
///
/// Search matches words in either display line, ignoring case. Five results per
/// page keep layout and composition bounded. Tab/Shift-Tab navigate controls;
/// Enter activates a choice. Closing leaves selection unchanged. Only a choice
/// activation calls `on_select`, with its opaque ID; the caller owns selection.
#[composable]
pub fn CompactChoice(
    palette: Palette,
    label: &'static str,
    items: Vec<ChoiceItem>,
    selected: String,
    on_select: impl Fn(&str) + 'static,
) {
    let expanded = rememberMutableStateOf(|| false);
    // Keep field state above the conditional branch so detach precedes disposal.
    let search = remember(|| TextFieldState::new("")).with(|state| *state);
    let page = rememberMutableStateOf(|| (String::new(), 0usize));
    let trigger = remember(FocusRequester::new).with(Clone::clone);
    let on_select: Rc<dyn Fn(&str)> = Rc::new(on_select);
    let selected_item = items.iter().find(|item| item.id == selected).cloned();
    let selected_page = items
        .iter()
        .position(|item| item.id == selected)
        .unwrap_or(0)
        / PAGE_SIZE;
    let count = items.len();
    let open = expanded.get();
    let title = selected_item
        .as_ref()
        .map_or_else(|| format!("Choose {label}"), |item| item.label.clone());
    let detail = selected_item.map_or_else(String::new, |item| item.detail);
    let trigger_modifier = Modifier::empty()
        .fill_max_width()
        .rounded_corners(7.0)
        .background(palette.selection())
        .padding(8.0)
        .focus_requester(&trigger)
        .content_description(format!("{label}: {title}. {} choices", count));
    Button(
        trigger_modifier,
        ButtonSpec::default(),
        move || {
            if !open {
                search.set_text("");
                page.set((String::new(), selected_page));
            }
            expanded.set(!open);
        },
        move || {
            let title = title.clone();
            let detail = detail.clone();
            Column(
                Modifier::empty().fill_max_width(),
                ColumnSpec::default(),
                move || {
                    ChoiceLine(title.clone(), palette, false);
                    if !detail.is_empty() {
                        ChoiceLine(detail.clone(), palette, true);
                    }
                    Text(
                        if open {
                            "Close choices".to_owned()
                        } else {
                            format!("Change {label} · {count} choices")
                        },
                        Modifier::empty().padding_symmetric(0.0, 3.0),
                        label_style(palette, true),
                    );
                },
            );
        },
    );
    if !open {
        return;
    }
    let escape_focus = trigger.clone();
    UnhandledKeyEvents(move |event| {
        if event.is_key_down() && event.key_code == KeyCode::Escape {
            expanded.set(false);
            let _ = escape_focus.request_focus();
            true
        } else {
            false
        }
    });
    Text(
        format!("Search {label}"),
        Modifier::empty(),
        label_style(palette, true),
    );
    BasicTextFieldWithOptions(
        search,
        Modifier::empty()
            .fill_max_width()
            .height(32.0)
            .background(palette.background)
            .rounded_corners(6.0)
            .padding(6.0)
            .content_description(format!("Search {label}")),
        BasicTextFieldOptions {
            text_style: label_style(palette, false),
            cursor_color: palette.accent,
            line_limits: TextFieldLineLimits::SingleLine,
        },
    );
    let query = search.text();
    let matches = matching(&items, &query);
    let (previous_query, requested_page) = page.get();
    if query != previous_query {
        page.set((query.clone(), 0));
    }
    let start = page_start(
        if query == previous_query {
            requested_page
        } else {
            0
        },
        matches.len(),
    );
    if matches.is_empty() {
        Text(
            "No matching choices",
            Modifier::empty(),
            label_style(palette, true),
        );
    }
    for &index in matches.iter().skip(start).take(PAGE_SIZE) {
        let item = items[index].clone();
        let id = item.id.clone();
        let chosen = item.id == selected;
        let select = on_select.clone();
        let focus = trigger.clone();
        Button(
            Modifier::empty()
                .fill_max_width()
                .rounded_corners(6.0)
                .background(if chosen {
                    palette.selection()
                } else {
                    palette.background
                })
                .padding(7.0),
            ButtonSpec::default(),
            move || {
                expanded.set(false);
                let _ = focus.request_focus();
                select(&id);
            },
            move || {
                let item = item.clone();
                Column(
                    Modifier::empty().fill_max_width(),
                    ColumnSpec::default(),
                    move || {
                        ChoiceLine(item.label.clone(), palette, false);
                        ChoiceLine(item.detail.clone(), palette, true);
                    },
                );
            },
        );
    }
    if matches.len() > PAGE_SIZE {
        let total = matches.len();
        Row(
            Modifier::empty(),
            RowSpec::default().horizontal_arrangement(LinearArrangement::spaced_by(8.0)),
            move || {
                let previous_query = query.clone();
                crate::controls::ActionButton(palette, "Previous", start > 0, false, move || {
                    page.set((
                        previous_query.clone(),
                        (start / PAGE_SIZE).saturating_sub(1),
                    ));
                });
                Text(
                    format!(
                        "{}–{} of {total}",
                        start + 1,
                        (start + PAGE_SIZE).min(total)
                    ),
                    Modifier::empty().padding(7.0),
                    label_style(palette, true),
                );
                let next_query = query.clone();
                crate::controls::ActionButton(
                    palette,
                    "Next",
                    start + PAGE_SIZE < total,
                    false,
                    move || {
                        page.set((next_query.clone(), start / PAGE_SIZE + 1));
                    },
                );
            },
        );
    }
}

#[composable]
fn ChoiceLine(text: String, palette: Palette, muted: bool) {
    TextWithOptions(
        text,
        Modifier::empty().fill_max_width(),
        label_style(palette, muted),
        TextOptions {
            max_lines: Some(1),
            soft_wrap: false,
            overflow: TextOverflow::Ellipsis,
            ..Default::default()
        },
    );
}

#[cfg(test)]
#[path = "tests/choice_tests.rs"]
mod tests;
