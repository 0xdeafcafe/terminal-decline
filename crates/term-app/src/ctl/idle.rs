//! The idle limit on `ctl` sessions (docs/design/agent-api.md §3.2): a
//! session nobody has made a request about for its timeout is killed.
//! The server arms one timer for the soonest deadline and re-arms it
//! when that passes or a session is added; it never polls.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A session's idle clock: when a request last named it, how long it may
/// go without one, and how many requests are in flight about it.
pub struct Idle {
    timeout: Option<Duration>,
    last: Mutex<Instant>,
    active: AtomicUsize,
}

impl Idle {
    /// `timeout` of `None` is never.
    pub fn new(timeout: Option<Duration>, now: Instant) -> Idle {
        Idle {
            timeout,
            last: Mutex::new(now),
            active: AtomicUsize::new(0),
        }
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    pub fn touch(&self) {
        *lock(&self.last) = Instant::now();
    }

    /// A request about the session starts: it is not idle until the
    /// matching [`Idle::end`], however long the request waits.
    pub fn begin(&self) {
        self.active.fetch_add(1, Ordering::AcqRel);
        self.touch();
    }

    pub fn end(&self) {
        self.touch();
        self.active.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn last(&self) -> Instant {
        *lock(&self.last)
    }

    /// When the session is due to be killed; `None`: never. `running`
    /// is whether its program still runs, `typed` whether the user has
    /// typed into its window.
    pub fn deadline(&self, running: bool, typed: bool) -> Option<Instant> {
        if self.active.load(Ordering::Acquire) > 0 {
            return None;
        }
        deadline(self.last(), self.timeout, running, typed)
    }

    /// The fields `list` and `stats` show.
    pub fn json(&self, now: Instant, running: bool, typed: bool) -> serde_json::Value {
        let ago = now.saturating_duration_since(self.last());
        let unix_ms = SystemTime::now()
            .checked_sub(ago)
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        serde_json::json!({
            "last_request_unix_ms": unix_ms,
            "idle_ms": ago.as_millis() as u64,
            "idle_timeout_ms": self.timeout.map(|t| t.as_millis() as u64),
            "expires_in_ms": self
                .deadline(running, typed)
                .map(|d| d.saturating_duration_since(now).as_millis() as u64),
            "user_typed": typed,
        })
    }
}

/// How long a new session may go without a request: what `spawn` asked
/// for (ms), else the config's minutes. 0 is never.
pub fn timeout(asked_ms: Option<u64>, config_minutes: u32) -> Option<Duration> {
    let ms = asked_ms.unwrap_or(u64::from(config_minutes) * 60_000);
    (ms > 0).then(|| Duration::from_millis(ms))
}

/// The moment a session is due: `timeout` after its last request, unless
/// it never times out, its program has exited (`kill` forgets it) or the
/// user has typed into its window (then it is theirs).
pub fn deadline(last: Instant, timeout: Option<Duration>, running: bool, typed: bool) -> Option<Instant> {
    if !running || typed {
        return None;
    }
    timeout.map(|t| last + t)
}

/// Of `entries` (a key and its deadline), the keys due at `now`, and the
/// soonest deadline among the rest.
pub fn split_due<K>(entries: impl IntoIterator<Item = (K, Option<Instant>)>, now: Instant) -> (Vec<K>, Option<Instant>) {
    let (mut due, mut next) = (Vec::new(), None::<Instant>);
    for (key, deadline) in entries {
        match deadline {
            Some(d) if d <= now => due.push(key),
            Some(d) => next = Some(next.map_or(d, |n| n.min(d))),
            None => {}
        }
    }
    (due, next)
}

/// The one-shot timer: sleeps until a deadline, or until re-armed.
#[derive(Default)]
pub struct Timer {
    generation: Mutex<u64>,
    changed: Condvar,
}

impl Timer {
    /// Read before working out the deadline to sleep to, and give to
    /// [`Timer::sleep`]: a re-arm in between is not missed.
    pub fn generation(&self) -> u64 {
        *lock(&self.generation)
    }

    /// The deadlines have changed (a session was added): wake the sleeper
    /// to work them out again.
    pub fn rearm(&self) {
        *lock(&self.generation) += 1;
        self.changed.notify_all();
    }

    /// Sleep until `until` (for ever with `None`) or a re-arm since
    /// `seen`. Returns whether the deadline passed.
    pub fn sleep(&self, seen: u64, until: Option<Instant>) -> bool {
        let mut g = lock(&self.generation);
        while *g == seen {
            match until {
                None => g = self.changed.wait(g).unwrap_or_else(PoisonError::into_inner),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return true;
                    }
                    g = self.changed.wait_timeout(g, d - now).unwrap_or_else(PoisonError::into_inner).0;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn a_deadline_is_the_timeout_after_the_last_request() {
        let t0 = Instant::now();
        assert_eq!(deadline(t0, Some(30 * MIN), true, false), Some(t0 + 30 * MIN));
        // Never, exited, or the user's own window.
        assert_eq!(deadline(t0, None, true, false), None);
        assert_eq!(deadline(t0, Some(MIN), false, false), None);
        assert_eq!(deadline(t0, Some(MIN), true, true), None);
    }

    #[test]
    fn the_timeout_is_asked_for_or_the_configs() {
        assert_eq!(timeout(None, 30), Some(30 * MIN));
        assert_eq!(timeout(None, 0), None);
        assert_eq!(timeout(Some(1000), 30), Some(Duration::from_secs(1)));
        assert_eq!(timeout(Some(0), 30), None);
    }

    #[test]
    fn requests_move_the_deadline_and_requests_in_flight_hold_it() {
        let idle = Idle::new(Some(MIN), Instant::now());
        let first = idle.deadline(true, false).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        idle.touch();
        assert!(idle.deadline(true, false).unwrap() > first);
        idle.begin();
        assert_eq!(idle.deadline(true, false), None);
        idle.end();
        assert!(idle.deadline(true, false).is_some());
        assert_eq!(Idle::new(None, Instant::now()).deadline(true, false), None);
    }

    #[test]
    fn the_due_are_split_from_the_soonest_of_the_rest() {
        let t0 = Instant::now();
        let at = |s| Some(t0 + Duration::from_secs(s));
        let (due, next) = split_due(
            [("a", at(5)), ("b", at(20)), ("c", None), ("d", at(10)), ("e", at(15))],
            t0 + Duration::from_secs(10),
        );
        assert_eq!(due, ["a", "d"]);
        assert_eq!(next, at(15));
        let (due, next) = split_due([("c", None::<Instant>)], t0);
        assert!(due.is_empty() && next.is_none());
    }

    #[test]
    fn the_timer_sleeps_to_its_deadline_and_wakes_early_when_rearmed() {
        let timer = Arc::new(Timer::default());
        // The deadline passes.
        let seen = timer.generation();
        let t0 = Instant::now();
        assert!(timer.sleep(seen, Some(t0 + Duration::from_millis(50))));
        assert!(t0.elapsed() >= Duration::from_millis(50));
        // A re-arm during a long sleep ends it early, as a session added
        // with an earlier deadline does.
        let seen = timer.generation();
        let t = timer.clone();
        let sleeper = std::thread::spawn(move || t.sleep(seen, Some(Instant::now() + Duration::from_secs(60))));
        std::thread::sleep(Duration::from_millis(50));
        timer.rearm();
        assert!(!sleeper.join().unwrap());
        // A re-arm before the sleep is not missed.
        let seen = timer.generation();
        timer.rearm();
        let t0 = Instant::now();
        assert!(!timer.sleep(seen, None));
        assert!(t0.elapsed() < Duration::from_secs(1));
    }
}
