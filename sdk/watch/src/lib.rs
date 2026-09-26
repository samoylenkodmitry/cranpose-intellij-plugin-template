//! Bounded, deduplicated change delivery between a watcher and its consumer.
//! Filter irrelevant events before calling `push`; they must not extend the quiet period.
use std::{
    collections::BTreeSet,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub struct BatchPolicy {
    pub quiet: Duration,
    pub max_delay: Duration,
    pub capacity: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Batch<K> {
    pub items: BTreeSet<K>,
    /// The consumer must rescan or require a restart, never apply a partial batch.
    pub invalidated: bool,
}

struct Pending<K> {
    batch: Batch<K>,
    first: Option<Instant>,
    last: Option<Instant>,
}
impl<K: Ord> Pending<K> {
    fn new() -> Self {
        Self {
            batch: Batch {
                items: BTreeSet::new(),
                invalidated: false,
            },
            first: None,
            last: None,
        }
    }
    fn touch(&mut self, now: Instant) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }
    fn push(&mut self, item: K, now: Instant, capacity: usize) {
        self.touch(now);
        if self.batch.invalidated {
            return;
        }
        if self.batch.items.len() < capacity || self.batch.items.contains(&item) {
            self.batch.items.insert(item);
        } else {
            self.batch.items.clear();
            self.batch.invalidated = true;
        }
    }
    fn deadline(&self, policy: BatchPolicy) -> Option<Instant> {
        Some((self.first? + policy.max_delay).min(self.last? + policy.quiet))
    }
    fn take(&mut self) -> Batch<K> {
        std::mem::replace(self, Self::new()).batch
    }
}

pub struct ChangeQueue<K> {
    shared: Arc<(Mutex<Pending<K>>, Condvar)>,
    policy: BatchPolicy,
}
impl<K> Clone for ChangeQueue<K> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            policy: self.policy,
        }
    }
}
impl<K: Ord> ChangeQueue<K> {
    pub fn new(policy: BatchPolicy) -> Self {
        assert!(policy.capacity > 0 && !policy.max_delay.is_zero());
        Self {
            shared: Arc::new((Mutex::new(Pending::new()), Condvar::new())),
            policy,
        }
    }
    pub fn push(&self, item: K) {
        let (pending, wake) = &*self.shared;
        pending
            .lock()
            .expect("change queue")
            .push(item, Instant::now(), self.policy.capacity);
        wake.notify_one();
    }
    /// Watcher errors or lost-event notifications invalidate the entire pending batch.
    pub fn invalidate(&self) {
        let (pending, wake) = &*self.shared;
        let mut pending = pending.lock().expect("change queue");
        pending.touch(Instant::now());
        pending.batch.items.clear();
        pending.batch.invalidated = true;
        wake.notify_one();
    }
    /// Wait at most `timeout`, so consumers can check cancellation and child exit.
    /// A timeout preserves pending changes and their original maximum deadline.
    pub fn take(&self, timeout: Duration) -> Option<Batch<K>> {
        let until = Instant::now() + timeout;
        let (pending, wake) = &*self.shared;
        let mut pending = pending.lock().expect("change queue");
        loop {
            let now = Instant::now();
            let deadline = pending.deadline(self.policy);
            if deadline.is_some_and(|deadline| now >= deadline) {
                return Some(pending.take());
            }
            if now >= until {
                return None;
            }
            let wait = deadline
                .unwrap_or(until)
                .min(until)
                .saturating_duration_since(now);
            pending = wake.wait_timeout(pending, wait).expect("change queue").0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> BatchPolicy {
        BatchPolicy {
            quiet: Duration::from_millis(60),
            max_delay: Duration::from_millis(240),
            capacity: 2,
        }
    }
    #[test]
    fn atomic_save_events_coalesce_but_continuous_writes_have_a_deadline() {
        let mut pending = Pending::new();
        let start = Instant::now();
        for millis in [0, 20, 50] {
            pending.push("main.rs", start + Duration::from_millis(millis), 2);
        }
        assert_eq!(
            pending.deadline(policy()),
            Some(start + Duration::from_millis(110))
        );
        for millis in [100, 150, 200, 230] {
            pending.push("main.rs", start + Duration::from_millis(millis), 2);
        }
        assert_eq!(
            pending.deadline(policy()),
            Some(start + Duration::from_millis(240))
        );
        assert_eq!(pending.take().items, BTreeSet::from(["main.rs"]));
        assert_eq!(pending.deadline(policy()), None);
    }
    #[test]
    fn duplicate_storms_are_bounded_and_overflow_never_returns_partial_changes() {
        let mut pending = Pending::new();
        for _ in 0..100_000 {
            pending.push("a", Instant::now(), 2);
        }
        assert_eq!(pending.batch.items.len(), 1);
        pending.push("b", Instant::now(), 2);
        pending.push("c", Instant::now(), 2);
        pending.push("d", Instant::now(), 2);
        let batch = pending.take();
        assert!(batch.invalidated);
        assert!(batch.items.is_empty());
        pending.push("e", Instant::now(), 2);
        assert!(!pending.take().invalidated);
    }
    #[test]
    fn timeouts_preserve_changes_and_explicit_invalidation_reaches_consumer() {
        let queue = ChangeQueue::new(policy());
        assert!(queue.take(Duration::ZERO).is_none());
        queue.push("a");
        assert!(queue.take(Duration::ZERO).is_none());
        queue.clone().invalidate();
        let batch = queue.take(Duration::from_secs(1)).expect("batch");
        assert!(batch.invalidated);
        assert!(batch.items.is_empty());
        assert!(queue.take(Duration::ZERO).is_none());
    }
}
