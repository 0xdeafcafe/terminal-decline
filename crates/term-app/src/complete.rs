//! Completion and colouring at the shell's prompt
//! (docs/design/completion.md): [`lex`] reads the command line into
//! words and says what the cursor is on; [`ink`] colours it;
//! [`sources`] reads what can be offered, off the main thread, and
//! [`man`] a command's flags from its man page; [`list`]
//! picks the choices; [`PaneState`] opens the list after a pause, places
//! it, and handles its keys.

use std::sync::Arc;
use std::time::{Duration, Instant};

use term_core::{Ghost, Keymap, Popup, PopupItem, Terminal};
use unicode_width::UnicodeWidthStr;

pub mod ink;
pub mod lex;
pub mod list;
pub mod man;
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
    /// Flags and subcommands from man pages.
    pub flags: bool,
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
    /// The last key typed text (or deleted it), rather than moving
    /// through history or the line.
    typing: bool,
    /// The line last changed by something other than typing (history, a
    /// widget, zsh's own completion): the list waits for typing.
    quiet: bool,
    /// What zsh's completion found on Tab (§7), shown until a key that
    /// isn't the list's own.
    zsh: Option<Vec<list::Choice>>,
    open: Option<Open>,
}

/// The list while it is open.
#[derive(Debug)]
struct Open {
    offer: Offer,
    /// The choice moved to with Down; none until then, so Up still goes
    /// to the shell's history.
    selected: Option<usize>,
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
    /// zsh's matches: taking one sends its number for zsh to insert.
    zsh: bool,
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
    pub const RETURN: u16 = 36;
    pub const TAB: u16 = 48;
    pub const ESCAPE: u16 = 53;
    pub const KEYPAD_ENTER: u16 = 76;
    pub const HOME: u16 = 115;
    pub const PAGE_UP: u16 = 116;
    pub const END: u16 = 119;
    pub const PAGE_DOWN: u16 = 121;
    pub const LEFT: u16 = 123;
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
        self.zsh = None;
    }

    /// zsh's matches from Tab: shown in the list straight away.
    pub fn zsh_matches(&mut self, m: term_core::ShellMatches) {
        let choices = m
            .shown
            .into_iter()
            .map(|shown| {
                // `_describe`'s `name  -- what it is`.
                let (name, about) = match shown.split_once(" -- ") {
                    Some((n, a)) => (n.trim_end().to_string(), a.trim().to_string()),
                    None => (shown, String::new()),
                };
                list::Choice {
                    name,
                    what: What::Match,
                    about,
                }
            })
            .collect();
        self.zsh = Some(choices);
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
            self.quiet = !self.typing;
        }
        if self.zsh.is_some() {
            return self.show_zsh(t, report, s);
        }
        let since = self.since.unwrap_or(now);
        if self.open.is_none() && self.quiet {
            return Drawn::default();
        }
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
        // A report the screen doesn't show yet (zsh's own completion
        // changed the line without one): nothing until it does.
        if !shown_before_cursor(t, &report.text[..report.cursor]) {
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
            .and_then(|o| o.offer.choices.get(o.selected?))
            .and_then(|c| offer.choices.iter().position(|x| x.name == c.name));
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
            zsh: false,
        };
        let drawn = place(t, &mut open, s, report.cursor == report.text.len());
        self.open = Some(open);
        drawn
    }

    /// The list with zsh's matches, under the word at the cursor.
    fn show_zsh(&mut self, t: &Terminal, report: &term_core::LineReport, s: &Settings) -> Drawn {
        let Some(choices) = self.zsh.clone() else { return Drawn::default() };
        // zsh's redraw, with what the matches share inserted, comes after
        // the matches: wait for the screen to show it.
        if !shown_before_cursor(t, &report.text[..report.cursor]) {
            return Drawn::default();
        }
        let line = lex::read(&report.text, report.cursor);
        let start = line
            .words
            .iter()
            .find(|w| w.range.start <= report.cursor && report.cursor <= w.range.end)
            .map_or(report.cursor, |w| w.range.start);
        let previous = self.open.take().filter(|o| o.zsh);
        let mut open = Open {
            offer: Offer {
                choices,
                prefix: report.text[start..report.cursor].to_string(),
                keep: String::new(),
            },
            selected: previous.as_ref().and_then(|o| o.selected),
            scroll: previous.as_ref().map_or(0, |o| o.scroll),
            start,
            keep_bytes: 0,
            ghost: false,
            vi: report.keymap == Keymap::ViInsert,
            zsh: true,
        };
        let no_ghost = Settings { ghost: false, ..*s };
        let drawn = place(t, &mut open, &no_ghost, false);
        self.open = Some(open);
        drawn
    }

    /// The choices for what the cursor is on, if any are worth a list.
    fn offer(&self, at: &lex::At, t: &Terminal, s: &Settings, wake: &dyn Fn() -> sources::Wake) -> Option<Offer> {
        use lex::At;
        let prefix = match at {
            At::Command { prefix } | At::Argument { prefix, .. } | At::Path { prefix, .. } | At::Flag { prefix, .. } => prefix,
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
        let is_flag = matches!(at, At::Flag { .. });
        let folder = match (!is_command && !is_flag && s.files)
            .then(|| list::folder_of(prefix, t.cwd(), home.as_deref()))
            .flatten()
        {
            Some((dir, _)) => sources::folder(&dir, wake),
            None => None,
        };
        // A command's own name, for its man pages: `/usr/bin/ls` is `ls`.
        let base = |c: &str| c.rsplit('/').next().unwrap_or(c).to_string();
        let mut manual = None;
        let mut subcommands = None;
        match (at, env) {
            (At::Flag { command, subcommand, .. }, Some(env)) if s.flags => {
                let command = base(command);
                manual = match subcommand {
                    // `git fetch -`: git-fetch's page, else git's.
                    Some(sub) => match sources::manual(&env.path, &format!("{command}-{sub}"), wake) {
                        Some(m) if !m.found => sources::manual(&env.path, &command, wake),
                        m => m,
                    },
                    None => sources::manual(&env.path, &command, wake),
                };
                // Still being read: wait for it rather than show nothing.
                manual.as_ref()?;
            }
            (
                At::Argument {
                    command,
                    position: 0,
                    prefix,
                    ..
                },
                Some(env),
            ) if s.flags && !prefix.contains('/') => {
                subcommands = sources::subcommands(&env.path, &base(command), wake);
                subcommands.as_ref()?;
            }
            _ => {}
        }
        let offer = list::offer(
            at,
            &Listings {
                env,
                commands: commands.as_deref(),
                folder: folder.as_deref(),
                flags: manual.as_deref().map(|m| &m.page),
                subcommands: subcommands.as_deref(),
            },
        )?;
        // Nothing to add: no choices, or only what is typed already (a
        // folder's name without its `/` counts).
        match offer.choices.as_slice() {
            [] => None,
            [only] if only.name.strip_suffix('/').unwrap_or(&only.name) == offer.prefix => None,
            _ => Some(offer),
        }
    }

    /// A key, before it goes to the shell: `plain` when no modifier is
    /// held, `chord` when Control or Command is. Every key comes here, so
    /// the list knows whether the line changes from typing.
    pub fn key(&mut self, keycode: u16, plain: bool, chord: bool) -> KeyResult {
        use keycode::*;
        self.typing = !chord
            && !matches!(
                keycode,
                RETURN | TAB | ESCAPE | KEYPAD_ENTER | HOME | PAGE_UP | END | PAGE_DOWN | LEFT | RIGHT | DOWN | UP
            );
        let result = self.list_key(keycode, plain);
        // zsh's matches last until a key that isn't the list's own.
        if matches!(result, KeyResult::Pass | KeyResult::CloseAndPass) && self.zsh.take().is_some() {
            self.open = None;
        }
        result
    }

    fn list_key(&mut self, keycode: u16, plain: bool) -> KeyResult {
        use keycode::*;
        let Some(open) = &mut self.open else { return KeyResult::Pass };
        if !plain {
            return KeyResult::Pass;
        }
        let n = open.offer.choices.len();
        match keycode {
            DOWN => {
                open.selected = Some(open.selected.map_or(0, |i| (i + 1) % n));
                KeyResult::Redraw
            }
            // Up before Down is the shell's (its history); Up from the
            // first choice goes back to none.
            UP => match open.selected {
                None => {
                    self.close_on_word();
                    KeyResult::CloseAndPass
                }
                Some(0) => {
                    open.selected = None;
                    KeyResult::Redraw
                }
                Some(i) => {
                    open.selected = Some(i - 1);
                    KeyResult::Redraw
                }
            },
            TAB => self.take(),
            RIGHT if open.ghost => self.take(),
            ESCAPE => {
                let vi = open.vi;
                self.zsh = None;
                self.close_on_word();
                if vi { KeyResult::CloseAndPass } else { KeyResult::Redraw }
            }
            _ => KeyResult::Pass,
        }
    }

    /// Close the list until the word under the cursor changes.
    fn close_on_word(&mut self) {
        let Some(open) = self.open.take() else { return };
        let start = open.start;
        self.closed = self
            .seen
            .as_ref()
            .map(|(text, cursor)| (start, text[start.min(*cursor)..*cursor].to_string()));
    }

    /// Take the selected choice (the first when none is): the bytes that have zsh put it in place
    /// of the word (`ESC [ 7701 ~`, then `start;keep;name;suffix` and
    /// BEL, read by decline.zsh's widget, which quotes the name itself).
    fn take(&mut self) -> KeyResult {
        let Some(open) = self.open.take() else { return KeyResult::Pass };
        let index = open.selected.unwrap_or(0);
        let Some(c) = open.offer.choices.get(index) else {
            return KeyResult::Pass;
        };
        if open.zsh {
            // zsh inserts its own match, by its number.
            self.zsh = None;
            self.seen = None;
            return KeyResult::Send(format!("\x1b[7701~#{}\x07", index + 1).into_bytes());
        }
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
    let current = open.selected.unwrap_or(0);
    if current < open.scroll {
        open.scroll = current;
    } else if current >= open.scroll + shown {
        open.scroll = current + 1 - shown;
    }
    open.scroll = open.scroll.min(n.saturating_sub(shown));
    let rows_shown = &offer.choices[open.scroll..(open.scroll + shown).min(n)];
    let detail = |c: &list::Choice| if c.about.is_empty() { c.what.label() } else { c.about.as_str() }.to_string();
    let widest_text = rows_shown.iter().map(|c| c.name.width()).max().unwrap_or(0);
    // `│▸` text, two spaces, detail ` │`; details too wide for the
    // screen are cut short.
    let room = cols.saturating_sub(widest_text + 7);
    let mut details: Vec<String> = rows_shown.iter().map(|c| fit(&detail(c), room)).collect();
    let widest_detail = details.iter().map(|d| d.width()).max().unwrap_or(0);
    // Each padded to the widest, so they line up on the left.
    for d in &mut details {
        let pad = widest_detail - d.width();
        d.extend(std::iter::repeat_n(' ', pad));
    }
    let items: Vec<PopupItem> = rows_shown
        .iter()
        .zip(details)
        .enumerate()
        .map(|(i, (c, detail))| PopupItem {
            text: c.name.clone(),
            detail,
            selected: open.selected == Some(open.scroll + i),
        })
        .collect();
    let widest = widest_text + if widest_detail > 0 { 2 + widest_detail } else { 0 };
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
    let selected = &offer.choices[current];
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

/// `s` cut to `cells` wide, ending in `…` when cut.
fn fit(s: &str, cells: usize) -> String {
    if s.width() <= cells {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw + 1 > cells {
            break;
        }
        w += cw;
        out.push(c);
    }
    if cells > 0 {
        out.push('…');
    }
    out
}

/// Whether the screen shows the end of `before` (the line up to the
/// cursor) just before the cursor.
fn shown_before_cursor(t: &Terminal, before: &str) -> bool {
    let Some(last) = before.chars().next_back() else { return true };
    let Some((row, col)) = t.cursor_in_view() else { return false };
    let cells = UnicodeWidthStr::width(last.encode_utf8(&mut [0; 4]) as &str).max(1);
    match col.checked_sub(cells) {
        Some(c) => t.visible_row(row).cell(c).ch() == last,
        // Wrapped to the next row: the last character is on the row above.
        None => row > 0 && t.visible_row(row - 1).cell(t.cols().saturating_sub(cells)).ch() == last,
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
            flags: true,
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
            typing: true,
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
        assert!(p.items.iter().all(|i| !i.selected), "none until Down");
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
        assert_eq!(st.key(keycode::DOWN, true, false), KeyResult::Redraw);
        assert_eq!(st.key(keycode::DOWN, true, false), KeyResult::Redraw);
        assert_eq!(st.key(keycode::UP, false, false), KeyResult::Pass, "with a modifier: the shell's");
        assert_eq!(st.key(keycode::RETURN, true, false), KeyResult::Pass, "Return: the shell's");
        let later = Instant::now() + Duration::from_secs(1);
        let d = st.update(&t.state, &settings(), later, &no_wake);
        assert!(d.popup.unwrap().items[1].selected);
        // Tab takes grep: zsh's widget gets the word's start, nothing kept,
        // the name and a space after it.
        assert_eq!(
            st.key(keycode::TAB, true, false),
            KeyResult::Send(b"\x1b[7701~0;0;grep; \x07".to_vec())
        );
        assert!(!st.is_open());
        // Esc closes it, and it stays closed on the same word.
        st.key(b'i'.into(), true, false);
        let t = term("gi");
        settle(&mut st, &t);
        assert!(st.is_open());
        assert_eq!(st.key(keycode::ESCAPE, true, false), KeyResult::Redraw);
        let later = Instant::now() + Duration::from_secs(5);
        assert_eq!(st.update(&t.state, &settings(), later, &no_wake).popup, None);
        assert_eq!(st.key(keycode::DOWN, true, false), KeyResult::Pass);
    }

    #[test]
    fn up_before_down_is_the_shells() {
        let path = programs("history", &["gzip", "git", "grep"]);
        let mut st = state(&path);
        let t = term("g");
        settle(&mut st, &t);
        // Down, then Up back to none: still the list's.
        assert_eq!(st.key(keycode::DOWN, true, false), KeyResult::Redraw);
        assert_eq!(st.key(keycode::UP, true, false), KeyResult::Redraw);
        // Up with none chosen closes it and goes to the shell.
        assert_eq!(st.key(keycode::UP, true, false), KeyResult::CloseAndPass);
        assert!(!st.is_open());
        // The line history brings back doesn't open it again.
        let t = term("grep foo");
        let later = Instant::now() + Duration::from_secs(5);
        st.update(&t.state, &settings(), Instant::now(), &no_wake);
        assert_eq!(st.update(&t.state, &settings(), later, &no_wake).popup, None);
        // Typing does.
        st.key(b'o'.into(), true, false);
        assert!(settle(&mut st, &term("gr")).popup.is_some());
    }

    #[test]
    fn tab_takes_the_first_when_none_is_chosen() {
        let path = programs("first", &["gzip", "git", "grep"]);
        let mut st = state(&path);
        settle(&mut st, &term("g"));
        assert_eq!(
            st.key(keycode::TAB, true, false),
            KeyResult::Send(b"\x1b[7701~0;0;git; \x07".to_vec())
        );
    }

    #[test]
    fn not_before_the_screen_shows_the_line() {
        let path = programs("stale", &["gzip", "git", "grep"]);
        let mut st = state(&path);
        let mut t = Term::new(40, 10, 100, ImagePool::process());
        t.state.set_report_token(TOKEN.as_bytes().try_into().ok());
        // The report says `gi`, the screen still shows `g`.
        t.advance(format!("\x1b]133;A\x07$ \x1b]133;B\x07g\x1b]7701;line;{TOKEN};main;2;gi\x07").as_bytes());
        let later = Instant::now() + Duration::from_secs(5);
        st.update(&t.state, &settings(), Instant::now(), &no_wake);
        for _ in 0..50 {
            assert_eq!(st.update(&t.state, &settings(), later, &no_wake).popup, None);
            std::thread::sleep(Duration::from_millis(2));
        }
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
        assert_eq!(
            st.key(keycode::TAB, true, false),
            KeyResult::Send(b"\x1b[7701~4;4;macros/;\x07".to_vec())
        );
    }

    #[test]
    fn zshs_matches_show_at_once_and_are_taken_by_number() {
        let mut st = state("/nonexistent");
        st.typing = false;
        let t = term("git checkout ma");
        st.zsh_matches(term_core::ShellMatches {
            shown: vec!["main  -- the default".into(), "master".into()],
        });
        // No pause, even after a key that didn't type.
        let d = st.update(&t.state, &settings(), Instant::now(), &no_wake);
        let p = d.popup.expect("a list");
        assert_eq!(p.items[0].text, "main");
        assert!(p.items[0].detail.starts_with("the default"));
        assert_eq!(d.ghost, None);
        assert_eq!(st.key(keycode::DOWN, true, false), KeyResult::Redraw);
        assert_eq!(st.key(keycode::DOWN, true, false), KeyResult::Redraw);
        st.update(&t.state, &settings(), Instant::now(), &no_wake);
        assert_eq!(st.key(keycode::TAB, true, false), KeyResult::Send(b"\x1b[7701~#2\x07".to_vec()));
        // Any other key ends them.
        st.zsh_matches(term_core::ShellMatches {
            shown: vec!["a".into(), "b".into()],
        });
        st.update(&t.state, &settings(), Instant::now(), &no_wake);
        assert!(st.is_open());
        assert_eq!(st.key(b'x'.into(), true, false), KeyResult::Pass);
        assert!(!st.is_open());
        assert_eq!(st.update(&t.state, &settings(), Instant::now(), &no_wake).popup, None);
    }

    #[test]
    fn encoding_for_the_widget() {
        assert_eq!(encode("a;b%c\x07d é"), "a%3Bb%25c%07d é");
    }
}
