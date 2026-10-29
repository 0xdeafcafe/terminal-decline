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
//! | `FF 'A' w`           | set the ambiguous width to `w % 2 + 1` cells    |
//! | `FF 'P' n`           | pull `n % 8` history lines back onto the screen |
//! | `FF 'L' n`           | set the scrollback limit to `n` lines           |
//! | `FF 'H' l c`         | hover at visible (l, c) and take a snapshot     |
//! | `FF 'O' l c`         | select the command output at visible (l, c)     |
//! | `FF` anything else   | a literal 0xFF byte; the next byte is read again |
//!
//! A command cut short by the end of the input is dropped. Other bytes are fed
//! to the terminal in [`Step::Bytes`] of at most [`MAX_CHUNK`] bytes, so long
//! inputs also test escape sequences split across `advance` calls.

use std::fmt;

use term_core::{Selection, Snapshot, Term};

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
    /// The ambiguous-width setting: 1 or 2.
    Ambiguous(u8),
    /// Pull this many history lines back (0..8).
    PullBack(u8),
    /// The scrollback limit in lines.
    Limit(u8),
    /// The pointer at a visible line and column (wrapped into the grid),
    /// then a snapshot, which highlights the link there.
    Hover(u8, u8),
    /// Select the output of the command at a visible line and column.
    SelectOutput(u8, u8),
}

impl fmt::Debug for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Bytes(b) => write!(f, "Bytes(b\"{}\")", b.escape_ascii()),
            Step::Resize(c, l) => write!(f, "Resize({c}, {l})"),
            Step::ScrollView(d) => write!(f, "ScrollView({d})"),
            Step::Select(al, ac, hl, hc) => write!(f, "Select({al}, {ac}, {hl}, {hc})"),
            Step::ClearSelection => write!(f, "ClearSelection"),
            Step::Ambiguous(w) => write!(f, "Ambiguous({w})"),
            Step::PullBack(n) => write!(f, "PullBack({n})"),
            Step::Limit(n) => write!(f, "Limit({n})"),
            Step::Hover(l, c) => write!(f, "Hover({l}, {c})"),
            Step::SelectOutput(l, c) => write!(f, "SelectOutput({l}, {c})"),
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
            [b'A', w, ..] => (Some(Step::Ambiguous(w % 2 + 1)), 2),
            [b'H', l, c, ..] => (Some(Step::Hover(*l, *c)), 3),
            [b'O', l, c, ..] => (Some(Step::SelectOutput(*l, *c)), 3),
            [b'P', n, ..] => (Some(Step::PullBack(n % 8)), 2),
            [b'L', n, ..] => (Some(Step::Limit(*n)), 2),
            // A known command without all its operands: drop it.
            [b'R' | b'V' | b'S' | b'A' | b'H' | b'O' | b'P' | b'L', ..] => break,
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

/// Whether the terminal under test handles images (kitty graphics, iTerm2
/// files, Sixel): on unless `TD_FUZZ_IMAGES=0` is in the environment. This
/// stands in for the app's hidden `TD_IMAGES=1` switch, which the
/// process-wide pool reads once and which is off by default: with it off, a
/// fuzz run would never get past the first byte of an image.
pub fn images_on() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TD_FUZZ_IMAGES").is_none_or(|v| v != "0"))
}

/// The image budget of the terminal under test: small, so images are
/// evicted all the time.
pub const IMAGE_BUDGET: usize = 4 << 20;

/// A terminal whose style tables sweep at 8 ids, so sweeps and
/// renumberings happen every few prints and every holder of a style id is
/// exercised (§5.2 of the cells design). With [`images_on`] its image pool
/// is a small one with images enabled.
pub fn new_term() -> Term {
    let pool = if images_on() {
        term_core::ImagePool::new(IMAGE_BUDGET)
    } else {
        term_core::ImagePool::process()
    };
    let mut t = Term::new(80, 24, SCROLLBACK, pool);
    t.state.set_sweep_at(Some(8));
    t
}

pub fn apply(term: &mut Term, step: &Step) {
    let t = &mut term.state;
    match *step {
        Step::Bytes(ref b) => {
            term.advance(b);
            // What the app does after each parse slice. With no host to run
            // the decode and read jobs, they are dropped.
            let mut jobs = Vec::new();
            term.state.take_image_jobs(&mut jobs);
        }
        Step::Resize(c, l) => t.resize(usize::from(c), usize::from(l)),
        Step::ScrollView(d) => t.scroll_view(isize::from(d)),
        Step::Select(al, ac, hl, hc) => {
            let (lines, cols) = (t.lines(), t.cols());
            let anchor = t.point_at(usize::from(al) % lines, usize::from(ac) % cols);
            let head = t.point_at(usize::from(hl) % lines, usize::from(hc) % cols);
            t.set_selection(Some(Selection::new(anchor, head)));
        }
        Step::ClearSelection => t.set_selection(None),
        Step::Ambiguous(w) => t.set_ambiguous_width(w),
        Step::PullBack(n) => t.pull_back_for_test(usize::from(n)),
        Step::Limit(n) => t.set_scrollback_limit(usize::from(n)),
        Step::Hover(l, c) => {
            let at = t.point_at(usize::from(l) % t.lines(), usize::from(c) % t.cols());
            t.set_hover(Some(at));
            let mut snap = Snapshot::default();
            t.snapshot(&mut snap, |_| false);
            assert_eq!(snap.rows.len(), t.lines());
        }
        Step::SelectOutput(l, c) => {
            let at = t.point_at(usize::from(l) % t.lines(), usize::from(c) % t.cols());
            t.select_output(at);
        }
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
        // Links, marks and the prompt and output lookups read every line.
        let p = t.point_at(line, (line * 7) % t.cols());
        let _ = t.link_at(p);
        let _ = t.marks_at(p.line);
        let _ = (t.previous_prompt(p.line), t.next_prompt(p.line));
        if let Some((a, b)) = t.command_output(p) {
            assert!(a <= b, "output from {a:?} to {b:?}");
        }
    }
    let _ = t.selection_text();
    // Every stored line reads through: sections, text, checkpoints.
    if let Err(e) = t.check_history() {
        panic!("history broken: {e}");
    }
}
