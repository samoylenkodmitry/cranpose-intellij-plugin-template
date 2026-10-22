use super::*;

const POPUP: [i32; 2] = [400, 200];
const LINE: i32 = 24;
const GLYPH: i32 = 16;

/// A line of text starting at `line_left`, with a value glyph at `value_left`.
fn value_at(line_left: i32, line_top: i32, value_left: i32) -> Anchor {
    Anchor {
        line_left,
        line_top,
        line_height: LINE,
        value_left,
        value_right: value_left + GLYPH,
    }
}

fn overlaps(popup: [i32; 2], size: [i32; 2], anchor: Anchor) -> bool {
    let [x, y] = popup;
    let [width, height] = size;
    x < anchor.value_right
        && x + width > anchor.line_left
        && y < anchor.line_top + anchor.line_height
        && y + height > anchor.line_top
}

#[test]
fn the_popup_starts_at_the_value_and_just_under_its_line() {
    let screen = [0, 0, 1600, 900];
    let anchor = value_at(40, 100, 700);
    assert_eq!(
        position(screen, POPUP, anchor),
        Some([700, 100 + LINE + GAP])
    );
}

#[test]
fn the_popup_follows_the_value_not_the_pointer() {
    let screen = [0, 0, 1600, 900];
    let near = position(screen, POPUP, value_at(40, 100, 300)).expect("room below");
    let far = position(screen, POPUP, value_at(40, 100, 900)).expect("room below");
    assert_eq!(far[0] - near[0], 600);
    assert_eq!(far[1], near[1]);
}

#[test]
fn the_popup_stays_on_the_screen_near_its_right_edge() {
    for screen in [[0, 0, 1600, 900], [-1600, -200, 1600, 900]] {
        let value_left = screen[0] + screen[2] - 30;
        let [x, y] = position(
            screen,
            POPUP,
            value_at(screen[0] + 40, screen[1] + 100, value_left),
        )
        .expect("room below");
        assert_eq!(x, screen[0] + screen[2] - POPUP[0]);
        assert_eq!(y, screen[1] + 100 + LINE + GAP);
    }
}

#[test]
fn the_popup_stays_on_the_screen_near_its_left_edge() {
    let screen = [100, 0, 1600, 900];
    let [x, y] = position(screen, POPUP, value_at(60, 100, 80)).expect("room below");
    assert_eq!([x, y], [100, 100 + LINE + GAP]);
}

#[test]
fn without_room_below_the_popup_goes_above_the_line() {
    let screen = [0, 0, 1600, 900];
    let [x, y] = position(screen, POPUP, value_at(40, 800, 500)).expect("room above");
    assert_eq!([x, y], [500, 800 - POPUP[1] - GAP]);
}

#[test]
fn without_room_above_or_below_the_popup_goes_beside_the_value() {
    let screen = [0, 0, 1600, 300];
    let anchor = value_at(40, 120, 500);
    let [x, y] = position(screen, POPUP, anchor).expect("room beside");
    assert_eq!(x, anchor.value_right + GAP);
    assert!((0..=300 - POPUP[1]).contains(&y));
    assert!(!overlaps([x, y], POPUP, anchor));
}

#[test]
fn beside_the_line_means_left_of_it_when_the_right_side_is_too_narrow() {
    let screen = [0, 0, 1000, 300];
    let anchor = value_at(440, 120, 900);
    let [x, y] = position(screen, POPUP, anchor).expect("room to the left");
    assert_eq!(x, anchor.line_left - POPUP[0] - GAP);
    assert!(!overlaps([x, y], POPUP, anchor));
}

#[test]
fn a_popup_that_cannot_clear_its_line_stays_hidden() {
    // Taller than the screen.
    assert!(position([0, 0, 400, 400], [400, 490], value_at(0, 100, 200)).is_none());
    // Too small for the popup, and the line takes the whole width.
    assert!(position([0, 0, 500, 600], [400, 490], value_at(0, 290, 484)).is_none());
    // Nothing to place.
    assert!(position([0, 0, 500, 600], [0, 100], value_at(0, 290, 100)).is_none());
}

#[test]
fn the_source_line_stays_clear_on_every_row_of_every_monitor() {
    for screen in [[0, 0, 1600, 900], [-1600, -200, 1600, 900]] {
        for top in (0..880).step_by(37) {
            for value in [0, 60, 700, 1560] {
                let anchor = value_at(screen[0] + 40, screen[1] + top, screen[0] + 40 + value);
                let size = [400, 490];
                if let Some(popup) = position(screen, size, anchor) {
                    assert!(
                        !overlaps(popup, size, anchor),
                        "{popup:?} covers {:?} on {screen:?}",
                        [anchor.line_left, anchor.line_top]
                    );
                    assert!(popup[0] >= screen[0] && popup[0] + size[0] <= screen[0] + screen[2]);
                    assert!(popup[1] >= screen[1] && popup[1] + size[1] <= screen[1] + screen[3]);
                }
            }
        }
    }
}
