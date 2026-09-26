//! Restarts a development UI after an atomic Cargo binary replacement settles.
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, SystemTime},
};
type Stamp = Option<(SystemTime, u64)>;
fn stamp(path: &Path) -> Stamp {
    let m = path.metadata().ok()?;
    Some((m.modified().ok()?, m.len()))
}
pub struct Watcher {
    changes: Receiver<()>,
    stopped: Arc<AtomicBool>,
}
impl Watcher {
    pub fn new(path: PathBuf) -> Self {
        let (tx, changes) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        std::thread::spawn(move || {
            let mut settle = Settled::new(stamp(&path));
            while !stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(500));
                if settle.observe(stamp(&path)) {
                    let _ = tx.try_send(());
                }
            }
        });
        Self { changes, stopped }
    }
    pub fn changed(&self) -> bool {
        self.changes.try_iter().count() > 0
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}
struct Settled {
    accepted: Stamp,
    candidate: Stamp,
    repeats: u8,
}
impl Settled {
    fn new(stamp: Stamp) -> Self {
        Self {
            accepted: stamp,
            candidate: stamp,
            repeats: 0,
        }
    }
    fn observe(&mut self, value: Stamp) -> bool {
        if value == self.accepted || value.is_none() {
            self.candidate = value;
            self.repeats = 0;
            return false;
        }
        if value != self.candidate {
            self.candidate = value;
            self.repeats = 0;
            return false;
        }
        self.repeats += 1;
        if self.repeats < 2 {
            return false;
        }
        self.accepted = value;
        self.repeats = 0;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waits_for_complete_replacement_and_ignores_temporary_missing_file() {
        let old = Some((SystemTime::UNIX_EPOCH, 10));
        let new = Some((SystemTime::UNIX_EPOCH, 20));
        let mut watcher = Settled::new(old);
        assert!(!watcher.observe(None));
        assert!(!watcher.observe(new));
        assert!(!watcher.observe(new));
        assert!(watcher.observe(new));
        assert!(!watcher.observe(new));
        assert!(!watcher.observe(old));
        assert!(!watcher.observe(None));
        assert!(!watcher.observe(new));
    }
}
