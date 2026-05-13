use std::time::{Duration, Instant};

pub struct Debouncer {
    delay: Duration,
    pending_since: Option<Instant>,
}

impl Debouncer {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            pending_since: None,
        }
    }

    pub fn mark_changed(&mut self, now: Instant) {
        self.pending_since = Some(now);
    }

    pub fn should_run(&mut self, now: Instant) -> bool {
        let Some(pending_since) = self.pending_since else {
            return false;
        };

        if now.duration_since(pending_since) < self.delay {
            return false;
        }

        self.pending_since = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_delay_before_running() {
        let start = Instant::now();
        let mut debouncer = Debouncer::new(Duration::from_millis(250));

        debouncer.mark_changed(start);

        assert!(!debouncer.should_run(start + Duration::from_millis(249)));
        assert!(debouncer.should_run(start + Duration::from_millis(250)));
        assert!(!debouncer.should_run(start + Duration::from_millis(500)));
    }

    #[test]
    fn later_change_resets_delay() {
        let start = Instant::now();
        let mut debouncer = Debouncer::new(Duration::from_millis(250));

        debouncer.mark_changed(start);
        debouncer.mark_changed(start + Duration::from_millis(100));

        assert!(!debouncer.should_run(start + Duration::from_millis(300)));
        assert!(debouncer.should_run(start + Duration::from_millis(350)));
    }
}
