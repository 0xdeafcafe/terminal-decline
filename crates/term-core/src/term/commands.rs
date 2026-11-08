//! The commands the shell ran, as its OSC 133 marks show them in the
//! view (docs/design/command-marks.md): where each prompt, command line
//! and output is, the text that was run, and how and when it finished.
//!
//! A C mark keeps its command's number in the log here (in the mark's
//! `status`, which only D marks otherwise use), so the text and times
//! follow it into history and through reflow without changing how marks
//! are stored. The log keeps the latest [`LOG_KEPT`] commands.

use std::collections::VecDeque;
use std::sync::Arc;

use super::Terminal;
use super::history::Piece;
use crate::cell::{Mark, MarkKind};
use crate::term::Point;

/// Commands kept in the log; older ones lose their text and times.
const LOG_KEPT: usize = 1024;

/// A command run: its text and when it started and finished, in the
/// app's clock ([`Terminal::set_clock`]).
#[derive(Clone, Debug)]
struct Record {
    id: u32,
    text: Option<Arc<str>>,
    started: u64,
    ended: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Log {
    /// Oldest first; ids only go up.
    records: VecDeque<Record>,
    /// The command started last, until its D mark.
    running: Option<u32>,
    /// The last number given (the first is 1).
    last_id: u32,
}

impl Log {
    /// A command ran (its C mark): its number.
    pub(super) fn started(&mut self, text: Option<String>, now: u64) -> u32 {
        self.last_id = self.last_id.wrapping_add(1).max(1);
        let id = self.last_id;
        if self.records.len() == LOG_KEPT {
            self.records.pop_front();
        }
        self.records.push_back(Record {
            id,
            text: text.map(Arc::from),
            started: now,
            ended: None,
        });
        self.running = Some(id);
        id
    }

    /// The running command finished (its D mark).
    pub(super) fn finished(&mut self, now: u64) {
        if let Some(id) = self.running.take()
            && let Ok(i) = self.records.binary_search_by_key(&id, |r| r.id)
        {
            self.records[i].ended = Some(now);
        }
    }

    fn get(&self, id: u32) -> Option<&Record> {
        let i = self.records.binary_search_by_key(&id, |r| r.id).ok()?;
        self.records.get(i)
    }
}

/// A command that finished (its D mark).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finished {
    /// Its exit status, if the shell gave one.
    pub status: Option<u32>,
    /// From its C mark to its D mark, in milliseconds, when both times
    /// are known.
    pub took_ms: Option<u64>,
}

/// A prompt in view, or the one above it whose command's output is in
/// view, with what its marks say: view rows and columns throughout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewCommand {
    /// Its command's number (from its C mark): the same wherever it
    /// scrolls, for caching by. `None` before it runs.
    pub id: Option<u32>,
    /// The view row of the prompt's first row (its A mark), if in view.
    pub prompt_row: Option<u16>,
    /// The pane's first prompt: none comes before it, so there's no
    /// command above to mark it off from (its line isn't drawn).
    pub first: bool,
    /// Where the command line starts (its B mark), if in view.
    pub input: Option<(u16, u16)>,
    /// The command line run: the shell's own (`cmdline_url`), else read
    /// from the cells when it ran. `None` before it runs, or when the log
    /// no longer has it.
    pub text: Option<Arc<str>>,
    /// Its output in view: from (row, column) to (row, column), the end
    /// not included. Output runs from the C mark to the D mark (or the
    /// next prompt); while the command runs, to the end of the view.
    pub output: Option<((u16, u16), (u16, u16))>,
    pub finished: Option<Finished>,
}

/// The worked/failed mark drawn at the right of a finished command's
/// line (`✓ 1.2s`, `✗ 2 · 1.2s`): put in the snapshot by the app.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CommandMark {
    pub row: u16,
    pub col: u16,
    pub text: String,
    /// Exit status 0: drawn dim green, else dim red.
    pub ok: bool,
}

/// How a command run ended, for its bar down the left edge: `Worked`
/// (exit 0) green, `Failed` red, `Running` and `Unknown` (no exit
/// status, or not run yet) dim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RunState {
    Running,
    Worked,
    Failed,
    #[default]
    Unknown,
}

/// The bar down the left edge of a command run (command-marks design):
/// the view rows from its prompt to the last row of its output, put in
/// the snapshot by the app.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RunBar {
    pub rows: std::ops::Range<u16>,
    pub state: RunState,
}

