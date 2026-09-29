//! Source-friendly placement and explicit activation of hover controls.

use crate::jvm::{self, A, J, O};
use anyhow::Result;
use serde_json::Value;

/// Give an already-visible hover popup keyboard focus after explicit activation.
/// Requesting focus inside its window is insufficient: IntelliJ creates hover
/// windows with native focus disabled when `setRequestFocus(false)` is used.
pub fn activate(j: &mut J<'_>, popup: &O, component: &O) -> Result<()> {
    j.void(popup, "setRequestFocus", "(Z)V", &[A::Z(true)])?;
    let popup = popup.clone();
    let component = component.clone();
    // Finish the editor's mouse dispatch before transferring keyboard focus.
    jvm::later(j, move |j| {
        if j.bool(&popup, "isDisposed")? || !j.bool(&popup, "isVisible")? {
            return Ok(());
        }
        let window = j.static_obj(
            "javax/swing/SwingUtilities",
            "getWindowAncestor",
            "(Ljava/awt/Component;)Ljava/awt/Window;",
            &[A::O(&component)],
        )?;
        if window.is_null() {
            return Ok(());
        }
        j.void(&window, "setFocusable", "(Z)V", &[A::Z(true)])?;
        // Restore the native window flags as well as the Swing component flag.
        // Bring it forward only for this explicit gesture, never on hover.
        j.void(&window, "setFocusableWindowState", "(Z)V", &[A::Z(true)])?;
        j.void(&window, "setAutoRequestFocus", "(Z)V", &[A::Z(true)])?;
        j.void(&window, "toFront", "()V", &[])?;
        let manager = j.static_obj(
            "com/intellij/openapi/wm/IdeFocusManager",
            "getGlobalInstance",
            "()Lcom/intellij/openapi/wm/IdeFocusManager;",
            &[],
        )?;
        j.obj(
            &manager,
            "requestFocus",
            "(Ljava/awt/Component;Z)Lcom/intellij/openapi/util/ActionCallback;",
            &[A::O(&component), A::Z(true)],
        )?;
        Ok(())
    })
}

/// Rectangles are [left, top, width, height] in Swing logical coordinates.
/// Prefer below, then above, then beside the protected source. If the screen
/// cannot fit the control without covering its source, leave source editing free.
pub fn position(screen: [i32; 4], size: [i32; 2], source: [i32; 4]) -> Option<[i32; 2]> {
    let [sx, sy, sw, sh] = screen;
    let [width, height] = size;
    let [left, top, span, line] = source;
    if width > sw || height > sh || width <= 0 || height <= 0 {
        return None;
    }
    let x = (left + span).clamp(sx, sx + sw - width);
    let y = top.clamp(sy, sy + sh - height);
    [
        [x, top + line + 6],
        [x, top - height - 6],
        [left + span + 6, y],
        [left - width - 6, y],
    ]
    .into_iter()
    .find(|[x, y]| *x >= sx && *y >= sy && x + width <= sx + sw && y + height <= sy + sh)
}

/// A native IntelliJ list popup for `items` (`id`, `label`, optional
/// `checked` and `separator`). Checked items carry the IDE checkmark and are
/// preselected; longer lists get speed search. `choose` receives the chosen
/// id. Returns the popup and its callback id, which the caller unregisters
/// when the popup is replaced or its owner closes.
pub(crate) fn list(
    j: &mut J<'_>,
    items: &[Value],
    choose: impl Fn(&str) + Send + Sync + 'static,
) -> Result<Option<(O, i64)>> {
    let items = &items[..items.len().min(500)];
    if items.is_empty() {
        return Ok(None);
    }
    let text = |item: &Value, key: &str| item[key].as_str().unwrap_or_default().to_owned();
    let labels = items.iter().map(|i| text(i, "label")).collect::<Vec<_>>();
    let ids = items.iter().map(|i| text(i, "id")).collect::<Vec<_>>();
    let separators = items
        .iter()
        .map(|i| i["separator"] == true)
        .collect::<Vec<_>>();
    let checked = items.iter().position(|i| i["checked"] == true);
    let search = items.len() > 7;
    let id = jvm::register(move |j, op, args| {
        let index = if args.first().is_some_and(|a| !a.is_null()) {
            j.int(&args[0], "intValue")? as usize
        } else {
            usize::MAX
        };
        match op {
            "MenuStep.getTextFor" => j.string(labels.get(index).map_or("", String::as_str)),
            "MenuStep.getSeparatorAbove" if separators.get(index) == Some(&true) => {
                j.new("com/intellij/openapi/ui/popup/ListSeparator", "()V", &[])
            }
            "MenuStep.isSpeedSearchEnabled" => j.boxed_bool(search),
            "MenuStep.onChosen" => {
                if let Some(item) = ids.get(index) {
                    choose(item);
                }
                j.null()
            }
            _ => j.null(),
        }
    });
    let result = (|| -> Result<O> {
        let step = j.new("dev/cranpose/rust/MenuStep", "(J)V", &[A::J(id)])?;
        let values = j.new("java/util/ArrayList", "()V", &[])?;
        let icons = j.new("java/util/ArrayList", "()V", &[])?;
        let mark = j.constant(
            "com/intellij/icons/AllIcons$Actions",
            "Checked",
            "Ljavax/swing/Icon;",
        )?;
        let blank = j.constant(
            "com/intellij/util/ui/EmptyIcon",
            "ICON_16",
            "Ljavax/swing/Icon;",
        )?;
        for (index, item) in items.iter().enumerate() {
            let value = j.boxed_int(index as i32)?;
            j.call(&values, "add", "(Ljava/lang/Object;)Z", &[A::O(&value)])?;
            let icon = if item["checked"] == true {
                &mark
            } else {
                &blank
            };
            j.call(&icons, "add", "(Ljava/lang/Object;)Z", &[A::O(icon)])?;
        }
        j.void(
            &step,
            "init",
            "(Ljava/lang/String;Ljava/util/List;Ljava/util/List;)V",
            &[
                A::Null,
                A::O(&values),
                if checked.is_some() {
                    A::O(&icons)
                } else {
                    A::Null
                },
            ],
        )?;
        if let Some(index) = checked {
            j.void(
                &step,
                "setDefaultOptionIndex",
                "(I)V",
                &[A::I(index as i32)],
            )?;
        }
        let factory = j.static_obj(
            "com/intellij/openapi/ui/popup/JBPopupFactory",
            "getInstance",
            "()Lcom/intellij/openapi/ui/popup/JBPopupFactory;",
            &[],
        )?;
        j.obj(
            &factory,
            "createListPopup",
            "(Lcom/intellij/openapi/ui/popup/ListPopupStep;)Lcom/intellij/openapi/ui/popup/ListPopup;",
            &[A::O(&step)],
        )
    })();
    match result {
        Ok(popup) => Ok(Some((popup, id))),
        Err(error) => {
            jvm::unregister(id);
            Err(error)
        }
    }
}

