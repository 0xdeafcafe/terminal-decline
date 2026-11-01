//! Output that changes the text under the selection clears it, as xterm
//! and Alacritty do, so what is shown selected is still what Cmd+C
//! copies (cells §11).
//!
//! The check is made as output writes, not by reading the selection's
//! text again after every parse call: while a parse call runs, the
//! selection's lines on the screen are watched, in buffer line numbers
//! (which stay with the text as the screen scrolls). The few places that
//! change cells ask [`SelectionWatch::covers`] whether a screen row is
//! one of them, a subtraction and a compare. Most writes to a watched row
//! can tell on the spot whether they change the selected text: a
//! character written over a plain one compares the two, an erase changes
//! it if it blanks a plain character that isn't a space, a soft wrap
//! changing adds or takes away a line break. Once a change is found the
//! watch stops, and the parse call's end clears the selection. A write
//! that can't tell (over a wide character or cluster, a cluster growing)
//! reads the selected text first and stops watching; the parse call's
//! end reads it again and compares. A selection kept to a program's
//! panel (§13.2) is watched in the panel's columns only, so the other
//! panels redrawing leave it alone.
//!
//! Text scrolling under the selection takes it along (§13.2). Lines
//! scrolling into history keep their text and numbers, so a flood
//! scrolling the selection away costs nothing; lines dropped from
//! history take the selection with them. When a scroll region moves
//! (CSI S and T, LF at its bottom margin, RI at its top, insert and
//! delete line, and the alternate screen scrolling), a selection on the
//! rows that move moves with them ([`Terminal::follow_scroll`]): its
//! lines are renumbered and watched again, cut at the region's edge
//! where its text leaves (at the panel's columns, or the screen's), and
//! cleared once all of it has left. A selection read to compare is
//! compared first, before its text moves. One partly on the rows that
//! move and partly not can't follow both: it is read and compared, and
//! cleared if other text moved under it, as before.

use std::ops::Range;

use super::{Panel, Point, Selection, Terminal};
use crate::cell::Cell;
use crate::image::placeholder::PLACEHOLDER;

/// The selection's lines on the screen, watched while output is parsed.
pub(super) struct SelectionWatch {
    /// The first watched buffer line, and how many follow it (0: none,
    /// outside a parse call or once the outcome is known).
    lo: u64,
    span: u64,
    /// The selection watched, in text order.
    sel: Option<Selection>,
    a: Point,
    b: Point,
    state: Outcome,
}

#[derive(Default)]
enum Outcome {
    /// The selected text hasn't changed (so far).
    #[default]
    Same,
    /// It changed: the selection goes at the end of the parse call.
    Changed,
    /// Output that may change it is coming: the text from `from` to the
    /// selection's end, as it was, to compare at the end of the call.
    Read { from: Point, text: String },
}

impl Default for SelectionWatch {
    fn default() -> SelectionWatch {
        let none = Point { line: 0, col: 0 };
        SelectionWatch {
            lo: 0,
            span: 0,
            sel: None,
            a: none,
            b: none,
            state: Outcome::Same,
        }
    }
}

impl SelectionWatch {
    /// Buffer line `line` is a watched line of the screen.
    #[inline(always)]
    pub(super) fn covers(&self, line: u64) -> bool {
        line.wrapping_sub(self.lo) < self.span
    }

    /// The selected columns of watched line `line`, `start..end`.
    #[inline]
    fn cols(&self, line: u64) -> Range<usize> {
        let mut start = if line == self.a.line { self.a.col } else { 0 };
        let mut end = if line == self.b.line { self.b.col + 1 } else { usize::MAX };
        if let Some(p) = self.panel() {
            // Only the panel's columns: the other panels' text changing
            // changes nothing selected.
            start = start.max(p.left);
            end = end.min(p.right + 1).max(start);
        }
        start..end
    }

    fn panel(&self) -> Option<Panel> {
        self.sel.and_then(|s| s.panel)
    }

    fn stop(&mut self) {
        self.span = 0;
    }
}

