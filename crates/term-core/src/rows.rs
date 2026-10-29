//! Reading the screen and history: [`RowRef`] for a row as it is shown and
//! [`CellRef`] for one of its cells. These are the only way term-render and
//! term-app read cells, so the storage behind them (8-byte cells holding an
//! id in a style table) can change without touching those crates.

use std::borrow::Cow;

pub use crate::cell::Width;
use crate::cell::{Cell, Row};
use crate::clusters::Clusters;
use crate::style::Style;

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

/// One cell, read from a [`RowRef`].
#[derive(Clone, Copy, Debug)]
pub struct CellRef<'a> {
    cell: Cell,
    /// The styles its row's style ids index: the screen's table, or a
    /// history row's own list.
    styles: &'a [Style],
    /// Its row's cluster store, which a cluster cell's slot indexes.
    clusters: Option<&'a Clusters>,
}

/// Identifies a cell's style cheaply: two cells **of the same row** with
/// equal keys have equal [`Style`]s. Frame building uses it to reuse the
/// previous cell's resolved colours across runs of same-styled text. It is
/// the cell's style id, so comparing keys is one compare.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StyleKey(u32);

impl<'a> CellRef<'a> {
    #[cfg(test)]
    pub(crate) fn new(cell: Cell, styles: &'a [Style]) -> CellRef<'a> {
        CellRef {
            cell,
            styles,
            clusters: None,
        }
    }

    #[inline]
    pub(crate) fn in_row(cell: Cell, styles: &'a [Style], clusters: Option<&'a Clusters>) -> CellRef<'a> {
        CellRef { cell, styles, clusters }
    }

    #[inline]
    pub fn content(&self) -> Content<'a> {
        let cp = self.cell.codepoint();
        if self.cell.is_cluster() {
            return Content::Cluster(self.clusters.map_or("", |x| x.get(cp)));
        }
        if cp == 0 {
            return Content::Empty;
        }
        match char::from_u32(cp) {
            Some(c) => Content::Char(c),
            None => Content::Empty,
        }
    }

    #[inline]
    pub fn width(&self) -> Width {
        self.cell.width()
    }

    #[inline]
    pub fn style(&self) -> Style {
        self.styles.get(self.cell.style_id() as usize).copied().unwrap_or_default()
    }

    /// See [`StyleKey`].
    #[inline]
    pub fn style_key(&self) -> StyleKey {
        StyleKey(self.cell.style_id())
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
        self.cell.is_spacer()
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

/// One row as shown: a screen row, or a history row expanded from its
/// stored form. History rows may be shorter than the screen is wide; cells
/// past the end read as blank.
pub struct RowRef<'a> {
    row: Cow<'a, Row>,
    styles: Cow<'a, [Style]>,
}

impl<'a> RowRef<'a> {
    /// `row`'s style ids index `styles`.
    pub(crate) fn new(row: Cow<'a, Row>, styles: Cow<'a, [Style]>) -> RowRef<'a> {
        RowRef { row, styles }
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
        CellRef::in_row(
            self.row.cells.get(col).copied().unwrap_or(Cell::BLANK),
            &self.styles,
            self.row.clusters(),
        )
    }

    /// The stored cells, left to right.
    pub fn cells(&self) -> impl DoubleEndedIterator<Item = CellRef<'_>> + ExactSizeIterator + '_ {
        let styles: &[Style] = &self.styles;
        let clusters = self.row.clusters();
        self.row.cells.iter().map(move |&c| CellRef::in_row(c, styles, clusters))
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
    use crate::style::{Attrs, Underline};

    fn styles() -> Vec<Style> {
        vec![
            Style::DEFAULT,
            Style {
                fg: Color::indexed(3),
                bg: Color::rgb(1, 2, 3),
                underline: Underline::Curly,
                attrs: Attrs::BOLD,
                ..Style::DEFAULT
            },
        ]
    }

    #[test]
    fn style_comes_from_the_rows_table() {
        let styles = styles();
        let c = CellRef::new(Cell::template(1, true).with_char('a'), &styles);
        assert_eq!(c.style(), styles[1]);
        assert_eq!(c.style_key(), StyleKey(1));
        let plain = CellRef::new(Cell::BLANK.with_char('a'), &styles);
        assert_eq!(plain.style(), Style::DEFAULT);
        assert_ne!(plain.style_key(), c.style_key());
        // An id the list doesn't have reads as the default (never stored).
        assert_eq!(CellRef::new(Cell::template(9, false), &styles).style(), Style::DEFAULT);
    }

    #[test]
    fn content_and_width() {
        let styles = styles();
        let t = Cell::template(1, true);
        let wide = CellRef::new(t.with_char('漢').with_width(Width::Wide), &styles);
        assert_eq!((wide.content(), wide.width()), (Content::Char('漢'), Width::Wide));
        let spacer = CellRef::new(t.with_width(Width::Spacer), &styles);
        assert_eq!((spacer.content(), spacer.width()), (Content::Empty, Width::Spacer));
        assert!(spacer.is_spacer() && !wide.is_spacer());
        assert!(!spacer.is_blank(), "a spacer with a background shows");
        let blank = CellRef::new(Cell::BLANK, &styles);
        assert_eq!(blank.content(), Content::Empty);
        assert_eq!(blank.ch(), ' ');
        assert!(blank.is_blank());
        let space = CellRef::new(Cell::BLANK.with_char(' '), &styles);
        assert_eq!(space.content(), Content::Char(' '));
        assert!(!space.is_blank(), "a typed space is not blank");
    }

    #[test]
    fn row_ref_reads_past_the_end_as_blank() {
        let mut row = Row::new(3, Cell::BLANK);
        let t = Cell::template(1, true);
        row.cells[0] = t.with_char('a');
        row.cells[1] = t.with_char('漢').with_width(Width::Wide);
        row.cells[2] = t.with_width(Width::Spacer);
        row.wrapped = true;
        let r = RowRef::new(Cow::Owned(row), Cow::Owned(styles()));
        assert_eq!(r.len(), 3);
        assert!(r.wrapped());
        assert_eq!(r.text(), "a漢");
        assert!(r.cell(10).is_blank());
        assert_eq!(r.cells().count(), 3);
        assert_eq!(r.cell(2).style(), styles()[1]);
    }
}
