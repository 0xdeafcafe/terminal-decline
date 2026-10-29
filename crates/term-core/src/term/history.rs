//! History at another width (§12.3 of the cells design). Stored lines keep
//! the width they were written at; the view shows them laid out at the
//! current width, worked out from logical lines when they are read.
//!
//! History is read in **units**: a logical line, or, for a logical line
//! of more than [`UNIT`] stored lines, a piece of one (a stored line whose
//! number is a multiple of [`UNIT`] starts a new piece, so where pieces
//! start doesn't depend on the width or on what else is stored). A unit
//! whose lines were all written at the current width shows one row per
//! stored line, with no decoding; any other is decoded, its cells laid end
//! to end and split again at the current width, as the screen's reflow
//! does (§12.2).
//!
//! The view's place is the [`Point`] of its first cell. A view row of a
//! laid-out unit starts at any column of a stored line (and may take cells
//! from several), so everything held as Points in history (selection,
//! links, marks, search hits) is mapped through the unit's layout.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::reflow::{Flat, emit_row, split};
use super::{LinkValue, Point, Terminal};
use crate::cell::{Cell, Row};
use crate::rows::RowRef;
use crate::style::Style;

/// Stored lines in a unit at most.
pub(super) const UNIT: u64 = 256;

/// Laid-out units kept between reads (the view's, while it doesn't move).
const KEPT_UNITS: usize = 512;

/// A run of stored lines laid out at a width.
#[derive(Default)]
pub(super) struct Unit {
    /// Its stored lines, by number: `lo..hi`.
    pub(super) lo: u64,
    pub(super) hi: u64,
    /// Every line was written at the current width: its rows are its
    /// stored lines, and nothing below is filled in.
    pub(super) fast: bool,
    /// The offset of each stored line's first cell, and the cells after
    /// the last.
    old_starts: Vec<usize>,
    /// The offset of each row's first cell.
    starts: Vec<usize>,
    /// The cells shown (trailing padding trimmed).
    len: usize,
    /// The last stored line soft-wraps onto a line past the unit.
    continues: bool,
    /// The last stored line fills its row with its last style (`FILL`).
    fill: bool,
    flat: Flat,
    /// The styles `flat`'s cells index, and the links they index.
    pub(super) styles: Vec<Style>,
    pub(super) links: Vec<Option<Arc<LinkValue>>>,
}

/// A part of a stored line that a view row shows: stored columns
/// `from..to` of line `line`, from view column `at`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Piece {
    pub(crate) line: u64,
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) at: usize,
}

impl Piece {
    /// Stored columns `a..b` of `line` as view columns of this piece's row.
    pub(crate) fn view_cols(&self, line: u64, a: usize, b: usize) -> Option<std::ops::Range<usize>> {
        if line != self.line {
            return None;
        }
        let (a, b) = (a.max(self.from), b.min(self.to));
        (a < b).then(|| self.at + (a - self.from)..self.at.saturating_add(b - self.from))
    }
}

/// A row of the view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ViewRow {
    /// Row `row` of the unit of stored lines `lo..hi`.
    History { lo: u64, hi: u64, row: usize },
    /// A screen row.
    Screen(usize),
}

/// What the layout cache was filled for: the width, history's first and
/// end line, and its epoch.
type CacheKey = (usize, u64, u64, u64);

/// Units laid out, and the view's rows, kept while history and the width
/// stay as they are.
#[derive(Default)]
pub(super) struct Layouts {
    key: Option<CacheKey>,
    units: Vec<Arc<Unit>>,
    view: Option<(Option<Point>, usize, Vec<ViewRow>)>,
    counts: RowCounts,
}

impl Unit {
    /// How many rows it takes.
    pub(super) fn rows(&self) -> usize {
        if self.fast {
            (self.hi - self.lo) as usize
        } else {
            self.starts.len()
        }
    }

    /// The Point of the cell at offset `off` in the unit.
    fn point_of(&self, off: usize) -> Point {
        let n = self.old_starts.len() - 1;
        let k = self.old_starts[..n].partition_point(|&s| s <= off).max(1) - 1;
        Point {
            line: self.lo + k as u64,
            col: off - self.old_starts[k],
        }
    }

