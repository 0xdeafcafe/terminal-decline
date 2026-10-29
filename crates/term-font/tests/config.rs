//! Font settings (roadmap E.4): the fallback list, face overrides, icon
//! fitting, line height and cell width. Only fonts that ship with macOS:
//! Menlo, Monaco, Apple Symbols, STIX Two Math.

use term_font::{CellWidth, FontConfig, Fonts, GlyphKey, RasterGlyph, RunText, ShapedGlyph, Style};

const STYLES: [Style; 4] = [Style::Regular, Style::Bold, Style::Italic, Style::BoldItalic];

fn menlo(edit: impl FnOnce(&mut FontConfig)) -> Fonts {
    let mut c = FontConfig::new("Menlo", 13.0);
    edit(&mut c);
    Fonts::with_config(&c, 2.0)
}

fn fallback(names: &[&str]) -> Fonts {
    menlo(|c| c.fallback = names.iter().map(|s| s.to_string()).collect())
}

fn bitmap(g: &RasterGlyph) -> (u32, u32, i32, i32, &[u8]) {
    (g.width, g.height, g.left, g.top, &g.data)
}

#[test]
fn defaults_are_fonts_new() {
    for (family, size) in [("Menlo", 13.0), ("Monaco", 18.5), ("Menlo", 9.0)] {
        let a = Fonts::new(family, size * 2.0);
        let b = Fonts::with_config(&FontConfig::new(family, size), 2.0);
        assert_eq!(format!("{:?}", a.metrics), format!("{:?}", b.metrics), "{family} {size}");
        assert!(b.missing().is_empty());
        for style in STYLES {
            for ch in ['A', 'g', '_', '漢', '😀', '─'] {
                assert_eq!(a.glyph_key(ch, style), b.glyph_key(ch, style), "{ch:?}");
                let (ga, gb) = (a.rasterize(ch, style).unwrap(), b.rasterize(ch, style).unwrap());
                assert_eq!(bitmap(&ga), bitmap(&gb), "{family} {ch:?} {style:?}");
            }
        }
    }
}

#[test]
fn fallback_order_wins_over_the_cascade() {
    // ⨀ is in both Apple Symbols and STIX Two Math but not Menlo; Core
    // Text's own cascade picks Apple Symbols.
    let ch = '\u{2A00}';
    assert_eq!(
        Fonts::new("Menlo", 26.0).family_for(ch, Style::Regular).as_deref(),
        Some("Apple Symbols")
    );
    let stix_first = fallback(&["STIX Two Math", "Apple Symbols"]);
    let symbols_first = fallback(&["Apple Symbols", "STIX Two Math"]);
    for style in STYLES {
        assert_eq!(stix_first.family_for(ch, style).as_deref(), Some("STIX Two Math"), "{style:?}");
        assert_eq!(symbols_first.family_for(ch, style).as_deref(), Some("Apple Symbols"), "{style:?}");
        // ⏻ isn't in Apple Symbols: the second on the list draws it.
        assert_eq!(symbols_first.family_for('⏻', style).as_deref(), Some("STIX Two Math"));
        // The face's own glyphs still come first.
        assert_eq!(stix_first.family_for('A', style).as_deref(), Some("Menlo"));
    }

    // Shaping with CTLine follows the same order, on both paths.
    for ligatures in [false, true] {
        for (fonts, family) in [(&stix_first, "STIX Two Math"), (&symbols_first, "Apple Symbols")] {
            let mut run = RunText::new();
            run.push_char(ch, 1);
            run.push("\u{2A00}\u{0301}", 1);
            let mut out = Vec::new();
            fonts.shape(&run, Style::Regular, term_font::Features { ligatures }, &mut out);
            assert!(out.len() >= 2, "{out:?}");
            assert_eq!(
                fonts.family_of(out[0].key.font).as_deref(),
                Some(family),
                "single, ligatures {ligatures}"
            );
            let cluster: Vec<&ShapedGlyph> = out.iter().filter(|g| g.cell == 1).collect();
            assert_eq!(
                fonts.family_of(cluster[0].key.font).as_deref(),
                Some(family),
                "cluster, ligatures {ligatures}"
            );
        }
    }
}