#[cfg(feature = "ide-tests")]
pub(crate) fn list_test(j: &mut J<'_>) -> Result<()> {
    use anyhow::ensure;
    use std::sync::{Arc, Mutex};
    let chosen = Arc::new(Mutex::new(None::<String>));
    let sink = chosen.clone();
    let items = serde_json::json!([
        {"id":"a","label":"Compact  360 × 640"},
        {"id":"b","label":"Phone  480 × 760","checked":true},
        {"id":"rotate","label":"Rotate","separator":true}
    ]);
    let (popup, id) = list(j, items.as_array().expect("items"), move |item| {
        *sink.lock().expect("chosen") = Some(item.to_owned());
    })?
    .ok_or_else(|| anyhow::anyhow!("Empty menu"))?;
    let result = (|| -> Result<()> {
        let step = j.obj(
            &popup,
            "getListStep",
            "()Lcom/intellij/openapi/ui/popup/ListPopupStep;",
            &[],
        )?;
        let values = j.obj(&step, "getValues", "()Ljava/util/List;", &[])?;
        ensure!(j.int(&values, "size")? == 3, "Menu values");
        let value = |j: &mut J<'_>, index: i32| {
            j.obj(&values, "get", "(I)Ljava/lang/Object;", &[A::I(index)])
        };
        let second = value(j, 1)?;
        let label = j.obj(
            &step,
            "getTextFor",
            "(Ljava/lang/Object;)Ljava/lang/String;",
            &[A::O(&second)],
        )?;
        ensure!(j.read_string(&label)? == "Phone  480 × 760", "Menu label");
        ensure!(
            j.int(&step, "getDefaultOptionIndex")? == 1,
            "Checked item preselected"
        );
        let icon = j.obj(
            &step,
            "getIconFor",
            "(Ljava/lang/Object;)Ljavax/swing/Icon;",
            &[A::O(&second)],
        )?;
        ensure!(!icon.is_null(), "Checkmark icon");
        let third = value(j, 2)?;
        for (item, separated) in [(&second, false), (&third, true)] {
            let separator = j.obj(
                &step,
                "getSeparatorAbove",
                "(Ljava/lang/Object;)Lcom/intellij/openapi/ui/popup/ListSeparator;",
                &[A::O(item)],
            )?;
            ensure!(separator.is_null() != separated, "Menu separators");
        }
        ensure!(
            !j.bool(&step, "isSpeedSearchEnabled")?,
            "Short menus need no speed search"
        );
        j.obj(
            &step,
            "onChosen",
            "(Ljava/lang/Object;Z)Lcom/intellij/openapi/ui/popup/PopupStep;",
            &[A::O(&third), A::Z(true)],
        )?;
        ensure!(
            chosen.lock().expect("chosen").as_deref() == Some("rotate"),
            "Menu choice was not reported"
        );
        Ok(())
    })();
    j.void(&popup, "cancel", "()V", &[])?;
    jvm::unregister(id);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_remains_clear_near_edges_and_on_offset_monitors() {
        for screen in [[0, 0, 1600, 900], [-1600, -200, 1600, 900]] {
            for top in [10, 390, 780] {
                let source = [screen[0] + 40, screen[1] + top, 700, 24];
                let [x, y] = position(screen, [400, 490], source).expect("space beside line");
                assert!(
                    y + 490 <= source[1]
                        || y >= source[1] + 24
                        || x + 400 <= source[0]
                        || x >= source[0] + 700
                );
            }
        }
        assert!(position([0, 0, 400, 400], [400, 490], [0, 100, 400, 24]).is_none());
        assert!(position([0, 0, 500, 600], [400, 490], [0, 290, 500, 24]).is_none());
    }
}