    /// The offset in the unit of the cell at `p`.
    pub(super) fn offset_of(&self, p: Point, cols: usize) -> usize {
        let k = (p.line.clamp(self.lo, self.hi - 1) - self.lo) as usize;
        if self.fast {
            return k * cols + p.col;
        }
        self.old_starts[k] + p.col
    }

    /// The row `p` is on and its column there (a place between rows, as
    /// a view position from another width, is rounded down to its row).
    pub(super) fn row_of(&self, p: Point, cols: usize) -> (usize, usize) {
        if self.fast {
            return ((p.line.clamp(self.lo, self.hi - 1) - self.lo) as usize, p.col);
        }
        let off = self.offset_of(p, cols);
        let r = self.starts.partition_point(|&s| s <= off).max(1) - 1;
        (r, off - self.starts[r])
    }

    fn row_end(&self, r: usize) -> usize {
        self.starts.get(r + 1).copied().unwrap_or(self.len)
    }

    /// The first cell of row `r`.
    pub(super) fn row_start(&self, r: usize) -> Point {
        if self.fast {
            return Point {
                line: self.lo + r as u64,
                col: 0,
            };
        }
        self.point_of(self.starts[r])
    }

    /// The Point shown at column `col` of row `r`: past the end of a row
    /// that wraps, its last cell; past the end of the unit, columns past
    /// its last stored line's cells.
    pub(super) fn point_at(&self, r: usize, col: usize) -> Point {
        if self.fast {
            return Point {
                line: self.lo + r as u64,
                col,
            };
        }
        let mut off = self.starts[r] + col;
        if r + 1 < self.starts.len() {
            off = off.min(self.row_end(r).saturating_sub(1).max(self.starts[r]));
        }
        self.point_of(off)
    }

    /// Whether row `r` may still change as history grows: the unit's last
    /// row, when its line goes on in a stored line not yet written.
    pub(super) fn growing(&self, r: usize) -> bool {
        !self.fast && self.continues && r + 1 == self.starts.len()
    }

    /// The parts of stored lines row `r` shows. The last row of a unit
    /// that ends its line takes the rest of the view row too, so a
    /// selection past the text shows to the end.
    pub(super) fn pieces(&self, r: usize, cols: usize, out: &mut Vec<Piece>) {
        out.clear();
        if self.fast {
            out.push(Piece {
                line: self.lo + r as u64,
                from: 0,
                to: usize::MAX,
                at: 0,
            });
            return;
        }
        let (s, e) = (self.starts[r], self.row_end(r));
        let n = self.old_starts.len() - 1;
        for k in 0..n {
            let (a, b) = (self.old_starts[k].max(s), self.old_starts[k + 1].min(e));
            if a < b {
                out.push(Piece {
                    line: self.lo + k as u64,
                    from: a - self.old_starts[k],
                    to: b - self.old_starts[k],
                    at: a - s,
                });
            }
        }
        let last = r + 1 == self.starts.len();
        if out.is_empty() {
            let p = self.point_of(s);
            out.push(Piece {
                line: p.line,
                from: p.col,
                to: p.col + 1,
                at: 0,
            });
        }
        if last && !self.continues {
            let p = out.last_mut().expect("a piece");
            p.to = usize::MAX;
        } else if let Some(p) = out.last_mut() {
            // A row cut short by a wide character: its last column shows
            // nothing of the next stored cells, but is part of the line.
            p.to = p.to.max(p.from + (cols - p.at).min(p.to - p.from));
        }
    }

    /// Row `r`, laid out `cols` wide.
    pub(super) fn row(&self, r: usize, cols: usize) -> (Row, bool) {
        let mut row = emit_row(&self.flat, self.len, &self.starts, cols, r);
        if r + 1 == self.starts.len() && self.continues {
            // Its line goes on below: the row ends where its cells do (a
            // short row where the unit meets the rest of the line).
            row.wrapped = true;
            row.cells.truncate((self.len - self.starts[r]).min(cols));
        }
        (row, self.fill && r + 1 == self.starts.len())
    }
}

impl Layouts {
    /// Drop everything, the row counts too.
    pub(super) fn clear(&mut self) {
        *self = Layouts::default();
    }

