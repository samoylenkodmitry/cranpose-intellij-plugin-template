//! Windowing for fixed-height lists rendered by a scrollable Cranpose Column.
use std::ops::Range;

/// Compose `rows` between two spacers of `before` and `after` logical pixels.
/// Every row must occupy exactly the supplied height, including padding.
/// Read reactive scroll state at the call site so scrolling updates the window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowWindow {
    pub rows: Range<usize>,
    pub before: f32,
    pub after: f32,
}

impl RowWindow {
    /// Include partially visible rows and a small number of rows on either side.
    /// A stale offset after filtering is clamped to the new content extent.
    /// Invalid dimensions produce an empty window without non-finite spacers.
    pub fn new(count: usize, row_height: f32, viewport: f32, offset: f32, overscan: usize) -> Self {
        let total = count as f32 * row_height;
        if count == 0
            || !row_height.is_finite()
            || row_height <= 0.0
            || !viewport.is_finite()
            || viewport <= 0.0
            || !total.is_finite()
        {
            return Self::default();
        }
        let offset =
            if offset.is_finite() { offset } else { 0.0 }.clamp(0.0, (total - viewport).max(0.0));
        let first = (offset / row_height).floor() as usize;
        let last = ((offset + viewport) / row_height).ceil() as usize;
        let start = first.saturating_sub(overscan).min(count);
        let end = last.saturating_add(overscan).min(count);
        Self {
            rows: start..end,
            before: start as f32 * row_height,
            after: (count - end) as f32 * row_height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_rows_and_overscan_keep_the_full_scroll_extent() {
        let window = RowWindow::new(1000, 28.0, 100.0, 280.5, 2);
        assert_eq!(window.rows, 8..16);
        assert_eq!(
            window.before + window.rows.len() as f32 * 28.0 + window.after,
            28000.0
        );
        assert_eq!(RowWindow::new(1000, 28.0, 112.0, 280.0, 0).rows, 10..14);
    }

    #[test]
    fn beginning_end_and_filter_shrink_remain_reachable() {
        assert_eq!(RowWindow::new(1000, 28.0, 112.0, 0.0, 2).rows, 0..6);
        assert_eq!(
            RowWindow::new(1000, 28.0, 112.0, f32::MAX, 2).rows,
            994..1000
        );
        assert_eq!(RowWindow::new(3, 28.0, 112.0, 28000.0, 2).rows, 0..3);
        assert_eq!(RowWindow::new(0, 28.0, 112.0, 0.0, 2), RowWindow::default());
    }

    #[test]
    fn large_lists_compose_only_viewport_rows() {
        for count in [1000, 10_000, 1_000_000] {
            for offset in [0.0, 11.0, 1800.0, 1_000_000.0] {
                let window = RowWindow::new(count, 28.0, 400.0, offset, 2);
                assert!(window.rows.len() <= 20);
                assert!(window.rows.end <= count);
            }
        }
    }

    #[test]
    fn invalid_dimensions_never_produce_invalid_geometry() {
        for height in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                RowWindow::new(10, height, 100.0, 0.0, 2),
                RowWindow::default()
            );
        }
        for viewport in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                RowWindow::new(10, 28.0, viewport, 0.0, 2),
                RowWindow::default()
            );
        }
        assert_eq!(
            RowWindow::new(10, 28.0, 28.0, f32::NAN, usize::MAX).rows,
            0..10
        );
    }
}
