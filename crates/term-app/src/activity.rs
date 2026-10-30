//! What a pane's shell is doing, from its OSC 133 marks (the snippets in
//! `shell/` send them): a command running since when, or one that just
//! finished, how long it took and how it ended. A long command that
//! finishes where you aren't looking is a notification
//! ([`should_notify`]); bursts of notifications from one window are
//! merged into one ([`Coalescer`]).
//!
//! Everything here is plain data and decisions, tested without AppKit;
//! the view module feeds it the marks as they are parsed.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use term_core::Command;

/// A command the shell started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    /// What was typed, if the shell's marks showed it.
    pub command: Option<String>,
    pub since: Instant,
}

/// A command that finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    pub command: Option<String>,
    /// The exit status, if the shell gave one.
    pub status: Option<u32>,
    pub took: Duration,
    pub at: Instant,
}

impl Finished {
    pub fn ok(&self) -> bool {
        self.status.is_none_or(|s| s == 0)
    }
}

/// One pane's shell, as its marks tell it.
#[derive(Clone, Debug, Default)]
pub struct Activity {
    /// The shell has sent a C or D mark: what it does is known.
    pub marks: bool,
    pub running: Option<Running>,
    /// The last command that finished.
    pub finished: Option<Finished>,
}

impl Activity {
    /// Take in one mark at `now`. Returns the command that finished, if
    /// this mark ended one (a D without a C before it ends nothing).
    pub fn apply(&mut self, c: Command, now: Instant) -> Option<Finished> {
        self.marks = true;
        match c {
            Command::Started(command) => {
                self.running = Some(Running { command, since: now });
                None
            }
            Command::Finished(status) => {
                let r = self.running.take()?;
                let f = Finished {
                    command: r.command,
                    status,
                    took: now.saturating_duration_since(r.since),
                    at: now,
                };
                self.finished = Some(f.clone());
                Some(f)
            }
        }
    }
}

/// A duration as the title and notifications show it: `12s` under a
/// minute, `1m 12s` under an hour, then `1h 3m`.
pub fn elapsed(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

/// The command's name for a title or notification: what was typed, or
/// "command" when the marks didn't show it.
fn name(command: &Option<String>) -> &str {
    command.as_deref().unwrap_or("command")
}

/// A finished command's headline: `✓ cargo build — 1m 12s`, or
/// `✗ cargo build — exit 101`.
pub fn finish_title(f: &Finished) -> String {
    let what = name(&f.command);
    match f.status {
        Some(s) if s != 0 => format!("✗ {what} — exit {s}"),
        _ => format!("✓ {what} — {}", elapsed(f.took)),
    }
}

/// A finished command's notification text: the folder it ran in, and
/// for a failure how long it took.
pub fn finish_body(f: &Finished, folder: Option<&str>) -> String {
    let folder = folder.unwrap_or("");
    match (f.ok(), folder.is_empty()) {
        (true, _) => folder.to_string(),
        (false, true) => format!("after {}", elapsed(f.took)),
        (false, false) => format!("{folder} · after {}", elapsed(f.took)),
    }
}

/// Where you are looking, for a pane: the app in front, its window (tab)
/// key, and the pane focused in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Looking {
    pub app_active: bool,
    pub window_key: bool,
    pub pane_focused: bool,
}

impl Looking {
    /// You are looking at this pane: anything it says needs no
    /// notification.
    pub fn at_pane(self) -> bool {
        self.app_active && self.window_key && self.pane_focused
    }

    /// You are looking at its window: a notification for it isn't
    /// counted on the Dock icon.
    pub fn at_window(self) -> bool {
        self.app_active && self.window_key
    }
}

/// Whether `f` is a notification: `notify_on_finish` is on, it ran at
/// least `after`, and you aren't looking at its pane.
pub fn should_notify(f: &Finished, on: bool, after: Duration, looking: Looking) -> bool {
    on && f.took >= after && !looking.at_pane()
}

/// What a notification is about, for merging a burst.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The bell rang.
    Bell,
    /// A program asked for one (OSC 9 / OSC 777).
    Message,
    /// A long command finished.
    Finished,
}

/// One notification to show, or to show in place of one shown before
/// (same `id`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    /// Notifications with the same id replace each other.
    pub id: u64,
    pub title: String,
    pub body: String,
    /// It replaces one already shown (not counted again on the Dock).
    pub replaces: bool,
}

/// A burst of notifications from one window.
struct Burst {
    id: u64,
    started: Instant,
    last: Instant,
    /// What it holds, oldest first: kind, pane, title and body.
    items: Vec<(Kind, usize, String, String)>,
}

/// A burst goes on while notifications come at most this far apart…
pub const BURST_GAP: Duration = Duration::from_secs(10);
/// …and for at most this long; then the next one is shown on its own.
pub const BURST_MAX: Duration = Duration::from_secs(60);