    /// Drop the laid-out units and the view's rows, keeping the row
    /// counts (output arriving doesn't change a sealed block's).
    fn clear_layouts(&mut self) {
        self.key = None;
        self.units.clear();
        self.view = None;
    }
}

/// The row count of one sealed block at the current width (§12.3
/// `rows_at`): the rows of the units starting in it.
#[derive(Clone, Copy, Debug)]
struct BlockRows {
    /// The first unit starting in it (past its end if none does).
    start: u64,
    /// One past its last line, when counted.
    end: u64,
    /// The first unit starting after it.
    next: u64,
    rows: usize,
}

/// Row counts per sealed block, by its first line, for one width and
/// history epoch. Kept while output arrives; dropped with the layouts
/// when lines are popped (a reflow) or the width changes.
#[derive(Default)]
struct RowCounts {
    key: Option<(usize, u64)>,
    blocks: HashMap<u64, BlockRows>,
}

/// Units at least this many rows past the target are skipped a block at
/// a time when scrolling (a short scroll walks units, and doesn't count
/// a whole block it won't leave).
const SKIP_BLOCKS_PAST: usize = 256;

/// Block row counts kept before those of freed blocks are dropped (the
/// 32 MB backstop holds about 512 blocks).
const KEPT_COUNTS: usize = 1024;

impl Terminal {
    /// The line number of history's first stored line (on the primary
    /// screen).
    pub(super) fn history_first(&self) -> u64 {
        self.top_line - self.scrollback.len() as u64
    }

    fn idx(&self, line: u64) -> usize {
        (line - self.history_first()) as usize
    }

    /// The unit holding stored line `line`: its first line and one past
    /// its last.
    pub(super) fn unit_bounds(&self, line: u64) -> (u64, u64) {
        let first = self.history_first();
        let sb = &self.scrollback;
        let mut lo = line;
        while lo > first && !lo.is_multiple_of(UNIT) && sb.wrapped(self.idx(lo - 1)) {
            lo -= 1;
        }
        let mut hi = line + 1;
        while hi < self.top_line && !hi.is_multiple_of(UNIT) && sb.wrapped(self.idx(hi - 1)) {
            hi += 1;
        }
        (lo, hi)
    }

    fn cache_key(&self) -> CacheKey {
        (self.cols, self.history_first(), self.top_line, self.history_epoch)
    }

    /// Lay out stored lines `lo..hi` (a unit) at the current width.
    fn lay_out(&self, lo: u64, hi: u64) -> Unit {
        let sb = &self.scrollback;
        let (a, b) = (self.idx(lo), self.idx(hi));
        if sb.all_at_width(a, b, self.cols) {
            return Unit {
                lo,
                hi,
                fast: true,
                ..Unit::default()
            };
        }
        let mut u = Unit {
            lo,
            hi,
            links: vec![None],
            ..Unit::default()
        };
        let (mut row, mut styles, mut links) = (Row::default(), Vec::new(), Vec::new());
        for i in a..b {
            sb.get_at_width(i, 0, &mut row, &mut styles, &mut links);
            let base = u.styles.len() as u32;
            let lbase = u.links.len();
            u.styles.extend(styles.iter().map(|s| {
                let mut s = *s;
                if s.link != 0 {
                    s.link = (lbase + s.link as usize - 1) as _;
                }
                s
            }));
            u.links.extend(links.iter().skip(1).cloned());
            for c in &mut row.cells {
                *c = c.with_style(Cell::template(c.style_id() + base, c.shows_empty()));
            }
            u.old_starts.push(u.flat.push_row(&row));
        }
        u.old_starts.push(u.flat.cells.len());
        u.continues = sb.wrapped(b - 1);
        u.fill = sb.fill(b - 1);
        u.flat.fill = match row.cells.last() {
            Some(&c) if u.fill => c.style_part(),
            _ => Cell::BLANK,
        };
        u.len = if u.continues { u.flat.cells.len() } else { u.flat.trimmed_len(0) };
        u.starts = split(&u.flat.cells, u.len, self.cols);
        u
    }

