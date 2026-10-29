//! Row change counters and the snapshot the renderer draws from (§9 of
//! the cells design).
//!
//! Every screen row carries a `seq`, its change counter: it changes when
//! the row's content does, and stays when the row only moves (a scroll, a
//! region scroll, IL/DL). [`Terminal::snapshot`] copies the visible rows the
//! renderer's row cache doesn't have yet, keyed by [`RowKey`], along with
//! the styles interned since the last snapshot, the highlight ranges of
//! each row and the cursor.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{CursorShape, Terminal};
use crate::cell::{Cell, Row, Width};
use crate::style::Style;

/// Where [`Terminal`] ids come from: unique in the process, so a terminal
/// made to replace another (after a caught parser panic) can't be taken
/// for it.
static TERMINAL_IDS: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_terminal_id() -> u64 {
    TERMINAL_IDS.fetch_add(1, Ordering::Relaxed)
}

/// Which screen a snapshot shows, and so which of the renderer's two style
/// mirrors its screen rows' style ids refer to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ScreenId {
    #[default]
    Primary,
    Alternate,
}

/// What the renderer's row cache keys a row by.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RowKey {
    /// A screen row, by its change counter: unique to its content.
    Screen(u64),
    /// A history row: its stored line, and the column of the stored line
    /// it starts at (0 until history is shown at other widths).
    History { line: u64, col: u16 },
}

/// What a row is, apart from its cells.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct RowFlags(u8);

impl RowFlags {
    /// The line continues onto the next row (soft wrap).
    pub const WRAPPED: RowFlags = RowFlags(1 << 0);
    /// The row comes from history: its cells' style ids index
    /// [`Snapshot::history_styles`], not the screen's table.
    pub const HISTORY: RowFlags = RowFlags(1 << 1);

    #[inline]
    pub const fn contains(self, other: RowFlags) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline]
    fn set(&mut self, other: RowFlags, on: bool) {
        if on {
            self.0 |= other.0;
        }
    }
}

/// One visible row of a [`Snapshot`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SnapRow {
    pub key: RowKey,
    pub flags: RowFlags,
    /// Its cells in [`Snapshot::cells`]; `None` when the renderer said it
    /// already has this row. A history row may have fewer cells than the
    /// screen is wide (the rest are blank) or more (cut off).
    pub cells: Option<Range<u32>>,
    /// Its highlights in [`Snapshot::highlights`], left to right.
    pub highlights: Range<u32>,
}

/// Columns of one row drawn highlighted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Highlight {
    pub cols: Range<u16>,
    pub kind: HighlightKind,
}

/// Search hits come with search, and the hovered link with the link table;
/// only the selection is highlighted so far.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HighlightKind {
    Selection,
    SearchHit,
    CurrentSearchHit,
    HoveredLink,
}

/// The cursor, as the renderer draws it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SnapCursor {
    /// View row and column, when the cursor is in view.
    pub at: Option<(u16, u16)>,
    pub shape: CursorShape,
    /// The program hasn't hidden it (DECTCEM).
    pub visible: bool,
    /// The program asked for a blinking cursor.
    pub blinking: bool,
    /// It is on the first cell of a wide character, so it covers two cells.
    pub wide: bool,
}

/// What the renderer needs to draw a frame, taken under the lock by
/// [`Terminal::snapshot`]. The renderer owns it and passes it in again
/// each frame, so its buffers are reused: in steady state a snapshot
/// allocates nothing.
///
/// The renderer drops its cached screen rows when `terminal_id` changes;
/// its cached history rows when `terminal_id` or `history_epoch` changes,
/// or the width does; and the cached history rows at or after
/// `popped_from` when that is set. It keeps a mirror of each screen's
/// style table: when `styles_epoch` changes for `screen`, it clears that
/// mirror first (`style_updates` is then the whole table), and in every
/// case applies `style_updates` to it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub terminal_id: u64,
    pub history_epoch: u64,
    pub popped_from: Option<u64>,
    pub screen: ScreenId,
    pub styles_epoch: u64,
    /// The terminal's change counter when taken: every screen row's
    /// `seq` is at most this.
    pub counter: u64,
    pub cols: u16,
    pub lines: u16,
    /// One per visible row, top to bottom.
    pub rows: Vec<SnapRow>,
    /// The cells of the rows copied this time, back to back. Screen rows'
    /// style ids index `screen`'s table (the mirror); history rows' index
    /// `history_styles`.
    pub cells: Vec<Cell>,
    /// `screen`'s style ids interned since the last snapshot, with their
    /// styles (the whole table after a renumbering, a new table or a new
    /// terminal).
    pub style_updates: Vec<(u32, Style)>,
    /// The styles of the history rows copied this time.
    pub history_styles: Vec<Style>,
    pub highlights: Vec<Highlight>,
    pub cursor: SnapCursor,
    /// The epoch of each screen's table (primary, alternate) whose styles
    /// have all been sent through this snapshot, so a renderer that is new
    /// to the terminal, or hasn't seen a screen's table yet, is sent the
    /// whole table.
    mirrored: [Option<u64>; 2],
    /// Decoding buffers for history rows, kept to save allocating.
    row: Row,
    row_styles: Vec<Style>,
}

