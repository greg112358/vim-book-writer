use crate::buffer::Buffer;
use std::time::{Duration, Instant};

/// Decides when to save on its own: once the text has changed and then stayed still for `delay`.
/// Time is passed in, so tests can use a made-up clock.
#[derive(Debug)]
pub struct AutoSave {
    pub enabled: bool,
    delay: Duration,
    seen_revision: u64,
    last_edit: Instant,
    /// True if the last poll found unsaved changes and saving is on.
    armed: bool,
}

impl AutoSave {
    pub fn new(delay: Duration, now: Instant) -> Self {
        Self {
            enabled: true,
            delay,
            seen_revision: 0,
            last_edit: now,
            armed: false,
        }
    }

    /// Call after every key and on a timer. True means save now; the caller then saves and calls
    /// `buf.mark_saved()`. If the save fails and it doesn't, this asks again after another
    /// `delay` rather than on every call.
    pub fn poll(&mut self, now: Instant, buf: &Buffer) -> bool {
        if buf.revision() != self.seen_revision {
            self.seen_revision = buf.revision();
            self.last_edit = now;
        }
        self.armed = self.enabled && buf.is_modified();
        let due = self.armed && now.duration_since(self.last_edit) >= self.delay;
        if due {
            self.last_edit = now;
        }
        due
    }

    /// When the next poll could say yes, for the event loop to wake at. `None` when there is
    /// nothing to save or saving is off.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.armed.then(|| self.last_edit + self.delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELAY: Duration = Duration::from_millis(1000);

    fn ms(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn saves_once_after_edits_stop() {
        let t0 = Instant::now();
        let mut auto = AutoSave::new(DELAY, t0);
        let mut buf = Buffer::new("a");
        // Nothing changed, so there is never anything to save.
        assert!(!auto.poll(ms(t0, 60_000), &buf));
        assert_eq!(auto.next_deadline(), None);

        buf.insert("b");
        assert!(!auto.poll(ms(t0, 100_000), &buf));
        assert_eq!(auto.next_deadline(), Some(ms(t0, 101_000)));
        assert!(!auto.poll(ms(t0, 100_999), &buf));
        assert!(auto.poll(ms(t0, 101_000), &buf));
        buf.mark_saved();
        assert!(!auto.poll(ms(t0, 200_000), &buf));
        assert_eq!(auto.next_deadline(), None);
    }

    #[test]
    fn every_edit_restarts_the_wait() {
        let t0 = Instant::now();
        let mut auto = AutoSave::new(DELAY, t0);
        let mut buf = Buffer::default();
        for at in [0, 400, 800] {
            buf.insert("x");
            assert!(!auto.poll(ms(t0, at), &buf));
        }
        assert!(!auto.poll(ms(t0, 1799), &buf));
        assert!(auto.poll(ms(t0, 1800), &buf));
        // Undo is a change too.
        buf.mark_saved();
        buf.undo();
        assert!(!auto.poll(ms(t0, 2000), &buf));
        assert!(auto.poll(ms(t0, 3000), &buf));
    }

    #[test]
    fn off_means_no_saves_but_pending_changes_are_kept() {
        let t0 = Instant::now();
        let mut auto = AutoSave::new(DELAY, t0);
        auto.enabled = false;
        let mut buf = Buffer::default();
        buf.insert("x");
        assert!(!auto.poll(ms(t0, 0), &buf));
        assert!(!auto.poll(ms(t0, 10_000), &buf));
        assert_eq!(auto.next_deadline(), None);
        // Turned on, it saves the text that changed while it was off.
        auto.enabled = true;
        assert!(auto.poll(ms(t0, 10_001), &buf));
    }

    #[test]
    fn a_failed_save_is_retried_after_another_wait() {
        let t0 = Instant::now();
        let mut auto = AutoSave::new(DELAY, t0);
        let mut buf = Buffer::default();
        buf.insert("x");
        auto.poll(ms(t0, 0), &buf);
        assert!(auto.poll(ms(t0, 1000), &buf));
        // The caller didn't mark it saved.
        assert!(!auto.poll(ms(t0, 1500), &buf));
        assert_eq!(auto.next_deadline(), Some(ms(t0, 2000)));
        assert!(auto.poll(ms(t0, 2000), &buf));
    }
}
