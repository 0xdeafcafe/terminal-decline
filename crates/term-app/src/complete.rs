//! Completion and colouring at the shell's prompt
//! (docs/design/completion.md): [`lex`] reads the command line into
//! words and says what the cursor is on; [`ink`] colours it;
//! [`sources`] reads what can be offered, off the main thread; [`list`]
//! picks the choices; [`PaneState`] opens the list after a pause, places
//! it, and handles its keys.

use std::sync::Arc;
use std::time::{Duration, Instant};

use term_core::{Ghost, Keymap, Popup, PopupItem, Terminal};
use unicode_width::UnicodeWidthStr;

pub mod ink;
pub mod lex;
pub mod list;
pub mod sources;

use list::{Listings, Offer, What};

/// The settings the list follows (docs/design/completion.md §10).
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub delay: Duration,
    /// Choices shown before the list scrolls.
    pub rows: usize,
    pub ghost: bool,
    pub files: bool,
    pub commands: bool,
}

/// What a pane keeps for completion and colouring.
#[derive(Default)]
pub struct PaneState {
    /// What its shell can run, as last reported (OSC 7701 `env`).
    pub shell_env: Option<Arc<term_core::ShellEnv>>,
    /// The line and cursor last seen, and since when: the list opens once
    /// they have stayed the same for the delay.
    seen: Option<(String, usize)>,
    since: Option<Instant>,
    /// The pause a wake-up was asked for, so one is asked per pause.
    timer_for: Option<Instant>,
    /// The word the list was closed on with Esc (its start and text): it
    /// stays closed until the word changes.
    closed: Option<(usize, String)>,
    open: Option<Open>,
}

/// The list while it is open.
#[derive(Debug)]
struct Open {
    offer: Offer,
    selected: usize,
    /// The first choice shown.
    scroll: usize,
    /// The word's start in the line (bytes), and how much of it to keep
    /// as typed when a choice is taken (bytes from its start).
    start: usize,
    keep_bytes: usize,
    /// The grey text after the cursor was shown.
    ghost: bool,
    /// Esc also goes to the shell (vi insert mode).
    vi: bool,
}

/// What a key did to the list.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyResult {
    /// The list isn't open, or the key isn't one of its own: to the shell.
    Pass,
    /// Handled; draw again.
    Redraw,
    /// Type these bytes into the shell (a choice taken), and draw again.
    Send(Vec<u8>),
    /// Closed; the key goes on to the shell too.
    CloseAndPass,
}

/// macOS key codes of the list's keys.
pub mod keycode {
    pub const TAB: u16 = 48;
    pub const ESCAPE: u16 = 53;
    pub const RIGHT: u16 = 124;
    pub const DOWN: u16 = 125;
    pub const UP: u16 = 126;
}

/// What [`PaneState::update`] has for the frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    pub popup: Option<Popup>,
    pub ghost: Option<Ghost>,
    /// Draw again after this long: when the pause will have lasted.
    pub wake_after: Option<Duration>,
}

impl PaneState {
    /// Whether the list is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Close the list (the setting was turned off, the pane lost focus).
    pub fn close(&mut self) {
        self.open = None;
    }

    /// Bring the list up to date with the line the shell reported, for
    /// the frame: open it after the pause, filter it while it is open,
    /// close it when there is nothing to offer. `wake` is called when
    /// something read on the worker is ready.
    pub fn update(&mut self, t: &Terminal, s: &Settings, now: Instant, wake: &dyn Fn() -> sources::Wake) -> Drawn {
        let Some(report) = t
            .line_report()
            .filter(|r| !r.too_long && matches!(r.keymap, Keymap::Insert | Keymap::ViInsert))
        else {
            self.open = None;
            self.seen = None;
            return Drawn::default();
        };
        let key = (report.text.clone(), report.cursor);
        if self.seen.as_ref() != Some(&key) {
            self.seen = Some(key);
            self.since = Some(now);
        }
        let since = self.since.unwrap_or(now);
        if self.open.is_none() {
            let waited = now.saturating_duration_since(since);
            if waited < s.delay {
                let mut drawn = Drawn::default();
                if self.timer_for != Some(since) {
                    self.timer_for = Some(since);
                    drawn.wake_after = Some(s.delay - waited);
                }
                return drawn;
            }
        }
        let line = lex::read(&report.text, report.cursor);
        let word = line
            .words
            .iter()
            .find(|w| w.range.start <= report.cursor && report.cursor <= w.range.end);
        let start = word.map_or(report.cursor, |w| w.range.start);
        let typed = &report.text[start..report.cursor];
        if self.closed.as_ref().is_some_and(|(at, text)| *at == start && text == typed) {
            self.open = None;
            return Drawn::default();
        }
        self.closed = None;
        let Some(offer) = self.offer(&line.at, t, s, wake) else {
            self.open = None;
            return Drawn::default();
        };
        // Keep the selected choice when it is still offered.
        let previous = self.open.as_ref().filter(|o| o.start == start);
        let selected = previous
            .and_then(|o| o.offer.choices.get(o.selected))
            .and_then(|c| offer.choices.iter().position(|x| x.name == c.name))
            .unwrap_or(0);
        let scroll = previous.map_or(0, |o| o.scroll);
        // What is typed before the matched part is kept as typed (`~/`,
        // `src/`) when it reads the same unquoted; else the whole word is
        // replaced, quoted by the shell.
        let keep_bytes = if typed.starts_with(&offer.keep) { offer.keep.len() } else { 0 };
        let mut open = Open {
            offer,
            selected,
            scroll,
            start,
            keep_bytes,
            ghost: false,
            vi: report.keymap == Keymap::ViInsert,
        };
        let drawn = place(t, &mut open, s, report.cursor == report.text.len());
        self.open = Some(open);
        drawn
    }

