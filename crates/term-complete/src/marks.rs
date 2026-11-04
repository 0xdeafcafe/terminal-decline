//! What the renderer draws around the commands already run
//! (docs/design/command-marks.md): their command lines coloured as they
//! were while typed, a faint line above each prompt, a worked/failed
//! mark with the time taken at the right of each finished command's
//! line, and (as settings) plain output coloured ([`super::output`]) and
//! `← cause` beside the line of a failed command's output that linetag's
//! hand-weighted net reads as why it failed.
//! All of it goes into the snapshot; the cells are never changed.
//!
//! Worked out again only when the terminal changed (its generation) or
//! the settings did, so an idle frame copies what it had; command lines
//! are read into words once per command, and output rows once per
//! content.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, LazyLock};

use linetag::log::Log;
use linetag::model::Tagger;

use term_core::{Color, CommandMark, Content, Finished, Ink, Snapshot, Terminal, ViewCommand};

use super::{ink, lex, output};

/// Which of it is on: `highlight_command_line` (past command lines),
/// `command_marks` (the lines and marks), `colour_output` and
/// `mark_failure_cause`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Settings {
    pub past_commands: bool,
    pub command_marks: bool,
    pub colour_output: bool,
    pub failure_cause: bool,
}

impl Settings {
    fn any(self) -> bool {
        self.past_commands || self.command_marks || self.colour_output || self.failure_cause
    }
}

/// What's put beside the line the net reads as why a command failed.
pub const CAUSE: &str = "← cause";

/// The most lines of one command's output marked as its cause.
const CAUSES_MARKED: usize = 2;

/// The net, built once (under a millisecond) the first time a failed
/// command is in view with the setting on.
static TAGGER: LazyLock<Tagger> = LazyLock::new(Tagger::new);

/// Command lines kept read into words, and output rows kept coloured,
/// before the caches start again.
const LEXED_KEPT: usize = 512;
const ROWS_KEPT: usize = 4096;
/// Failed commands' outputs kept read by the net.
const CAUSES_KEPT: usize = 256;

/// A pane's: what was worked out for its last frame, and the caches.
#[derive(Default)]
pub struct Marks {
    /// The terminal, its generation and the settings it was worked out
    /// for.
    key: Option<(u64, u64, Settings)>,
    commands: Vec<ViewCommand>,
    past: Vec<Ink>,
    /// A command was coloured plain because whether it exists wasn't
    /// known yet: the past lines are coloured again next frame.
    past_pending: bool,
    output: Vec<Ink>,
    separators: Vec<u16>,
    marks: Vec<CommandMark>,
    lexed: HashMap<u32, Arc<lex::Line>>,
    rows: HashMap<u64, Vec<(Range<u16>, u8)>>,
    text: Vec<char>,
    plain: Vec<bool>,
    /// A failed command's output in view (by its text) to the lines of
    /// it the net reads as the cause, as offsets into its rows.
    causes: HashMap<u64, Vec<u16>>,
}

impl Marks {
    /// Put this frame's colours, lines and marks into `snap`, after the
    /// snapshot was taken, with the terminal still locked. `known` says
    /// whether a command runs something, as for the line being typed.
    pub fn update(&mut self, t: &Terminal, s: Settings, known: &dyn Fn(&str) -> Option<bool>, snap: &mut Snapshot) {
        if !s.any() || t.alt_screen_active() {
            self.key = None;
            return;
        }
        let key = (t.terminal_id(), t.generation(), s);
        if self.key != Some(key) {
            self.key = Some(key);
            t.view_commands(&mut self.commands);
            self.marks_and_lines(t, s);
            self.cause_marks(t, s);
            self.output_ink(t, s);
            self.past_ink(t, s, known);
        } else if self.past_pending {
            self.past_ink(t, s, known);
        }
        if !(self.past.is_empty() && self.output.is_empty()) {
            snap.ink.extend_from_slice(&self.past);
            snap.ink.extend_from_slice(&self.output);
            // The renderer finds a row's colours by row.
            snap.ink.sort_by_key(|i| i.row);
        }
        snap.separators.extend_from_slice(&self.separators);
        snap.command_marks.extend(self.marks.iter().cloned());
    }

