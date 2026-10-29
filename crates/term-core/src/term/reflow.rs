//! Reflowing the primary screen on resize (§12.2 of the cells design):
//! wrapped lines are split again at the new width, with clusters and wide
//! characters kept whole; lines are pulled back from history when the
//! screen grows; and everything that holds a place in the text (the
//! cursor, the saved cursor, the selection, the hover point, the view)
//! moves with it. History is never rewritten: only the stored lines of the
//! logical line the screen starts in, and whole lines pulled back, are
//! taken out of it.

use std::collections::VecDeque;

use super::{Point, Terminal};
use crate::cell::{Cell, Mark, Row, Width};
use crate::style::Style;

/// A logical line's cells laid end to end, without the edge spacers that
/// padded its rows. Cluster cells hold an index into `texts` instead of a
/// slot in a row's store.
#[derive(Default)]
pub(super) struct Flat {
    pub(super) cells: Vec<Cell>,
    texts: Vec<String>,
    /// OSC 133 marks, by offset in the line.
    marks: Vec<(usize, Mark)>,
    /// What the last row is padded with: the style of an erased tail that
    /// shows (a `FILL` line), or the blank.
    pub(super) fill: Cell,
}

/// Where a logical line was, and where it is now.
struct Line {
    /// The line number its first old row had.
    old_line: u64,
    /// The offset in the line of each old row's first cell, and the
    /// line's length after them.
    old_starts: Vec<usize>,
    /// The offset in the line of each new row's first cell.
    starts: Vec<usize>,
    /// The cells it keeps (trailing blanks trimmed, never before the
    /// cursor).
    len: usize,
    /// Index of its first new row, counted from the first row of `out`.
    first_row: usize,
}

/// A row taken out of history, its styles interned in the screen's table,
/// and the cell its line is padded with.
type Pulled = (Row, Cell);

/// The cursor, by where it is in the text.
#[derive(Clone, Copy)]
struct TextCursor {
    line: usize,
    offset: usize,
}

impl Flat {
    /// Append `row` (the next row of the line) at `self.cells.len()`.
    pub(super) fn push_row(&mut self, row: &Row) -> usize {
        let start = self.cells.len();
        let mut n = row.cells.len();
        if row.wrapped && n > 0 && row.cells[n - 1].width() == Width::EdgeSpacer {
            n -= 1;
        }
        for (col, &c) in row.cells[..n].iter().enumerate() {
            if c.is_cluster() {
                let mut text = String::new();
                row.text_at(col, &mut text);
                self.cells
                    .push(Cell::from_bits(c.bits() & !(Cell::CONTENT | Cell::KIND)).with_cluster(self.texts.len() as u32));
                self.texts.push(text);
            } else if c.width() == Width::EdgeSpacer {
                // Not at the end of a wrapped row: padding all the same.
                self.cells.push(c.style_part());
            } else {
                self.cells.push(c);
            }
        }
        for m in row.marks() {
            self.marks.push((start + m.col as usize, m));
        }
        start
    }

    /// The cells kept: trailing cells that look like the padding are
    /// dropped, but none before `keep`.
    pub(super) fn trimmed_len(&self, keep: usize) -> usize {
        let fill = self.fill;
        let mut len = self.cells.len();
        while len > keep {
            let c = self.cells[len - 1];
            let empty = c.style_part() == c;
            if empty && (c == fill || (fill.is_blank() && c.is_blank())) {
                len -= 1;
            } else {
                break;
            }
        }
        // Marks past the text keep their cells.
        let marks = self.marks.iter().map(|&(off, _)| off + 1).max().unwrap_or(0);
        len.max(keep).max(marks.min(self.cells.len()))
    }
}

/// The cell an erased tail ending `row` shows, if its last cell is one: a
/// line padded with it at any width.
fn fill_of(row: &Row) -> Cell {
    match row.cells.last() {
        Some(&c) if c.style_part() == c && !c.is_blank() => c,
        _ => Cell::BLANK,
    }
}

/// The offsets at which rows `cols` wide start, for the first `len` cells:
/// a wide character that doesn't fit in a row's last column goes to the
/// next row whole.
pub(super) fn split(cells: &[Cell], len: usize, cols: usize) -> Vec<usize> {
    let mut starts = vec![0];
    let (mut pos, mut col) = (0, 0);
    while pos < len {
        let w = if cells.get(pos).is_some_and(|c| c.is_wide()) { 2 } else { 1 };
        if col + w > cols {
            starts.push(pos);
            col = 0;
        }
        col += w;
        pos += w;
        if col >= cols && pos < len {
            starts.push(pos);
            col = 0;
        }
    }
    starts
}

