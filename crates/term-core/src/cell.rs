//! Packed cell storage. One cell is 8 bytes (§4 of the cells design): what
//! it shows, how wide it is, and an id in its screen's style table.

/// A colour as stored in a style. The top byte says what kind it is:
/// 0 = the terminal's default, 1 = a palette index, 2 = 24-bit RGB.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[repr(transparent)]
pub struct Color(pub u32);

impl Color {
    pub const DEFAULT: Color = Color(0);

    pub const fn indexed(i: u8) -> Color {
        Color(0x0100_0000 | i as u32)
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color(0x0200_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    pub const fn kind(self) -> ColorKind {
        match self.0 >> 24 {
            1 => ColorKind::Indexed(self.0 as u8),
            2 => ColorKind::Rgb((self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8),
            _ => ColorKind::Default,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorKind {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
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

/// One screen cell, 8 bytes:
///
/// ```text
///  63            32 31   27  26  25  24 23  22 21  20         0
/// ┌────────────────┬───────┬───┬───┬──────┬──────┬────────────┐
/// │ style id (32)  │ 0 (5) │SE │ P │width │ kind │ content(21)│
/// └────────────────┴───────┴───┴───┴──────┴──────┴────────────┘
/// ```
///
/// - content: a Unicode scalar, 0 = empty (kind 0; the other kinds come
///   with clusters and placeholders);
/// - width: a [`Width`];
/// - P: protected (DECSCA), always 0 for now;
/// - SE, "shows when empty": a copy of the style's
///   [`Style::shows_empty`](crate::Style::shows_empty), so blank tests need
///   no table lookup;
/// - style id: an index into the screen's style table, 0 = the default.
///
/// The blank cell is all zero, so a row of blanks resets with a memset.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash, Debug)]
pub struct Cell(u64);

const _: () = assert!(std::mem::size_of::<Cell>() == 8);

impl Cell {
    pub const BLANK: Cell = Cell(0);
    pub const CONTENT: u64 = (1 << 21) - 1;
    pub const KIND: u64 = 3 << 21;
    pub const WIDTH: u64 = 3 << 23;
    const WIDTH_SHIFT: u32 = 23;
    pub const PROTECTED: u64 = 1 << 25;
    pub const SHOWS_EMPTY: u64 = 1 << 26;
    /// The bits a cell's style gives it: the style id and SE.
    pub const STYLE: u64 = 0xFFFF_FFFF << 32 | Self::SHOWS_EMPTY;
    /// Bits that make a cell draw something when a row ends there.
    pub const NOT_BLANK: u64 = Self::CONTENT | Self::KIND | Self::SHOWS_EMPTY;

    /// The template the pen writes with: style id and SE, empty, narrow.
    #[inline]
    pub const fn template(style: u32, shows_empty: bool) -> Cell {
        Cell((style as u64) << 32 | (shows_empty as u64) << 26)
    }

    /// The raw bits.
    #[inline]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn from_bits(bits: u64) -> Cell {
        Cell(bits)
    }

    /// This cell's style (id and SE) with `c` in it, narrow.
    #[inline]
    pub const fn with_char(self, c: char) -> Cell {
        Cell(self.0 & Self::STYLE | c as u64)
    }

    #[inline]
    pub const fn with_width(self, w: Width) -> Cell {
        Cell(self.0 & !Self::WIDTH | (w as u64) << Self::WIDTH_SHIFT)
    }

    /// The same content and width in another style.
    #[inline]
    pub const fn with_style(self, template: Cell) -> Cell {
        Cell(self.0 & !Self::STYLE | template.0 & Self::STYLE)
    }

    /// Just the style part (id and SE), as a template.
    #[inline]
    pub const fn style_part(self) -> Cell {
        Cell(self.0 & Self::STYLE)
    }

    #[inline]
    pub const fn style_id(self) -> u32 {
        (self.0 >> 32) as u32
    }

    #[inline]
    pub const fn shows_empty(self) -> bool {
        self.0 & Self::SHOWS_EMPTY != 0
    }

    #[inline]
    pub const fn width(self) -> Width {
        match (self.0 >> Self::WIDTH_SHIFT) & 3 {
            0 => Width::Narrow,
            1 => Width::Wide,
            2 => Width::Spacer,
            _ => Width::EdgeSpacer,
        }
    }

    #[inline]
    pub const fn is_wide(self) -> bool {
        self.0 & Self::WIDTH == (Width::Wide as u64) << Self::WIDTH_SHIFT
    }

    /// The right half of a wide character, or an edge spacer.
    #[inline]
    pub const fn is_spacer(self) -> bool {
        self.0 & (2 << Self::WIDTH_SHIFT) != 0
    }

    /// Either half of a wide character (or an edge spacer).
    #[inline]
    pub const fn is_wide_part(self) -> bool {
        self.0 & Self::WIDTH != 0
    }

    /// The content bits: a Unicode scalar, 0 = empty.
    #[inline]
    pub const fn codepoint(self) -> u32 {
        (self.0 & Self::CONTENT) as u32
    }

    /// The character shown; a space for an empty cell.
    #[inline]
    pub fn ch(self) -> char {
        match self.codepoint() {
            0 => ' ',
            c => char::from_u32(c).unwrap_or(' '),
        }
    }

    /// Nothing would be drawn here if the row ended at this cell.
    #[inline]
    pub const fn is_blank(self) -> bool {
        self.0 & Self::NOT_BLANK == 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// The row's change counter (§9.1): changes whenever its content does,
    /// and only then. 0 for rows not on a screen.
    pub seq: u64,
    pub cells: Vec<Cell>,
    /// The line continues onto the next row (soft wrap).
    pub wrapped: bool,
}

impl Row {
    pub fn new(cols: usize, fill: Cell) -> Row {
        Row {
            seq: 0,
            cells: vec![fill; cols],
            wrapped: false,
        }
    }

    pub fn reset(&mut self, cols: usize, fill: Cell) {
        if self.cells.len() == cols && fill == Cell::BLANK {
            // The usual case: the all-zero blank, which fills as a memset.
            self.cells.fill(Cell::BLANK);
        } else {
            self.cells.clear();
            self.cells.resize(cols, fill);
        }
        self.wrapped = false;
    }

    pub fn get(&self, col: usize) -> Cell {
        self.cells.get(col).copied().unwrap_or(Cell::BLANK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_kinds_round_trip() {
        assert_eq!(Color::DEFAULT.kind(), ColorKind::Default);
        assert_eq!(Color::default(), Color::DEFAULT);
        for i in [0u8, 1, 7, 8, 15, 16, 231, 232, 255] {
            assert_eq!(Color::indexed(i).kind(), ColorKind::Indexed(i));
        }
        for (r, g, b) in [
            (0, 0, 0),
            (255, 255, 255),
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (1, 2, 3),
            (254, 128, 127),
        ] {
            assert_eq!(Color::rgb(r, g, b).kind(), ColorKind::Rgb(r, g, b));
        }
    }

    #[test]
    fn color_kinds_never_collide() {
        // Index 0 and black are not the default colour, nor each other.
        assert_ne!(Color::indexed(0), Color::DEFAULT);
        assert_ne!(Color::rgb(0, 0, 0), Color::DEFAULT);
        assert_ne!(Color::rgb(0, 0, 0), Color::indexed(0));
        assert_ne!(Color::rgb(0, 0, 255), Color::indexed(255));
    }

    #[test]
    fn unknown_color_tags_read_as_default() {
        assert_eq!(Color(0x0300_0000).kind(), ColorKind::Default);
        assert_eq!(Color(0xFF12_3456).kind(), ColorKind::Default);
    }

    #[test]
    fn bit_layout() {
        // The fields don't overlap and leave bits 27–31 spare.
        let fields = [
            Cell::CONTENT,
            Cell::KIND,
            Cell::WIDTH,
            Cell::PROTECTED,
            Cell::SHOWS_EMPTY,
            0xFFFF_FFFF << 32,
        ];
        let mut seen = 0u64;
        for f in fields {
            assert_eq!(seen & f, 0, "{f:#x} overlaps");
            seen |= f;
        }
        assert_eq!(!seen, 0x1F << 27);
        assert!(Cell::CONTENT >= char::MAX as u64);

        let t = Cell::template(0xABCD_1234, true);
        assert_eq!(t.bits(), 0xABCD_1234 << 32 | 1 << 26);
        assert_eq!((t.style_id(), t.shows_empty()), (0xABCD_1234, true));
        let c = t.with_char('\u{10FFFF}').with_width(Width::Wide);
        assert_eq!((c.ch(), c.codepoint(), c.width()), ('\u{10FFFF}', 0x10FFFF, Width::Wide));
        assert_eq!(c.style_part(), t);
        assert!(c.is_wide() && !c.is_spacer() && c.is_wide_part());
        for w in [Width::Narrow, Width::Wide, Width::Spacer, Width::EdgeSpacer] {
            let c = t.with_width(w);
            assert_eq!(c.width(), w);
            assert_eq!(c.is_spacer(), matches!(w, Width::Spacer | Width::EdgeSpacer));
            assert_eq!(c.is_wide(), w == Width::Wide);
            assert_eq!(c.is_wide_part(), w != Width::Narrow);
            assert_eq!(c.style_part(), t, "width leaves the style alone");
        }
        let other = Cell::template(7, false);
        assert_eq!(c.with_style(other).style_part(), other);
        assert_eq!(c.with_style(other).ch(), '\u{10FFFF}');
        assert_eq!(c.with_style(other).width(), Width::Wide);
    }

    #[test]
    fn blank_is_zero() {
        assert_eq!(Cell::BLANK.bits(), 0);
        assert_eq!(Cell::default(), Cell::BLANK);
        assert!(Cell::BLANK.is_blank());
        assert_eq!(Cell::BLANK.ch(), ' ');
        // A style id alone (a foreground colour, bold) doesn't show.
        assert!(Cell::template(5, false).is_blank());
        assert!(Cell::template(5, false).with_width(Width::Spacer).is_blank());
        // A character, even a space, or a style that shows when empty does.
        assert!(!Cell::BLANK.with_char(' ').is_blank());
        assert!(!Cell::BLANK.with_char('a').is_blank());
        assert!(!Cell::template(5, true).is_blank());
        for invalid in [0xD800u64, 0xDFFF, 0x11_0000, Cell::CONTENT] {
            assert_eq!(Cell::from_bits(invalid).ch(), ' ', "{invalid:#x}");
        }
    }

    #[test]
    fn row_helpers() {
        let fill = Cell::template(4, true);
        let mut row = Row::new(3, fill);
        assert_eq!(row.cells, vec![fill; 3]);
        assert!(!row.wrapped);
        assert_eq!(row.get(2), fill);
        assert_eq!(row.get(3), Cell::BLANK);
        row.wrapped = true;
        row.reset(5, Cell::BLANK);
        assert_eq!(row.cells, vec![Cell::BLANK; 5]);
        assert!(!row.wrapped);
        // Same length: the blank fill (a memset) and any other fill.
        row.cells[1] = Cell::template(9, true).with_char('x');
        row.wrapped = true;
        row.reset(5, Cell::BLANK);
        assert_eq!(row.cells, vec![Cell::BLANK; 5]);
        assert!(!row.wrapped);
        row.reset(5, fill);
        assert_eq!(row.cells, vec![fill; 5]);
        row.reset(2, Cell::BLANK);
        assert_eq!(row.cells, vec![Cell::BLANK; 2]);
    }
}
