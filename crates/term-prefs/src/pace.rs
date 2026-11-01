//! Holding back work that a held key would otherwise repeat on every
//! step: a font sample drawn only once the selection rests (debounce),
//! a theme previewed at most so often while it moves (throttle). Pure:
//! the caller passes the time, and asks again at [`Pace::deadline`].

use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Pace<T> {
    wait: Duration,
    /// A throttle: the first value goes at once.
    leading: bool,
    /// When a value last went.
    last: Option<Instant>,
    /// The value held back, and when it is due.
    held: Option<(T, Instant)>,
}

impl<T> Pace<T> {
    /// Each value waits until `wait` has passed with no newer one.
    pub const fn debounce(wait: Duration) -> Pace<T> {
        Pace {
            wait,
            leading: false,
            last: None,
            held: None,
        }
    }

    /// A value goes at once if none went in the last `wait`; otherwise
    /// the latest goes when `wait` is up.
    pub const fn throttle(wait: Duration) -> Pace<T> {
        Pace {
            wait,
            leading: true,
            last: None,
            held: None,
        }
    }

    /// A new value at `now`: returned if it is to be acted on now, else
    /// held back (replacing any held before).
    pub fn offer(&mut self, value: T, now: Instant) -> Option<T> {
        if self.leading && self.held.is_none() && self.last.is_none_or(|t| now.duration_since(t) >= self.wait) {
            self.last = Some(now);
            return Some(value);
        }
        let due = match (self.leading, self.last) {
            (true, Some(t)) => t + self.wait,
            _ => now + self.wait,
        };
        self.held = Some((value, due));
        None
    }

    /// When the held value is due, if one is held.
    pub fn deadline(&self) -> Option<Instant> {
        self.held.as_ref().map(|(_, due)| *due)
    }

    /// The held value, if it is due at `now`.
    pub fn poll(&mut self, now: Instant) -> Option<T> {
        if self.deadline().is_some_and(|due| now >= due) {
            self.last = Some(now);
            return self.held.take().map(|(v, _)| v);
        }
        None
    }

    /// Whether a value is held back.
    pub fn is_held(&self) -> bool {
        self.held.is_some()
    }

    /// Drops the held value.
    pub fn cancel(&mut self) {
        self.held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_debounce_waits_for_the_selection_to_rest() {
        let t0 = Instant::now();
        let mut p = Pace::debounce(200 * MS);
        // Moving quickly: each step pushes the deadline back.
        assert_eq!(p.offer("Menlo", t0), None);
        assert_eq!(p.offer("Monaco", t0 + 50 * MS), None);
        assert_eq!(p.offer("SF Mono", t0 + 100 * MS), None);
        assert_eq!(p.deadline(), Some(t0 + 300 * MS));
        assert_eq!(p.poll(t0 + 299 * MS), None);
        assert_eq!(p.poll(t0 + 300 * MS), Some("SF Mono"), "only the last, once");
        assert_eq!(p.poll(t0 + 400 * MS), None);
        assert_eq!(p.deadline(), None);
        // A single step also waits.
        assert_eq!(p.offer("Menlo", t0 + 1000 * MS), None);
        assert_eq!(p.poll(t0 + 1200 * MS), Some("Menlo"));
        p.offer("Monaco", t0 + 1300 * MS);
        p.cancel();
        assert!(!p.is_held() && p.poll(t0 + 2000 * MS).is_none());
    }

    #[test]
    fn a_throttle_goes_at_once_then_at_most_every_wait() {
        let t0 = Instant::now();
        let mut p = Pace::throttle(100 * MS);
        assert_eq!(p.offer(1, t0), Some(1), "the first at once");
        // Key repeat: held, the latest kept, due 100 ms after the first.
        assert_eq!(p.offer(2, t0 + 30 * MS), None);
        assert_eq!(p.offer(3, t0 + 60 * MS), None);
        assert_eq!(p.deadline(), Some(t0 + 100 * MS));
        assert_eq!(p.poll(t0 + 100 * MS), Some(3));
        // Still repeating: the next is held from that one.
        assert_eq!(p.offer(4, t0 + 120 * MS), None);
        assert_eq!(p.deadline(), Some(t0 + 200 * MS));
        assert_eq!(p.poll(t0 + 200 * MS), Some(4));
        // After a rest, at once again.
        assert_eq!(p.offer(5, t0 + 1000 * MS), Some(5));
    }
}
