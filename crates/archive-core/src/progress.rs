//! Renderer-independent, coarse operation snapshots.
//!
//! Counters distinguish physical I/O, decoder work, and selected output. Observers
//! must return promptly and must not block the decoder. Callbacks contain no
//! passwords. [`NoProgress`] selects a statically disabled reporting path.
use std::sync::{Arc, Mutex};

/// Operation stage, independent of terminal rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Reading,
    Decoding,
    Writing,
    Verifying,
    Complete,
    Failed,
    Cancelled,
}
/// Batched cumulative counters. Unknown totals are represented explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub stage: Stage,
    pub physical_bytes_read: u64,
    /// False when this adapter cannot observe physical source reads.
    pub physical_reads_known: bool,
    pub decoded_bytes: u64,
    /// False when shared decoder work cannot be measured by this adapter.
    pub decoded_work_known: bool,
    pub selected_bytes_written: u64,
    pub entries_completed: u64,
    pub verified: bool,
    pub terminal: bool,
    pub total_selected_bytes: Option<u64>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            stage: Stage::Reading,
            physical_bytes_read: 0,
            physical_reads_known: false,
            decoded_bytes: 0,
            decoded_work_known: false,
            selected_bytes_written: 0,
            entries_completed: 0,
            verified: false,
            terminal: false,
            total_selected_bytes: None,
        }
    }
}
/// Fast, nonblocking observer. Invocation frequency is bounded by I/O chunks.
pub trait Observer {
    const ENABLED: bool = true;
    fn snapshot(&mut self, snapshot: Snapshot);
}
/// Compile-time disabled reporting; no allocations, clocks, locks, or callbacks.
#[derive(Default)]
pub struct NoProgress;
impl Observer for NoProgress {
    const ENABLED: bool = false;
    fn snapshot(&mut self, _: Snapshot) {}
}
/// One bounded slot for an adapter sampling at 5-10 Hz outside decoder execution.
/// Intermediate updates are dropped under contention; terminal updates are retained.
#[derive(Clone, Default)]
pub struct CoalescingObserver {
    latest: Arc<Mutex<Option<Snapshot>>>,
}
impl CoalescingObserver {
    /// Take the newest snapshot, replacing superseded intermediate events.
    pub fn take(&self) -> Option<Snapshot> {
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}
impl Observer for CoalescingObserver {
    fn snapshot(&mut self, snapshot: Snapshot) {
        if snapshot.terminal {
            *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(snapshot);
        } else if let Ok(mut latest) = self.latest.try_lock() {
            *latest = Some(snapshot);
        }
    }
}
/// Operation-local counters. Updates use existing coarse boundaries, never codec symbols.
pub struct Reporter<'a, O: Observer> {
    observer: &'a mut O,
    snapshot: Snapshot,
}
impl<'a, O: Observer> Reporter<'a, O> {
    pub fn new(observer: &'a mut O, total_selected_bytes: Option<u64>) -> Self {
        Self {
            observer,
            snapshot: Snapshot {
                total_selected_bytes,
                ..Default::default()
            },
        }
    }
    pub fn stage(&mut self, stage: Stage) {
        if O::ENABLED {
            self.snapshot.stage = stage;
        }
    }
    pub fn read(&mut self, bytes: u64) {
        if O::ENABLED {
            self.snapshot.physical_reads_known = true;
            self.snapshot.physical_bytes_read =
                self.snapshot.physical_bytes_read.saturating_add(bytes);
        }
    }
    pub fn decoded(&mut self, bytes: u64) {
        if O::ENABLED {
            self.snapshot.decoded_work_known = true;
            self.snapshot.decoded_bytes = self.snapshot.decoded_bytes.saturating_add(bytes);
        }
    }
    pub fn written(&mut self, bytes: u64) {
        if O::ENABLED {
            self.snapshot.selected_bytes_written =
                self.snapshot.selected_bytes_written.saturating_add(bytes);
        }
    }
    /// Count an entry only after its verification succeeds.
    pub fn verified_entry(&mut self) {
        if O::ENABLED {
            self.snapshot.entries_completed = self.snapshot.entries_completed.saturating_add(1);
        }
    }
    pub fn publish(&mut self) {
        if O::ENABLED {
            self.observer.snapshot(self.snapshot);
        }
    }
    /// Publish exact final counters and status even when intermediate updates coalesce.
    pub fn finish(&mut self, stage: Stage) {
        if O::ENABLED {
            self.snapshot.stage = stage;
            self.snapshot.verified = stage == Stage::Complete;
            self.snapshot.terminal = true;
            self.publish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Recording(Vec<Snapshot>);
    impl Observer for Recording {
        fn snapshot(&mut self, snapshot: Snapshot) {
            self.0.push(snapshot);
        }
    }
    #[test]
    fn final_counters_distinguish_decode_work_from_selected_payload() {
        let mut observer = Recording::default();
        let mut reporter = Reporter::new(&mut observer, Some(12));
        reporter.read(8);
        reporter.decoded(30);
        reporter.written(12);
        reporter.verified_entry();
        reporter.publish();
        reporter.finish(Stage::Complete);
        let final_snapshot = observer.0.last().unwrap();
        assert_eq!(
            (
                final_snapshot.physical_bytes_read,
                final_snapshot.decoded_bytes,
                final_snapshot.selected_bytes_written
            ),
            (8, 30, 12)
        );
        assert!(final_snapshot.terminal && final_snapshot.verified);
    }
    #[test]
    fn coalescing_preserves_terminal_failure() {
        let mut observer = CoalescingObserver::default();
        let receiver = observer.clone();
        let mut reporter = Reporter::new(&mut observer, None);
        for _ in 0..100 {
            reporter.decoded(10);
            reporter.publish();
        }
        reporter.finish(Stage::Failed);
        let snapshot = receiver.take().unwrap();
        assert_eq!(snapshot.decoded_bytes, 1000);
        assert_eq!(snapshot.stage, Stage::Failed);
        assert!(!snapshot.verified);
        assert!(snapshot.terminal);
        assert!(receiver.take().is_none());
    }
    #[test]
    fn disabled_observer_never_receives_callback() {
        struct Disabled;
        impl Observer for Disabled {
            const ENABLED: bool = false;
            fn snapshot(&mut self, _: Snapshot) {
                panic!("disabled observer called");
            }
        }
        let mut observer = Disabled;
        let mut reporter = Reporter::new(&mut observer, None);
        reporter.read(1);
        reporter.decoded(1);
        reporter.written(1);
        reporter.verified_entry();
        reporter.publish();
        reporter.finish(Stage::Cancelled);
    }
}
