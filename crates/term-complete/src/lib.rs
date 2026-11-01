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

pub mod help;
pub mod ink;
pub mod lex;
pub mod list;
pub mod man;
pub mod marks;
pub mod output;
pub mod sources;

use list::{Listings, Offer, What};

/// The settings the list follows (docs/design/completion.md §10).
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// The list opens by itself after a pause in typing; without it, it
    /// only shows zsh's matches on Tab.
    pub as_you_type: bool,
    pub delay: Duration,
    /// Choices shown before the list scrolls.
    pub rows: usize,
    pub ghost: bool,
    pub files: bool,
    pub commands: bool,
    /// Flags and subcommands from man pages.
    pub flags: bool,
    /// Commands by how often they were run.
    pub history_order: bool,
    /// Return puts in a chosen choice instead of running the line (off:
    /// Return always goes to the shell).
    pub enter_takes: bool,
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
    /// A folder was just put in: its list opens without the pause.
    soon: bool,
    /// What zsh's completion found on Tab (§7), shown until a key that
    /// isn't the list's own.
    zsh: Option<Vec<list::Choice>>,
    /// zsh's matches without a description were given one from the man
    /// page or `--help` (or there was nothing to give).
    zsh_described: bool,
    /// The last offer and what it was made from: the same again (each
    /// frame while the list is open) isn't worked out again.
    last_offer: Option<(OfferKey, Option<Arc<Offer>>)>,
    open: Option<Open>,
}

/// What an offer is made from: what the cursor is on, the settings that
/// change it, and the listings read (by address: a new read is a new one).
#[derive(PartialEq, Eq)]
struct OfferKey {
    at: lex::At,
    settings: (bool, bool, bool, bool),
    listings: [usize; 6],
}

fn address<T>(a: &Option<Arc<T>>) -> usize {
    a.as_ref().map_or(0, |a| Arc::as_ptr(a) as *const u8 as usize)
}

/// The list while it is open.
#[derive(Debug)]
struct Open {
    offer: Arc<Offer>,
    /// The choice moved to with Down (or the mouse); none until then, so
    /// Up still goes to the shell's history and Return runs the line.
    selected: Option<usize>,
    /// `selected` is the one under the mouse: it is let go when the mouse
    /// leaves the list or the line is typed on, so Return runs the line
    /// again.
    hovered: bool,
    /// Where the list was last drawn: its top row and left column (border
    /// included), its width and how many choices it shows.
    drawn_at: Option<(usize, usize, usize, usize)>,
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
    /// Return puts in a chosen choice ([`Settings::enter_takes`]).
    enter_takes: bool,
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
        self.zsh_described = false;
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
        let changed = self.seen.as_ref() != Some(&key);
        if changed {
            self.seen = Some(key);
            self.since = Some(now);
            self.quiet = !self.typing;
        }
        if self.zsh.is_some() {
            return self.show_zsh(t, report, s, wake);
        }
        if !s.as_you_type {
            self.open = None;
            return Drawn::default();
        }
        let since = self.since.unwrap_or(now);
        if self.open.is_none() && self.quiet {
            return Drawn::default();
        }
        if self.open.is_none() && !self.soon {
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
        // Keep the selected choice when it is still offered; the one under
        // the mouse only until the line changes.
        let previous = self.open.as_ref().filter(|o| o.start == start && !(o.hovered && changed));
        let selected = previous
            .and_then(|o| o.offer.choices.get(o.selected?))
            .and_then(|c| offer.choices.iter().position(|x| x.name == c.name));
        let hovered = selected.is_some() && previous.is_some_and(|o| o.hovered);
        let scroll = previous.map_or(0, |o| o.scroll);
        // What is typed before the matched part is kept as typed (`~/`,
        // `src/`) when it reads the same unquoted; else the whole word is
        // replaced, quoted by the shell.
        let keep_bytes = if typed.starts_with(&offer.keep) { offer.keep.len() } else { 0 };
        let mut open = Open {
            offer,
            selected,
            hovered,
            drawn_at: None,
            scroll,
            start,
            keep_bytes,
            ghost: false,
            vi: report.keymap == Keymap::ViInsert,
            enter_takes: s.enter_takes,
            zsh: false,
        };
        let drawn = place(t, &mut open, s, report.cursor == report.text.len());
        self.open = Some(open);
        self.soon = false;
        drawn
    }

    /// After Read Flags from --help: the list may open without more
    /// typing.
    pub fn help_read(&mut self) {
        self.typing = true;
        self.quiet = false;
        self.since = None;
        self.seen = None;
    }

