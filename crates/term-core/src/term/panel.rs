//! Panels a program draws (cells §13.2): the box a drag starts in, which
//! the selection then keeps to.
//!
//! Its rows are the program's scrolling area, the scroll region's top
//! and bottom margins (DECSTBM), when one narrower than the screen is set
//! and the drag starts inside it. Its columns are those between the
//! vertical box-drawing borders nearest left and right of where the drag
//! starts, on that row. A border character counts only if it really is
//! a vertical line, with the same column holding a vertical border (or a
//! corner or tee joining one) on the row above or below, so a stray `│`
//! in prose doesn't split the screen. At the shell's prompt there are no
//! panels: one command's output isn't one.

use super::{Point, Terminal};
use crate::cell::Cell;

/// A panel a program draws: the columns inside its vertical borders and,
/// when the selection keeps to the scroll region, the region's rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Panel {
    /// The first and last column inside its borders (both included); the
    /// screen's edge where there is no border on that side.
    pub left: usize,
    pub right: usize,
    /// The scroll region's first and last screen row (both included),
    /// when the panel is held to them.
    pub rows: Option<(usize, usize)>,
}

/// A vertical border: a vertical line, or a tee or cross with one
/// running through it.
fn vertical(c: u32) -> bool {
    matches!(
        c,
        0x2502 | 0x2503 // │ ┃
        | 0x2506 | 0x2507 // ┆ ┇
        | 0x250A | 0x250B // ┊ ┋
        | 0x251C..=0x252B // ├ … ┫
        | 0x253C..=0x254B // ┼ … ╋
        | 0x254E | 0x254F // ╎ ╏
        | 0x2551 // ║
        | 0x255E..=0x2563 // ╞ … ╣
        | 0x256A..=0x256C // ╪ ╫ ╬
        | 0x257D | 0x257F // ╽ ╿
    )
}

/// A character a vertical border above or below joins: another vertical
/// border, a corner or a horizontal tee.
fn joins_vertical(c: u32) -> bool {
    vertical(c)
        || matches!(
            c,
            0x250C..=0x251B // ┌ … ┛
            | 0x252C..=0x253B // ┬ … ┻
            | 0x2552..=0x255D // ╒ … ╝
            | 0x2564..=0x2569 // ╤ … ╩
            | 0x256D..=0x2570 // ╭ ╮ ╯ ╰
        )
}

/// A plain narrow cell's character (none for wide ones, clusters and
/// placeholders).
fn scalar(cell: Cell) -> Option<u32> {
    (cell.bits() & (Cell::WIDTH | Cell::KIND) == 0).then(|| cell.codepoint())
}

impl Terminal {
    /// The panel a drag starting at `p` keeps to, if it starts in one:
    /// on the screen, while a program runs (not at the shell's prompt, as
    /// the shell's OSC 133 marks tell), inside a scroll region narrower
    /// than the screen or between vertical borders.
    pub fn panel_at(&self, p: Point) -> Option<Panel> {
        if p.line < self.top_line || p.line >= self.top_line + self.lines as u64 {
            return None;
        }
        if self.at_prompt && !self.alt_active {
            return None;
        }
        let row = (p.line - self.top_line) as usize;
        let col = p.col.min(self.cols - 1);
        let narrower = self.scroll_top > 0 || self.scroll_bottom < self.lines - 1;
        let rows = (narrower && (self.scroll_top..=self.scroll_bottom).contains(&row)).then_some((self.scroll_top, self.scroll_bottom));
        let (mut left, mut right) = (0, self.cols - 1);
        if !self.border_at(row, col) {
            if let Some(c) = (0..col).rev().find(|&c| self.border_at(row, c)) {
                left = c + 1;
            }
            if let Some(c) = (col + 1..self.cols).find(|&c| self.border_at(row, c)) {
                right = c - 1;
            }
        }
        let bordered = left > 0 || right < self.cols - 1;
        (bordered || rows.is_some()).then_some(Panel { left, right, rows })
    }

    /// Screen cell (`row`, `col`) is a vertical border: a vertical line
    /// character, with one joining it in the same column above or below.
    fn border_at(&self, row: usize, col: usize) -> bool {
        let at = |r: usize| self.screen.get(r).and_then(|row| row.cells.get(col).copied()).and_then(scalar);
        at(row).is_some_and(vertical)
            && (row.checked_sub(1).and_then(at).is_some_and(joins_vertical) || at(row + 1).is_some_and(joins_vertical))
    }

    /// `to` moved inside `panel`: into its columns, and onto the scroll
    /// region's rows when it keeps to them (a drag past an edge stops at
    /// the edge, under the mouse's column).
    pub(super) fn clamp_to_panel(&self, panel: Panel, to: Point) -> Point {
        let line = match panel.rows {
            Some((top, bottom)) => to.line.clamp(self.top_line + top as u64, self.top_line + bottom as u64),
            None => to.line,
        };
        Point {
            line,
            col: to.col.clamp(panel.left, panel.right),
        }
    }
}
