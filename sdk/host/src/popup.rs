//! Source-friendly placement and explicit activation of hover controls.

use crate::jvm::{self, A, J, O};
use anyhow::Result;

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