impl Snapshot {
    /// The cells copied for `row`, if it was copied.
    pub fn row_cells(&self, row: &SnapRow) -> Option<&[Cell]> {
        let r = row.cells.as_ref()?;
        Some(&self.cells[r.start as usize..r.end as usize])
    }

    /// `row`'s highlights.
    pub fn row_highlights(&self, row: &SnapRow) -> &[Highlight] {
        &self.highlights[row.highlights.start as usize..row.highlights.end as usize]
    }
}

fn to_u16(n: usize) -> u16 {
    n.min(u16::MAX as usize) as u16
}

/// [`Terminal::touch_row`] for a row the caller already holds.
#[inline(always)]
pub(super) fn touch(row: &mut Row, counter: &mut u64, batch_start: u64) {
    if row.seq < batch_start {
        *counter += 1;
        row.seq = *counter;
    }
}

impl Terminal {
    /// Copy what the renderer needs for a frame into `out` (§9.2). `have`
    /// says whether the renderer's row cache already has a row; those rows
    /// aren't copied. Takes the styles interned since the last snapshot,
    /// so every snapshot must reach the renderer.
    ///
    /// `out` is the previous snapshot, and the terminal compares against
    /// it: `have` is never asked about a row the renderer must drop by the
    /// rules on [`Snapshot`] (a new terminal, cleared history, another
    /// width, lines pulled back). Those rows are copied, so the renderer
    /// can apply the rules after taking the snapshot.
    pub fn snapshot(&mut self, out: &mut Snapshot, have: impl Fn(RowKey) -> bool) {
        let cols = to_u16(self.cols);
        let same_terminal = out.terminal_id == self.terminal_id;
        let same_history = same_terminal && out.history_epoch == self.history_epoch && out.cols == cols;
        let popped = self.popped_from;
        out.rows.clear();
        out.cells.clear();
        out.style_updates.clear();
        out.history_styles.clear();
        out.highlights.clear();
        out.terminal_id = self.terminal_id;
        out.history_epoch = self.history_epoch;
        out.popped_from = self.popped_from.take();
        out.screen = if self.alt_active { ScreenId::Alternate } else { ScreenId::Primary };
        if !same_terminal {
            out.mirrored = [None; 2];
        }
        let mirrored = &mut out.mirrored[self.alt_active as usize];
        if *mirrored != Some(self.styles.epoch()) {
            self.styles.resend_all();
            *mirrored = Some(self.styles.epoch());
        }
        self.styles.drain_updates(&mut out.style_updates);
        out.styles_epoch = self.styles.epoch();
        out.counter = self.counter;
        out.cols = cols;
        out.lines = to_u16(self.lines);

        let selection = self.selection.map(|s| s.ordered());
        let history = self.scrollback.len();
        let first = history - self.display_offset;
        for view_line in 0..self.lines {
            let line = self.point_at(view_line, 0).line;
            let highlights = out.highlights.len() as u32;
            if let Some((a, b)) = selection
                && (a.line..=b.line).contains(&line)
            {
                let start = if line == a.line { a.col } else { 0 };
                let end = if line == b.line { b.col + 1 } else { self.cols };
                let end = end.min(self.cols);
                if start < end {
                    out.highlights.push(Highlight {
                        cols: to_u16(start)..to_u16(end),
                        kind: HighlightKind::Selection,
                    });
                }
            }
            let highlights = highlights..out.highlights.len() as u32;

            let i = first + view_line;
            let mut flags = RowFlags::default();
            let start = out.cells.len() as u32;
            let (key, copied) = if i < history {
                let key = RowKey::History { line, col: 0 };
                flags.set(RowFlags::HISTORY, true);
                flags.set(RowFlags::WRAPPED, self.scrollback.wrapped(i));
                let kept = same_history && popped.is_none_or(|p| line < p) && have(key);
                let copied = !kept && self.scrollback.get_into(i, &mut out.row, &mut out.row_styles);
                if copied {
                    // The row's own style list joins the snapshot's, and its
                    // cells' ids move up by where it starts.
                    let base = out.history_styles.len() as u32;
                    out.history_styles.extend_from_slice(&out.row_styles);
                    out.cells.extend(
                        out.row
                            .cells
                            .iter()
                            .map(|c| c.with_style(Cell::template(c.style_id() + base, c.shows_empty()))),
                    );
                }
                (key, copied)
            } else {
                let row = &self.screen[i - history];
                let key = RowKey::Screen(row.seq);
                flags.set(RowFlags::WRAPPED, row.wrapped);
                let copied = !(same_terminal && have(key));
                if copied {
                    out.cells.extend_from_slice(&row.cells);
                }
                (key, copied)
            };
            out.rows.push(SnapRow {
                key,
                flags,
                cells: copied.then_some(start..out.cells.len() as u32),
                highlights,
            });
        }

        let at = self.cursor_in_view();
        out.cursor = SnapCursor {
            at: at.map(|(r, c)| (to_u16(r), to_u16(c))),
            shape: self.cursor_shape,
            visible: self.modes.cursor_visible,
            blinking: self.cursor_blink,
            wide: self.screen[self.cursor.row].get(self.cursor.col).width() == Width::Wide,
        };

        // Whatever changes from here on gets a new counter.
        self.batch_start = self.counter + 1;
    }

