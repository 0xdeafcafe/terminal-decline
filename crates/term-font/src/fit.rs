//! Fitting icons to their cells (roadmap E.4).
//!
//! Nerd Font icons live in the Private Use Area and are drawn at whatever
//! size their font was made at, often wider than a cell and taller than
//! the line. The rule, after Ghostty's Nerd Font constraints and WezTerm's
//! `allow_square_glyphs_to_overflow_width = "WhenFollowedBySpace"`:
//!
//! - An icon is a character in the Private Use Area (U+E000–U+F8FF and
//!   planes 15 and 16), from any font, or a symbol (arrows, technical,
//!   dingbats, geometric shapes, pictographs) drawn from a font on the
//!   user's fallback list. Colour glyphs keep their own rule (fitted to 2
//!   cells).
//! - It gets one cell, or two when its ink is wider than one cell and the
//!   next cell in the run is blank (Nerd Font's convention: wide icons are
//!   followed by a space). [`Fonts::rasterize`](crate::Fonts::rasterize)
//!   alone, with no neighbours to look at, always gives one cell.
//! - It is scaled down, keeping its proportions, until its ink fits that
//!   box (the cells' width by the cell height), never scaled up, and
//!   centred in the box both ways.
//! - It is drawn into a bitmap no bigger than the box, so whatever
//!   antialiasing spills past the edge is cut off rather than drawn into a
//!   neighbour.

/// How to draw an ink rectangle (x, y, width, height, in pixels at the
/// font's size, y up from the baseline) into a box `w` × `h` pixels:
/// `(scale, x, y)`, the scale and where the glyph origin goes, in pixels
/// from the box's bottom-left corner.
pub(crate) fn fit(ink: (f64, f64, f64, f64), w: f64, h: f64) -> (f64, f64, f64) {
    let (ix, iy, iw, ih) = ink;
    let scale = (w / iw).min(h / ih).min(1.0);
    let x = w / 2.0 - scale * (ix + iw / 2.0);
    let y = h / 2.0 - scale * (iy + ih / 2.0);
    (scale, x, y)
}

/// A Private Use Area character: where Nerd Fonts and other icon fonts put
/// their icons.
pub fn is_private_use(ch: char) -> bool {
    matches!(ch as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Symbols an icon font on the fallback list may draw for a character it
/// shares with Unicode: arrows through miscellaneous symbols and arrows,
/// and the pictographs. Fitted only when they come from a fallback font.
pub(crate) fn is_symbol(ch: char) -> bool {
    matches!(ch as u32, 0x2190..=0x2BFF | 0x1F300..=0x1FAFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the ink lands: (left, bottom, right, top) in the box.
    fn placed(ink: (f64, f64, f64, f64), w: f64, h: f64) -> (f64, f64, f64, f64) {
        let (s, x, y) = fit(ink, w, h);
        let (ix, iy, iw, ih) = ink;
        (x + s * ix, y + s * iy, x + s * (ix + iw), y + s * (iy + ih))
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_wide_icon_is_shrunk_into_one_cell_and_centred() {
        // A 30 × 26 px icon sitting 4 px below the baseline, into a
        // 16 × 32 cell.
        let ink = (1.0, -4.0, 30.0, 26.0);
        let (s, _, _) = fit(ink, 16.0, 32.0);
        assert!(close(s, 16.0 / 30.0));
        let (l, b, r, t) = placed(ink, 16.0, 32.0);
        assert!(close(l, 0.0) && close(r, 16.0), "fills the width: {l} {r}");
        assert!(b >= 0.0 && t <= 32.0 && close((b + t) / 2.0, 16.0), "centred: {b} {t}");
    }

    #[test]
    fn two_cells_leave_it_at_its_size() {
        let ink = (1.0, -4.0, 30.0, 26.0);
        let (s, _, _) = fit(ink, 32.0, 32.0);
        assert!(close(s, 1.0), "never scaled up");
        let (l, b, r, t) = placed(ink, 32.0, 32.0);
        assert!(close(l, 1.0) && close(r, 31.0), "centred across both cells: {l} {r}");
        assert!(close(b, 3.0) && close(t, 29.0));
    }

    #[test]
    fn a_tall_icon_is_shrunk_to_the_cell_height() {
        let ink = (0.0, -10.0, 10.0, 50.0);
        let (s, _, _) = fit(ink, 16.0, 32.0);
        assert!(close(s, 32.0 / 50.0));
        let (l, b, r, t) = placed(ink, 16.0, 32.0);
        assert!(close(b, 0.0) && close(t, 32.0));
        assert!(close((l + r) / 2.0, 8.0) && l >= 0.0 && r <= 16.0);
    }

    #[test]
    fn private_use_and_symbols() {
        for ch in ['\u{E000}', '\u{E62B}', '\u{F8FF}', '\u{F0001}', '\u{10017A}'] {
            assert!(is_private_use(ch), "{ch:?}");
        }
        for ch in ['A', '漢', '\u{F900}', '→'] {
            assert!(!is_private_use(ch), "{ch:?}");
        }
        assert!(is_symbol('→') && is_symbol('⏻') && is_symbol('★') && !is_symbol('A'));
    }
}
