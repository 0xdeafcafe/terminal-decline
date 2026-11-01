//! The text settings: thickening, synthetic bold and italic, variable
//! font axes, per-face features, metric adjustments, the symbol map and
//! the emoji font. Only fonts that ship with macOS: Menlo, Monaco, Andale
//! Mono (no bold or italic face), Skia (a variable font) and Apple Color
//! Emoji.

use term_font::{Adjust, FontConfig, Fonts, GlyphKey, RasterGlyph, Style, Tag};

fn fonts(family: &str, edit: impl FnOnce(&mut FontConfig)) -> Fonts {
    let mut c = FontConfig::new(family, 13.0);
    edit(&mut c);
    Fonts::with_config(&c, 2.0)
}

fn ink(g: &RasterGlyph) -> u64 {
    g.data.iter().map(|&b| b as u64).sum()
}

#[test]
fn thickening_adds_ink_to_text_but_not_to_emoji() {
    let plain = fonts("Menlo", |_| {});
    let thick = fonts("Menlo", |c| c.thicken = 0.5);
    for ch in ['l', 'M', 'e'] {
        let (a, b) = (
            plain.rasterize(ch, Style::Regular).unwrap(),
            thick.rasterize(ch, Style::Regular).unwrap(),
        );
        assert!(ink(&b) > ink(&a) * 11 / 10, "{ch:?}: {} vs {}", ink(&b), ink(&a));
    }
    let (a, b) = (
        plain.rasterize('😀', Style::Regular).unwrap(),
        thick.rasterize('😀', Style::Regular).unwrap(),
    );
    assert_eq!(
        (a.width, a.height, &a.data),
        (b.width, b.height, &b.data),
        "colour glyphs aren't thickened"
    );
    // Metrics don't change.
    assert_eq!(format!("{:?}", plain.metrics), format!("{:?}", thick.metrics));
}

#[test]
fn families_without_bold_or_italic_get_synthetic_ones() {
    // Andale Mono has one face.
    let andale = fonts("Andale Mono", |_| {});
    assert_eq!(andale.family(), "Andale Mono", "Andale Mono ships with macOS");
    assert!(andale.is_synthetic_bold(Style::Bold) && andale.is_synthetic_bold(Style::BoldItalic));
    assert!(!andale.is_synthetic_bold(Style::Regular) && !andale.is_synthetic_bold(Style::Italic));
    assert!(andale.is_synthetic_italic(Style::Italic) && andale.is_synthetic_italic(Style::BoldItalic));
    let regular = andale.glyph_key('A', Style::Regular).unwrap();
    let bold = andale.glyph_key('A', Style::Bold).unwrap();
    assert_eq!((bold.font, bold.glyph), (regular.font, regular.glyph), "the regular face's glyph");
    assert_ne!(bold.variant & GlyphKey::EMBOLDEN, 0);
    let ink_of = |s| ink(&andale.rasterize('A', s).unwrap());
    assert!(ink_of(Style::Bold) > ink_of(Style::Regular) * 12 / 10);
    // Slanted: its own face, and wider ink for an upright letter.
    let italic = andale.glyph_key('l', Style::Italic).unwrap();
    assert_ne!(italic.font, regular.font);
    let w = |s| andale.rasterize('l', s).unwrap().width;
    assert!(
        w(Style::Italic) > w(Style::Regular),
        "{} vs {}",
        w(Style::Italic),
        w(Style::Regular)
    );
    // Shaped runs keep the flag.
    let mut run = term_font::RunText::new();
    run.push_char('A', 1);
    let mut out = Vec::new();
    andale.shape(&run, Style::Bold, term_font::Features { ligatures: false }, &mut out);
    assert_ne!(out[0].key.variant & GlyphKey::EMBOLDEN, 0, "{out:?}");

    // Off: bold and italic text is drawn with the regular face.
    let off = fonts("Andale Mono", |c| {
        c.synthetic_bold = false;
        c.synthetic_italic = false;
    });
    for s in [Style::Bold, Style::Italic, Style::BoldItalic] {
        assert!(!off.is_synthetic_bold(s) && !off.is_synthetic_italic(s), "{s:?}");
        assert_eq!(off.glyph_key('A', s), off.glyph_key('A', Style::Regular), "{s:?}");
    }

    // The slant survives the copies features, variations and a fallback
    // list make of each face.
    let busy = fonts("Andale Mono", |c| {
        c.features = vec![(Tag::new("zero").unwrap(), 1)];
        c.variations[Style::Italic as usize] = vec![(Tag::new("wght").unwrap(), 500.0)];
        c.fallback = vec!["Apple Symbols".into()];
    });
    for s in [Style::Italic, Style::BoldItalic] {
        assert!(busy.is_synthetic_italic(s), "{s:?}");
        let w = |s| busy.rasterize('l', s).unwrap().width;
        assert_eq!(w(s), andale.rasterize('l', s).unwrap().width, "{s:?}");
    }

    // Menlo has all four: nothing synthetic.
    let menlo = fonts("Menlo", |_| {});
    for s in Style::ALL {
        assert!(!menlo.is_synthetic_bold(s) && !menlo.is_synthetic_italic(s), "{s:?}");
        assert_eq!(menlo.glyph_key('A', s).unwrap().variant, 0, "{s:?}");
    }
}