/// A plain cell (narrow, a scalar or nothing): its text, a space for an
/// empty one. None for wide characters, spacers, clusters and
/// placeholders.
#[inline]
fn plain_text(cell: Cell) -> Option<u32> {
    if cell.bits() & (Cell::WIDTH | Cell::KIND) != 0 {
        return None;
    }
    match cell.codepoint() {
        0 => Some(b' ' as u32),
        cp => Some(cp),
    }
}

impl Terminal {
    /// Before parsing: watch the selection's lines on the screen. Whether
    /// there is a selection, for [`Terminal::check_selection`] after.
    pub(crate) fn watch_selection(&mut self) -> bool {
        let Some(sel) = self.selection else {
            return false;
        };
        let (a, b) = sel.ordered();
        let lo = a.line.max(self.top_line);
        let hi = b.line.min(self.top_line + self.lines as u64 - 1);
        self.watch = SelectionWatch {
            lo,
            span: if lo <= hi { hi - lo + 1 } else { 0 },
            sel: Some(sel),
            a,
            b,
            state: Outcome::Same,
        };
        true
    }

    /// Screen rows `top..=bottom` are about to scroll `n` rows, up (to
    /// the top) or down, with the screen's top line moving on `n` lines
    /// when `moves_top` (output scrolling from the screen's top row): a selection on them moves with its text, cut at
    /// the region's edge where its text leaves, and cleared once all of
    /// it has. Whether that was seen to (or the scroll doesn't touch the
    /// selection); false when the selection is partly on the rows that
    /// move and partly not, which the caller watches as text changing
    /// under it, as before.
    #[inline(always)]
    pub(super) fn follow_scroll(&mut self, top: usize, bottom: usize, n: usize, up: bool, moves_top: bool) -> bool {
        if self.watch.sel.is_none() {
            return true;
        }
        // The whole primary screen scrolling into history: the selection's
        // lines keep their text, at no cost.
        if moves_top && bottom == self.lines - 1 && !self.alt_active {
            return true;
        }
        self.follow_scroll_slow(top, bottom, n, up, moves_top)
    }

    #[cold]
    #[inline(never)]
    fn follow_scroll_slow(&mut self, top: usize, bottom: usize, n: usize, up: bool, moves_top: bool) -> bool {
        let Some(sel) = self.selection.filter(|s| Some(*s) == self.watch.sel) else {
            return true;
        };
        if matches!(self.watch.state, Outcome::Changed) {
            return true;
        }
        let (a, b) = sel.ordered();
        let t0 = self.top_line;
        let (first, last) = (t0 + top as u64, t0 + bottom as u64);
        // How the selection's lines move, in buffer lines, and the lines
        // whose text is lost.
        let (shift, kept): (i64, Range<u64>) = if moves_top {
            // The screen's top line moves on: the region's lines (and
            // history's) keep their numbers; the rows below it stay where
            // they are on the screen, so their numbers move on too.
            if a.line > last {
                (n as i64, 0..u64::MAX)
            } else if b.line > last {
                return false;
            } else {
                let t1 = t0 + n as u64;
                let kept = if self.alt_active { t1..u64::MAX } else { 0..u64::MAX };
                (0, kept)
            }
        } else {
            if b.line < first || a.line > last {
                return true;
            }
            if a.line < first || b.line > last {
                return false;
            }
            if up {
                (-(n as i64), first..u64::MAX)
            } else {
                (n as i64, 0..last + 1)
            }
        };
        // Text read to compare: compare it now, before it moves.
        if let Outcome::Read { from, text } = &self.watch.state {
            if self.text_in(*from, b, sel.panel) != *text {
                self.watch_changed();
                return true;
            }
            self.watch.state = Outcome::Same;
        }
        let moved = |p: Point| Point {
            line: p.line.saturating_add_signed(shift),
            col: p.col,
        };
        let (mut na, mut nb) = (moved(a), moved(b));
        let (left, right) = sel.panel.map_or((0, self.cols - 1), |p| (p.left, p.right));
        if nb.line < kept.start || na.line >= kept.end {
            // All of it scrolled out.
            self.selection = None;
            self.watch.stop();
            self.watch.sel = None;
            return true;
        }
        if na.line < kept.start {
            na = Point {
                line: kept.start,
                col: left,
            };
        }
        if nb.line >= kept.end {
            nb = Point {
                line: kept.end - 1,
                col: right,
            };
        }
        let (anchor, head) = if sel.anchor <= sel.head { (na, nb) } else { (nb, na) };
        let new = Selection { anchor, head, ..sel };
        self.selection = Some(new);
        // Watch its new lines (on the screen as it will be).
        let t1 = if moves_top { t0 + n as u64 } else { t0 };
        let lo = na.line.max(t1);
        let hi = nb.line.min(t1 + self.lines as u64 - 1);
        self.watch = SelectionWatch {
            lo,
            span: if lo <= hi { hi - lo + 1 } else { 0 },
            sel: Some(new),
            a: na,
            b: nb,
            state: Outcome::Same,
        };
        true
    }

