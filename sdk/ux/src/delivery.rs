//! Suppress repeated host messages without losing the first value after reconnection.
#[derive(Default)]
pub struct LastValue<T>(Option<T>);
impl<T: PartialEq + Clone> LastValue<T> {
    /// Returns true for the first value and for subsequent changes.
    pub fn update(&mut self, value: &T) -> bool {
        if self.0.as_ref() == Some(value) {
            return false;
        }
        self.0 = Some(value.clone());
        true
    }
    /// A replacement connection needs its initial value even if it is unchanged.
    pub fn reset(&mut self) {
        self.0 = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_layouts_are_skipped_but_changes_and_reconnections_are_delivered() {
        let mut last = LastValue::default();
        assert!(last.update(&(1, 480, 640, false)));
        for _ in 0..1000 {
            assert!(!last.update(&(1, 480, 640, false)));
        }
        assert!(last.update(&(1, 720, 640, false)));
        assert!(last.update(&(1, 720, 640, true)));
        assert!(last.update(&(2, 720, 640, true)));
        last.reset();
        assert!(last.update(&(2, 720, 640, true)));
    }
}
