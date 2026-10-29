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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::history::{Unit, ViewRow};
use super::{CursorShape, LinkValue, Terminal};
use crate::cell::{Cell, Row, Width};
use crate::scrollback::HistoryReader;
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
    /// A history row whose last style fills to the end of the row (a
    /// full-width background, §11.3): its cells already carry it to the
    /// screen's width.
    pub const FILL: RowFlags = RowFlags(1 << 2);

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
    /// Its clusters in [`Snapshot::cluster_spans`], by slot: a cluster
    /// cell's slot `s` is `cluster_spans[clusters.start + s]`. Empty when
    /// the row wasn't copied or has no clusters.
    pub clusters: Range<u32>,
    /// Its highlights in [`Snapshot::highlights`], left to right.
    pub highlights: Range<u32>,
}

/// Columns of one row drawn highlighted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Highlight {
    pub cols: Range<u16>,
    pub kind: HighlightKind,
}

/// What a highlight is. Search hits come with search; the selection and
/// the hovered link ([`Terminal::set_hover`]) are highlighted now.

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
    /// The cluster text of the rows copied this time, back to back.
    pub cluster_text: String,
    /// Per copied row, by slot: (offset, length) into `cluster_text`; a
    /// free slot has length 0.
    pub cluster_spans: Vec<(u32, u16)>,
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
    row_links: Vec<Option<Arc<LinkValue>>>,
}

impl Snapshot {
    /// The cells copied for `row`, if it was copied.
    pub fn row_cells(&self, row: &SnapRow) -> Option<&[Cell]> {
        let r = row.cells.as_ref()?;
        Some(&self.cells[r.start as usize..r.end as usize])
    }

    /// The text of the cluster in slot `slot` of `row` (a cluster cell's
    /// [`Cell::codepoint`]); empty if there is none.
    pub fn cluster(&self, row: &SnapRow, slot: u32) -> &str {
        let i = row.clusters.start as usize + slot as usize;
        if i >= row.clusters.end as usize {
            return "";
        }
        let (at, len) = self.cluster_spans[i];
        &self.cluster_text[at as usize..at as usize + len as usize]
    }

    /// `row`'s highlights.
    pub fn row_highlights(&self, row: &SnapRow) -> &[Highlight] {
        &self.highlights[row.highlights.start as usize..row.highlights.end as usize]
    }
}