#[test]
fn variations_set_a_variable_fonts_axes() {
    let wght = Tag::new("wght").unwrap();
    let skia = |w: Option<f64>| {
        fonts("Skia", |c| {
            if let Some(w) = w {
                c.variations[Style::Regular as usize] = vec![(wght, w)];
            }
        })
    };
    let (light, plain, heavy) = (skia(Some(0.5)), skia(None), skia(Some(3.0)));
    assert_eq!(plain.family(), "Skia", "Skia ships with macOS");
    let m = |f: &Fonts| ink(&f.rasterize('M', Style::Regular).unwrap());
    assert!(
        m(&light) < m(&plain) && m(&plain) < m(&heavy),
        "{} {} {}",
        m(&light),
        m(&plain),
        m(&heavy)
    );
    // Only the face it is set for.
    assert_eq!(
        ink(&heavy.rasterize('M', Style::Bold).unwrap()),
        ink(&plain.rasterize('M', Style::Bold).unwrap())
    );
    // A font without the axis ignores it.
    let menlo = fonts("Menlo", |c| c.variations[0] = vec![(wght, 3.0)]);
    let plain = fonts("Menlo", |_| {});
    assert_eq!(
        menlo.rasterize('M', Style::Regular).unwrap().data,
        plain.rasterize('M', Style::Regular).unwrap().data
    );
}

#[test]
fn per_face_features_follow_the_shared_ones() {
    let t = |s| Tag::new(s).unwrap();
    let mut c = FontConfig::new("Menlo", 13.0);
    c.features = vec![(t("liga"), 0), (t("zero"), 1)];
    c.style_features[Style::Bold as usize] = vec![(t("zero"), 0)];
    assert_eq!(c.features_of(Style::Regular), [(t("liga"), 0), (t("zero"), 1)]);
    assert_eq!(c.features_of(Style::Bold), [(t("liga"), 0), (t("zero"), 1), (t("zero"), 0)]);
    // Builds, and the faces still draw.
    let f = Fonts::with_config(&c, 2.0);
    for s in Style::ALL {
        assert!(f.rasterize('0', s).is_some(), "{s:?}");
    }
}

