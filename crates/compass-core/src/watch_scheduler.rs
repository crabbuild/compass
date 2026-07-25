use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MAX_ADAPTIVE_WINDOW: Duration = Duration::from_secs(5);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(crate) struct WatchScheduler {
    quiet_window: Duration,
    maximum_window: Duration,
    reconciliation_interval: Duration,
    pending: BTreeSet<PathBuf>,
    first_event: Option<Instant>,
    last_event: Option<Instant>,
    next_reconciliation: Instant,
}

impl WatchScheduler {
    pub(crate) fn new(
        now: Instant,
        quiet_window: Duration,
        reconciliation_interval: Duration,
    ) -> Self {
        Self {
            quiet_window,
            maximum_window: quiet_window.saturating_mul(5).min(MAX_ADAPTIVE_WINDOW),
            reconciliation_interval,
            pending: BTreeSet::new(),
            first_event: None,
            last_event: None,
            next_reconciliation: now + reconciliation_interval,
        }
    }

    pub(crate) fn record(&mut self, path: PathBuf, now: Instant) {
        self.pending.insert(path);
        self.first_event.get_or_insert(now);
        self.last_event = Some(now);
    }

    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn is_batch_ready(&self, now: Instant) -> bool {
        !self.pending.is_empty()
            && (self
                .last_event
                .is_some_and(|last| now.saturating_duration_since(last) >= self.quiet_window)
                || self.first_event.is_some_and(|first| {
                    now.saturating_duration_since(first) >= self.maximum_window
                }))
    }

    pub(crate) fn take_batch(&mut self) -> Vec<PathBuf> {
        self.first_event = None;
        self.last_event = None;
        std::mem::take(&mut self.pending).into_iter().collect()
    }

    #[cfg(test)]
    pub(crate) fn restore_batch(&mut self, paths: Vec<PathBuf>, now: Instant) {
        self.pending.extend(paths);
        self.first_event = Some(now);
        self.last_event = Some(now);
    }

    pub(crate) fn reconciliation_due(&self, now: Instant) -> bool {
        self.pending.is_empty() && now >= self.next_reconciliation
    }

    pub(crate) fn mark_build_succeeded(&mut self, now: Instant) {
        self.next_reconciliation = now + self.reconciliation_interval;
    }

    pub(crate) fn next_timeout(
        &self,
        now: Instant,
        poll_interval: Duration,
        retry_at: Option<Instant>,
    ) -> Duration {
        let mut timeout = poll_interval;
        if let Some(retry_at) = retry_at {
            timeout = timeout.min(duration_until(now, retry_at));
        } else {
            if let Some(last) = self.last_event {
                timeout = timeout.min(duration_until(now, last + self.quiet_window));
            }
            if let Some(first) = self.first_event {
                timeout = timeout.min(duration_until(now, first + self.maximum_window));
            }
            timeout = timeout.min(duration_until(now, self.next_reconciliation));
        }
        timeout
    }

    pub(crate) fn maximum_window(&self) -> Duration {
        self.maximum_window
    }
}

pub(crate) fn retry_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(5);
    Duration::from_secs(1_u64 << shift).min(MAX_RETRY_DELAY)
}

fn duration_until(now: Instant, deadline: Instant) -> Duration {
    deadline.saturating_duration_since(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_bursts_but_enforces_the_maximum_window() {
        let start = Instant::now();
        let mut scheduler =
            WatchScheduler::new(start, Duration::from_millis(150), Duration::from_secs(300));
        scheduler.record(PathBuf::from("src/a.rs"), start);
        scheduler.record(
            PathBuf::from("src/b.rs"),
            start + Duration::from_millis(140),
        );
        assert!(!scheduler.is_batch_ready(start + Duration::from_millis(200)));
        assert!(scheduler.is_batch_ready(start + Duration::from_millis(290)));

        scheduler.take_batch();
        for offset in [0, 140, 280, 420, 560, 700] {
            scheduler.record(
                PathBuf::from(format!("src/{offset}.rs")),
                start + Duration::from_millis(offset),
            );
        }
        assert!(scheduler.is_batch_ready(start + Duration::from_millis(750)));
    }

    #[test]
    fn deduplicates_paths_restores_failed_work_and_resets_reconciliation() {
        let start = Instant::now();
        let mut scheduler =
            WatchScheduler::new(start, Duration::from_millis(100), Duration::from_secs(10));
        scheduler.record(PathBuf::from("src/lib.rs"), start);
        scheduler.record(
            PathBuf::from("src/lib.rs"),
            start + Duration::from_millis(50),
        );
        assert_eq!(scheduler.pending_len(), 1);
        let batch = scheduler.take_batch();
        scheduler.restore_batch(batch, start + Duration::from_secs(1));
        assert_eq!(scheduler.pending_len(), 1);
        scheduler.take_batch();
        assert!(scheduler.reconciliation_due(start + Duration::from_secs(10)));
        scheduler.mark_build_succeeded(start + Duration::from_secs(10));
        assert!(!scheduler.reconciliation_due(start + Duration::from_secs(15)));
        assert!(scheduler.reconciliation_due(start + Duration::from_secs(20)));
    }

    #[test]
    fn retry_delay_is_bounded() {
        assert_eq!(retry_delay(1), Duration::from_secs(1));
        assert_eq!(retry_delay(2), Duration::from_secs(2));
        assert_eq!(retry_delay(5), Duration::from_secs(16));
        assert_eq!(retry_delay(6), Duration::from_secs(30));
        assert_eq!(retry_delay(100), Duration::from_secs(30));
    }

    #[test]
    fn custom_debounce_caps_the_maximum_window() {
        let scheduler = WatchScheduler::new(
            Instant::now(),
            Duration::from_secs(2),
            Duration::from_secs(300),
        );
        assert_eq!(scheduler.maximum_window(), Duration::from_secs(5));
    }
}