    /// The unit `lo..hi`, laid out (kept while history doesn't change).
    pub(super) fn unit(&self, lo: u64, hi: u64) -> Arc<Unit> {
        let key = self.cache_key();
        let mut cache = self.layouts.lock().expect("layouts");
        if cache.key != Some(key) {
            cache.clear_layouts();
            cache.key = Some(key);
        }
        if let Some(u) = cache.units.iter().find(|u| u.lo == lo && u.hi == hi) {
            return u.clone();
        }
        drop(cache);
        let u = Arc::new(self.lay_out(lo, hi));
        let mut cache = self.layouts.lock().expect("layouts");
        if cache.key == Some(key) {
            if cache.units.len() >= KEPT_UNITS {
                cache.units.clear();
            }
            if !u.fast {
                cache.units.push(u.clone());
            }
        }
        u
    }

    /// The unit holding stored line `line`.
    pub(super) fn unit_at(&self, line: u64) -> Arc<Unit> {
        let (lo, hi) = self.unit_bounds(line);
        self.unit(lo, hi)
    }

    /// Where the view starts when scrolled back: its first unit and row.
    pub(super) fn view_start(&self) -> Option<(Arc<Unit>, usize)> {
        let p = self.view.filter(|_| !self.alt_active && !self.scrollback.is_empty())?;
        if p.line >= self.top_line {
            return None;
        }
        let p = if p.line < self.history_first() {
            Point {
                line: self.history_first(),
                col: 0,
            }
        } else {
            p
        };
        let u = self.unit_at(p.line);
        let (r, _) = u.row_of(p, self.cols);
        let r = r.min(u.rows() - 1);
        Some((u, r))
    }

    /// The first cell shown, when scrolled back.
    pub(super) fn view_point(&self) -> Option<Point> {
        self.view_start().map(|(u, r)| u.row_start(r))
    }

    /// View row `line`, which may be past the view's last (as if it were
    /// taller).
    pub(super) fn view_row_at(&self, line: usize) -> ViewRow {
        if line < self.lines {
            return self.view_rows()[line];
        }
        self.walk_view(line + 1)[line]
    }

    /// The rows the view shows, top to bottom.
    pub(super) fn view_rows(&self) -> Vec<ViewRow> {
        let key = self.cache_key();
        {
            let cache = self.layouts.lock().expect("layouts");
            if cache.key == Some(key)
                && let Some((v, lines, rows)) = &cache.view
                && *v == self.view
                && *lines == self.lines
            {
                return rows.clone();
            }
        }
        let rows = self.walk_view(self.lines);
        let mut cache = self.layouts.lock().expect("layouts");
        if cache.key != Some(key) {
            cache.clear_layouts();
            cache.key = Some(key);
        }
        cache.view = Some((self.view, self.lines, rows.clone()));
        rows
    }

    /// The view's first `n` rows.
    fn walk_view(&self, n: usize) -> Vec<ViewRow> {
        let mut rows = Vec::with_capacity(n);
        if let Some((mut u, mut r)) = self.view_start() {
            while rows.len() < n {
                rows.push(ViewRow::History {
                    lo: u.lo,
                    hi: u.hi,
                    row: r,
                });
                if r + 1 < u.rows() {
                    r += 1;
                } else if u.hi < self.top_line {
                    u = self.unit_at(u.hi);
                    r = 0;
                } else {
                    break;
                }
            }
        }
        let mut s = 0;
        while rows.len() < n {
            rows.push(ViewRow::Screen(s));
            s += 1;
        }
        rows
    }

