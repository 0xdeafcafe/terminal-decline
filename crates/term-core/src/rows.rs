//! Reading the screen and history: [`RowRef`] for a row as it is shown and
//! [`CellRef`] for one of its cells. These are the only way term-render and
//! term-app read cells, so the storage behind them (today 12-byte cells with
//! the style inline) can change without touching those crates.

use std::borrow::Cow;
use std::marker::PhantomData;

use crate::cell::{Cell, Row, flags};
use crate::style::{Attrs, Style, Underline};

/// What a cell shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Content<'a> {
    /// Nothing written (or erased). Draws as a space.
    Empty,
    /// One Unicode scalar.
    Char(char),
    /// A grapheme cluster of several scalars (`e` + U+0301, emoji sequences).
    Cluster(&'a str),
    /// A piece of an image (kitty Unicode placeholders): its row and column.
    Placeholder { row: u16, col: u16 },
}

/// How much of a row a cell's content takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Width {
    /// One cell.
    Narrow = 0,
    /// The left half of a two-cell character.
    Wide = 1,
    /// The right half of a two-cell character; draws nothing itself.
    Spacer = 2,
    /// The last column of a row whose next wide character didn't fit and
    /// moved to the next row.
    EdgeSpacer = 3,
}

/// One cell, read from a [`RowRef`].
#[derive(Clone, Copy, Debug)]
pub struct CellRef<'a> {
    cell: Cell,
    style: Style,
    _row: PhantomData<&'a Row>,
}

impl<'a> CellRef<'a> {
    #[inline]
    fn new(cell: Cell) -> CellRef<'a> {
        CellRef {
            cell,
            style: style_of(cell),
            _row: PhantomData,
        }
    }

    #[inline]
    pub fn content(&self) -> Content<'a> {
        match char::from_u32(self.cell.codepoint()) {
            Some(c) if c != '\0' => Content::Char(c),
            _ => Content::Empty,
        }
    }

    #[inline]
    pub fn width(&self) -> Width {
        if self.cell.has(flags::WIDE) {
            Width::Wide
        } else if self.cell.has(flags::WIDE_SPACER) {
            Width::Spacer
        } else {
            Width::Narrow
        }
    }

    #[inline]
    pub fn style(&self) -> &Style {
        &self.style
    }

    /// Nothing would be drawn here if the row ended at this cell: no
    /// content, and no background or decoration that shows on an empty
    /// cell. A typed space is not blank.
    #[inline]
    pub fn is_blank(&self) -> bool {
        self.cell.is_blank()
    }

    /// A cell that only makes room for a wide character (its right half, or
    /// an edge spacer) and draws nothing of its own.
    #[inline]
    pub fn is_spacer(&self) -> bool {
        matches!(self.width(), Width::Spacer | Width::EdgeSpacer)
    }

    /// The content as text: a space for empty cells and placeholders.
    pub fn push_text(&self, out: &mut String) {
        match self.content() {
            Content::Char(c) => out.push(c),
            Content::Cluster(s) => out.push_str(s),
            Content::Empty | Content::Placeholder { .. } => out.push(' '),
        }
    }

    /// The first character of the content; a space for empty cells and
    /// placeholders.
    #[inline]
    pub fn ch(&self) -> char {
        match self.content() {
            Content::Char(c) => c,
            Content::Cluster(s) => s.chars().next().unwrap_or(' '),
            Content::Empty | Content::Placeholder { .. } => ' ',
        }
    }
}

/// The style a 12-byte cell carries inline.
#[inline]
pub(crate) fn style_of(cell: Cell) -> Style {
    let p = cell.packed;
    let mut attrs = Attrs::empty();
    for (flag, attr) in [
        (flags::BOLD, Attrs::BOLD),
        (flags::DIM, Attrs::DIM),
        (flags::ITALIC, Attrs::ITALIC),
        (flags::INVERSE, Attrs::INVERSE),
        (flags::HIDDEN, Attrs::HIDDEN),
        (flags::STRIKE, Attrs::STRIKE),
        (flags::BLINK, Attrs::BLINK),
    ] {
        if p & flag != 0 {
            attrs |= attr;
        }
    }
    let underline = if p & flags::DOUBLE_UNDERLINE != 0 {
        Underline::Double
    } else if p & flags::UNDERLINE != 0 {
        Underline::Single
    } else {
        Underline::None
    };
    Style {
        fg: cell.fg,
        bg: cell.bg,
        attrs,
        underline,
        ..Style::DEFAULT
    }
}