#[test]
fn missing_fallbacks_are_skipped_and_reported() {
    let f = fallback(&["No Such Font 4321", "STIX Two Math"]);
    assert_eq!(f.missing(), ["No Such Font 4321"]);
    assert_eq!(f.family_for('\u{2A00}', Style::Regular).as_deref(), Some("STIX Two Math"));
}

#[test]
fn face_overrides_are_applied() {
    let f = menlo(|c| {
        c.bold = Some("Monaco".into()); // a family: its bold, or itself
        c.italic = Some("Menlo-Bold".into()); // a PostScript name
        c.bold_italic = Some("Menlo Regular".into()); // a full name
    });
    assert!(f.missing().is_empty(), "{:?}", f.missing());
    let ps = |s| f.postscript_name_of(Fonts::face_id(s)).unwrap();
    assert_eq!(ps(Style::Regular), "Menlo-Regular");
    assert_eq!(ps(Style::Bold), "Monaco");
    assert_eq!(ps(Style::Italic), "Menlo-Bold");
    assert_eq!(ps(Style::BoldItalic), "Menlo-Regular");
    assert_eq!(f.family_for('A', Style::Bold).as_deref(), Some("Monaco"));

    // Glyphs come from the override.
    let defaults = Fonts::new("Menlo", 26.0);
    let monaco = Fonts::new("Monaco", 26.0);
    let a = |f: &Fonts, s| f.rasterize('A', s).unwrap();
    assert_eq!(bitmap(&a(&f, Style::Bold)), bitmap(&a(&monaco, Style::Regular)));
    assert_eq!(bitmap(&a(&f, Style::Italic)), bitmap(&a(&defaults, Style::Bold)));
    assert_eq!(bitmap(&a(&f, Style::BoldItalic)), bitmap(&a(&defaults, Style::Regular)));
    // Metrics are the regular face's.
    assert_eq!(format!("{:?}", f.metrics), format!("{:?}", defaults.metrics));
}

#[test]
fn unknown_face_overrides_fall_back_to_the_family() {
    let f = menlo(|c| c.bold = Some("No Such Face Bold".into()));
    assert_eq!(f.missing(), ["No Such Face Bold"]);
    assert_eq!(f.postscript_name_of(Fonts::face_id(Style::Bold)).unwrap(), "Menlo-Bold");
}

/// The bitmap stays inside `cells` cells: x in 0..cells × width, y in
/// the cell's height.
fn assert_inside(f: &Fonts, g: &RasterGlyph, cells: u32, what: &str) {
    let m = f.metrics;
    let top = m.baseline as i32 - g.top; // from the cell's top
    assert!(
        g.left >= 0 && g.left + g.width as i32 <= (cells * m.cell_width) as i32,
        "{what}: x {g:?} in {m:?}",
        g = (g.left, g.width)
    );
    assert!(
        top >= 0 && top + g.height as i32 <= m.cell_height as i32,
        "{what}: y {top}+{} in {m:?}",
        g.height
    );
}

#[test]
fn private_use_icons_fit_their_cell() {
    // U+E000 is in STIX Two Math; U+F8FF (the Apple logo) comes from Core
    // Text's own cascade. A short, narrow cell makes them shrink.
    for (line_height, width) in [
        (1.0, CellWidth::default()),
        (0.7, CellWidth::Percent(-30.0)),
        (1.5, CellWidth::Points(4.0)),
    ] {
        let f = menlo(|c| {
            c.fallback = vec!["STIX Two Math".into()];
            c.line_height = line_height;
            c.cell_width = width;
        });
        assert_eq!(f.family_for('\u{E000}', Style::Regular).as_deref(), Some("STIX Two Math"));
        for ch in ['\u{E000}', '\u{F8FF}'] {
            for style in STYLES {
                let key = f.glyph_key(ch, style).unwrap();
                assert_eq!(key.variant & GlyphKey::FITTED, GlyphKey::FITTED, "{ch:?}");
                assert_eq!(key.cells, 1);
                let g = f.rasterize_key(key).unwrap();
                assert!(g.data.iter().any(|&b| b > 128), "{ch:?} has ink");
                assert_inside(&f, &g, 1, &format!("{ch:?} {style:?} lh {line_height} {width:?}"));
                // Centred: as much room left as right, to a pixel.
                let right = f.metrics.cell_width as i32 - g.left - g.width as i32;
                assert!((g.left - right).abs() <= 1, "{ch:?} centred: {} | {right}", g.left);
            }
        }
    }
}

