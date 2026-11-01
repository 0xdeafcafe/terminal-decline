//! What a pane's shell is doing, from its OSC 133 marks (the snippets in
//! `shell/` send them): a command running since when, or one that just
//! finished, how long it took and how it ended. A long command that
//! finishes where you aren't looking is a notification
//! ([`should_notify`]); bursts of notifications from one window are
//! merged into one ([`Coalescer`]). With `title_status` on, the window
//! and tab title says what the focused pane does ([`status_title`]),
//! ticking only while a command runs ([`next_change`]).
//!
//! Everything here is plain data and decisions, tested without AppKit;
//! the view module feeds it the marks as they are parsed.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use term_core::Command;

/// A command the shell started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    /// What was typed, if the shell's marks showed it (its first line).
    pub command: Option<String>,
    /// The whole command line, if the shell sent it
    /// ([`Command::CommandLine`]: zsh and fish do), typed again after a
    /// restart.
    pub line: Option<String>,
    pub since: Instant,
    /// The program set the title (OSC 0/2) since it started: its title
    /// is shown as it is.
    pub titled: bool,
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
    /// The shell has sent an OSC 133 mark: what it does is known.
    pub marks: bool,
    pub running: Option<Running>,
    /// The last command that finished.
    pub finished: Option<Finished>,
    /// The command line sent with the next command's start.
    next_line: Option<String>,
}

