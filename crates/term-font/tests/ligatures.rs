//! Ligatures (B.1): which faces shape whole runs, features, and runs
//! shaped through one CTLine with every cell the face doesn't draw kept
//! out. JetBrains Mono Regular is checked in under `tests/fonts` (OFL) and
//! registered for this process, so nothing needs installing.

use std::path::Path;
use std::sync::Once;

use term_font::{CellWidth, Features, FontConfig, Fonts, GlyphKey, RunText, ShapedGlyph, Style, Tag, register_font_file};

const CLUSTERS: Features = Features { ligatures: false };
const LINE: Features = Features { ligatures: true };

fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/JetBrainsMono-Regular.ttf");
        assert!(register_font_file(Path::new(path)), "registering {path}");
    });
}

/// `family` at 13 pt, scale 2 (26 px), with `features` and runs allowed.
fn fonts_with(family: &str, features: &[&str], edit: impl FnOnce(&mut FontConfig)) -> Fonts {
    register();
    let mut c = FontConfig::new(family, 13.0);
    c.features = features.iter().map(|s| Tag::parse_feature(s).unwrap()).collect();
    edit(&mut c);
    let mut f = Fonts::with_config(&c, 2.0);
    assert_eq!(f.family(), family, "the test needs {family}");
    f.allow_runs(true);
    f
}

fn jb(features: &[&str]) -> Fonts {
    fonts_with("JetBrains Mono", features, |_| {})
}

fn runs(f: &Fonts) -> [bool; 4] {
    Style::ALL.map(|s| f.features(s).ligatures)
}

/// A run of one-cell characters; spaces are blank cells.
fn text(s: &str) -> RunText {
    let mut r = RunText::new();
    for ch in s.chars() {
        let mut buf = [0; 4];
        r.push(if ch == ' ' { "" } else { ch.encode_utf8(&mut buf) }, 1);
    }
    r
}

fn shape(f: &Fonts, run: &RunText, style: Style, features: Features) -> Vec<ShapedGlyph> {
    let mut out = Vec::new();
    f.shape(run, style, features, &mut out);
    out
}

/// The glyphs at `cell`.
fn at(g: &[ShapedGlyph], cell: u16) -> Vec<ShapedGlyph> {
    g.iter().copied().filter(|g| g.cell == cell).collect()
}

/// Every cell of `run` that isn't blank is covered by a glyph.
fn assert_covered(run: &str, g: &[ShapedGlyph]) {
    for (i, ch) in run.chars().enumerate() {
        let i = i as u16;
        if ch != ' ' {
            assert!(
                g.iter().any(|g| g.cell <= i && i < g.cell + g.cells as u16),
                "cell {i} ({ch:?}) of {run:?} uncovered: {g:?}"
            );
        }
    }
}

#[test]
fn runs_stay_off_until_the_renderer_allows_them() {
    register();
    let mut f = Fonts::with_config(&FontConfig::new("JetBrains Mono", 13.0), 2.0);
    assert_eq!(runs(&f), [false; 4]);
    f.allow_runs(true);
    assert_eq!(runs(&f), [true; 4]);
}

#[test]
fn apple_monospaced_fonts_stay_on_the_fast_path() {
    // Menlo, Monaco and Courier have `morx` and are probed; Courier New
    // lists `liga` and `rlig` for Arabic only.
    for family in ["Menlo", "Monaco", "Courier", "Courier New"] {
        let f = fonts_with(family, &[], |_| {});
        assert_eq!(runs(&f), [false; 4], "{family}");
    }
    register();
    let sf = Fonts::new("SF Mono", 26.0);
    if sf.family() == "SF Mono" {
        let mut sf = sf;
        sf.allow_runs(true);
        assert_eq!(runs(&sf), [false; 4], "SF Mono");
    }
}