#[test]
fn a_wide_icon_followed_by_a_blank_gets_two_cells() {
    // At 0.5 × Menlo's advance, the Apple logo is wider than a cell.
    let f = menlo(|c| c.cell_width = CellWidth::Percent(-50.0));
    let ch = '\u{F8FF}';
    let unfitted = {
        let mut k = f.glyph_key(ch, Style::Regular).unwrap();
        k.variant = 0;
        f.rasterize_key(k).unwrap()
    };
    assert!(
        unfitted.width > f.metrics.cell_width + 2,
        "the test needs an icon wider than a cell"
    );
    for ligatures in [false, true] {
        let mut run = RunText::new();
        run.push_char(ch, 1);
        run.push(" ", 1);
        run.push_char(ch, 1);
        run.push_char('x', 1);
        let mut out = Vec::new();
        f.shape(&run, Style::Regular, term_font::Features { ligatures }, &mut out);
        let icons: Vec<&ShapedGlyph> = out.iter().filter(|g| g.key.variant & GlyphKey::FITTED != 0).collect();
        assert_eq!(icons.len(), 2, "{out:?}");
        assert_eq!(
            (icons[0].cell, icons[0].cells, icons[0].key.cells),
            (0, 2, 2),
            "followed by a blank"
        );
        assert_eq!((icons[1].cell, icons[1].cells, icons[1].key.cells), (2, 1, 1), "followed by text");
        let two = f.rasterize_key(icons[0].key).unwrap();
        let one = f.rasterize_key(icons[1].key).unwrap();
        assert_inside(&f, &two, 2, "two cells");
        assert_inside(&f, &one, 1, "one cell");
        assert!(two.width > one.width);
    }
}

#[test]
fn line_height_keeps_the_baseline_centred() {
    let base = Fonts::new("Menlo", 26.0).metrics;
    for lh in [1.1, 1.25, 1.5, 2.0] {
        let m = menlo(|c| c.line_height = lh).metrics;
        assert_eq!(m.cell_height, (base.cell_height as f64 * lh).round() as u32, "{lh}");
        let extra = (m.cell_height - base.cell_height) as f64;
        let moved = m.baseline as f64 - base.baseline as f64;
        assert!((moved - extra / 2.0).abs() <= 1.0, "{lh}: baseline moved {moved} for {extra} extra");
        // Space above the text and below it stay equal, to a pixel.
        assert_eq!(m.cell_width, base.cell_width);
        assert_eq!(
            (m.underline_offset, m.underline_thickness, m.strike_offset),
            (base.underline_offset, base.underline_thickness, base.strike_offset),
            "underline and strike follow the baseline"
        );
    }
    let short = menlo(|c| c.line_height = 0.75).metrics;
    assert!(short.cell_height < base.cell_height);
    assert!(short.baseline < short.cell_height);
    assert!(
        short.baseline + short.underline_offset + short.underline_thickness <= short.cell_height,
        "{short:?}"
    );
}

#[test]
fn cell_width_adjusts_and_glyphs_stay_centred() {
    let base = Fonts::new("Menlo", 26.0);
    let a = base.rasterize('A', Style::Regular).unwrap();
    for (adjust, extra) in [
        (CellWidth::Points(2.0), 4),
        (CellWidth::Points(-1.0), -2),
        (CellWidth::Percent(25.0), 4),
    ] {
        let f = menlo(|c| c.cell_width = adjust);
        assert_eq!(f.metrics.cell_width as i32, base.metrics.cell_width as i32 + extra, "{adjust:?}");
        assert_eq!(f.metrics.cell_height, base.metrics.cell_height);
        let g = f.rasterize('A', Style::Regular).unwrap();
        assert_eq!(g.data, a.data);
        assert_eq!(g.left, a.left + extra.div_euclid(2), "{adjust:?}");
    }
}