/// One row as shown: a screen row, or a history row expanded from its
/// stored form. History rows may be shorter than the screen is wide; cells
/// past the end read as blank.
pub struct RowRef<'a> {
    row: Cow<'a, Row>,
}

impl<'a> RowRef<'a> {
    pub(crate) fn new(row: Cow<'a, Row>) -> RowRef<'a> {
        RowRef { row }
    }

    /// Cells stored in this row.
    #[inline]
    pub fn len(&self) -> usize {
        self.row.cells.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.row.cells.is_empty()
    }

    /// The cell at `col`; blank past the end of the row.
    #[inline]
    pub fn cell(&self, col: usize) -> CellRef<'_> {
        CellRef::new(self.row.get(col))
    }

    /// The stored cells, left to right.
    pub fn cells(&self) -> impl DoubleEndedIterator<Item = CellRef<'_>> + ExactSizeIterator + '_ {
        self.row.cells.iter().map(|&c| CellRef::new(c))
    }

    /// The line continues onto the next row (soft wrap).
    #[inline]
    pub fn wrapped(&self) -> bool {
        self.row.wrapped
    }

    /// The row's text: one entry per character (spacers skipped), empty
    /// cells as spaces, nothing trimmed.
    pub fn text(&self) -> String {
        let mut s = String::with_capacity(self.len());
        for c in self.cells().filter(|c| !c.is_spacer()) {
            c.push_text(&mut s);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Color;

    fn cell(packed: u32) -> Cell {
        Cell {
            packed,
            fg: Color::indexed(3),
            bg: Color::rgb(1, 2, 3),
        }
    }

    #[test]
    fn every_flag_reads_as_its_attribute() {
        for (flag, attr) in [
            (flags::BOLD, Attrs::BOLD),
            (flags::DIM, Attrs::DIM),
            (flags::ITALIC, Attrs::ITALIC),
            (flags::INVERSE, Attrs::INVERSE),
            (flags::HIDDEN, Attrs::HIDDEN),
            (flags::STRIKE, Attrs::STRIKE),
            (flags::BLINK, Attrs::BLINK),
        ] {
            let s = style_of(cell('a' as u32 | flag));
            assert_eq!(s.attrs, attr, "{flag:#x}");
            assert_eq!(s.underline, Underline::None);
            assert_eq!((s.fg, s.bg), (Color::indexed(3), Color::rgb(1, 2, 3)));
        }
        let s = style_of(cell(flags::STYLE_MASK));
        assert_eq!(s.underline, Underline::Double, "double wins over single");
        assert_eq!(style_of(cell(flags::UNDERLINE)).underline, Underline::Single);
        assert_eq!(style_of(Cell::BLANK), Style::DEFAULT);
    }

    #[test]
    fn content_and_width() {
        let wide = CellRef::new(cell('漢' as u32 | flags::WIDE));
        assert_eq!((wide.content(), wide.width()), (Content::Char('漢'), Width::Wide));
        let spacer = CellRef::new(cell(flags::WIDE_SPACER));
        assert_eq!((spacer.content(), spacer.width()), (Content::Empty, Width::Spacer));
        assert!(spacer.is_spacer() && !wide.is_spacer());
        let blank = CellRef::new(Cell::BLANK);
        assert_eq!(blank.content(), Content::Empty);
        assert_eq!(blank.ch(), ' ');
        assert!(blank.is_blank());
        let space = CellRef::new(Cell {
            packed: ' ' as u32,
            ..Cell::BLANK
        });
        assert_eq!(space.content(), Content::Char(' '));
        assert!(!space.is_blank(), "a typed space is not blank");
    }

    #[test]
    fn row_ref_reads_past_the_end_as_blank() {
        let mut row = Row::new(3, Cell::BLANK);
        row.cells[0] = cell('a' as u32);
        row.cells[1] = cell('漢' as u32 | flags::WIDE);
        row.cells[2] = cell(flags::WIDE_SPACER);
        row.wrapped = true;
        let r = RowRef::new(Cow::Owned(row));
        assert_eq!(r.len(), 3);
        assert!(r.wrapped());
        assert_eq!(r.text(), "a漢");
        assert!(r.cell(10).is_blank());
        assert_eq!(r.cells().count(), 3);
    }
}