    /// Process-wide unique (§9.1).
    pub fn terminal_id(&self) -> u64 {
        self.terminal_id
    }

    /// Bumped each time history is cleared (ED 3, RIS).
    pub fn history_epoch(&self) -> u64 {
        self.history_epoch
    }

    /// Test aid: the change counter of each screen row, top to bottom.
    #[doc(hidden)]
    pub fn row_seqs(&self) -> Vec<u64> {
        self.screen.iter().map(|r| r.seq).collect()
    }

    /// Test aid: the terminal's change counter.
    #[doc(hidden)]
    pub fn change_counter(&self) -> u64 {
        self.counter
    }

    /// Screen row `r` is about to change: give it a new counter, unless it
    /// already has one since the last snapshot. One compare per row
    /// segment written.
    #[inline]
    pub(super) fn touch_row(&mut self, r: usize) {
        touch(&mut self.screen[r], &mut self.counter, self.batch_start);
    }

    pub(super) fn touch_rows(&mut self, rows: Range<usize>) {
        for r in rows {
            self.touch_row(r);
        }
    }

    /// Every row of the active screen changed.
    pub(super) fn touch_screen_rows(&mut self) {
        self.touch_rows(0..self.screen.len());
    }

    /// A new counter for every row of both screens, whatever they had: for
    /// rows just made (a new terminal, a reset) and a resize.
    pub(super) fn touch_all_rows(&mut self) {
        for row in self.screen.iter_mut().chain(self.other_screen.iter_mut()) {
            self.counter += 1;
            row.seq = self.counter;
        }
    }

    pub(super) fn with_rows_touched(mut self) -> Terminal {
        self.touch_all_rows();
        self
    }

    /// The counters' rules (§17.3): every screen row has one, no two rows
    /// share one, and none is past the terminal's counter.
    pub(super) fn check_counters(&self) -> Result<(), String> {
        let mut seqs: Vec<u64> = self.screen.iter().chain(&self.other_screen).map(|r| r.seq).collect();
        if let Some(&s) = seqs.iter().find(|&&s| s == 0 || s > self.counter) {
            return Err(format!("row change counter {s}, terminal's at {}", self.counter));
        }
        seqs.sort_unstable();
        if let Some(w) = seqs.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("two rows with change counter {}", w[0]));
        }
        if self.batch_start > self.counter + 1 {
            return Err(format!("batch starts at {}, counter at {}", self.batch_start, self.counter));
        }
        Ok(())
    }
}
