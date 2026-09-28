# Compact choices

`cranpose-plugin-ui::choice::CompactChoice` keeps one selected item visible and
opens a searchable list on explicit activation. Use it for application targets,
profiles, devices or other catalogs that should not fill the tool window.

```rust,ignore
use cranpose_plugin_ui::choice::{ChoiceItem, CompactChoice};

CompactChoice(palette, "profile", vec![
    ChoiceItem {
        id: "development".into(),
        label: "Development".into(),
        detail: "Local workspace".into(),
    },
    ChoiceItem {
        id: "release".into(),
        label: "Release".into(),
        detail: "Optimized package".into(),
    },
], selected.get(), move |id| selected.set(id.to_owned()));
```

The caller owns the selected ID. Labels may repeat; IDs must be unique. A missing
ID displays “Choose …” instead of silently selecting another item. Activation
calls the callback once and closes the list. Closing without choosing preserves
selection. Reopening clears the search and returns to the selected item's page.

Search matches all entered words against both display lines, ignoring case.
Only five results are composed per page. Search starts on the first page;
catalog shrink clamps the page. Tab and Shift-Tab move between the search,
results and page controls, and Enter activates buttons. Focus returns to the
summary after choosing. No hover opening or idle timer is involved.

Labels use one ellipsized line each to keep rows compact. Include distinguishing
package, platform or directory information in `detail` for repeated labels.

## Regression fixture

`choice_demo::ChoiceDemo` is a small standalone consumer. Both the template and
Studio renderer expose it through `CRANPOSE_AUTHORING=choices`. Run the shared
Rust check against either renderer:

```sh
cargo run -p cranpose-template-tools -- choice-smoke \
  --binary target/debug/cranpose-intellij-ui \
  --log choices.log --report choices.json
```

The check uses the production embedded protocol at 1x and 2x. It exercises
pointer and keyboard activation, duplicate-label IDs, searching, cancellation,
paging, a 10,001-item catalog, a removed selection, an empty catalog and process
shutdown. It asserts a bounded UI node count and unchanged collapsed height
when catalog size grows. These checks do not measure whole-IDE CPU or latency.

For a Studio renderer, add `--dashboard --require-background` to record the
seven-target dashboard's control positions and verify that its background fills
the viewport. This compares rendered pixels at the top and bottom, catching
short-content layouts that expose the renderer's fallback clear color.