#[test]
fn metric_adjustments() {
    let plain = fonts("Menlo", |_| {}).metrics;
    let at = |edit: fn(&mut FontConfig)| fonts("Menlo", edit).metrics;

    // Cell height: points at 2 pixels a point, or a percentage; the text
    // stays centred.
    let m = at(|c| c.cell_height = Adjust::Points(2.0));
    assert_eq!(m.cell_height, plain.cell_height + 4);
    assert_eq!(m.baseline, plain.baseline + 2);
    let m = at(|c| c.cell_height = Adjust::Percent(50.0));
    assert_eq!(m.cell_height, plain.cell_height + (plain.cell_height as f64 * 0.5).round() as u32);
    let m = at(|c| {
        c.line_height = 2.0;
        c.cell_height = Adjust::Points(-1.0);
    });
    assert_eq!(m.cell_height, plain.cell_height * 2 - 2, "after line_height");

    // Baseline: positive raises the text.
    let m = at(|c| c.baseline = Adjust::Points(1.0));
    assert_eq!((m.baseline, m.cell_height), (plain.baseline - 2, plain.cell_height));

    // Underline and strikethrough.
    let m = at(|c| {
        c.underline_position = Adjust::Points(1.0);
        c.underline_thickness = Adjust::Percent(100.0);
    });
    assert_eq!(m.underline_offset, plain.underline_offset + 2);
    assert_eq!(m.underline_thickness, plain.underline_thickness * 2);
    assert_eq!(m.strike_thickness(), plain.underline_thickness, "the strike keeps the font's");
    let m = at(|c| {
        c.strike_position = Adjust::Points(1.0);
        c.strike_thickness = Adjust::Points(1.0);
    });
    assert_eq!(m.strike_offset, plain.strike_offset - 2, "positive moves it down");
    assert_eq!(m.strike_thickness(), plain.underline_thickness + 2);

    // The cursor: an eighth of the cell by default.
    assert_eq!(plain.cursor_thickness(), (plain.cell_width / 8).max(2));
    let m = at(|c| c.cursor_thickness = Adjust::Percent(100.0));
    assert_eq!(m.cursor_thickness(), plain.cursor_thickness() * 2);
    let m = at(|c| c.cursor_thickness = Adjust::Points(-100.0));
    assert_eq!(m.cursor_thickness(), 1, "at least a pixel");

    // Nothing set: the font's own.
    assert_eq!(format!("{plain:?}"), format!("{:?}", Fonts::new("Menlo", 26.0).metrics));
}

#[test]
fn the_symbol_map_draws_ranges_from_their_font() {
    let f = fonts("Menlo", |c| {
        c.symbol_map = vec![(0x41..=0x42, "Monaco".into()), (0xE0A0..=0xE0A3, "No Such Font".into())]
    });
    for s in Style::ALL {
        assert_eq!(f.family_for('A', s).as_deref(), Some("Monaco"), "{s:?}");
        assert_eq!(f.family_for('B', s).as_deref(), Some("Monaco"));
        assert_eq!(f.family_for('C', s).as_deref(), Some("Menlo"));
    }
    assert_eq!(f.missing(), ["No Such Font"]);
    // A mapped font that lacks the character: the usual lookup.
    let f = fonts("Menlo", |c| c.symbol_map = vec![(0x6F22..=0x6F22, "Monaco".into())]);
    assert_ne!(f.family_for('漢', Style::Regular).as_deref(), Some("Monaco"));
    assert!(f.glyph_key('漢', Style::Regular).is_some());
}

#[test]
fn the_emoji_font_draws_colour_glyphs_it_has() {
    let plain = fonts("Menlo", |_| {});
    assert_eq!(plain.family_for('😀', Style::Regular).as_deref(), Some("Apple Color Emoji"));
    // A font without emoji: Core Text's pick stays.
    let f = fonts("Menlo", |c| c.emoji_font = Some("Monaco".into()));
    assert_eq!(f.family_for('😀', Style::Regular).as_deref(), Some("Apple Color Emoji"));
    assert!(f.missing().is_empty());
    let f = fonts("Menlo", |c| c.emoji_font = Some("Apple Color Emoji".into()));
    assert_eq!(f.family_for('😀', Style::Regular).as_deref(), Some("Apple Color Emoji"));
    let f = fonts("Menlo", |c| c.emoji_font = Some("No Such Emoji".into()));
    assert_eq!(f.missing(), ["No Such Emoji"]);
    // Text isn't affected.
    assert_eq!(f.family_for('A', Style::Regular).as_deref(), Some("Menlo"));
}