    /// After parsing: clear the selection if the output changed the text
    /// under it, or dropped its lines.
    pub(crate) fn check_selection(&mut self) {
        let w = std::mem::take(&mut self.watch);
        if self.selection.is_none() || self.selection != w.sel {
            // The parser itself moved or cleared it.
            return;
        }
        let first_kept = if self.alt_active {
            self.top_line
        } else {
            self.top_line.saturating_sub(self.scrollback.len() as u64)
        };
        let changed = w.a.line < first_kept
            || match w.state {
                Outcome::Same => false,
                Outcome::Changed => true,
                Outcome::Read { from, ref text } => self.text_in(from, w.b, w.panel()) != *text,
            };
        if changed {
            self.selection = None;
        }
    }

    /// Screen row `row` holds a watched line.
    #[inline(always)]
    pub(super) fn watched(&self, row: usize) -> bool {
        self.watch.covers(self.top_line + row as u64)
    }

    /// Output that may change the selected text is coming: read it as it
    /// is, to compare at the end of the parse call, and stop watching.
    #[cold]
    #[inline(never)]
    pub(super) fn watch_read(&mut self) {
        if self.watch.span == 0 {
            return;
        }
        self.watch.stop();
        let from = self.watch.a.max(Point {
            line: self.top_line,
            col: 0,
        });
        let text = self.text_in(from, self.watch.b, self.watch.panel());
        self.watch.state = Outcome::Read { from, text };
    }

    #[cold]
    #[inline(never)]
    fn watch_changed(&mut self) {
        self.watch.stop();
        self.watch.state = Outcome::Changed;
    }

    /// Some of the selection's lines are watched (on the screen, with
    /// the outcome not yet known).
    #[inline(always)]
    pub(super) fn watching(&self) -> bool {
        self.watch.span != 0
    }

    /// Screen rows `rows` are about to move or be lost (a scroll inside
    /// the screen): if any is watched, read the selected text first.
    #[inline(always)]
    pub(super) fn watch_rows(&mut self, rows: Range<usize>) {
        if self.watching() {
            self.watch_rows_slow(rows);
        }
    }

    #[cold]
    #[inline(never)]
    fn watch_rows_slow(&mut self, rows: Range<usize>) {
        if rows.is_empty() {
            return;
        }
        let (first, last) = (self.top_line + rows.start as u64, self.top_line + rows.end as u64 - 1);
        if first < self.watch.lo + self.watch.span && last >= self.watch.lo {
            self.watch_read();
        }
    }

    /// Watched row `row` is about to change in a way only reading its
    /// text tells apart.
    #[inline]
    pub(super) fn watch_row(&mut self, row: usize) {
        if self.watched(row) {
            self.watch_read();
        }
    }

    /// Set row `row`'s soft wrap: a selected line's text depends on it.
    #[inline]
    pub(super) fn set_wrapped(&mut self, row: usize, wrapped: bool) {
        if self.screen[row].wrapped != wrapped && self.watched(row) {
            self.watch_wrap(row);
        }
        self.screen[row].wrapped = wrapped;
    }

