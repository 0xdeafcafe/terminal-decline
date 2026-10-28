//! Core Text fonts (macOS): cell metrics, monospacing, styles, fallback
//! fonts, emoji sizing and bitmap borders.
//!
//! Not tested on purpose: that a missing glyph gives `None` (Core Text
//! draws a Last Resort glyph instead), and "SF Mono" by name (it silently
//! becomes Helvetica unless installed; see `Fonts::new`).

use term_font::{Fonts, RasterGlyph, Style};

#[test]
fn metrics_and_glyphs() {
    let f = Fonts::new("Menlo", 26.0);
    let m = f.metrics;
    assert!(m.cell_width > 10 && m.cell_width < 25, "{m:?}");
    assert!(m.cell_height > m.cell_width);
    assert!(m.baseline > 0 && m.baseline < m.cell_height, "baseline inside the cell: {m:?}");
    let a = f.rasterize('A', Style::Regular).unwrap();
    assert!(!a.color && a.data.iter().any(|&b| b > 200));
    assert!(f.rasterize(' ', Style::Regular).is_none());
    let e = f.rasterize('😀', Style::Regular).unwrap();
    assert!(e.color, "emoji should come from a colour font");
    assert!(f.rasterize('漢', Style::Bold).is_some(), "CJK via fallback");
}

#[test]
fn menlo_and_monaco_are_monospaced() {
    for family in ["Menlo", "Monaco"] {
        for size in [13.0, 26.0, 37.0] {
            let f = Fonts::new(family, size);
            assert_eq!(f.family(), family, "{family} should be installed");
            let i = f.advance('i', Style::Regular).unwrap();
            let w = f.advance('M', Style::Regular).unwrap();
            assert_eq!(i, w, "{family} {size}: 'i' and 'M' advances");
            assert_eq!(f.metrics.cell_width, w.round() as u32, "{family} {size}: cell width is the advance");
        }
    }
}

#[test]
fn unknown_family_does_not_panic() {
    let f = Fonts::new("No Such Font Family 1234", 26.0);
    assert_ne!(f.family(), "No Such Font Family 1234");
    let m = f.metrics;
    assert!(m.cell_width > 0 && m.cell_height > 0, "{m:?}");
    assert!(f.rasterize('A', Style::Regular).is_some());
    assert!(f.rasterize('A', Style::BoldItalic).is_some());
}

#[test]
fn styles_differ() {
    let f = Fonts::new("Menlo", 26.0);
    let get = |s| {
        let g = f.rasterize('A', s).unwrap();
        (g.width, g.height, g.left, g.data)
    };
    let (regular, bold, italic) = (get(Style::Regular), get(Style::Bold), get(Style::Italic));
    assert_ne!(regular, bold, "bold");
    assert_ne!(regular, italic, "italic");
    assert_ne!(bold, italic, "bold vs italic");
}

/// True if every pixel on the bitmap's outer 1-px border is transparent, so
/// rasterising didn't clip the glyph.
fn border_is_empty(g: &RasterGlyph) -> bool {
    let bpp = if g.color { 4 } else { 1 };
    let (w, h) = (g.width as usize, g.height as usize);
    let blank = |x: usize, y: usize| g.data[(y * w + x) * bpp..][..bpp].iter().all(|&b| b == 0);
    (0..w).all(|x| blank(x, 0) && blank(x, h - 1)) && (0..h).all(|y| blank(0, y) && blank(w - 1, y))
}

#[test]
fn bitmaps_have_an_empty_border() {
    let f = Fonts::new("Menlo", 26.0);
    for style in [Style::Regular, Style::Bold, Style::Italic, Style::BoldItalic] {
        for ch in ['A', 'g', 'W', 'j', 'f', '_', '|', '@', '█', '漢', '→', '😀'] {
            let g = f.rasterize(ch, style).unwrap_or_else(|| panic!("{ch:?} {style:?}"));
            assert!(g.width >= 3 && g.height >= 3);
            assert_eq!(g.data.len(), (g.width * g.height) as usize * if g.color { 4 } else { 1 });
            assert!(border_is_empty(&g), "{ch:?} {style:?} is clipped");
            // And there is ink inside the border.
            assert!(g.data.iter().any(|&b| b != 0), "{ch:?} {style:?} is blank");
        }
    }
}

#[test]
fn emoji_fits_two_cells() {
    for size in [13.0, 26.0, 52.0] {
        let f = Fonts::new("Menlo", size);
        let m = f.metrics;
        for ch in ['😀', '🎉', '🦀', '👍'] {
            let g = f.rasterize(ch, Style::Regular).unwrap();
            assert!(g.color, "{ch:?}");
            // The bitmap is the ink plus a 1-px border and up to 1 px of
            // rounding on each side.
            let (ink_w, ink_h) = (g.width - 2, g.height - 2);
            assert!(ink_w <= 2 * m.cell_width + 2, "{ch:?} at {size}: {ink_w} px wide, cell {m:?}");
            assert!(ink_h <= m.cell_height + 2, "{ch:?} at {size}: {ink_h} px tall, cell {m:?}");
        }
    }
}

#[test]
fn cjk_comes_from_a_fallback_font() {
    let f = Fonts::new("Menlo", 26.0);
    assert_eq!(f.family_for('A', Style::Regular).as_deref(), Some("Menlo"));
    for ch in ['漢', 'あ', '한'] {
        let family = f.family_for(ch, Style::Regular).unwrap();
        assert_ne!(family, "Menlo", "{ch:?}");
        let g = f.rasterize(ch, Style::Regular).unwrap();
        assert!(!g.color && g.data.iter().any(|&b| b > 200), "{ch:?} from {family}");
    }
    assert_ne!(f.family_for('😀', Style::Regular).as_deref(), Some("Menlo"));
}