/// Merges bursts of notifications per window, so a window doesn't spam:
/// the first is shown, later ones in the burst replace it with a summary
/// (the newest, and how many more). A bell joining a burst is never shown
/// again on its own, and a program's message after a bell from the same
/// pane (Claude Code rings and then says it is waiting) replaces it
/// rather than adding to it. No timers: a burst ends when the next
/// notification comes too late for it.
#[derive(Default)]
pub struct Coalescer {
    bursts: HashMap<usize, Burst>,
    next_id: u64,
}

impl Coalescer {
    /// A notification from pane `pane` of window `window` at `now`: what
    /// to show, or `None` to show nothing new (a bell joining a burst).
    pub fn add(&mut self, window: usize, pane: usize, kind: Kind, title: &str, body: &str, now: Instant) -> Option<Post> {
        let item = (kind, pane, title.to_string(), body.to_string());
        let going = self
            .bursts
            .get(&window)
            .is_some_and(|b| now.saturating_duration_since(b.last) <= BURST_GAP && now.saturating_duration_since(b.started) <= BURST_MAX);
        if !going {
            self.next_id += 1;
            let id = self.next_id;
            self.bursts.insert(
                window,
                Burst {
                    id,
                    started: now,
                    last: now,
                    items: vec![item],
                },
            );
            return Some(Post {
                id,
                title: title.into(),
                body: body.into(),
                replaces: false,
            });
        }
        let b = self.bursts.get_mut(&window).expect("going");
        b.last = now;
        if kind == Kind::Bell {
            b.items.push(item);
            return None;
        }
        // A message after the same pane's bell takes the bell's place.
        if kind == Kind::Message {
            b.items.retain(|(k, p, ..)| !(*k == Kind::Bell && *p == pane));
        }
        b.items.push(item);
        let counted: Vec<_> = b.items.iter().filter(|(k, ..)| *k != Kind::Bell).collect();
        let (_, _, title, body) = counted.last().copied().expect("one just added");
        let more = counted.len() - 1;
        let body = match (more, body.is_empty()) {
            (0, _) => body.clone(),
            (n, true) => format!("and {n} more"),
            (n, false) => format!("{body} · and {n} more"),
        };
        Some(Post {
            id: b.id,
            title: title.clone(),
            body,
            replaces: true,
        })
    }

    /// Window `window` closed.
    pub fn forget(&mut self, window: usize) {
        self.bursts.remove(&window);
    }
}