    /// The command lines run, coloured where their cells still show them.
    fn past_ink(&mut self, t: &Terminal, s: Settings, known: &dyn Fn(&str) -> Option<bool>) {
        self.past.clear();
        self.past_pending = false;
        if !s.past_commands {
            return;
        }
        if self.lexed.len() > LEXED_KEPT {
            self.lexed.clear();
        }
        for c in &self.commands {
            let (Some(id), Some(text), Some((row, col))) = (c.id, &c.text, c.input) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let line = self.lexed.entry(id).or_insert_with(|| Arc::new(lex::read(text, text.len())));
            self.past_pending |= ink::text_ink(t, text, line, (row as usize, col as usize), known, &mut self.past);
        }
    }

    /// The faint line above each prompt but the top row's and the pane's
    /// first (under a restored or welcome line, it only underlined that),
    /// and the mark at the right of each finished command's line where it
    /// is blank.
    fn marks_and_lines(&mut self, t: &Terminal, s: Settings) {
        self.separators.clear();
        self.marks.clear();
        if !s.command_marks {
            return;
        }
        let cols = t.cols();
        for c in &self.commands {
            if let Some(r) = c.prompt_row.filter(|&r| r > 0 && !c.first)
                && self.separators.last() != Some(&r)
            {
                self.separators.push(r);
            }
            let (Some(f), Some((row, _))) = (c.finished, c.input) else {
                continue;
            };
            let Some(text) = mark_text(f) else { continue };
            // A cell's margin at the right, and a blank one before it.
            let w = text.chars().count();
            let Some(start) = cols.checked_sub(w + 1).filter(|&s| s > 0) else {
                continue;
            };
            if blank(t, row, start - 1) {
                self.marks.push(CommandMark {
                    row,
                    col: start as u16,
                    text,
                    ok: f.status == Some(0),
                });
            }
        }
    }

    /// `← cause` at the right of the lines of each failed command's
    /// output in view that the net reads as the cause (at most two), where
    /// the cells there are blank. Read once per output, by its text.
    fn cause_marks(&mut self, t: &Terminal, s: Settings) {
        if !s.failure_cause {
            return;
        }
        if self.causes.len() > CAUSES_KEPT {
            self.causes.clear();
        }
        let (cols, lines) = (t.cols(), t.lines());
        let w = CAUSE.chars().count();
        let Some(start) = cols.checked_sub(w + 1).filter(|&s| s > 0) else {
            return;
        };
        let mut texts: Vec<String> = Vec::new();
        let mut rows: Vec<u16> = Vec::new();
        for c in &self.commands {
            let (Some(f), Some((from, to))) = (c.finished, c.output) else {
                continue;
            };
            if f.status.is_none_or(|s| s == 0) {
                continue;
            }
            texts.clear();
            rows.clear();
            for row in from.0..=to.0 {
                let a = if row == from.0 { from.1 as usize } else { 0 };
                let b = if row == to.0 { to.1 as usize } else { cols };
                if a >= b || row as usize >= lines {
                    continue;
                }
                let r = t.visible_row(row as usize);
                let mut line = String::with_capacity(b - a);
                for col in a..b {
                    let cell = r.cell(col);
                    if cell.is_spacer() {
                        continue;
                    }
                    match cell.content() {
                        Content::Char(ch) => line.push(ch),
                        Content::Empty => line.push(' '),
                        _ => line.push('\u{fffc}'),
                    }
                }
                texts.push(line);
                rows.push(row);
            }
            if texts.is_empty() {
                continue;
            }
            let mut h = DefaultHasher::new();
            texts.hash(&mut h);
            let found = self.causes.entry(h.finish()).or_insert_with(|| {
                let log = Log::from_output(texts.iter().map(String::as_str), true);
                TAGGER.causes(&log, CAUSES_MARKED).into_iter().map(|i| i as u16).collect()
            });
            for &k in found.iter() {
                let row = rows[k as usize];
                if blank(t, row, start - 1) {
                    self.marks.push(CommandMark {
                        row,
                        col: start as u16,
                        text: CAUSE.to_string(),
                        ok: false,
                    });
                }
            }
        }
    }