/// The row and column of offset `off` in a line split at `starts`.
pub(super) fn locate(starts: &[usize], off: usize, cols: usize) -> (usize, usize) {
    let r = starts.partition_point(|&s| s <= off).max(1) - 1;
    (r, (off - starts[r]).min(cols - 1))
}

/// The rows of `flat`'s first `len` cells, split at `starts`.
fn emit(flat: &Flat, len: usize, starts: &[usize], cols: usize, out: &mut Vec<Row>) {
    for r in 0..starts.len() {
        out.push(emit_row(flat, len, starts, cols, r));
    }
}

/// Row `r` of `flat`'s first `len` cells, split at `starts`.
pub(super) fn emit_row(flat: &Flat, len: usize, starts: &[usize], cols: usize, r: usize) -> Row {
    let from = starts[r];
    let last = r + 1 == starts.len();
    let end = if last { len } else { starts[r + 1] };
    let mut row = Row::new(cols, if last { flat.fill } else { Cell::BLANK });
    // Cells past the end of the old line (kept for the cursor) stay as
    // the padding.
    let to = end.min(from + cols).min(flat.cells.len().max(from));
    for (col, &c) in flat.cells[from..to].iter().enumerate() {
        if c.is_cluster() {
            row.cells[col] = Cell::from_bits(c.bits() & !(Cell::CONTENT | Cell::KIND));
            row.set_cluster(col, &flat.texts[c.codepoint() as usize]);
        } else {
            row.cells[col] = c;
        }
    }
    if !last {
        row.wrapped = true;
        if end - from < cols {
            row.cells[cols - 1] = Cell::BLANK.with_width(Width::EdgeSpacer);
        }
    }
    // A wide character cut off by `len` (it can't be, unless the line
    // was broken) loses its other half.
    if row.cells.last().is_some_and(|c| c.is_wide()) {
        row.free_clusters(cols - 1..cols);
        row.cells[cols - 1] = Cell::BLANK;
    }
    for &(off, m) in &flat.marks {
        if locate(starts, off, cols).0 == r {
            row.set_mark(Mark {
                col: (off - from).min(cols - 1) as u16,
                ..m
            });
        }
    }
    row
}

fn is_blank_row(row: &Row) -> bool {
    !row.wrapped && !row.has_marks() && row.cells.iter().all(|c| c.is_blank() && !c.is_wide_part())
}

impl Terminal {
    /// Take the newest history line out, its styles and link interned in
    /// the screen's table. The row is as wide as it was stored.
    pub(super) fn pop_history_row(&mut self) -> Option<Pulled> {
        if self.scrollback.is_empty() {
            return None;
        }
        let fill = self.scrollback.fill(self.scrollback.len() - 1);
        let (mut row, mut styles, mut links) = (Row::default(), Vec::new(), Vec::new());
        if !self.scrollback.pop_into(0, &mut row, &mut styles, &mut links) {
            return None;
        }
        let ids: Vec<Cell> = styles
            .iter()
            .map(|s| {
                let mut s = *s;
                if let Some(Some(l)) = links.get(s.link as usize) {
                    s.link = self.styles.links_mut().intern(l);
                }
                if s == Style::DEFAULT { Cell::BLANK } else { self.intern(s) }
            })
            .collect();
        for c in &mut row.cells {
            *c = c.with_style(ids[c.style_id() as usize]);
        }
        let fill = match row.cells.last() {
            Some(&c) if fill => c.style_part(),
            _ => Cell::BLANK,
        };
        Some((row, fill))
    }

    /// How many rows the newest logical line in history takes at `cols`,
    /// and how many stored lines it has; `None` if history is empty, the
    /// newest stored line is wrapped (the line goes on below), or the line
    /// has more than `max` stored lines.
    fn newest_line_rows(&self, cols: usize, max: usize) -> Option<(usize, usize)> {
        let len = self.scrollback.len();
        if len == 0 || self.scrollback.wrapped(len - 1) {
            return None;
        }
        let mut first = len - 1;
        while first > 0 && self.scrollback.wrapped(first - 1) {
            first -= 1;
            if len - first > max {
                return None;
            }
        }
        let mut flat = Flat::default();
        let (mut row, mut styles) = (Row::default(), Vec::new());
        for i in first..len {
            self.scrollback.get_into(i, &mut row, &mut styles);
            flat.push_row(&row);
        }
        // Style ids index each row's own `styles` here, so trimming a tail
        // that reaches back over a row compares ids loosely. The count is
        // only used to decide whether the line fits: a line that turns out
        // longer has its top rows pushed back into history (step 5).
        flat.fill = if self.scrollback.fill(len - 1) {
            row.cells.last().map_or(Cell::BLANK, |c| c.style_part())
        } else {
            Cell::BLANK
        };
        let n = flat.trimmed_len(0);
        Some((split(&flat.cells, n, cols).len(), len - first))
    }