/// `path` with the home folder as `~`.
pub fn tilde(path: &str, home: Option<&str>) -> String {
    match home.filter(|h| !h.is_empty() && *h != "/") {
        Some(h) if path == h => "~".into(),
        Some(h) => match path.strip_prefix(h).filter(|rest| rest.starts_with('/')) {
            Some(rest) => format!("~{rest}"),
            None => path.into(),
        },
        None => path.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn elapsed_reads_like_a_clock() {
        for (s, want) in [
            (0, "0s"),
            (12, "12s"),
            (59, "59s"),
            (60, "1m 0s"),
            (72, "1m 12s"),
            (3599, "59m 59s"),
            (3600, "1h 0m"),
            (3780, "1h 3m"),
            (90061, "25h 1m"),
        ] {
            assert_eq!(elapsed(secs(s)), want, "{s}");
        }
    }

    #[test]
    fn marks_make_commands() {
        let t0 = Instant::now();
        let mut a = Activity::default();
        assert!(!a.marks);
        // A D with nothing running (the first prompt) ends nothing.
        assert_eq!(a.apply(Command::Finished(Some(0)), t0), None);
        assert!(a.marks);
        assert_eq!(a.apply(Command::Started(Some("cargo build".into())), t0), None);
        assert_eq!(a.running.as_ref().unwrap().command.as_deref(), Some("cargo build"));
        let f = a.apply(Command::Finished(Some(101)), t0 + secs(72)).unwrap();
        assert_eq!(f.took, secs(72));
        assert!(!f.ok());
        assert!(a.running.is_none());
        assert_eq!(a.finished, Some(f.clone()));
        assert_eq!(finish_title(&f), "✗ cargo build — exit 101");
        assert_eq!(finish_body(&f, Some("~/src/td")), "~/src/td · after 1m 12s");
        let ok = Finished {
            status: Some(0),
            ..f.clone()
        };
        assert_eq!(finish_title(&ok), "✓ cargo build — 1m 12s");
        assert_eq!(finish_body(&ok, Some("~/src/td")), "~/src/td");
        let unknown = Finished {
            command: None,
            status: None,
            ..f
        };
        assert_eq!(finish_title(&unknown), "✓ command — 1m 12s");
        assert_eq!(finish_body(&unknown, None), "");
    }

    #[test]
    fn when_a_finished_command_notifies() {
        let f = Finished {
            command: Some("make".into()),
            status: Some(0),
            took: secs(12),
            at: Instant::now(),
        };
        let away = Looking::default();
        let here = Looking {
            app_active: true,
            window_key: true,
            pane_focused: true,
        };
        let after = secs(10);
        assert!(should_notify(&f, true, after, away));
        assert!(!should_notify(&f, false, after, away), "turned off");
        assert!(!should_notify(&f, true, secs(13), away), "too short");
        assert!(should_notify(&f, true, secs(12), away), "exactly notify_after");
        assert!(!should_notify(&f, true, after, here), "you're looking at it");
        // Each way of not looking: another app, another tab, another pane.
        for looking in [
            Looking { app_active: false, ..here },
            Looking { window_key: false, ..here },
            Looking {
                pane_focused: false,
                ..here
            },
        ] {
            assert!(should_notify(&f, true, after, looking), "{looking:?}");
        }
        assert!(
            Looking {
                pane_focused: false,
                ..here
            }
            .at_window()
        );
        assert!(!Looking { window_key: false, ..here }.at_window());
    }

    #[test]
    fn bursts_merge_per_window() {
        let t0 = Instant::now();
        let mut c = Coalescer::default();
        let first = c.add(1, 10, Kind::Finished, "✓ make — 12s", "~/a", t0).unwrap();
        assert!(!first.replaces);
        assert_eq!(first.body, "~/a");
        // Another window: a notification of its own.
        let other = c.add(2, 20, Kind::Finished, "✓ ls — 11s", "~/b", t0).unwrap();
        assert_ne!(other.id, first.id);
        // The same window, soon after: the first replaced by a summary.
        let second = c.add(1, 11, Kind::Finished, "✗ test — exit 1", "~/c", t0 + secs(5)).unwrap();
        assert_eq!(second.id, first.id);
        assert!(second.replaces);
        assert_eq!(second.title, "✗ test — exit 1");
        assert_eq!(second.body, "~/c · and 1 more");
        // A bell joining it shows nothing new.
        assert_eq!(c.add(1, 11, Kind::Bell, "Bell", "", t0 + secs(6)), None);
        // Too long a gap: a new burst.
        let later = c
            .add(1, 10, Kind::Finished, "✓ make — 12s", "~/a", t0 + secs(6) + BURST_GAP + secs(1))
            .unwrap();
        assert_ne!(later.id, first.id);
        assert!(!later.replaces);
        // A burst ends after BURST_MAX even with small gaps.
        let mut c = Coalescer::default();
        let mut t = t0;
        let id = c.add(1, 1, Kind::Message, "m", "", t).unwrap().id;
        while t < t0 + BURST_MAX {
            t += secs(9);
            let p = c.add(1, 1, Kind::Message, "m", "", t).unwrap();
            if t <= t0 + BURST_MAX {
                assert_eq!(p.id, id);
            } else {
                assert_ne!(p.id, id);
            }
        }
        c.forget(1);
        assert!(!c.add(1, 1, Kind::Message, "m", "", t).unwrap().replaces);
    }

    #[test]
    fn a_bell_then_a_message_is_one_notification() {
        // Claude Code rings, then says it is waiting.
        let t0 = Instant::now();
        let mut c = Coalescer::default();
        let bell = c.add(1, 5, Kind::Bell, "claude", "Bell", t0).unwrap();
        let msg = c
            .add(1, 5, Kind::Message, "Claude Code", "waiting for your input", t0 + secs(1))
            .unwrap();
        assert_eq!(msg.id, bell.id);
        assert_eq!(msg.body, "waiting for your input", "the bell isn't counted");
        // A bell from another pane in the burst isn't counted either.
        let mut c = Coalescer::default();
        c.add(1, 5, Kind::Message, "a", "x", t0).unwrap();
        assert_eq!(c.add(1, 6, Kind::Bell, "b", "Bell", t0 + secs(1)), None);
        assert_eq!(c.add(1, 5, Kind::Message, "a", "y", t0 + secs(2)).unwrap().body, "y · and 1 more");
    }

    #[test]
    fn home_is_a_tilde() {
        let home = Some("/Users/lw");
        assert_eq!(tilde("/Users/lw", home), "~");
        assert_eq!(tilde("/Users/lw/src/td", home), "~/src/td");
        assert_eq!(tilde("/Users/lwx/src", home), "/Users/lwx/src");
        assert_eq!(tilde("/tmp", home), "/tmp");
        assert_eq!(tilde("/tmp", None), "/tmp");
        assert_eq!(tilde("/tmp", Some("/")), "/tmp");
    }
}