    /// Plain output coloured, row by row.
    fn output_ink(&mut self, t: &Terminal, s: Settings) {
        self.output.clear();
        if !s.colour_output {
            return;
        }
        if self.rows.len() > ROWS_KEPT {
            self.rows.clear();
        }
        let (cols, lines) = (t.cols(), t.lines());
        let mut out = std::mem::take(&mut self.output);
        for i in 0..self.commands.len() {
            let Some((from, to)) = self.commands[i].output else {
                continue;
            };
            for row in from.0..=to.0 {
                let a = if row == from.0 { from.1 as usize } else { 0 };
                let b = if row == to.0 { to.1 as usize } else { cols };
                if a >= b || row as usize >= lines {
                    continue;
                }
                let colors = self.row_colors(t, row as usize);
                for (r, color) in colors {
                    if r.start as usize >= a && r.end as usize <= b {
                        out.push(Ink {
                            row,
                            cols: r.clone(),
                            color: *color,
                        });
                    }
                }
            }
        }
        self.output = out;
    }

    /// The colours of view row `row`'s text, kept by its content.
    fn row_colors(&mut self, t: &Terminal, row: usize) -> &[(Range<u16>, u8)] {
        let r = t.visible_row(row);
        self.text.clear();
        self.plain.clear();
        for col in 0..t.cols() {
            let cell = r.cell(col);
            self.text.push(if cell.is_spacer() {
                '\0'
            } else {
                match cell.content() {
                    Content::Char(c) => c,
                    Content::Empty => ' ',
                    _ => '\u{fffc}',
                }
            });
            let style = cell.style();
            self.plain
                .push(style.fg == Color::DEFAULT && style.bg == Color::DEFAULT && style.attrs.is_empty());
        }
        let mut h = DefaultHasher::new();
        self.text.hash(&mut h);
        self.plain.hash(&mut h);
        let (text, plain) = (&self.text, &self.plain);
        self.rows.entry(h.finish()).or_insert_with(|| {
            let mut out = Vec::new();
            output::row_colors(text, plain, &mut out);
            out
        })
    }
}

/// Whether view row `row` is blank from column `from` to its end: no
/// text, colour or attributes, so a mark there hides nothing.
fn blank(t: &Terminal, row: u16, from: usize) -> bool {
    let r = t.visible_row(row as usize);
    (from..t.cols()).all(|col| {
        let cell = r.cell(col);
        let style = cell.style();
        matches!(cell.content(), Content::Empty | Content::Char(' ')) && style.bg == Color::DEFAULT && style.attrs.is_empty()
    })
}

/// The mark's text: `✓` when the command exited 0, `✗` and the status
/// otherwise, with the time taken when it was a second or more.
/// Nothing when the shell gave no status.
pub fn mark_text(f: Finished) -> Option<String> {
    let status = f.status?;
    let took = f.took_ms.filter(|&ms| ms >= 1000).map(duration);
    Some(match (status, took) {
        (0, None) => "✓".to_string(),
        (0, Some(d)) => format!("✓ {d}"),
        (code, None) => format!("✗ {code}"),
        (code, Some(d)) => format!("✗ {code} · {d}"),
    })
}