#[test]
fn jetbrains_mono_shapes_runs_unless_turned_off() {
    // Regular only: bold and italic are the same font, and ligate too.
    assert_eq!(runs(&jb(&[])), [true; 4]);
    assert_eq!(runs(&jb(&["-calt", "-liga"])), [false; 4]);
    // Its programming ligatures are all contextual alternates.
    assert_eq!(runs(&jb(&["-liga"])), [true; 4], "calt still on");
    assert_eq!(runs(&jb(&["-calt"])), [false; 4]);
    assert_eq!(runs(&jb(&["-calt", "+dlig"])), [false; 4], "it has no dlig");
    // Single-glyph features alone stay on the fast path, and still pick
    // their glyphs.
    let zero = jb(&["-calt", "-liga", "+zero"]);
    assert_eq!(runs(&zero), [false; 4]);
    let plain = jb(&[]);
    for style in Style::ALL {
        assert_ne!(zero.glyph_key('0', style), plain.glyph_key('0', style), "{style:?}");
        assert_eq!(zero.glyph_key('1', style), plain.glyph_key('1', style));
    }
    let r = text("10");
    assert_eq!(
        shape(&zero, &r, Style::Regular, CLUSTERS)[1].key,
        zero.glyph_key('0', Style::Regular).unwrap()
    );
}

#[test]
fn plus_liga_turns_on_menlos_fi() {
    let f = fonts_with("Menlo", &["+liga"], |_| {});
    assert!(runs(&f)[0]);
    let g = shape(&f, &text("fi"), Style::Regular, f.features(Style::Regular));
    assert_eq!(g.len(), 1, "{g:?}");
    assert_eq!((g[0].cell, g[0].cells, g[0].split), (0, 2, true));
}

#[test]
fn jetbrains_ligatures_are_split_and_cover_their_cells() {
    let f = jb(&[]);
    for style in [Style::Regular, Style::Bold, Style::Italic] {
        for lig in ["=>", "->", "!=", "===", "<=>", "::", "/*"] {
            let s = format!("ab {lig} cd");
            let g = shape(&f, &text(&s), style, f.features(style));
            assert_covered(&s, &g);
            for &(cell, ch) in &[(0, 'a'), (1, 'b'), (4 + lig.len() as u16, 'c')] {
                let here = at(&g, cell);
                assert_eq!(here.len(), 1, "{s:?} {style:?}: {g:?}");
                assert_eq!(
                    (here[0].key, here[0].split),
                    (f.glyph_key(ch, style).unwrap(), false),
                    "{ch:?} in {s:?}"
                );
            }
            let mine: Vec<_> = g.iter().filter(|g| (3..3 + lig.len() as u16).contains(&g.cell)).collect();
            assert!(mine.iter().any(|g| g.split), "{lig:?} {style:?} ligates: {mine:?}");
            for g in mine {
                let ch = s.chars().nth(g.cell as usize).unwrap();
                let alone = f.glyph_key(ch, style).unwrap();
                assert_eq!(
                    g.split,
                    g.key != alone || g.cells > 1,
                    "split on exactly the substituted glyphs: {g:?}"
                );
                assert_eq!(g.key.cells, 1);
            }
        }
    }
}

#[test]
fn plain_text_is_the_per_character_glyphs() {
    let f = jb(&[]);
    let r = text("hello world  and some more text   ");
    let (line, cells) = (shape(&f, &r, Style::Regular, LINE), shape(&f, &r, Style::Regular, CLUSTERS));
    assert_eq!(line, cells);
    assert!(line.iter().all(|g| !g.split));
}

#[test]
fn helvetica_fi_is_one_glyph_over_two_cells() {
    let f = Fonts::new("Helvetica", 26.0);
    let g = shape(&f, &text("fix"), Style::Regular, LINE);
    assert_eq!(g.len(), 2, "{g:?}");
    assert_eq!((g[0].cell, g[0].cells, g[0].split), (0, 2, true));
    assert_ne!(g[0].key, f.glyph_key('f', Style::Regular).unwrap());
    assert_eq!((g[1].cell, g[1].cells, g[1].split), (2, 1, false));
}