    /// What Read Flags from --help runs for the line at the prompt: the
    /// program under (or before) the cursor, and its subcommand when the
    /// cursor is past one (`kubectl get -` runs `kubectl get --help`).
    /// Only programs: an alias, function or builtin can't be run alone.
    pub fn help_target(&self, t: &Terminal) -> Result<HelpTarget, String> {
        let at_prompt = "Read Flags from --help works at zsh's prompt, with a command typed";
        let report = t.line_report().ok_or(at_prompt)?;
        let env = self.shell_env.as_deref().ok_or(at_prompt)?;
        let line = lex::read(&report.text, report.cursor);
        let i = line
            .words
            .iter()
            .rposition(|w| w.range.start <= report.cursor)
            .ok_or("type a command first")?;
        let c = line.words[i].command.ok_or("type a command first")?;
        let command = &line.words[c];
        if command.expands {
            return Err(format!("{} isn't a program's name", command.text));
        }
        let name = command.text.as_str();
        let listed = |list: &[String]| list.iter().any(|n| n == name);
        for (list, what) in [
            (&env.aliases, "an alias"),
            (&env.functions, "a function"),
            (&env.builtins, "a builtin"),
            (&env.reserved_words, "a keyword"),
        ] {
            if listed(list) {
                return Err(format!("{name} is {what}: only programs can be asked for --help"));
            }
        }
        let sub = (c + 1..i)
            .find(|&j| line.words[j].command == Some(c) && line.words[j].role == lex::Role::Argument)
            .map(|j| line.words[j].text.clone())
            .filter(|s| {
                s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
                    && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            });
        let exe = if name.contains('/') {
            let p = std::path::Path::new(name);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::path::Path::new(t.cwd().unwrap_or("/")).join(p)
            }
        } else {
            std::env::split_paths(&env.path)
                .filter(|d| d.is_absolute())
                .map(|d| d.join(name))
                .find(|p| is_executable(p))
                .ok_or_else(|| format!("{name} isn't a program on your PATH"))?
        };
        if !is_executable(&exe) {
            return Err(format!("{name} isn't a program"));
        }
        let base = name.rsplit('/').next().unwrap_or(name);
        Ok(HelpTarget {
            name: match &sub {
                Some(s) => format!("{base}-{s}"),
                None => base.to_string(),
            },
            exe,
            sub,
            path: env.path.clone(),
        })
    }

    /// The list with zsh's matches, under the word at the cursor.
    fn show_zsh(&mut self, t: &Terminal, report: &term_core::LineReport, s: &Settings, wake: &dyn Fn() -> sources::Wake) -> Drawn {
        // zsh's redraw, with what the matches share inserted, comes after
        // the matches: wait for the screen to show it.
        if !shown_before_cursor(t, &report.text[..report.cursor]) {
            return Drawn::default();
        }
        let line = lex::read(&report.text, report.cursor);
        // What zsh didn't describe, described from what we read: a flag's
        // line in its man page, a subcommand's.
        if !self.zsh_described
            && let Some(known) = self.descriptions(&line.at, s, wake)
        {
            if let Some(choices) = &mut self.zsh {
                describe(choices, &known);
            }
            self.zsh_described = true;
        }
        let Some(choices) = self.zsh.clone() else { return Drawn::default() };
        let start = line
            .words
            .iter()
            .find(|w| w.range.start <= report.cursor && report.cursor <= w.range.end)
            .map_or(report.cursor, |w| w.range.start);
        let previous = self.open.take().filter(|o| o.zsh);
        let mut open = Open {
            offer: Arc::new(Offer {
                choices,
                prefix: report.text[start..report.cursor].to_string(),
                keep: String::new(),
            }),
            selected: previous.as_ref().and_then(|o| o.selected),
            hovered: previous.as_ref().is_some_and(|o| o.hovered),
            drawn_at: None,
            scroll: previous.as_ref().map_or(0, |o| o.scroll),
            start,
            keep_bytes: 0,
            ghost: false,
            vi: report.keymap == Keymap::ViInsert,
            enter_takes: s.enter_takes,
            zsh: true,
        };
        let no_ghost = Settings { ghost: false, ..*s };
        let drawn = place(t, &mut open, &no_ghost, false);
        self.open = Some(open);
        drawn
    }

    /// Names and what each is, for describing zsh's matches at `at`:
    /// flags from the command's man page (or `--help`), subcommands from
    /// theirs. `None` while still being read; empty when there is nothing.
    fn descriptions(&self, at: &lex::At, s: &Settings, wake: &dyn Fn() -> sources::Wake) -> Option<Vec<(String, String)>> {
        use lex::At;
        let Some(env) = self.shell_env.as_deref().filter(|_| s.flags) else {
            return Some(Vec::new());
        };
        let base = |c: &str| c.rsplit('/').next().unwrap_or(c).to_string();
        match at {
            At::Flag { command, subcommand, .. } => {
                let m = manual_for(&env.path, &base(command), subcommand.as_deref(), wake)?;
                Some(m.page.flags.iter().map(|f| (f.name.clone(), f.about.clone())).collect())
            }
            At::Argument {
                command,
                position: 0,
                prefix,
                ..
            } if !prefix.contains('/') => Some(sources::subcommands(&env.path, &base(command), wake)?.list.clone()),
            _ => Some(Vec::new()),
        }
    }

    /// The choices for what the cursor is on, if any are worth a list.
    fn offer(&mut self, at: &lex::At, t: &Terminal, s: &Settings, wake: &dyn Fn() -> sources::Wake) -> Option<Arc<Offer>> {
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
        // Read at the first list of commands, not at the prompt; A–Z
        // until it is.
        let history = match env.and_then(|e| e.histfile.as_deref()) {
            Some(file) if is_command && s.commands && s.history_order => sources::history(std::path::Path::new(file), wake),
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
                manual = manual_for(&env.path, &base(command), subcommand.as_deref(), wake);
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
        let key = OfferKey {
            at: at.clone(),
            settings: (s.files, s.commands, s.flags, s.history_order),
            listings: [
                address(&self.shell_env),
                address(&commands),
                address(&folder),
                address(&manual),
                address(&subcommands),
                address(&history),
            ],
        };
        if let Some((k, offer)) = &self.last_offer
            && *k == key
        {
            return offer.clone();
        }
        let offer = list::offer(
            at,
            &Listings {
                env,
                commands: commands.as_deref(),
                folder: folder.as_deref(),
                flags: manual.as_deref().map(|m| &m.page),
                subcommands: subcommands.as_deref(),
                history: history.as_deref(),
            },
        )
        // Nothing to add: no choices, or only what is typed already (a
        // folder's name without its `/` counts).
        .filter(|offer| match offer.choices.as_slice() {
            [] => false,
            [only] => only.name.strip_suffix('/').unwrap_or(&only.name) != offer.prefix,
            _ => true,
        })
        .map(Arc::new);
        self.last_offer = Some((key, offer.clone()));
        offer
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
        self.soon = false;
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
        if matches!(keycode, DOWN | UP) {
            open.hovered = false;
        }
        match keycode {
            DOWN => {
                open.selected = Some(open.selected.map_or(0, |i| (i + 1) % n));
                KeyResult::Redraw
            }
            // With the setting, Return puts in the choice moved to; with
            // none chosen (or without it), it runs the line as typed.
            RETURN | KEYPAD_ENTER if open.enter_takes && open.selected.is_some() => self.take(),
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

    /// The mouse moved to view `row`, `col`: the choice under it is
    /// selected, and let go when it leaves the list. True if that changed
    /// what is drawn.
    pub fn hover(&mut self, row: usize, col: usize) -> bool {
        let Some(open) = &mut self.open else { return false };
        match open.choice_at(row, col) {
            Some(i) if open.selected == Some(i) && open.hovered => false,
            Some(i) => {
                open.selected = Some(i);
                open.hovered = true;
                true
            }
            None if open.hovered => {
                open.selected = None;
                open.hovered = false;
                true
            }
            None => false,
        }
    }

    /// A click at view `row`, `col`: on a choice it is taken; elsewhere on
    /// the list it does nothing. `None` when the click isn't on the list,
    /// and goes on as usual.
    pub fn click(&mut self, row: usize, col: usize) -> Option<KeyResult> {
        let open = self.open.as_mut()?;
        let (top, left, width, shown) = open.drawn_at?;
        if !(top..top + shown + 2).contains(&row) || !(left..left + width).contains(&col) {
            return None;
        }
        let Some(i) = open.choice_at(row, col) else {
            return Some(KeyResult::Redraw);
        };
        open.selected = Some(i);
        Some(self.take())
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
        let folder = c.what == What::Folder;
        let suffix = if folder { "" } else { " " };
        let bytes = format!("\x1b[7701~{};{};{};{suffix}\x07", open.start, open.keep_bytes, encode(&name));
        // The word changes: what is offered next is for the new one. A
        // folder's list opens again straight away, for what is in it.
        self.seen = None;
        self.typing = folder;
        self.soon = folder;
        KeyResult::Send(bytes.into_bytes())
    }
}

/// Give each choice without a description the one `known` has for its
/// name.
fn describe(choices: &mut [list::Choice], known: &[(String, String)]) {
    for c in choices.iter_mut().filter(|c| c.about.is_empty()) {
        if let Some((_, about)) = known.iter().find(|(name, _)| *name == c.name) {
            c.about = about.clone();
        }
    }
}

/// `command`'s man page, or for `git fetch -` git-fetch's, else git's.
/// `None` while it is being read.
fn manual_for(path: &str, command: &str, sub: Option<&str>, wake: &dyn Fn() -> sources::Wake) -> Option<Arc<sources::Manual>> {
    match sub {
        Some(sub) => match sources::manual(path, &format!("{command}-{sub}"), wake) {
            Some(m) if !m.found => sources::manual(path, command, wake),
            m => m,
        },
        None => sources::manual(path, command, wake),
    }
}

/// A program and subcommand for Read Flags from --help.
#[derive(Debug, PartialEq, Eq)]
pub struct HelpTarget {
    pub exe: std::path::PathBuf,
    pub sub: Option<String>,
    /// What its flags are kept under: `kubectl-get`.
    pub name: String,
    pub path: String,
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

impl Open {
    /// The choice drawn at view `row`, `col`, if any.
    fn choice_at(&self, row: usize, col: usize) -> Option<usize> {
        let (top, left, width, shown) = self.drawn_at?;
        let i = row.checked_sub(top + 1).filter(|&i| i < shown)?;
        (left < col && col + 1 < left + width).then_some(self.scroll + i)
    }
}

/// The keys the list's bottom border shows, the longest that fits in
/// `width`: Return only puts in a choice once one is chosen.
fn hint(open: &Open, width: usize) -> String {
    let hints: &[&str] = if open.selected.is_some() && open.enter_takes {
        &["⏎ put in · esc close", "⏎ put in"]
    } else if open.selected.is_some() {
        &["⇥ put in · esc close", "⇥ put in"]
    } else {
        &["⇥ put in · ↓ choose", "⇥ put in"]
    };
    hints
        .iter()
        .find(|h| h.width() + 6 <= width)
        .map_or(String::new(), |h| h.to_string())
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
    open.drawn_at = Some((row, col, width, items.len()));
    let popup = Popup {
        row: row as u16,
        col: col as u16,
        width: width as u16,
        hint: hint(open, width),
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
            as_you_type: true,
            delay: Duration::from_millis(150),
            rows: 8,
            ghost: true,
            files: true,
            commands: true,
            flags: true,
            history_order: true,
            enter_takes: true,
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
        // The folder's own list opens straight away, without typing.
        let mut t = term("cat src/macros/");
        t.advance(format!("\x1b]7;file://host{}\x07", cwd.display()).as_bytes());
        std::fs::write(dir.join("src/macros/one.rs"), "").unwrap();
        std::fs::write(dir.join("src/macros/two.rs"), "").unwrap();
        let now = Instant::now();
        let deadline = now + Duration::from_secs(5);
        let d = loop {
            let d = st.update(&t.state, &settings(), now, &no_wake);
            if d.popup.is_some() || Instant::now() > deadline {
                break d;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let names: Vec<String> = d.popup.expect("the folder's list").items.into_iter().map(|i| i.text).collect();
        assert_eq!(names, ["one.rs", "two.rs"]);
    }

    #[test]
    fn return_runs_the_line_until_a_choice_is_chosen() {
        let path = programs("return", &["gzip", "git", "grep"]);
        let mut st = state(&path);
        let t = term("g");
        let d = settle(&mut st, &t);
        // The bottom border says what puts a choice in.
        assert_eq!(d.popup.unwrap().hint, "⇥ put in");
        assert_eq!(
            st.key(keycode::RETURN, true, false),
            KeyResult::Pass,
            "nothing chosen: the line runs"
        );
        let later = Instant::now() + Duration::from_secs(1);
        st.update(&t.state, &settings(), later, &no_wake);
        assert_eq!(st.key(keycode::DOWN, true, false), KeyResult::Redraw);
        let d = st.update(&t.state, &settings(), later, &no_wake);
        assert_eq!(d.popup.unwrap().hint, "⏎ put in");
        assert_eq!(
            st.key(keycode::KEYPAD_ENTER, true, false),
            KeyResult::Send(b"\x1b[7701~0;0;git; \x07".to_vec())
        );
        // Without the setting, Return is always the shell's.
        let s = Settings {
            enter_takes: false,
            ..settings()
        };
        let mut st = state(&path);
        settle(&mut st, &t);
        st.key(keycode::DOWN, true, false);
        let d = st.update(&t.state, &s, later, &no_wake);
        assert_eq!(d.popup.unwrap().hint, "⇥ put in");
        assert_eq!(st.key(keycode::RETURN, true, false), KeyResult::Pass);
    }

    #[test]
    fn the_mouse_chooses_and_a_click_takes() {
        let path = programs("mouse", &["gzip", "git", "grep"]);
        let mut st = state(&path);
        let t = term("g");
        let p = settle(&mut st, &t).popup.unwrap();
        // The list's border is at rows 1 and 5, its choices on 2 to 4.
        assert_eq!((p.row, p.col), (1, 0));
        let s = settings();
        let later = Instant::now() + Duration::from_secs(1);
        assert!(st.hover(3, 4));
        assert!(!st.hover(3, 5), "the same choice");
        let d = st.update(&t.state, &s, later, &no_wake);
        assert!(d.popup.unwrap().items[1].selected);
        // The grey text follows it.
        assert_eq!(d.ghost.unwrap().text, "rep");
        // Off the list: let go, so Return runs the line again.
        assert!(st.hover(8, 4));
        assert_eq!(st.key(keycode::RETURN, true, false), KeyResult::Pass);
        // On the border: nothing, but the click is the list's.
        st.update(&t.state, &s, later, &no_wake);
        assert_eq!(st.click(1, 4), Some(KeyResult::Redraw));
        assert_eq!(st.click(8, 4), None, "not on the list");
        assert_eq!(st.click(4, 4), Some(KeyResult::Send(b"\x1b[7701~0;0;gzip; \x07".to_vec())));
        // Typing lets go of the choice under the mouse.
        let t = term("g");
        let mut st = state(&path);
        settle(&mut st, &t);
        st.hover(2, 4);
        st.key(b'r'.into(), true, false);
        let t2 = term("gr");
        st.update(&t2.state, &s, later, &no_wake);
        assert_eq!(st.key(keycode::RETURN, true, false), KeyResult::Pass);
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
    fn zshs_matches_described_from_what_we_read() {
        let choice = |name: &str, about: &str| list::Choice {
            name: name.into(),
            what: What::Match,
            about: about.into(),
        };
        let mut choices = vec![choice("--all", ""), choice("--tags", "zsh's own"), choice("--zzz", "")];
        let known = [
            ("--all".to_string(), "Fetch all remotes".to_string()),
            ("--tags".to_string(), "Fetch all tags".to_string()),
        ];
        describe(&mut choices, &known);
        let abouts: Vec<&str> = choices.iter().map(|c| c.about.as_str()).collect();
        assert_eq!(abouts, ["Fetch all remotes", "zsh's own", ""]);
    }

    #[test]
    fn only_tab_without_the_list_as_you_type() {
        let path = programs("tab-only", &["gzip", "git", "grep"]);
        let mut st = state(&path);
        let s = Settings {
            as_you_type: false,
            ..settings()
        };
        let t = term("g");
        let later = Instant::now() + Duration::from_secs(5);
        st.update(&t.state, &s, Instant::now(), &no_wake);
        assert_eq!(st.update(&t.state, &s, later, &no_wake).popup, None, "not by itself");
        st.zsh_matches(term_core::ShellMatches {
            shown: vec!["git".into(), "grep".into()],
        });
        assert!(st.update(&t.state, &s, later, &no_wake).popup.is_some(), "zsh's matches on Tab");
    }

    #[test]
    fn what_read_flags_from_help_runs() {
        let path = programs("help", &["kubectl"]);
        let mut st = state(&path);
        let target = |st: &PaneState, line: &str| st.help_target(&term(line).state);
        let t = target(&st, "kubectl get -").unwrap();
        assert_eq!((t.sub.as_deref(), t.name.as_str()), (Some("get"), "kubectl-get"));
        assert!(t.exe.ends_with("kubectl"));
        // On the subcommand itself: the command's own.
        assert_eq!(target(&st, "kubectl ge").unwrap().name, "kubectl");
        assert_eq!(target(&st, "kubectl").unwrap().sub, None);
        assert!(target(&st, "cd -").unwrap_err().contains("builtin"));
        assert!(target(&st, "nothere -").unwrap_err().contains("PATH"));
        assert!(target(&st, "").is_err());
        st.shell_env = None;
        assert!(target(&st, "kubectl -").is_err());
    }

    #[test]
    fn encoding_for_the_widget() {
        assert_eq!(encode("a;b%c\x07d é"), "a%3Bb%25c%07d é");
    }
}