/// Each view row's pieces of stored lines, in order.
struct ViewMap {
    pieces: Vec<(u16, Piece)>,
    rows: u16,
}

impl ViewMap {
    fn first_line(&self) -> Option<u64> {
        self.pieces.first().map(|(_, p)| p.line)
    }

    fn last_line(&self) -> Option<u64> {
        self.pieces.last().map(|(_, p)| p.line)
    }

    /// Where `p` is shown: the row of the piece of its line it falls in
    /// (or the last one starting before it, for a mark past the line's
    /// text), and its column there, kept inside `cols`.
    fn pos(&self, p: Point, cols: usize) -> Option<(u16, u16)> {
        let start = self.pieces.partition_point(|(_, q)| q.line < p.line);
        let (row, piece) = self.pieces[start..]
            .iter()
            .take_while(|(_, q)| q.line == p.line)
            .filter(|(_, q)| q.from <= p.col)
            .last()?;
        let col = (piece.at + (p.col - piece.from)).min(cols.saturating_sub(1));
        Some((*row, col.min(u16::MAX as usize) as u16))
    }
}

impl Terminal {
    /// The prompts in view, and the one above the view whose output
    /// reaches into it, top to bottom, into `out` (emptied first).
    /// Nothing on the alternate screen, which has no prompts.
    pub fn view_commands(&self, out: &mut Vec<ViewCommand>) {
        out.clear();
        if self.alt_active {
            return;
        }
        let mut map = ViewMap {
            pieces: Vec::new(),
            rows: 0,
        };
        let mut pieces = Vec::new();
        for (r, at) in self.view_rows().into_iter().enumerate() {
            self.view_pieces(at, &mut pieces);
            map.pieces.extend(pieces.iter().map(|p| (r as u16, *p)));
            map.rows = r as u16 + 1;
        }
        let (Some(first), Some(last)) = (map.first_line(), map.last_line()) else {
            return;
        };
        let mut prompt = self.previous_prompt(first + 1).or_else(|| self.next_prompt(first));
        while let Some(p) = prompt {
            if p.line > last {
                break;
            }
            if let Some(c) = self.view_command(p, &map, first, last) {
                out.push(c);
            }
            prompt = self.next_prompt(p.line);
        }
    }

    /// [`Terminal::view_commands`]' entry for the prompt at `prompt`;
    /// `None` if nothing of it is in view.
    fn view_command(&self, prompt: Point, map: &ViewMap, first: u64, last: u64) -> Option<ViewCommand> {
        let mut input = None;
        let mut output: Option<(Point, Mark)> = None;
        let mut end: Option<(Point, Mark)> = None;
        let mut next_prompt = None;
        for (p, m) in self.marks_after_prompt(prompt) {
            match m.kind {
                MarkKind::Input if output.is_none() => input = Some(p),
                MarkKind::Output if output.is_none() => output = Some((p, m)),
                MarkKind::End => {
                    end = Some((p, m));
                    break;
                }
                MarkKind::Prompt => {
                    next_prompt = Some(p);
                    break;
                }
                _ => {}
            }
        }
        let cols = self.cols;
        let in_view = |p: Point| p.line >= first && p.line <= last;
        let record = output.and_then(|(_, m)| self.command_log.get(m.status?));
        let output_view = output.and_then(|(c, _)| {
            if c.line > last {
                return None;
            }
            let from = if c.line < first { (0, 0) } else { map.pos(c, cols)? };
            let stop = end.map(|(p, _)| p).or(next_prompt);
            let to = match stop {
                Some(p) if p.line < first => return None,
                Some(p) if p.line <= last => map.pos(p, cols)?,
                _ => (map.rows, 0),
            };
            (from < to).then_some((from, to))
        });
        let c = ViewCommand {
            id: output.and_then(|(_, m)| m.status),
            prompt_row: in_view(prompt).then(|| map.pos(prompt, cols)).flatten().map(|p| p.0),
            first: self.previous_prompt(prompt.line).is_none(),
            input: input.filter(|&p| in_view(p)).and_then(|p| map.pos(p, cols)),
            text: record.and_then(|r| r.text.clone()),
            output: output_view,
            finished: end.map(|(_, m)| Finished {
                status: m.status,
                took_ms: record.and_then(|r| Some(r.ended?.saturating_sub(r.started))),
            }),
        };
        (c.prompt_row.is_some() || c.input.is_some() || c.output.is_some()).then_some(c)
    }
}
