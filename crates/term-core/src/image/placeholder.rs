//! kitty Unicode placeholders (cells §7): U+10EEEE cells naming a piece
//! of an image. The image id is in the foreground colour (and the style's
//! `image_id_high`), the placement id in the underline colour, and up to
//! three diacritics give the row, the column and bits 24–31 of the id.
//! Missing ones are inferred from the cell to the left, step by step as
//! they arrive, with kitty's rules.

/// The placeholder character.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// The largest column a placeholder cell stores (its content has 9 bits
/// for it); a guess past it stores a blank instead.
pub const MAX_COL: u16 = 511;

/// The number of row and column values (kitty's table).
pub const VALUES: u16 = 297;

/// kitty's `rowcolumn-diacritics.txt`: the diacritic for value `n` is
/// `DIACRITICS[n]`, in code point order.
pub static DIACRITICS: [u32; VALUES as usize] = [
    0x00305, 0x0030D, 0x0030E, 0x00310, 0x00312, 0x0033D, 0x0033E, 0x0033F, 0x00346, 0x0034A, 0x0034B, 0x0034C, 0x00350, 0x00351, 0x00352,
    0x00357, 0x0035B, 0x00363, 0x00364, 0x00365, 0x00366, 0x00367, 0x00368, 0x00369, 0x0036A, 0x0036B, 0x0036C, 0x0036D, 0x0036E, 0x0036F,
    0x00483, 0x00484, 0x00485, 0x00486, 0x00487, 0x00592, 0x00593, 0x00594, 0x00595, 0x00597, 0x00598, 0x00599, 0x0059C, 0x0059D, 0x0059E,
    0x0059F, 0x005A0, 0x005A1, 0x005A8, 0x005A9, 0x005AB, 0x005AC, 0x005AF, 0x005C4, 0x00610, 0x00611, 0x00612, 0x00613, 0x00614, 0x00615,
    0x00616, 0x00617, 0x00657, 0x00658, 0x00659, 0x0065A, 0x0065B, 0x0065D, 0x0065E, 0x006D6, 0x006D7, 0x006D8, 0x006D9, 0x006DA, 0x006DB,
    0x006DC, 0x006DF, 0x006E0, 0x006E1, 0x006E2, 0x006E4, 0x006E7, 0x006E8, 0x006EB, 0x006EC, 0x00730, 0x00732, 0x00733, 0x00735, 0x00736,
    0x0073A, 0x0073D, 0x0073F, 0x00740, 0x00741, 0x00743, 0x00745, 0x00747, 0x00749, 0x0074A, 0x007EB, 0x007EC, 0x007ED, 0x007EE, 0x007EF,
    0x007F0, 0x007F1, 0x007F3, 0x00816, 0x00817, 0x00818, 0x00819, 0x0081B, 0x0081C, 0x0081D, 0x0081E, 0x0081F, 0x00820, 0x00821, 0x00822,
    0x00823, 0x00825, 0x00826, 0x00827, 0x00829, 0x0082A, 0x0082B, 0x0082C, 0x0082D, 0x00951, 0x00953, 0x00954, 0x00F82, 0x00F83, 0x00F86,
    0x00F87, 0x0135D, 0x0135E, 0x0135F, 0x017DD, 0x0193A, 0x01A17, 0x01A75, 0x01A76, 0x01A77, 0x01A78, 0x01A79, 0x01A7A, 0x01A7B, 0x01A7C,
    0x01B6B, 0x01B6D, 0x01B6E, 0x01B6F, 0x01B70, 0x01B71, 0x01B72, 0x01B73, 0x01CD0, 0x01CD1, 0x01CD2, 0x01CDA, 0x01CDB, 0x01CE0, 0x01DC0,
    0x01DC1, 0x01DC3, 0x01DC4, 0x01DC5, 0x01DC6, 0x01DC7, 0x01DC8, 0x01DC9, 0x01DCB, 0x01DCC, 0x01DD1, 0x01DD2, 0x01DD3, 0x01DD4, 0x01DD5,
    0x01DD6, 0x01DD7, 0x01DD8, 0x01DD9, 0x01DDA, 0x01DDB, 0x01DDC, 0x01DDD, 0x01DDE, 0x01DDF, 0x01DE0, 0x01DE1, 0x01DE2, 0x01DE3, 0x01DE4,
    0x01DE5, 0x01DE6, 0x01DFE, 0x020D0, 0x020D1, 0x020D4, 0x020D5, 0x020D6, 0x020D7, 0x020DB, 0x020DC, 0x020E1, 0x020E7, 0x020E9, 0x020F0,
    0x02CEF, 0x02CF0, 0x02CF1, 0x02DE0, 0x02DE1, 0x02DE2, 0x02DE3, 0x02DE4, 0x02DE5, 0x02DE6, 0x02DE7, 0x02DE8, 0x02DE9, 0x02DEA, 0x02DEB,
    0x02DEC, 0x02DED, 0x02DEE, 0x02DEF, 0x02DF0, 0x02DF1, 0x02DF2, 0x02DF3, 0x02DF4, 0x02DF5, 0x02DF6, 0x02DF7, 0x02DF8, 0x02DF9, 0x02DFA,
    0x02DFB, 0x02DFC, 0x02DFD, 0x02DFE, 0x02DFF, 0x0A66F, 0x0A67C, 0x0A67D, 0x0A6F0, 0x0A6F1, 0x0A8E0, 0x0A8E1, 0x0A8E2, 0x0A8E3, 0x0A8E4,
    0x0A8E5, 0x0A8E6, 0x0A8E7, 0x0A8E8, 0x0A8E9, 0x0A8EA, 0x0A8EB, 0x0A8EC, 0x0A8ED, 0x0A8EE, 0x0A8EF, 0x0A8F0, 0x0A8F1, 0x0AAB0, 0x0AAB2,
    0x0AAB3, 0x0AAB7, 0x0AAB8, 0x0AABE, 0x0AABF, 0x0AAC1, 0x0FE20, 0x0FE21, 0x0FE22, 0x0FE23, 0x0FE24, 0x0FE25, 0x0FE26, 0x10A0F, 0x10A38,
    0x1D185, 0x1D186, 0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];