    /// Watched row `row`'s soft wrap is about to change. The selection's
    /// last line reads the same either way; any other gains or loses the
    /// line break after it, which no other change in its cells can make
    /// up for.
    #[cold]
    #[inline(never)]
    fn watch_wrap(&mut self, row: usize) {
        // A panel's selection is a line for each row, wrapped or not.
        if self.top_line + row as u64 != self.watch.b.line && self.watch.panel().is_none() {
            self.watch_changed();
        }
    }

    /// Watched row `row`'s cells from `col` are about to be written with
    /// the printable ASCII `bytes`. A plain cell written with another
    /// character changes the text; the others (wide characters and
    /// clusters) are written one by one, through [`Terminal::watch_char`].
    #[cold]
    #[inline(never)]
    pub(super) fn watch_ascii(&mut self, row: usize, col: usize, bytes: &[u8]) {
        let sel = self.watch.cols(self.top_line + row as u64);
        let cells = &self.screen[row].cells;
        let from = sel.start.max(col);
        let to = sel.end.min(col + bytes.len());
        let changed = (from..to).any(|c| plain_text(cells[c]).is_some_and(|t| t != bytes[c - col] as u32));
        if changed {
            self.watch_changed();
        }
    }

    /// Watched row `row`'s cell at `col` is about to be written with `c`,
    /// `width` cells wide (and whatever wide character it overwrites part
    /// of erased).
    #[cold]
    #[inline(never)]
    pub(super) fn watch_char(&mut self, row: usize, col: usize, c: char, width: usize) {
        let sel = self.watch.cols(self.top_line + row as u64);
        let touched = |r: Range<usize>| sel.start < r.end && r.start < sel.end;
        if c == PLACEHOLDER {
            // It becomes a placeholder cell, which reads as a space, when
            // images are on.
            if touched(col.saturating_sub(1)..col + width + 1) {
                self.watch_read();
            }
            return;
        }
        let cells = &self.screen[row].cells;
        let old = cells[col];
        if width == 1
            && let Some(t) = plain_text(old)
        {
            if sel.contains(&col) && t != c as u32 {
                self.watch_changed();
            }
            return;
        }
        if width == 2 && plain_text(old).is_some() && cells.get(col + 1).copied().and_then(plain_text).is_some() {
            // Two characters become one wide one: a change where it
            // starts. Its spacer reads as nothing, which a space at the
            // end of a line also may.
            if sel.contains(&col) {
                self.watch_changed();
            } else if sel.contains(&(col + 1)) {
                self.watch_read();
            }
            return;
        }
        // A wide character or cluster is overwritten: the erase that
        // cleans it up may reach a cell either side.
        if !touched(col.saturating_sub(1)..col + width + 1) {
            return;
        }
        let same_wide = width == 2
            && old.is_wide()
            && !old.is_cluster()
            && !old.is_placeholder()
            && old.codepoint() == c as u32
            && cells.get(col + 1).is_some_and(|s| s.is_spacer());
        if !same_wide {
            self.watch_read();
        }
    }

    /// Cells `start..end` of watched row `row` are about to be erased.
    /// Blank cells erased change nothing. Plain ones, one of them not a
    /// space, change the text: the first such one reads as a space or is
    /// trimmed after, with the text before it as it was (or, past where a
    /// wider row wrapped, the row's text ends sooner). Wide characters
    /// and clusters are read.
    #[cold]
    #[inline(never)]
    pub(super) fn watch_erase(&mut self, row: usize, start: usize, end: usize) {
        let sel = self.watch.cols(self.top_line + row as u64);
        let cells = &self.screen[row].cells;
        let (from, to) = (sel.start.max(start), sel.end.min(end));
        let mut text = false;
        for &cell in cells.get(from..to).unwrap_or_default() {
            match plain_text(cell) {
                Some(0x20) => {}
                Some(_) => text = true,
                None => return self.watch_read(),
            }
        }
        if text {
            self.watch_changed();
        }
    }
}
