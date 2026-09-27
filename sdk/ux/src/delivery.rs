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
/// Reject delayed observations without making transport bookkeeping observable UI state.
/// Clones intentionally share the watermark: an unchanged model can be discarded by a
/// composable state's equality policy without forgetting the newest accepted revision.
/// Create a fresh gate when a connection starts a new sequence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SequenceGate(std::rc::Rc<std::cell::Cell<(u64, u64)>>);
impl SequenceGate {
    /// Allocate a distinct request even while an older reply is still in flight.
    pub fn next_request(&self) -> u64 {
        let (accepted, issued) = self.0.get();
        let next = accepted.max(issued).saturating_add(1);
        self.0.set((accepted, next));
        next
    }
    pub fn accept(&self, revision: u64) -> bool {
        let (accepted, issued) = self.0.get();
        if revision < accepted {
            return false;
        }
        self.0.set((revision, issued));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_numbers_advance_without_rejecting_in_flight_replies() {
        let gate = SequenceGate::default();
        assert_eq!(gate.next_request(), 1);
        assert_eq!(gate.next_request(), 2);
        assert!(gate.accept(1));
        assert!(gate.accept(2));
        assert!(!gate.accept(1));
        assert_eq!(gate.clone().next_request(), 3);
        assert_eq!(gate.next_request(), 4);
        assert!(gate.accept(10));
        assert_eq!(gate.next_request(), 11);
    }
    #[test]
    fn unchanged_model_clones_share_the_latest_revision() {
        let gate = SequenceGate::default();
        let next = gate.clone();
        assert!(next.accept(20));
        assert_eq!(gate, next);
        assert!(!gate.accept(19));
        assert!(gate.accept(20));
        assert!(gate.accept(21));
        assert!(!next.accept(20));
        assert!(SequenceGate::default().accept(0));
    }

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