    /// View row `at`, as a row to draw.
    pub(super) fn view_row(&self, at: ViewRow) -> RowRef<'_> {
        match at {
            ViewRow::Screen(r) => self.screen_row(r),
            ViewRow::History { lo, hi, row } => {
                let u = self.unit(lo, hi);
                if u.fast {
                    return self.history_row(self.idx(lo) + row);
                }
                let (row, _) = u.row(row, self.cols);
                RowRef::new(Cow::Owned(row), Cow::Owned(u.styles.clone())).with_links(Cow::Owned(u.links.clone()))
            }
        }
    }

    /// The pieces of stored lines view row `at` shows; a screen row is one
    /// piece of its own line.
    pub(super) fn view_pieces(&self, at: ViewRow, out: &mut Vec<Piece>) {
        match at {
            ViewRow::Screen(r) => {
                out.clear();
                out.push(Piece {
                    line: self.top_line + r as u64,
                    from: 0,
                    to: usize::MAX,
                    at: 0,
                });
            }
            ViewRow::History { lo, hi, row } => self.unit(lo, hi).pieces(row, self.cols, out),
        }
    }

    /// Rows from row `r` of unit `u` to the end of history.
    pub(super) fn rows_below(&self, u: &Unit, r: usize) -> usize {
        u.rows() - r + self.rows_from(u.hi)
    }

    /// Rows from the unit starting at `line` to the end of history: a
    /// block at a time from the row counts, where `line` starts a block's
    /// first unit.
    fn rows_from(&self, mut line: u64) -> usize {
        let mut n = 0;
        while line < self.top_line {
            if let Some(e) = self.block_rows(line).filter(|e| e.start == line) {
                n += e.rows;
                line = e.next;
                continue;
            }
            let (_, end, _, _) = self.scrollback.block_lines(line);
            let (rows, next) = self.count_in_block(line, end);
            n += rows;
            line = next;
        }
        n
    }

    /// The row count of the sealed block holding `line`, worked out and
    /// kept if it isn't yet. `None` for the open block and the first
    /// block (which may hold hidden lines, so where its first unit
    /// starts can change).
    fn block_rows(&self, line: u64) -> Option<BlockRows> {
        let (first, end, open, _) = self.scrollback.block_lines(line);
        if open || first <= self.history_first() {
            return None;
        }
        let key = (self.cols, self.history_epoch);
        {
            let cache = self.layouts.lock().expect("layouts");
            if cache.counts.key == Some(key)
                && let Some(e) = cache.counts.blocks.get(&first)
                && e.end == end
            {
                return Some(*e);
            }
        }
        let (lo, hi) = self.unit_bounds(first);
        let start = if lo == first { first } else { hi };
        let (rows, next) = if start < end { self.count_in_block(start, end) } else { (0, start) };
        let e = BlockRows { start, end, next, rows };
        // The last unit is still growing if it reaches the screen: kept
        // only once it ends in history.
        if next < self.top_line {
            let mut cache = self.layouts.lock().expect("layouts");
            if cache.counts.key != Some(key) {
                cache.counts = RowCounts {
                    key: Some(key),
                    ..RowCounts::default()
                };
            }
            if cache.counts.blocks.len() >= KEPT_COUNTS {
                // Blocks freed from the oldest end.
                let oldest = self.history_first();
                cache.counts.blocks.retain(|&f, _| f >= oldest);
            }
            cache.counts.blocks.insert(first, e);
        }
        Some(e)
    }

    /// Rows of the units starting from `line` (a unit's first line) up to
    /// `end`, the end of the block holding `line`, and the first unit
    /// starting at or after `end`. In a block written at the current
    /// width every unit wholly inside takes a row per line.
    fn count_in_block(&self, line: u64, end: u64) -> (usize, u64) {
        let (_, _, _, w) = self.scrollback.block_lines(line);
        if w as usize == self.cols && end > line {
            let (last, _) = self.unit_bounds(end - 1);
            if last >= line {
                let (rows, next) = self.count_units(last, last + 1);
                return ((last - line) as usize + rows, next);
            }
        }
        self.count_units(line, end)
    }

    /// Rows of the units starting from `from` (a unit's first line) up to
    /// `until`, and the first unit starting at or after `until`. A unit
    /// written at the current width takes a row per line; one of narrow
    /// ASCII is counted from its text's length; any other is laid out.
    fn count_units(&self, from: u64, until: u64) -> (usize, u64) {
        let (mut rows, mut next) = (0, from);
        if from >= until {
            return (0, from);
        }
        let first = self.history_first();
        let mut lo = from;
        let mut at_width = true;
        let mut shapes: Vec<(Option<usize>, Option<usize>)> = Vec::new();
        self.scrollback.shapes(self.idx(from), self.idx(self.top_line), |i, w, s| {
            let line = first + i as u64;
            at_width &= w as usize == self.cols;
            shapes.push((s.cells, s.blank_tail));
            let hi = line + 1;
            if s.wrapped && !hi.is_multiple_of(UNIT) && hi < self.top_line {
                return true;
            }
            rows += if at_width {
                (hi - lo) as usize
            } else {
                match self.text_rows(&shapes, s.wrapped) {
                    Some(n) => {
                        debug_assert_eq!(n, self.lay_out(lo, hi).rows(), "rows of lines {lo}..{hi} from their text");
                        n
                    }
                    None => self.unit(lo, hi).rows(),
                }
            };
            (lo, next, at_width) = (hi, hi, true);
            shapes.clear();
            hi < until
        });
        (rows, next)
    }

    /// The rows a unit of narrow cells takes, from its lines' shapes
    /// (`None` unless every line is narrow and the trimmed tail is
    /// known): `max(1, ⌈n / cols⌉)` for its `n` cells.
    fn text_rows(&self, shapes: &[(Option<usize>, Option<usize>)], continues: bool) -> Option<usize> {
        let mut n = 0;
        for &(cells, _) in shapes {
            n += cells?;
        }
        if !continues {
            // Trailing blanks are trimmed, back through lines that are
            // all blank.
            for &(cells, tail) in shapes.iter().rev() {
                let tail = tail?;
                n -= tail;
                if tail < cells? {
                    break;
                }
            }
        }
        Some(n.div_ceil(self.cols).max(1))
    }

    /// The rows of the unit starting at `line`, and where it ends.
    fn unit_rows(&self, line: u64) -> (usize, u64) {
        self.count_units(line, line + 1)
    }

    /// Move the view `d` rows back from row `r` of `u`, stopping at the
    /// first row of history.
    pub(super) fn rows_back(&self, u: Arc<Unit>, r: usize, d: usize) -> (Arc<Unit>, usize) {
        if r >= d {
            return (u, r - d);
        }
        let first = self.history_first();
        if u.lo <= first {
            return (u, 0);
        }
        // Rows still to go above the first row of the unit at `line`.
        let mut need = d - r;
        let mut line = u.lo;
        while line > first {
            if need > SKIP_BLOCKS_PAST
                && let Some(e) = self.block_rows(line - 1)
                && e.next == line
                && e.start < line
                && e.rows < need
            {
                need -= e.rows;
                line = e.start;
                continue;
            }
            let (lo, hi) = self.unit_bounds(line - 1);
            let (rows, _) = self.unit_rows(lo);
            if rows >= need {
                let u = self.unit(lo, hi);
                let r = u.rows().saturating_sub(need);
                return (u, r);
            }
            need -= rows;
            line = lo;
        }
        (self.unit_at(first), 0)
    }

    /// Move the view `d` rows forward from row `r` of `u`; `None` once it
    /// reaches the screen.
    pub(super) fn rows_forward(&self, u: Arc<Unit>, r: usize, d: usize) -> Option<(Arc<Unit>, usize)> {
        if r + d < u.rows() {
            return Some((u, r + d));
        }
        let mut d = d - (u.rows() - r);
        let mut line = u.hi;
        while line < self.top_line {
            if d > SKIP_BLOCKS_PAST
                && let Some(e) = self.block_rows(line).filter(|e| e.start == line)
                && e.rows <= d
            {
                d -= e.rows;
                line = e.next;
                continue;
            }
            let (rows, hi) = self.unit_rows(line);
            if d < rows {
                return Some((self.unit(line, hi), d));
            }
            d -= rows;
            line = hi;
        }
        None
    }

    /// Put the view's first row at the row of history holding `p`
    /// (rounded down to its row's first cell).
    pub(super) fn set_view_point(&mut self, p: Option<Point>) {
        let new = p
            .filter(|p| !self.alt_active && p.line < self.top_line && !self.scrollback.is_empty())
            .map(|p| {
                let p = if p.line < self.history_first() {
                    Point {
                        line: self.history_first(),
                        col: 0,
                    }
                } else {
                    p
                };
                let u = self.unit_at(p.line);
                let (r, _) = u.row_of(p, self.cols);
                u.row_start(r.min(u.rows() - 1))
            });
        if new != self.view {
            self.view = new;
            self.touch();
        }
    }

    /// Drop the laid-out units (history or the width changed in a way
    /// the cache key can't see).
    pub(super) fn drop_layouts(&self) {
        self.layouts.lock().expect("layouts").clear();
    }
}

pub(super) type LayoutCache = Mutex<Layouts>;