impl Activity {
    /// Take in one mark at `now`. Returns the command that finished, if
    /// this mark ended one (a D without a C before it ends nothing).
    pub fn apply(&mut self, c: Command, now: Instant) -> Option<Finished> {
        self.marks = true;
        match c {
            Command::Prompt => {
                self.next_line = None;
                None
            }
            Command::CommandLine(line) => {
                self.next_line = Some(line);
                None
            }
            Command::Started(command) => {
                self.running = Some(Running {
                    command,
                    line: self.next_line.take(),
                    since: now,
                    titled: false,
                });
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

    /// The program set the title: while a command runs, its title wins.
    pub fn titled(&mut self) {
        if let Some(r) = &mut self.running {
            r.titled = true;
        }
    }
}

/// How long the title shows a long command's result before it says
/// "waiting" again.
pub const RESULT_SHOWN: Duration = Duration::from_secs(5);

/// What the title says pane's shell is doing, with `title_status` on:
///
/// - running: `cargo build · 12s — Terminal Decline`, or the program's own
///   title as it set it (a program that sets one, an editor or Claude
///   Code, says better what it does, and its clock would only tick for
///   nothing);
/// - just after a command of at least `after` finished, for
///   [`RESULT_SHOWN`]: `✓ cargo build · 1m 12s — Terminal Decline` or
///   `✗ cargo build · exit 101 — Terminal Decline`;
/// - at the prompt: `waiting · ~/src/project — Terminal Decline`.
///
/// `None` without marks: the shell doesn't say, so the title is the
/// program's, as without `title_status`.
pub fn status_title(a: &Activity, program: Option<&str>, folder: Option<&str>, after: Duration, now: Instant) -> Option<String> {
    let app = crate::PRODUCT_NAME;
    if !a.marks {
        return None;
    }
    if let Some(r) = &a.running {
        if r.titled
            && let Some(t) = program.filter(|t| !t.is_empty())
        {
            return Some(t.into());
        }
        let took = elapsed(now.saturating_duration_since(r.since));
        return Some(format!("{} · {took} — {app}", name(&r.command)));
    }
    if let Some(f) = a
        .finished
        .as_ref()
        .filter(|f| f.took >= after && now.saturating_duration_since(f.at) < RESULT_SHOWN)
    {
        let what = name(&f.command);
        return Some(match f.status {
            Some(s) if s != 0 => format!("✗ {what} · exit {s} — {app}"),
            _ => format!("✓ {what} · {} — {app}", elapsed(f.took)),
        });
    }
    Some(match folder {
        Some(f) => format!("waiting · {f} — {app}"),
        None => format!("waiting — {app}"),
    })
}

/// How long until [`status_title`] says something else by itself: the
/// next whole second of a running command's time under a minute, then
/// the next whole minute; the end of a result's [`RESULT_SHOWN`]. `None`
/// when it only changes with the next mark (at the prompt, or a program
/// that set its own title).
pub fn next_change(a: &Activity, after: Duration, now: Instant) -> Option<Duration> {
    if !a.marks {
        return None;
    }
    if let Some(r) = &a.running {
        if r.titled {
            return None;
        }
        let e = now.saturating_duration_since(r.since);
        let step = if e < Duration::from_secs(60) {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(60)
        };
        let into = Duration::from_nanos((e.as_nanos() % step.as_nanos()) as u64);
        return Some(step - into);
    }
    let f = a.finished.as_ref()?;
    let since = now.saturating_duration_since(f.at);
    (f.took >= after && since < RESULT_SHOWN).then(|| RESULT_SHOWN - since)
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
    fn the_shells_command_line_goes_with_its_command() {
        let t0 = Instant::now();
        let mut a = Activity::default();
        a.apply(Command::CommandLine("rush --resume".into()), t0);
        a.apply(Command::Started(Some("rush --resume".into())), t0);
        assert_eq!(a.running.as_ref().unwrap().line.as_deref(), Some("rush --resume"));
        a.apply(Command::Finished(Some(0)), t0);
        // A shell that sends none (bash): no line, and none left over.
        a.apply(Command::CommandLine("stale".into()), t0);
        a.apply(Command::Prompt, t0);
        a.apply(Command::Started(Some("ls".into())), t0);
        assert_eq!(a.running.as_ref().unwrap().line, None);
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
    fn the_title_says_what_the_shell_does() {
        let t0 = Instant::now();
        let after = secs(10);
        let mut a = Activity::default();
        assert_eq!(status_title(&a, Some("vim"), Some("~/td"), after, t0), None, "no marks");
        a.apply(Command::Prompt, t0);
        assert_eq!(
            status_title(&a, None, Some("~/td"), after, t0).unwrap(),
            "waiting · ~/td — Terminal Decline"
        );
        assert_eq!(status_title(&a, None, None, after, t0).unwrap(), "waiting — Terminal Decline");
        a.apply(Command::Started(Some("cargo build".into())), t0);
        let at = |s: u64| t0 + secs(s);
        assert_eq!(
            status_title(&a, None, None, after, at(12)).unwrap(),
            "cargo build · 12s — Terminal Decline"
        );
        assert_eq!(
            status_title(&a, None, None, after, at(72)).unwrap(),
            "cargo build · 1m 12s — Terminal Decline"
        );
        assert_eq!(
            status_title(&a, None, None, after, at(3780)).unwrap(),
            "cargo build · 1h 3m — Terminal Decline"
        );
        a.apply(Command::Finished(Some(0)), at(72));
        assert_eq!(
            status_title(&a, None, None, after, at(73)).unwrap(),
            "✓ cargo build · 1m 12s — Terminal Decline"
        );
        assert_eq!(
            status_title(&a, None, Some("~/td"), after, at(77)).unwrap(),
            "waiting · ~/td — Terminal Decline"
        );
        // A failure; a quick command shows no result.
        a.apply(Command::Started(Some("cargo test".into())), at(100));
        a.apply(Command::Finished(Some(101)), at(111));
        assert_eq!(
            status_title(&a, None, None, after, at(112)).unwrap(),
            "✗ cargo test · exit 101 — Terminal Decline"
        );
        a.apply(Command::Started(Some("ls".into())), at(200));
        a.apply(Command::Finished(Some(0)), at(201));
        assert_eq!(status_title(&a, None, None, after, at(201)).unwrap(), "waiting — Terminal Decline");
        // A program's own title wins while it runs.
        a.apply(Command::Started(Some("vim x".into())), at(300));
        assert_eq!(
            status_title(&a, Some("old"), None, after, at(301)).unwrap(),
            "vim x · 1s — Terminal Decline"
        );
        a.titled();
        assert_eq!(status_title(&a, Some("x - VIM"), None, after, at(301)).unwrap(), "x - VIM");
        // A title set at the prompt doesn't count for the next command.
        let mut b = Activity::default();
        b.titled();
        b.apply(Command::Started(None), t0);
        assert_eq!(
            status_title(&b, Some("zsh"), None, after, t0).unwrap(),
            "command · 0s — Terminal Decline"
        );
    }

    #[test]
    fn the_clock_ticks_at_the_next_change() {
        let t0 = Instant::now();
        let after = secs(10);
        let ms = Duration::from_millis;
        let mut a = Activity::default();
        assert_eq!(next_change(&a, after, t0), None);
        a.apply(Command::Prompt, t0);
        assert_eq!(next_change(&a, after, t0), None, "idle at the prompt");
        a.apply(Command::Started(Some("make".into())), t0);
        assert_eq!(next_change(&a, after, t0), Some(secs(1)));
        assert_eq!(next_change(&a, after, t0 + ms(12_300)), Some(ms(700)));
        assert_eq!(next_change(&a, after, t0 + ms(59_900)), Some(ms(100)));
        assert_eq!(next_change(&a, after, t0 + secs(60)), Some(secs(60)), "minutes from here");
        assert_eq!(next_change(&a, after, t0 + secs(72)), Some(secs(48)));
        assert_eq!(next_change(&a, after, t0 + secs(3790)), Some(secs(50)));
        // The result shows for RESULT_SHOWN, then nothing ticks.
        a.apply(Command::Finished(Some(0)), t0 + secs(72));
        assert_eq!(next_change(&a, after, t0 + secs(72)), Some(RESULT_SHOWN));
        assert_eq!(next_change(&a, after, t0 + secs(74)), Some(RESULT_SHOWN - secs(2)));
        assert_eq!(next_change(&a, after, t0 + secs(72) + RESULT_SHOWN), None);
        // A quick command shows no result to time.
        a.apply(Command::Started(None), t0 + secs(100));
        a.apply(Command::Finished(Some(0)), t0 + secs(101));
        assert_eq!(next_change(&a, after, t0 + secs(101)), None);
        // A program with its own title: nothing to tick.
        a.apply(Command::Started(None), t0 + secs(200));
        a.titled();
        assert_eq!(next_change(&a, after, t0 + secs(201)), None);
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