    /// The choices for what the cursor is on, if any are worth a list.
    fn offer(&self, at: &lex::At, t: &Terminal, s: &Settings, wake: &dyn Fn() -> sources::Wake) -> Option<Offer> {
        use lex::At;
        let prefix = match at {
            At::Command { prefix } | At::Argument { prefix, .. } | At::Path { prefix, .. } => prefix,
            _ => return None,
        };
        // Not on an empty word: a list at every prompt and after every
        // space would be in the way.
        if prefix.is_empty() {
            return None;
        }
        let env = self.shell_env.as_deref();
        let is_command = matches!(at, At::Command { prefix } if !prefix.contains('/'));
        let commands = match (is_command && s.commands, env) {
            (true, Some(env)) => sources::commands(&env.path, wake),
            _ => None,
        };
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let folder = match (!is_command && s.files)
            .then(|| list::folder_of(prefix, t.cwd(), home.as_deref()))
            .flatten()
        {
            Some((dir, _)) => sources::folder(&dir, wake),
            None => None,
        };
        let offer = list::offer(
            at,
            &Listings {
                env,
                commands: commands.as_deref(),
                folder: folder.as_deref(),
            },
        )?;
        // Nothing to add: no choices, or only what is typed already.
        match offer.choices.as_slice() {
            [] => None,
            [only] if only.name == offer.prefix => None,
            _ => Some(offer),
        }
    }

    /// A key while the list may be open; `plain` when no modifier is held.
    pub fn key(&mut self, keycode: u16, plain: bool) -> KeyResult {
        let Some(open) = &mut self.open else { return KeyResult::Pass };
        if !plain {
            return KeyResult::Pass;
        }
        let n = open.offer.choices.len();
        match keycode {
            keycode::DOWN => {
                open.selected = (open.selected + 1) % n;
                KeyResult::Redraw
            }
            keycode::UP => {
                open.selected = (open.selected + n - 1) % n;
                KeyResult::Redraw
            }
            keycode::TAB => self.take(),
            keycode::RIGHT if open.ghost => self.take(),
            keycode::ESCAPE => {
                let (vi, start) = (open.vi, open.start);
                self.closed = self
                    .seen
                    .as_ref()
                    .map(|(text, cursor)| (start, text[start.min(*cursor)..*cursor].to_string()));
                self.open = None;
                if vi { KeyResult::CloseAndPass } else { KeyResult::Redraw }
            }
            _ => KeyResult::Pass,
        }
    }

    /// Take the selected choice: the bytes that have zsh put it in place
    /// of the word (`ESC [ 7701 ~`, then `start;keep;name;suffix` and
    /// BEL, read by decline.zsh's widget, which quotes the name itself).
    fn take(&mut self) -> KeyResult {
        let Some(open) = self.open.take() else { return KeyResult::Pass };
        let Some(c) = open.offer.choices.get(open.selected) else {
            return KeyResult::Pass;
        };
        let name = if open.keep_bytes == 0 {
            format!("{}{}", open.offer.keep, c.name)
        } else {
            c.name.clone()
        };
        let suffix = if c.what == What::Folder { "" } else { " " };
        let bytes = format!("\x1b[7701~{};{};{};{suffix}\x07", open.start, open.keep_bytes, encode(&name));
        // The word changes: what is offered next is for the new one.
        self.seen = None;
        KeyResult::Send(bytes.into_bytes())
    }
}