    /// Reflow the primary screen (the active one) to `cols` × `lines`, as
    /// §12.2 says. The caller blocks style sweeps around it.
    pub(super) fn reflow(&mut self, cols: usize, lines: usize) {
        debug_assert!(!self.alt_active);
        let old_cols = self.screen.front().map_or(cols, |r| r.cells.len());
        let old_lines = self.screen.len();
        if old_cols == cols && old_lines == lines {
            return;
        }
        let max_pull = 4 * lines;
        let old_top = self.top_line;

        // 1. The stored lines of the logical line the screen starts in.
        let mut pulled: Vec<Pulled> = Vec::new();
        if cols != old_cols || lines > old_lines {
            while pulled.len() < max_pull && !self.scrollback.is_empty() && self.scrollback.wrapped(self.scrollback.len() - 1) {
                match self.pop_history_row() {
                    Some(p) => pulled.push(p),
                    None => break,
                }
            }
            pulled.reverse();
        }
        let first_line = self.top_line - pulled.len() as u64;
        let view = self.view_point();

        // 2. Group the old rows into logical lines.
        let screen: Vec<Row> = std::mem::take(&mut self.screen).into();
        let screen_fills: Vec<Cell> = screen.iter().map(fill_of).collect();
        let n_pulled = pulled.len();
        let mut old: Vec<(Row, Cell)> = pulled;
        old.extend(screen.into_iter().zip(screen_fills));

        let cursor_row = n_pulled + self.cursor.row.min(old.len().saturating_sub(n_pulled + 1));
        let cursor_off_in_row = self.cursor.col + self.cursor.pending_wrap as usize;
        let mut lines_meta: Vec<Line> = Vec::new();
        let mut out: Vec<Row> = Vec::new();
        let mut cursor = TextCursor { line: 0, offset: 0 };
        let mut i = 0;
        while i < old.len() {
            let mut j = i;
            while j + 1 < old.len() && old[j].0.wrapped {
                j += 1;
            }
            let mut flat = Flat::default();
            let mut old_starts = Vec::with_capacity(j + 1 - i);
            for (k, (row, _)) in old[i..=j].iter().enumerate() {
                let start = flat.push_row(row);
                old_starts.push(start);
                if i + k == cursor_row {
                    cursor = TextCursor {
                        line: lines_meta.len(),
                        offset: start + cursor_off_in_row,
                    };
                }
            }
            old_starts.push(flat.cells.len());
            flat.fill = old[j].1;
            let keep = if cursor.line == lines_meta.len() && (i..=j).contains(&cursor_row) {
                cursor.offset
            } else {
                0
            };
            let len = flat.trimmed_len(keep);
            let starts = split(&flat.cells, len, cols);
            let first_row = out.len();
            emit(&flat, len, &starts, cols, &mut out);
            lines_meta.push(Line {
                old_line: first_line + i as u64,
                old_starts,
                starts,
                len,
                first_row,
            });
            i = j + 1;
        }
        drop(old);

        // Where the cursor is now: a cursor just past the end of a line
        // that fills its last row waits there to wrap.
        let place_cursor = |l: &Line, offset: usize| -> (usize, usize, bool) {
            if offset > 0 && offset == l.len && offset.is_multiple_of(cols) && l.starts.last() != Some(&offset) {
                let (r, _) = locate(&l.starts, offset - 1, cols);
                (l.first_row + r, cols - 1, true)
            } else {
                let (r, c) = locate(&l.starts, offset, cols);
                (l.first_row + r, c, false)
            }
        };
        let (mut cur_row, cur_col, cur_pending) = place_cursor(&lines_meta[cursor.line], cursor.offset);

        // 3. Too many rows: blank rows below the cursor go first.
        while out.len() > lines && out.len() - 1 > cur_row && out.last().is_some_and(is_blank_row) {
            out.pop();
        }

        // 4. Too few: pull whole logical lines back from history, newest
        // first, while they fit.
        let mut front: VecDeque<Row> = VecDeque::new();
        let mut pulled_lines: Vec<Line> = Vec::new();
        let mut first_line = first_line;
        while out.len() + front.len() < lines {
            let Some((rows, stored)) = self.newest_line_rows(cols, max_pull) else {
                break;
            };
            if out.len() + front.len() + rows > lines {
                break;
            }
            let mut back: Vec<Pulled> = (0..stored).filter_map(|_| self.pop_history_row()).collect();
            back.reverse();
            first_line -= back.len() as u64;
            let mut flat = Flat::default();
            let mut old_starts: Vec<usize> = back.iter().map(|(r, _)| flat.push_row(r)).collect();
            old_starts.push(flat.cells.len());
            flat.fill = back.last().map_or(Cell::BLANK, |b| b.1);
            let len = flat.trimmed_len(0);
            let starts = split(&flat.cells, len, cols);
            let mut rows = Vec::new();
            emit(&flat, len, &starts, cols, &mut rows);
            for r in rows.into_iter().rev() {
                front.push_front(r);
            }
            pulled_lines.push(Line {
                old_line: first_line,
                old_starts,
                starts,
                len,
                // Counted back from the first row of `out`, fixed up below.
                first_row: front.len(),
            });
        }
        let shift = front.len();
        for l in &mut lines_meta {
            l.first_row += shift;
        }
        for l in &mut pulled_lines {
            l.first_row = shift - l.first_row;
        }
        cur_row += shift;
        let mut all: VecDeque<Row> = front;
        all.extend(out);
        pulled_lines.reverse();
        pulled_lines.extend(lines_meta);
        let lines_meta = pulled_lines;

        // 5. Still too many: rows from the top go into history.
        let new_first = first_line;
        let mut pushed = 0usize;
        while all.len() > lines {
            let row = all.pop_front().expect("rows");
            self.push_scrollback(&row);
            pushed += 1;
        }
        let total_rows = all.len() + pushed;
        while all.len() < lines {
            all.push_back(Row::new(cols, Cell::BLANK));
        }
        self.screen = all;
        self.top_line = new_first + pushed as u64;

        // 6. Tracked positions back to Points. None: in a blank row below
        // the cursor that was dropped.
        let find = |p: Point| -> Option<Point> {
            if p.line < first_line {
                return Some(p);
            }
            let k = lines_meta.partition_point(|l| l.old_line <= p.line).max(1) - 1;
            let l = &lines_meta[k];
            let r = ((p.line - l.old_line) as usize).min(l.old_starts.len() - 2);
            // A column past the row's cells (history at another width) is its
            // last cell.
            let cap = l.old_starts[r + 1] - l.old_starts[r];
            let off = l.old_starts[r] + p.col.min(cap.saturating_sub(1));
            let (nr, nc) = locate(&l.starts, off, cols);
            let row = l.first_row + nr;
            if row >= total_rows {
                return None;
            }
            Some(Point {
                line: new_first + row as u64,
                col: nc,
            })
        };
        // A position in a dropped row goes to the end of the last row left.
        let remap = |p: Point| -> Point {
            find(p).unwrap_or(Point {
                line: new_first + total_rows.max(1) as u64 - 1,
                col: cols - 1,
            })
        };
        // kitty placements keep their size in cells and move with their
        // top-left cell; one whose cell was dropped goes.
        self.graphics.remap_placements(first_line, |line, col| {
            find(Point { line, col: col as usize }).map(|p| (p.line, p.col.min(u16::MAX as usize) as u16))
        });
        let (row, col, pending) = (cur_row, cur_col, cur_pending);
        if row < pushed {
            // The cursor's row went into history: it stays at the top.
            self.cursor.row = 0;
            self.cursor.col = col;
            self.cursor.pending_wrap = false;
        } else {
            self.cursor.row = (row - pushed).min(lines - 1);
            self.cursor.col = col;
            self.cursor.pending_wrap = pending;
        }
        if let Some(saved) = self.saved_primary.as_ref().map(|s| s.cursor) {
            let old = Point {
                line: old_top + saved.row as u64,
                col: saved.col + saved.pending_wrap as usize,
            };
            let p = remap(old);
            let s = self.saved_primary.as_mut().expect("saved");
            s.cursor.row = p.line.saturating_sub(self.top_line).min(lines as u64 - 1) as usize;
            s.cursor.col = p.col.min(cols - 1);
            s.cursor.pending_wrap = false;
        }
        if let Some(sel) = &mut self.selection {
            sel.anchor = remap(sel.anchor);
            sel.head = remap(sel.head);
        }
        self.hover = self.hover.map(remap);
        // The view's first cell, on its row at the new width (§12.3); on the
        // screen now, the view follows output.
        // (`Terminal::resize` puts it at its row's start once the width is
        // set; reads round it down to that meanwhile.)
        self.drop_layouts();
        self.view = view.map(remap).filter(|p| p.line < self.top_line);
    }
}
