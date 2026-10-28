//! Packed cell storage. One cell is 12 bytes: the character and its style
//! flags share one `u32`, and each colour is one `u32`.

/// A colour as stored in a cell. The top byte says what kind it is:
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

/// Style flags, stored in the 11 bits above the 21-bit Unicode scalar.
pub mod flags {
    pub const BOLD: u32 = 1 << 21;
    pub const DIM: u32 = 1 << 22;
    pub const ITALIC: u32 = 1 << 23;
    pub const UNDERLINE: u32 = 1 << 24;
    pub const STRIKE: u32 = 1 << 25;
    pub const INVERSE: u32 = 1 << 26;
    pub const HIDDEN: u32 = 1 << 27;
    /// First half of a double-width character.
    pub const WIDE: u32 = 1 << 28;
    /// Second half of a double-width character; draws nothing itself.
    pub const WIDE_SPACER: u32 = 1 << 29;
    pub const BLINK: u32 = 1 << 30;
    pub const DOUBLE_UNDERLINE: u32 = 1 << 31;

    pub const STYLE_MASK: u32 = BOLD | DIM | ITALIC | UNDERLINE | STRIKE | INVERSE | HIDDEN | BLINK | DOUBLE_UNDERLINE;
    pub const CHAR_MASK: u32 = (1 << 21) - 1;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(C)]
pub struct Cell {
    /// Low 21 bits: Unicode scalar (0 = empty). High 11 bits: [`flags`].
    pub packed: u32,
    pub fg: Color,
    pub bg: Color,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 12);

impl Cell {
    pub const BLANK: Cell = Cell {
        packed: 0,
        fg: Color::DEFAULT,
        bg: Color::DEFAULT,
    };

    #[inline]
    pub fn ch(self) -> char {
        match self.packed & flags::CHAR_MASK {
            0 => ' ',
            c => char::from_u32(c).unwrap_or(' '),
        }
    }

    #[inline]
    pub fn codepoint(self) -> u32 {
        self.packed & flags::CHAR_MASK
    }

    #[inline]
    pub fn flags(self) -> u32 {
        self.packed & !flags::CHAR_MASK
    }

    #[inline]
    pub fn has(self, flag: u32) -> bool {
        self.packed & flag != 0
    }

    #[inline]
    pub fn is_blank(self) -> bool {
        self.codepoint() == 0 && self.bg == Color::DEFAULT && self.flags() & (flags::INVERSE | flags::UNDERLINE | flags::STRIKE) == 0
    }

    /// An empty cell that keeps the given background (used when erasing).
    #[inline]
    pub fn erased(bg: Color) -> Cell {
        Cell {
            packed: 0,
            fg: Color::DEFAULT,
            bg,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The line continues onto the next row (soft wrap).
    pub wrapped: bool,
}

impl Row {
    pub fn new(cols: usize, fill: Cell) -> Row {
        Row {
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

    const ALL_FLAGS: [u32; 11] = [
        flags::BOLD,
        flags::DIM,
        flags::ITALIC,
        flags::UNDERLINE,
        flags::STRIKE,
        flags::INVERSE,
        flags::HIDDEN,
        flags::WIDE,
        flags::WIDE_SPACER,
        flags::BLINK,
        flags::DOUBLE_UNDERLINE,
    ];

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
    fn flag_bits_are_distinct_and_above_the_char_bits() {
        let mut seen = 0u32;
        for f in ALL_FLAGS {
            assert_eq!(f.count_ones(), 1, "{f:#x} is one bit");
            assert_eq!(f & flags::CHAR_MASK, 0, "{f:#x} overlaps the character");
            assert_eq!(seen & f, 0, "{f:#x} used twice");
            seen |= f;
        }
        // Together with the 21 character bits they fill the u32 exactly.
        assert_eq!(seen | flags::CHAR_MASK, u32::MAX);
        assert!(flags::CHAR_MASK >= char::MAX as u32);
        // The style mask is every flag except the two wide-character markers.
        assert_eq!(flags::STYLE_MASK, seen & !(flags::WIDE | flags::WIDE_SPACER));
    }

    #[test]
    fn flags_do_not_change_the_character() {
        let all = ALL_FLAGS.iter().fold(0, |a, f| a | f);
        for c in ['a', '漢', '\u{10FFFF}', '😀'] {
            let cell = Cell {
                packed: c as u32 | all,
                ..Cell::BLANK
            };
            assert_eq!(cell.ch(), c);
            assert_eq!(cell.codepoint(), c as u32);
            assert_eq!(cell.flags(), all);
            for f in ALL_FLAGS {
                assert!(cell.has(f));
            }
        }
    }

    #[test]
    fn ch_of_empty_and_invalid_scalars_is_space() {
        assert_eq!(Cell::BLANK.ch(), ' ');
        assert_eq!(Cell::BLANK.codepoint(), 0);
        for invalid in [0xD800u32, 0xDBFF, 0xDC00, 0xDFFF, 0x11_0000, flags::CHAR_MASK] {
            let cell = Cell {
                packed: invalid | flags::BOLD,
                ..Cell::BLANK
            };
            assert_eq!(cell.ch(), ' ', "{invalid:#x}");
        }
    }

    #[test]
    fn is_blank() {
        assert!(Cell::BLANK.is_blank());
        // Foreground colour and most styles don't show on an empty cell.
        let plain = |packed, fg, bg| Cell { packed, fg, bg };
        assert!(plain(0, Color::indexed(1), Color::DEFAULT).is_blank());
        assert!(
            plain(
                flags::BOLD | flags::ITALIC | flags::DIM | flags::BLINK,
                Color::DEFAULT,
                Color::DEFAULT
            )
            .is_blank()
        );
        // A character, a background, or a style that draws on empty cells does.
        assert!(!plain('a' as u32, Color::DEFAULT, Color::DEFAULT).is_blank());
        assert!(!plain(' ' as u32, Color::DEFAULT, Color::DEFAULT).is_blank());
        assert!(!plain(0, Color::DEFAULT, Color::indexed(0)).is_blank());
        assert!(!Cell::erased(Color::rgb(0, 0, 0)).is_blank());
        assert!(Cell::erased(Color::DEFAULT).is_blank());
        for f in [flags::INVERSE, flags::UNDERLINE, flags::STRIKE] {
            assert!(!plain(f, Color::DEFAULT, Color::DEFAULT).is_blank(), "{f:#x}");
        }
    }

    #[test]
    fn row_helpers() {
        let fill = Cell::erased(Color::indexed(4));
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
        row.cells[1] = Cell {
            packed: 'x' as u32 | flags::BOLD,
            fg: Color::indexed(1),
            bg: Color::rgb(1, 2, 3),
        };
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
