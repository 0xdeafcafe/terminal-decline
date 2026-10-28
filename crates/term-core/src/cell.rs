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
        self.cells.clear();
        self.cells.resize(cols, fill);
        self.wrapped = false;
    }

    pub fn get(&self, col: usize) -> Cell {
        self.cells.get(col).copied().unwrap_or(Cell::BLANK)
    }
}
