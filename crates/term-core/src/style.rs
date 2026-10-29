//! How a cell looks, apart from its text: colours, attributes, underline
//! and link. Readers get a [`Style`] by reference from a
//! [`CellRef`](crate::CellRef); how cells store it is term-core's business.

use crate::cell::Color;

/// An OSC 8 link in its screen's link table; 0 = none.
pub type LinkId = u32;

/// Text attributes as a set of bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
#[repr(transparent)]
pub struct Attrs(u16);

impl Attrs {
    pub const BOLD: Attrs = Attrs(1 << 0);
    pub const DIM: Attrs = Attrs(1 << 1);
    pub const ITALIC: Attrs = Attrs(1 << 2);
    pub const INVERSE: Attrs = Attrs(1 << 3);
    pub const HIDDEN: Attrs = Attrs(1 << 4);
    pub const STRIKE: Attrs = Attrs(1 << 5);
    pub const BLINK: Attrs = Attrs(1 << 6);
    pub const OVERLINE: Attrs = Attrs(1 << 7);
    /// Placeholder cells of decline's own images (Sixel, iTerm2).
    pub const OWN_IMAGE: Attrs = Attrs(1 << 8);

    pub const fn empty() -> Attrs {
        Attrs(0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn from_bits(bits: u16) -> Attrs {
        Attrs(bits)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Every attribute in `other` is set.
    #[inline]
    pub const fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0
    }

    /// Any attribute in `other` is set.
    #[inline]
    pub const fn intersects(self, other: Attrs) -> bool {
        self.0 & other.0 != 0
    }

    #[inline]
    pub fn insert(&mut self, other: Attrs) {
        self.0 |= other.0;
    }

    #[inline]
    pub fn remove(&mut self, other: Attrs) {
        self.0 &= !other.0;
    }

    #[inline]
    pub fn set(&mut self, other: Attrs, on: bool) {
        if on { self.insert(other) } else { self.remove(other) }
    }
}

impl std::ops::BitOr for Attrs {
    type Output = Attrs;
    #[inline]
    fn bitor(self, rhs: Attrs) -> Attrs {
        Attrs(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Attrs {
    #[inline]
    fn bitor_assign(&mut self, rhs: Attrs) {
        self.0 |= rhs.0;
    }
}

/// The underline style (SGR 4, 4:0–4:5, 21, 24).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
#[repr(u8)]
pub enum Underline {
    #[default]
    None = 0,
    Single = 1,
    Double = 2,
    Curly = 3,
    Dotted = 4,
    Dashed = 5,
}

/// Everything about how a cell looks except its text. 20 bytes, no padding.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    /// SGR 58/59. Default = draw underlines in the foreground colour.
    pub underline_color: Color,
    /// OSC 8 link in this screen's link table; 0 = none.
    pub link: LinkId,
    pub attrs: Attrs,
    pub underline: Underline,
    /// Placeholder cells only: bits 24–31 of the image id (kitty's third
    /// diacritic). 0 everywhere else.
    pub image_id_high: u8,
}

const _: () = assert!(std::mem::size_of::<Style>() == 20);

impl Style {
    pub const DEFAULT: Style = Style {
        fg: Color::DEFAULT,
        bg: Color::DEFAULT,
        underline_color: Color::DEFAULT,
        link: 0,
        attrs: Attrs::empty(),
        underline: Underline::None,
        image_id_high: 0,
    };

    /// An empty cell in this style still draws something.
    pub fn shows_empty(&self) -> bool {
        self.bg != Color::DEFAULT
            || self.attrs.intersects(Attrs::INVERSE | Attrs::STRIKE | Attrs::OVERLINE)
            || self.underline != Underline::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Attrs; 9] = [
        Attrs::BOLD,
        Attrs::DIM,
        Attrs::ITALIC,
        Attrs::INVERSE,
        Attrs::HIDDEN,
        Attrs::STRIKE,
        Attrs::BLINK,
        Attrs::OVERLINE,
        Attrs::OWN_IMAGE,
    ];

    #[test]
    fn attrs_are_distinct_bits() {
        let mut seen = Attrs::empty();
        for a in ALL {
            assert_eq!(a.bits().count_ones(), 1);
            assert!(!seen.intersects(a));
            seen |= a;
        }
        assert!(seen.contains(Attrs::BOLD | Attrs::OWN_IMAGE));
        let mut s = seen;
        s.remove(Attrs::BOLD);
        assert!(!s.contains(Attrs::BOLD) && s.contains(Attrs::DIM));
        s.set(Attrs::BOLD, true);
        assert_eq!(s, seen);
    }

    #[test]
    fn default_style_is_all_zero_and_blank() {
        assert_eq!(Style::default(), Style::DEFAULT);
        assert!(!Style::DEFAULT.shows_empty());
        let fg_only = Style {
            fg: Color::indexed(1),
            attrs: Attrs::BOLD | Attrs::ITALIC | Attrs::DIM | Attrs::BLINK | Attrs::HIDDEN,
            ..Style::DEFAULT
        };
        assert!(!fg_only.shows_empty());
        for s in [
            Style {
                bg: Color::indexed(0),
                ..Style::DEFAULT
            },
            Style {
                attrs: Attrs::INVERSE,
                ..Style::DEFAULT
            },
            Style {
                attrs: Attrs::OVERLINE,
                ..Style::DEFAULT
            },
            Style {
                underline: Underline::Curly,
                ..Style::DEFAULT
            },
        ] {
            assert!(s.shows_empty(), "{s:?}");
        }
    }
}