/// Ligatures either side of cells the face doesn't draw: CJK and an emoji
/// sequence from fallback fonts, a combining mark, a fitted icon from the
/// fallback list, a sprite. Those cells get exactly what shaping them
/// alone gives; the ligatures survive.
#[test]
fn cells_the_face_doesnt_draw_are_shaped_alone() {
    let f = fonts_with("JetBrains Mono", &[], |c| c.fallback = vec!["STIX Two Math".into()]);
    // A symbol JetBrains Mono lacks, from the fallback list: fitted.
    let icon = (0x2190..0x2C00)
        .filter_map(char::from_u32)
        .find(|&ch| f.family_for(ch, Style::Regular).as_deref() == Some("STIX Two Math"))
        .expect("a symbol from STIX Two Math");
    let icon = icon.to_string();
    let cells: &[(&str, u8)] = &[
        ("=", 1),
        (">", 1),
        ("漢", 2),
        ("👨\u{200D}👩\u{200D}👧", 2),
        ("e\u{301}", 1),
        (&icon, 1),
        ("x", 1),
        ("─", 1),
        ("-", 1),
        (">", 1),
        ("", 1),
    ];
    let mut r = RunText::new();
    for &(s, w) in cells {
        r.push(s, w);
    }
    let line = shape(&f, &r, Style::Regular, f.features(Style::Regular));
    let alone = shape(&f, &r, Style::Regular, CLUSTERS);
    // Cells 2..=8: 漢 (2–3), family (4–5), é (6), icon (7), x (8), sprite (9).
    for cell in [2, 4, 6, 7, 9] {
        assert_eq!(at(&line, cell), at(&alone, cell), "cell {cell}");
        assert!(at(&line, cell).iter().all(|g| !g.split));
    }
    assert!(at(&line, 7)[0].key.variant & GlyphKey::FITTED != 0, "the icon is fitted");
    assert!(at(&line, 9)[0].key.sprite_id().is_some());
    // `x` is the face's but alone between non-face cells.
    assert_eq!(at(&line, 8)[0].key, f.glyph_key('x', Style::Regular).unwrap());
    for cells in [0..2, 10..12] {
        let lig: Vec<_> = line.iter().filter(|g| cells.contains(&g.cell)).collect();
        assert!(lig.iter().any(|g| g.split), "{cells:?}: {lig:?}");
    }
    assert!(line.iter().all(|g| g.cell != 12), "the trailing blank");
}

/// Right-to-left text is kept out of the CTLine, which would otherwise
/// reorder the run and mirror the brackets beside it.
#[test]
fn right_to_left_cells_stay_in_order_and_unmirrored() {
    let f = Fonts::new("Courier New", 26.0);
    let s = "(א)<ב>xy";
    let r = text(s);
    let line = shape(&f, &r, Style::Regular, LINE);
    let alone = shape(&f, &r, Style::Regular, CLUSTERS);
    assert_eq!(line, alone);
    for (i, ch) in s.chars().enumerate() {
        assert_eq!(at(&line, i as u16)[0].key, f.glyph_key(ch, Style::Regular).unwrap(), "{ch:?}");
    }
}

#[test]
fn invisible_characters_dont_widen_their_neighbours() {
    let f = jb(&[]);
    for invisible in ["\u{AD}", "\u{200B}"] {
        let mut r = RunText::new();
        for s in ["a", "=", invisible, "=", "b"] {
            r.push(s, 1);
        }
        let line = shape(&f, &r, Style::Regular, f.features(Style::Regular));
        let alone = shape(&f, &r, Style::Regular, CLUSTERS);
        assert_eq!(at(&line, 2), at(&alone, 2), "{invisible:?}");
        for cell in [1, 3] {
            assert!(at(&line, cell).iter().all(|g| g.cells == 1), "{invisible:?} {line:?}");
        }
    }
}