/// A copied row's cluster store, appended: its text in one copy, holes
/// and all (at most half of it, §6.1), with its spans moved up.
fn copy_clusters(row: &Row, text: &mut String, spans: &mut Vec<(u32, u16)>) {
    if let Some(store) = row.clusters() {
        let (t, s) = store.raw();
        let base = text.len() as u32;
        text.push_str(t);
        spans.extend(s.map(|(at, len)| if len == 0 { (base, 0) } else { (base + at, len) }));
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

/// `ranges` sorted and joined where they touch.
fn merge(ranges: &mut [Range<usize>]) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::new();
    for r in ranges.iter() {
        match out.last_mut() {
            Some(l) if r.start <= l.end => l.end = l.end.max(r.end),
            _ => out.push(r.clone()),
        }
    }
    out
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
        self.refresh_search();
        let popped = self.scrollback.take_popped_from();
        out.rows.clear();
        out.cells.clear();
        out.cluster_text.clear();
        out.cluster_spans.clear();
        out.style_updates.clear();
        out.history_styles.clear();
        out.highlights.clear();
        out.terminal_id = self.terminal_id;
        out.history_epoch = self.history_epoch;
        out.popped_from = popped;
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
        // The hovered link's id in the screen's table: cells are compared
        // with it, which is comparing by value within one table.
        let hovered_link = self.hover.and_then(|p| self.link_at(p));
        let hovered = hovered_link.as_ref().and_then(|l| self.styles.links().find(l));
        // Off an OSC 8 link, a plain URL there is highlighted the same way
        // (on history rows too: it is read from the text).
        let hovered_url = self
            .hover
            .filter(|_| hovered_link.is_none())
            .and_then(|p| self.url_at(p))
            .map(|(a, b, _)| (a, b));
        let history_first = self.history_first();
        let view_rows = self.view_rows();
        // History rows are read one after another.
        let mut reader = HistoryReader::default();
        let mut found = Vec::new();
        let mut pieces = Vec::new();
        let mut unit: Option<std::sync::Arc<Unit>> = None;
        for &at in view_rows.iter() {
            let highlights = out.highlights.len() as u32;
            self.view_pieces(at, &mut pieces);
            // Stored columns `a..b` of `line`, as this row's columns.
            let cols_of = |line: u64, a: usize, b: usize, out: &mut Vec<Range<usize>>| {
                for p in &pieces {
                    if let Some(r) = p.view_cols(line, a, b) {
                        let r = r.start..r.end.min(self.cols);
                        if r.start < r.end {
                            out.push(r);
                        }
                    }
                }
            };
            let mut ranges = Vec::new();
            // A span of Points (both ends included), on this row.
            let span = |(a, b): (super::Point, super::Point), ranges: &mut Vec<Range<usize>>| {
                for p in &pieces {
                    if p.line < a.line || p.line > b.line {
                        continue;
                    }
                    let start = if p.line == a.line { a.col } else { 0 };
                    let end = if p.line == b.line { b.col + 1 } else { usize::MAX };
                    cols_of(p.line, start, end, ranges);
                }
            };
            if let Some(sel) = selection {
                ranges.clear();
                span(sel, &mut ranges);
                for r in merge(&mut ranges) {
                    out.highlights.push(Highlight {
                        cols: to_u16(r.start)..to_u16(r.end),
                        kind: HighlightKind::Selection,
                    });
                }
            }
            // A history row's links are its own: compared by value.
            let history_row = (matches!(at, ViewRow::History { .. }) && hovered_link.is_some()).then(|| self.view_row(at));
            let screen_r = match at {
                ViewRow::Screen(r) => Some(r),
                ViewRow::History { .. } => None,
            };
            let on_link = |col: usize| -> bool {
                match (&history_row, screen_r) {
                    (Some(r), _) => col < r.len() && r.link(col) == hovered_link.as_ref(),
                    (None, Some(sr)) => {
                        let row = &self.screen[sr];
                        hovered.is_some_and(|id| row.cells.get(col).is_some_and(|c| self.styles.get(c.style_id()).link == id))
                    }
                    (None, None) => false,
                }
            };
            if hovered_link.is_some() {
                let len = match (&history_row, screen_r) {
                    (Some(r), _) => r.len(),
                    (None, Some(sr)) => self.screen[sr].cells.len(),
                    (None, None) => 0,
                };
                let mut run: Option<usize> = None;
                for col in 0..=len {
                    let on = on_link(col);
                    match (on, run) {
                        (true, None) => run = Some(col),
                        (false, Some(start)) => {
                            out.highlights.push(Highlight {
                                cols: to_u16(start)..to_u16(col),
                                kind: HighlightKind::HoveredLink,
                            });
                            run = None;
                        }
                        _ => {}
                    }
                }
            }
            if let Some(url) = hovered_url {
                ranges.clear();
                span(url, &mut ranges);
                for r in merge(&mut ranges) {
                    out.highlights.push(Highlight {
                        cols: to_u16(r.start)..to_u16(r.end),
                        kind: HighlightKind::HoveredLink,
                    });
                }
            }
            let mut lines: Vec<u64> = pieces.iter().map(|p| p.line).collect();
            lines.dedup();
            for line in lines {
                found.clear();
                self.search_highlights(line, &mut found);
                for (c, current) in found.drain(..) {
                    ranges.clear();
                    cols_of(line, c.start, c.end, &mut ranges);
                    for r in ranges.drain(..) {
                        out.highlights.push(Highlight {
                            cols: to_u16(r.start)..to_u16(r.end),
                            kind: if current {
                                HighlightKind::CurrentSearchHit
                            } else {
                                HighlightKind::SearchHit
                            },
                        });
                    }
                }
            }
            // Left to right.
            out.highlights[highlights as usize..].sort_by_key(|h| h.cols.start);
            let highlights = highlights..out.highlights.len() as u32;

            let mut flags = RowFlags::default();
            let start = out.cells.len() as u32;
            let clusters = out.cluster_spans.len() as u32;
            let (key, copied) = match at {
                ViewRow::History { lo, hi, row } => {
                    if unit.as_ref().is_none_or(|u| u.lo != lo || u.hi != hi) {
                        unit = Some(self.unit(lo, hi));
                    }
                    let u = unit.as_ref().expect("a unit");
                    let first = u.row_start(row);
                    let line = first.line;
                    let key = RowKey::History {
                        line,
                        col: to_u16(first.col),
                    };
                    flags.set(RowFlags::HISTORY, true);
                    let kept = same_history && popped.is_none_or(|p| line < p) && !u.growing(row) && have(key);
                    let copied = if u.fast {
                        let i = (line - history_first) as usize;
                        flags.set(RowFlags::WRAPPED, self.scrollback.wrapped(i));
                        flags.set(RowFlags::FILL, self.scrollback.fill(i));
                        !kept
                            && self
                                .scrollback
                                .read(i, self.cols, &mut out.row, &mut out.row_styles, &mut out.row_links, &mut reader)
                    } else {
                        let (r, fill) = u.row(row, self.cols);
                        flags.set(RowFlags::WRAPPED, r.wrapped);
                        flags.set(RowFlags::FILL, fill);
                        if !kept {
                            out.row = r;
                            out.row_styles.clear();
                            out.row_styles.extend_from_slice(&u.styles);
                        }
                        !kept
                    };
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
                        copy_clusters(&out.row, &mut out.cluster_text, &mut out.cluster_spans);
                    }
                    (key, copied)
                }
                ViewRow::Screen(r) => {
                    let row = &self.screen[r];
                    let key = RowKey::Screen(row.seq);
                    flags.set(RowFlags::WRAPPED, row.wrapped);
                    let copied = !(same_terminal && have(key));
                    if copied {
                        out.cells.extend_from_slice(&row.cells);
                        copy_clusters(row, &mut out.cluster_text, &mut out.cluster_spans);
                    }
                    (key, copied)
                }
            };
            out.rows.push(SnapRow {
                key,
                flags,
                cells: copied.then_some(start..out.cells.len() as u32),
                clusters: clusters..out.cluster_spans.len() as u32,
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