/// The value of diacritic `c`, if it is one of kitty's.
#[inline]
pub fn diacritic_value(c: char) -> Option<u16> {
    let cp = c as u32;
    if !(DIACRITICS[0]..=DIACRITICS[VALUES as usize - 1]).contains(&cp) {
        return None;
    }
    DIACRITICS.binary_search(&cp).ok().map(|i| i as u16)
}

/// The diacritic for value `n`.
pub fn diacritic(n: u16) -> Option<char> {
    DIACRITICS.get(n as usize).and_then(|&cp| char::from_u32(cp))
}

/// Which piece of an image a placeholder cell shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Piece {
    pub row: u16,
    pub col: u16,
    /// Bits 24–31 of the image id.
    pub high: u8,
}

/// The piece of a placeholder with no diacritics yet: the one after the
/// cell to the left (`left`: a placeholder with the same foreground and
/// underline colours), or the first.
pub fn on_base(left: Option<Piece>) -> Piece {
    match left {
        Some(l) => Piece {
            row: l.row,
            col: l.col + 1,
            high: l.high,
        },
        None => Piece::default(),
    }
}

/// The piece after diacritic number `step` (0 the row, 1 the column, 2
/// the high byte) with value `v`.
pub fn on_diacritic(step: u8, v: u16, cur: Piece, left: Option<Piece>) -> Piece {
    match step {
        0 => {
            let same = left.filter(|l| l.row == v);
            Piece {
                row: v,
                col: same.map_or(0, |l| l.col + 1),
                high: same.map_or(0, |l| l.high),
            }
        }
        1 => {
            let same = left.filter(|l| l.row == cur.row && l.col + 1 == v);
            Piece {
                col: v,
                high: same.map_or(0, |l| l.high),
                ..cur
            }
        }
        2 if v <= 255 => Piece { high: v as u8, ..cur },
        _ => cur,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_kittys() {
        assert_eq!(DIACRITICS.len(), 297);
        assert!(DIACRITICS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(diacritic(0), Some('\u{305}'));
        assert_eq!(diacritic(1), Some('\u{30D}'));
        assert_eq!(diacritic(296), Some('\u{1D244}'));
        assert_eq!(diacritic(297), None);
        for n in 0..VALUES {
            assert_eq!(diacritic_value(diacritic(n).unwrap()), Some(n));
            assert_eq!(crate::unicode::props(diacritic(n).unwrap()).width_with(1), 0, "{n} is combining");
        }
        assert_eq!(diacritic_value('\u{300}'), None);
        assert_eq!(diacritic_value('a'), None);
    }

    #[test]
    fn inference_follows_kittys_rules() {
        let left = Some(Piece { row: 3, col: 7, high: 9 });
        // Base only: right after the left cell.
        assert_eq!(on_base(left), Piece { row: 3, col: 8, high: 9 });
        assert_eq!(on_base(None), Piece::default());
        // A row diacritic: the column carries on only on the same row.
        let p = on_diacritic(0, 3, on_base(left), left);
        assert_eq!(p, Piece { row: 3, col: 8, high: 9 });
        let p = on_diacritic(0, 4, on_base(left), left);
        assert_eq!(p, Piece { row: 4, col: 0, high: 0 });
        // A column diacritic: the high byte carries on only right after.
        let p = on_diacritic(1, 8, Piece { row: 3, col: 8, high: 9 }, left);
        assert_eq!(p.high, 9);
        let p = on_diacritic(1, 2, Piece { row: 3, col: 8, high: 9 }, left);
        assert_eq!(p, Piece { row: 3, col: 2, high: 0 });
        // The third sets the high byte.
        assert_eq!(on_diacritic(2, 200, p, left).high, 200);
        assert_eq!(on_diacritic(2, 296, p, left), p);
        assert_eq!(on_diacritic(3, 5, p, left), p);
    }
}