#[test]
fn wide_glyphs_of_the_face_are_not_split() {
    let f = jb(&[]);
    // → is ambiguous-width: some terminals give it two cells.
    assert_eq!(f.family_for('→', Style::Regular).as_deref(), Some("JetBrains Mono"));
    let mut r = RunText::new();
    for (s, w) in [("a", 1), ("→", 2), ("b", 1), ("c", 1)] {
        r.push(s, w);
    }
    let g = shape(&f, &r, Style::Regular, f.features(Style::Regular));
    assert_eq!(at(&g, 1).len(), 1);
    let arrow = at(&g, 1)[0];
    assert_eq!(
        (arrow.cells, arrow.split, arrow.key),
        (2, false, f.glyph_key('→', Style::Regular).unwrap())
    );
}

/// The x range, from its cell's origin, of the ink of `g`.
fn ink(f: &Fonts, key: GlyphKey) -> (f64, f64) {
    let r = f.rasterize_key(key).expect("ink");
    let w = r.width as usize;
    let col = |c: usize| (0..r.height as usize).any(|y| r.data[y * w + c] > 0);
    let first = (0..w).find(|&c| col(c)).unwrap();
    let last = (0..w).rev().find(|&c| col(c)).unwrap();
    ((r.left + first as i32) as f64, (r.left + last as i32 + 1) as f64)
}

/// With cells wider or narrower than the advance, a ligature's ink is
/// stretched so it lands on its cells: where the font's own ink, scaled
/// by cell width / advance, says, within 1 px.
#[test]
fn scaled_ligatures_stay_on_their_cells() {
    let plain = jb(&[]);
    let advance = plain.advance('M', Style::Regular).unwrap();
    let lig = text("===");
    let inked = |f: &Fonts| {
        let g = shape(f, &lig, Style::Regular, f.features(Style::Regular));
        g.into_iter()
            .filter(|g| f.rasterize_key(g.key).is_some())
            .max_by_key(|g| f.rasterize_key(g.key).unwrap().width)
            .unwrap()
    };
    let base = inked(&plain);
    assert!(base.split);
    assert_eq!(base.key.variant & GlyphKey::SCALED, 0, "within 1 px at the font's own width");
    // The font's ink at its own advance: the default cell is round(advance).
    let (l0, r0) = ink(&plain, base.key);
    let (font_l, font_r) = (l0, r0);
    for width in [CellWidth::Points(2.0), CellWidth::Points(-1.0), CellWidth::Percent(20.0)] {
        let f = fonts_with("JetBrains Mono", &[], |c| c.cell_width = width);
        let g = inked(&f);
        assert_eq!(g.cell, base.cell);
        assert!(g.key.variant & GlyphKey::SCALED != 0, "{width:?}");
        let k = f.metrics.cell_width as f64 / advance;
        let (l, r) = ink(&f, g.key);
        assert!(
            (l - font_l * k).abs() <= 1.0 && (r - font_r * k).abs() <= 1.0,
            "{width:?}: {l}..{r}, want {}..{}",
            font_l * k,
            font_r * k
        );
        // Unscaled it would be off by more.
        let mut unscaled = g.key;
        unscaled.variant &= !GlyphKey::SCALED;
        let (ul, ur) = ink(&f, unscaled);
        assert!(
            (ul - font_l * k).abs().max((ur - font_r * k).abs()) > 1.0,
            "{width:?}: the test shows something"
        );
    }
}

#[test]
fn shaping_a_run_reuses_its_buffers() {
    // Same output however often a run is shaped, interleaved with others.
    let f = jb(&[]);
    let (a, b) = (text("a => b != c"), text("x === y -> z <=> w"));
    let first = shape(&f, &a, Style::Regular, LINE);
    for _ in 0..3 {
        shape(&f, &b, Style::Bold, LINE);
        assert_eq!(shape(&f, &a, Style::Regular, LINE), first);
    }
}