/// `%`, `;`, BEL and other control characters percent-encoded.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '%' || c == ';' || c.is_ascii_control() {
            out.push_str(&format!("%{:02X}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// Place the list for `open` under (or over) the cursor, with its text
/// under the word's start, scrolled to show the selected choice; and the
/// grey text after the cursor when it is at the line's end and nothing
/// is drawn after it.
fn place(t: &Terminal, open: &mut Open, s: &Settings, at_end: bool) -> Drawn {
    let Some((crow, ccol)) = t.cursor_in_view() else {
        return Drawn::default();
    };
    let (lines, cols) = (t.lines(), t.cols());
    let offer = &open.offer;
    let n = offer.choices.len();
    let below = lines.saturating_sub(crow + 1);
    let above = crow;
    let want = n.min(s.rows.max(1)) + 2;
    let (rows, row) = if below >= want || (above < want && below >= above) {
        (want.min(below), crow + 1)
    } else {
        let h = want.min(above);
        (h, crow - h)
    };
    if rows < 3 {
        return Drawn::default();
    }
    let shown = rows - 2;
    if open.selected < open.scroll {
        open.scroll = open.selected;
    } else if open.selected >= open.scroll + shown {
        open.scroll = open.selected + 1 - shown;
    }
    open.scroll = open.scroll.min(n.saturating_sub(shown));
    let items: Vec<PopupItem> = offer.choices[open.scroll..(open.scroll + shown).min(n)]
        .iter()
        .enumerate()
        .map(|(i, c)| PopupItem {
            text: c.name.clone(),
            detail: c.what.label().to_string(),
            selected: open.scroll + i == open.selected,
        })
        .collect();
    let widest = items.iter().map(|i| i.text.width() + 1 + i.detail.width()).max().unwrap_or(0);
    // `│▸` text … detail ` │`.
    let width = (widest + 5).clamp(16.min(cols), cols.max(1));
    // The text starts under the word being completed.
    let word_col = ccol.saturating_sub(offer.prefix.width());
    let col = word_col.saturating_sub(2).min(cols.saturating_sub(width));
    let popup = Popup {
        row: row as u16,
        col: col as u16,
        width: width as u16,
        items,
    };
    let selected = &offer.choices[open.selected];
    let ghost = (s.ghost && at_end)
        .then(|| selected.name.strip_prefix(offer.prefix.as_str()))
        .flatten()
        .filter(|rest| !rest.is_empty() && blank_after(t, crow, ccol))
        .map(|rest| Ghost {
            row: crow as u16,
            col: ccol as u16,
            text: rest.to_string(),
        });
    open.ghost = ghost.is_some();
    Drawn {
        popup: Some(popup),
        ghost,
        wake_after: None,
    }
}

/// Whether the cells from `col` to the end of view row `row` are blank (a
/// right prompt or another tool's suggestion would be there).
fn blank_after(t: &Terminal, row: usize, col: usize) -> bool {
    let r = t.visible_row(row);
    (col..t.cols()).all(|c| r.cell(c).is_blank())
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::{ImagePool, ShellEnv, Term};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn settings() -> Settings {
        Settings {
            delay: Duration::from_millis(150),
            rows: 8,
            ghost: true,
            files: true,
            commands: true,
        }
    }

    /// A 40×10 terminal at a prompt with `line` typed and reported.
    fn term(line: &str) -> Term {
        let mut t = Term::new(40, 10, 100, ImagePool::process());
        t.state.set_report_token(TOKEN.as_bytes().try_into().ok());
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        t.advance(line.as_bytes());
        t.advance(format!("\x1b]7701;line;{TOKEN};main;{};{line}\x07", line.len()).as_bytes());
        t
    }

    fn no_wake() -> sources::Wake {
        Box::new(|| {})
    }

    /// A pane state whose shell runs what is in `path`.
    fn state(path: &str) -> PaneState {
        PaneState {
            shell_env: Some(Arc::new(ShellEnv {
                path: path.into(),
                builtins: vec!["cd".into()],
                ..ShellEnv::default()
            })),
            ..PaneState::default()
        }
    }

    fn programs(name: &str, names: &[&str]) -> String {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("decline-complete-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in names {
            let p = dir.join(n);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir.canonicalize().unwrap().display().to_string()
    }

    /// Update past the pause until the worker has read what is asked and
    /// the list opens (or give up).
    fn settle(st: &mut PaneState, t: &Term) -> Drawn {
        let s = settings();
        let t0 = Instant::now();
        let first = st.update(&t.state, &s, t0, &no_wake);
        assert_eq!(first.popup, None, "not before the pause");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let d = st.update(&t.state, &s, t0 + s.delay, &no_wake);
            if d.popup.is_some() || Instant::now() > deadline {
                return d;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn opens_after_the_pause_under_the_word() {
        let path = programs("open", &["gzip", "git", "grep"]);
        let mut st = state(&path);
        let t = term("g");
        let s = settings();
        let t0 = Instant::now();
        assert_eq!(st.update(&t.state, &s, t0, &no_wake).wake_after, Some(s.delay));
        // One wake-up per pause.
        assert_eq!(st.update(&t.state, &s, t0, &no_wake).wake_after, None);
        let d = settle(&mut st, &t);
        let p = d.popup.expect("a list");
        let names: Vec<&str> = p.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(names, ["git", "grep", "gzip"]);
        assert!(p.items[0].selected);
        assert_eq!(p.items[0].detail, "command");
        // Below the cursor's row, its text under the word (column 2).
        assert_eq!((p.row, p.col + 2), (1, 2));
        // The grey text: the rest of the first choice.
        assert_eq!(d.ghost.map(|g| (g.row, g.col, g.text)), Some((0, 3, "it".to_string())));
    }

    #[test]
    fn keys_move_take_and_close() {
        let path = programs("keys", &["gzip", "git", "grep"]);
        let mut st = state(&path);
        let t = term("g");
        settle(&mut st, &t);
        assert_eq!(st.key(keycode::DOWN, true), KeyResult::Redraw);
        assert_eq!(st.key(keycode::UP, false), KeyResult::Pass, "with a modifier: the shell's");
        assert_eq!(st.key(36, true), KeyResult::Pass, "Return: the shell's");
        let later = Instant::now() + Duration::from_secs(1);
        let d = st.update(&t.state, &settings(), later, &no_wake);
        assert!(d.popup.unwrap().items[1].selected);
        // Tab takes grep: zsh's widget gets the word's start, nothing kept,
        // the name and a space after it.
        assert_eq!(st.key(keycode::TAB, true), KeyResult::Send(b"\x1b[7701~0;0;grep; \x07".to_vec()));
        assert!(!st.is_open());
        // Esc closes it, and it stays closed on the same word.
        let t = term("gi");
        settle(&mut st, &t);
        assert!(st.is_open());
        assert_eq!(st.key(keycode::ESCAPE, true), KeyResult::Redraw);
        let later = Instant::now() + Duration::from_secs(5);
        assert_eq!(st.update(&t.state, &settings(), later, &no_wake).popup, None);
        assert_eq!(st.key(keycode::DOWN, true), KeyResult::Pass);
    }

    #[test]
    fn not_on_an_empty_word_or_without_a_report() {
        let path = programs("empty", &["git"]);
        let mut st = state(&path);
        let s = settings();
        let later = Instant::now() + Duration::from_secs(1);
        for line in ["", "git "] {
            let t = term(line);
            st.update(&t.state, &s, Instant::now(), &no_wake);
            assert_eq!(st.update(&t.state, &s, later, &no_wake), Drawn::default(), "{line:?}");
        }
        let mut t = Term::new(40, 10, 100, ImagePool::process());
        t.advance(b"$ g");
        assert_eq!(st.update(&t.state, &s, later, &no_wake), Drawn::default());
    }

    #[test]
    fn files_keep_the_folder_typed() {
        let dir = std::env::temp_dir().join(format!("decline-complete-{}-files", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/macros")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        let mut st = state("/nonexistent");
        let mut t = term("cat src/ma");
        let cwd = dir.canonicalize().unwrap();
        t.advance(format!("\x1b]7;file://host{}\x07", cwd.display()).as_bytes());
        let d = settle(&mut st, &t);
        let p = d.popup.expect("a list");
        let names: Vec<&str> = p.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(names, ["macros/", "main.rs"]);
        // Taking the folder keeps `src/` as typed (4 bytes from the word's
        // start at 4), with no space after it.
        assert_eq!(st.key(keycode::TAB, true), KeyResult::Send(b"\x1b[7701~4;4;macros/;\x07".to_vec()));
    }

    #[test]
    fn encoding_for_the_widget() {
        assert_eq!(encode("a;b%c\x07d é"), "a%3Bb%25c%07d é");
    }
}
