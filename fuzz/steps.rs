//! The input format of the `parse` fuzz target, shared with the corpus
//! replay test (`crates/term-core/tests/fuzz_corpus.rs`), which includes this
//! file with `#[path]`. Keep it free of dependencies other than `term-core`.
//!
//! An input is mostly raw terminal output, so captures of real programs work
//! as seeds and libFuzzer's dictionary of escape sequences applies directly.
//! The byte 0xFF (never valid in UTF-8) starts a command:
//!
//! | bytes                | step                                            |
//! |----------------------|-------------------------------------------------|
//! | `FF 'R' c l`         | resize to `c % 200 + 1` cols × `l % 60 + 1` lines |
//! | `FF 'V' d`           | scroll the view by `d` (as `i8`)                |
//! | `FF 'S' al ac hl hc` | select from visible (al, ac) to (hl, hc)        |
//! | `FF 'C'`             | clear the selection                             |
//! | `FF` anything else   | a literal 0xFF byte; the next byte is read again |
//!
//! A command cut short by the end of the input is dropped. Other bytes are fed
//! to the terminal in [`Step::Bytes`] of at most [`MAX_CHUNK`] bytes, so long
//! inputs also test escape sequences split across `advance` calls.

use std::fmt;

use term_core::{Selection, Term};

/// Longest run of bytes in one `advance` call.
pub const MAX_CHUNK: usize = 64;
/// Scrollback lines kept by the terminal under test: small, so the oldest
/// lines get dropped often.
pub const SCROLLBACK: usize = 200;

pub const CMD: u8 = 0xFF;

pub enum Step {
    Bytes(Vec<u8>),
    /// Columns and lines, already clamped to 1..=200 × 1..=60.
    Resize(u16, u16),
    ScrollView(i8),
    /// Anchor line and column, head line and column, in visible coordinates
    /// (wrapped into the grid when applied).
    Select(u8, u8, u8, u8),
    ClearSelection,
}

impl fmt::Debug for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Bytes(b) => write!(f, "Bytes(b\"{}\")", b.escape_ascii()),
            Step::Resize(c, l) => write!(f, "Resize({c}, {l})"),
            Step::ScrollView(d) => write!(f, "ScrollView({d})"),
            Step::Select(al, ac, hl, hc) => write!(f, "Select({al}, {ac}, {hl}, {hc})"),
            Step::ClearSelection => write!(f, "ClearSelection"),
        }
    }
}

pub fn decode(data: &[u8]) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut run: Vec<u8> = Vec::new();
    let flush = |run: &mut Vec<u8>, steps: &mut Vec<Step>| {
        for chunk in run.chunks(MAX_CHUNK) {
            steps.push(Step::Bytes(chunk.to_vec()));
        }
        run.clear();
    };
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        i += 1;
        if b != CMD {
            run.push(b);
            continue;
        }
        let rest = &data[i..];
        let (step, used) = match rest {
            [b'R', c, l, ..] => (Some(Step::Resize(u16::from(*c) % 200 + 1, u16::from(*l) % 60 + 1)), 3),
            [b'V', d, ..] => (Some(Step::ScrollView(*d as i8)), 2),
            [b'S', al, ac, hl, hc, ..] => (Some(Step::Select(*al, *ac, *hl, *hc)), 5),
            [b'C', ..] => (Some(Step::ClearSelection), 1),
            // A known command without all its operands: drop it.
            [b'R' | b'V' | b'S', ..] => break,
            _ => {
                run.push(CMD);
                continue;
            }
        };
        flush(&mut run, &mut steps);
        steps.extend(step);
        i += used;
    }
    flush(&mut run, &mut steps);
    steps
}

pub fn new_term() -> Term {
    Term::new(80, 24, SCROLLBACK)
}

pub fn apply(term: &mut Term, step: &Step) {
    let t = &mut term.state;
    match *step {
        Step::Bytes(ref b) => term.advance(b),
        Step::Resize(c, l) => t.resize(usize::from(c), usize::from(l)),
        Step::ScrollView(d) => t.scroll_view(isize::from(d)),
        Step::Select(al, ac, hl, hc) => {
            let (lines, cols) = (t.lines(), t.cols());
            let anchor = t.point_at(usize::from(al) % lines, usize::from(ac) % cols);
            let head = t.point_at(usize::from(hl) % lines, usize::from(hc) % cols);
            t.set_selection(Some(Selection::new(anchor, head)));
        }
        Step::ClearSelection => t.set_selection(None),
    }
}

/// Reads everything the renderer and the clipboard would read, so a broken
/// state panics here rather than later in the app.
pub fn read_back(term: &Term) {
    let t = &term.state;
    assert!(t.display_offset() <= t.scrollback_len(), "view scrolled past the scrollback");
    for line in 0..t.lines() {
        // History rows may be wider or narrower than the screen after a
        // resize (no re-wrap yet), so only check that they can be read.
        let _ = t.visible_row(line);
    }
    let _ = t.selection_text();
}