/// A time taken, short: `1.2s`, `14s`, `2m03s`, `1h02m`.
pub fn duration(ms: u64) -> String {
    let tenths = ms / 100;
    let secs = ms / 1000;
    match secs {
        _ if tenths < 100 => format!("{}.{}s", tenths / 10, tenths % 10),
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, secs / 60 % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::{ImagePool, Term};

    #[test]
    fn durations() {
        assert_eq!(duration(1_000), "1.0s");
        assert_eq!(duration(1_249), "1.2s");
        assert_eq!(duration(9_999), "9.9s");
        assert_eq!(duration(14_700), "14s");
        assert_eq!(duration(123_000), "2m03s");
        assert_eq!(duration(3_599_000), "59m59s");
        assert_eq!(duration(3_720_000), "1h02m");
    }

    #[test]
    fn mark_texts() {
        let f = |status, took_ms| Finished { status, took_ms };
        assert_eq!(mark_text(f(Some(0), None)).as_deref(), Some("✓"));
        // Under a second: the mark alone.
        assert_eq!(mark_text(f(Some(0), Some(999))).as_deref(), Some("✓"));
        assert_eq!(mark_text(f(Some(0), Some(1_200))).as_deref(), Some("✓ 1.2s"));
        assert_eq!(mark_text(f(Some(2), Some(300))).as_deref(), Some("✗ 2"));
        assert_eq!(mark_text(f(Some(2), Some(1_200))).as_deref(), Some("✗ 2 · 1.2s"));
        assert_eq!(mark_text(f(None, Some(5_000))), None);
    }

    const ON: Settings = Settings {
        past_commands: true,
        command_marks: true,
        colour_output: true,
        failure_cause: false,
    };

    /// A terminal 30 wide, 8 high; `run` runs a command with zsh's marks:
    /// the prompt, the line typed (`typed`, as drawn), C with the
    /// shell's text (`sent`), the output, D with `status`, the clock
    /// moving `took` milliseconds.
    fn term() -> Term {
        Term::new(30, 8, 100, ImagePool::process())
    }

    fn run(t: &mut Term, typed: &str, sent: &str, out: &str, status: u32, took: u64) {
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        t.advance(typed.as_bytes());
        t.advance(b"\r\n");
        let enc = sent.replace('%', "%25").replace(';', "%3B").replace(' ', "%20");
        t.state.set_clock(10_000);
        t.advance(format!("\x1b]133;C;cmdline_url={enc}\x07").as_bytes());
        t.advance(out.as_bytes());
        t.state.set_clock(10_000 + took);
        t.advance(format!("\x1b]133;D;{status}\x07").as_bytes());
    }

    fn frame(t: &Term, s: Settings) -> Snapshot {
        let mut snap = Snapshot::default();
        let mut m = Marks::default();
        m.update(&t.state, s, &|name| Some(name == "ls"), &mut snap);
        snap
    }

    fn inks(snap: &Snapshot, row: u16) -> Vec<(Range<u16>, u8)> {
        snap.ink
            .iter()
            .filter(|i| i.row == row)
            .map(|i| (i.cols.clone(), i.color))
            .collect()
    }

    #[test]
    fn past_commands_keep_their_colours() {
        let mut t = term();
        run(&mut t, "ls -l \"a\"", "ls -l \"a\"", "x\r\n", 0, 10);
        run(&mut t, "nope $X", "nope $X", "", 127, 10);
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        let snap = frame(&t, ON);
        // `$ ` takes columns 0 and 1.
        assert_eq!(inks(&snap, 0), [(2..4, 2), (5..7, 4), (8..11, 3)]);
        assert_eq!(inks(&snap, 2), [(2..6, 1), (7..9, 6)]);
        let off = Settings {
            past_commands: false,
            ..ON
        };
        assert!(frame(&t, off).ink.iter().all(|i| i.row != 0 && i.row != 2));
    }

    #[test]
    fn a_command_that_does_not_match_its_cells() {
        let mut t = term();
        // The shell sent another text than the screen shows from `-`.
        run(&mut t, "ls -l -a", "ls -x \"q\"", "", 0, 10);
        let snap = frame(&t, ON);
        // `ls` and the `-` they share; nothing after the first mismatch.
        assert_eq!(inks(&snap, 0), [(2..4, 2), (5..6, 4)]);
        // Coloured by the shell: kept.
        let mut t = term();
        run(&mut t, "\x1b[31mls\x1b[0m -l", "ls -l", "", 0, 10);
        assert_eq!(inks(&frame(&t, ON), 0), [(5..7, 4)]);
    }

    #[test]
    fn separators_and_marks() {
        let mut t = term();
        run(&mut t, "ls", "ls", "", 0, 1_200);
        run(&mut t, "false", "false", "", 2, 300);
        run(&mut t, "sleep 70", "sleep 70", "", 0, 70_000);
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        let snap = frame(&t, ON);
        // Prompts on rows 0, 1, 2 and 3; not the top row.
        assert_eq!(snap.separators, [1, 2, 3]);
        let marks: Vec<_> = snap.command_marks.iter().map(|m| (m.row, m.col, m.text.as_str(), m.ok)).collect();
        assert_eq!(marks, [(0, 23, "✓ 1.2s", true), (1, 26, "✗ 2", false), (2, 22, "✓ 1m10s", true)]);
        let off = Settings {
            command_marks: false,
            ..ON
        };
        let snap = frame(&t, off);
        assert!(snap.separators.is_empty() && snap.command_marks.is_empty());
    }

    #[test]
    fn a_failed_commands_cause_is_marked() {
        let mut t = Term::new(60, 14, 100, ImagePool::process());
        let out =
            "   Compiling x v0.1.0\r\nerror[E0425]: cannot find value `y`\r\n  --> src/lib.rs:3:5\r\nerror: could not compile `x`\r\n";
        run(&mut t, "cargo build", "cargo build", out, 101, 2_000);
        // The same output from a command that worked: nothing to explain.
        run(&mut t, "cat log", "cat log", out, 0, 10);
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        let on = Settings { failure_cause: true, ..ON };
        let causes = |s| -> Vec<(u16, u16)> {
            frame(&t, s)
                .command_marks
                .iter()
                .filter(|m| m.text == CAUSE)
                .map(|m| (m.row, m.col))
                .collect()
        };
        // The prompt on row 0, the output from row 1: the error[E0425]
        // line, not the could-not-compile fallout after it.
        assert_eq!(causes(on), [(2, 52)]);
        assert!(causes(ON).is_empty());
        // Only cause marks: the setting alone, without the other marks.
        let alone = Settings {
            failure_cause: true,
            ..Settings::default()
        };
        assert_eq!(causes(alone), [(2, 52)]);
    }

    /// Under a line written before the shell's first prompt (restored
    /// from, the welcome line) there's no command to mark off: no line
    /// there, only between commands.
    #[test]
    fn no_line_above_the_first_prompt() {
        let mut t = term();
        t.advance(b"\x1b[2mrestored from Mon 2 Nov 13:24\x1b[0m\r\n");
        run(&mut t, "ls", "ls", "", 0, 10);
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        // Prompts on rows 1 and 2: only the second is marked off.
        assert_eq!(frame(&t, ON).separators, [2]);
    }

    #[test]
    fn a_mark_only_over_blank_cells() {
        let mut t = term();
        // A right prompt where the mark would go.
        run(&mut t, "ls\x1b[26G[rp]", "ls", "", 0, 10);
        assert!(frame(&t, ON).command_marks.is_empty());
        // Not finished: no mark.
        let mut t = term();
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07");
        assert!(frame(&t, ON).command_marks.is_empty());
    }

    #[test]
    fn output_coloured_only_plain_and_only_output() {
        let mut t = term();
        run(&mut t, "git log", "git log", "3f2a9c1 ok\r\n\x1b[31merror\x1b[0m 12%\r\n", 0, 10);
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07echo 1234567");
        let snap = frame(&t, ON);
        assert_eq!(inks(&snap, 1), [(0..7, 3), (8..10, 2)]);
        // `error` is the program's red: left; `12%` coloured.
        assert_eq!(inks(&snap, 2), [(6..9, 6)]);
        // The command line being typed isn't output.
        assert_eq!(inks(&snap, 3), []);
        let off = Settings {
            colour_output: false,
            ..ON
        };
        assert!(frame(&t, off).ink.iter().all(|i| i.row == 0));
    }

    #[test]
    fn nothing_on_the_alternate_screen() {
        let mut t = term();
        run(&mut t, "ls", "ls", "ok 12\r\n", 0, 2_000);
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07vim\r\n\x1b]133;C\x07\x1b[?1049h");
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07ls 12 ok\r\n");
        let snap = frame(&t, ON);
        assert!(snap.ink.is_empty() && snap.separators.is_empty() && snap.command_marks.is_empty());
    }
}
