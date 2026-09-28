//! Screen-space placement that leaves the hovered source line clickable.

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
