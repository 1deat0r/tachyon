//! Bounded task-list polling (plan D3): the list pane refreshes through
//! `ListTasks` at a bounded interval **while it is visible** — never by
//! subscription, never faster than the bound.

use std::time::{Duration, Instant};

/// Rate gate for the `ListTasks` poll.
pub struct PollGate {
    /// Minimum time between two polls.
    interval: Duration,
    /// When the last poll was granted, if any.
    last: Option<Instant>,
}

impl PollGate {
    /// Gate granting one poll per `interval`.
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    /// True at most once per interval, and only while `visible`.
    ///
    /// Hiding the pane does not consume the budget: the first poll after
    /// it becomes visible again fires immediately, then the bound applies.
    pub fn should_poll(&mut self, visible: bool, now: Instant) -> bool {
        if !visible {
            return false;
        }
        if let Some(last) = self.last
            && now.duration_since(last) < self.interval
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::PollGate;

    const SECOND: Duration = Duration::from_secs(1);

    /// D3: the task-list pane refreshes by bounded `ListTasks` poll with
    /// a >= 1 s interval while the list is visible — immediately when it
    /// becomes visible, never twice inside the bound, never while hidden.
    #[test]
    fn poll_fires_immediately_when_becomes_visible_then_at_most_once_per_second() {
        let mut gate = PollGate::new(SECOND);
        let start = Instant::now();

        assert!(
            gate.should_poll(true, start),
            "the first visible poll fires immediately"
        );
        assert!(
            !gate.should_poll(true, start + Duration::from_millis(999)),
            "a second poll inside the bound is refused"
        );
        assert!(
            gate.should_poll(true, start + SECOND),
            "the bound elapsing re-arms the poll"
        );
        assert!(
            !gate.should_poll(false, start + Duration::from_secs(10)),
            "a hidden pane polls never"
        );
        assert!(
            gate.should_poll(true, start + Duration::from_secs(10)),
            "hiding never consumed the visible budget"
        );
    }
}
