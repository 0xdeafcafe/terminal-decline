//! Shaping runs (cells design §10.2): clusters shape to the glyphs and keys
//! expected, both shaping paths agree, glyphs stay on the cell grid, and the
//! shaped-cluster cache gives the same keys and keeps to its cap.

use term_font::{Features, Fonts, GlyphKey, RunText, SPRITE_FONT, ShapedGlyph, SpriteId, Style};

const CLUSTERS: Features = Features { ligatures: false };
const LINE: Features = Features { ligatures: true };

fn shape(f: &Fonts, run: &RunText, features: Features) -> Vec<ShapedGlyph> {
    let mut out = Vec::new();
    f.shape(run, Style::Regular, features, &mut out);
    out
}

/// A run of (text, width) cells.
fn run(cells: &[(&str, u8)]) -> RunText {
    let mut r = RunText::new();
    for &(s, w) in cells {
        r.push(s, w);
    }
    r
}

fn key(f: &Fonts, ch: char) -> GlyphKey {
    f.glyph_key(ch, Style::Regular).unwrap()
}

fn is_emoji(f: &Fonts, k: GlyphKey) -> bool {
    f.family_of(k.font).as_deref() == Some("Apple Color Emoji")
}

#[test]
fn clusters_shape_to_the_expected_glyphs() {
    let f = Fonts::new("Menlo", 26.0);
    let family = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
    for features in [CLUSTERS, LINE] {
        // Each cluster between two ASCII cells, so the cell numbering shows.
        for (cluster, width) in [("e\u{301}", 1), ("🇬🇧", 2), (family, 2), ("👍🏽", 2), ("1\u{FE0F}\u{20E3}", 2)] {
            let g = shape(&f, &run(&[("a", 1), (cluster, width), ("b", 1)]), features);
            assert_eq!(g.len(), 3, "{cluster:?} is one glyph ({features:?}): {g:?}");
            assert_eq!((g[0].cell, g[0].key), (0, key(&f, 'a')));
            assert_eq!((g[1].cell, g[1].cells, g[1].dx, g[1].dy), (1, width, 0, 0), "{cluster:?}");
            assert_eq!((g[2].cell, g[2].key), (1 + width as u16, key(&f, 'b')));
            let k = g[1].key;
            if cluster == "e\u{301}" {
                // Menlo composes it: the same glyph as precomposed é.
                assert_eq!(k, key(&f, 'é'));
            } else {
                assert!(is_emoji(&f, k), "{cluster:?} from {:?}", f.family_of(k.font));
                assert_eq!(k.cells, 2, "colour glyphs are keyed at 2 cells");
            }
        }
        // Not the first scalar's glyph alone.
        let one = |s: &str| shape(&f, &run(&[(s, 2)]), features)[0].key;
        assert_ne!(one(family), key(&f, '👨'));
        assert_ne!(one("👍🏽"), key(&f, '👍'));
        assert_ne!(one("1\u{FE0F}\u{20E3}").font, key(&f, '1').font);
        assert_ne!(one("🇬🇧"), one("🇫🇷"));
    }
}

#[test]
fn combining_marks_without_a_precomposed_glyph_are_two_glyphs_in_one_cell() {
    let f = Fonts::new("Menlo", 26.0);
    for features in [CLUSTERS, LINE] {
        let g = shape(&f, &run(&[("x\u{301}", 1), ("y", 1)]), features);
        assert_eq!(g.len(), 3, "{g:?}");
        assert_eq!(g[0].key, key(&f, 'x'));
        assert_eq!((g[0].cell, g[1].cell, g[2].cell), (0, 0, 1));
        assert_eq!(g[1].key.font, 0, "Menlo has the combining acute");
        assert_ne!(g[1].key, g[0].key);
    }
}

/// With a font that has no ligatures the two paths give the same glyphs,
/// fallback fonts get the ids the per-character lookup gave them, and
/// glyphs sit on the grid even where the fallback's advance isn't the
/// cells' width (PingFang's 漢 is narrower than two Menlo cells).
#[test]
fn both_paths_agree_and_stay_on_the_grid() {
    let f = Fonts::new("Menlo", 26.0);
    let r = run(&[
        ("a", 1),
        ("漢", 2),
        ("", 1),
        ("漢", 2),
        ("e\u{301}", 1),
        ("─", 1),
        ("😀", 2),
        ("🇬🇧", 2),
        ("👍🏽", 2),
        ("x\u{301}", 1),
        ("z", 1),
    ]);
    let (cells, line) = (shape(&f, &r, CLUSTERS), shape(&f, &r, LINE));
    assert_eq!(cells, line);
    let at = |cell: u16| cells.iter().find(|g| g.cell == cell).unwrap();
    assert_eq!(at(1).key, key(&f, '漢'));
    assert_eq!((at(4).key, at(4).dx), (key(&f, '漢'), 0), "the second 漢 doesn't drift");
    assert_eq!(at(7).key, GlyphKey::sprite(SpriteId::for_char('─').unwrap(), 1));
    assert_eq!(at(8).key, key(&f, '😀'), "the emoji font has one id, however it was found");
    assert_eq!(cells.last().unwrap().key, key(&f, 'z'));
    assert_eq!(cells.last().unwrap().cell, r.width() - 1);
    assert!(cells.iter().all(|g| g.cell != 3), "a blank cell has no glyph");
}

#[test]
fn sprites_cover_their_cells() {
    let f = Fonts::new("Menlo", 26.0);
    for features in [CLUSTERS, LINE] {
        let g = shape(&f, &run(&[("─", 2), ("│", 1)]), features);
        assert_eq!(g.len(), 2);
        assert_eq!((g[0].key.font, g[0].key.cells, g[0].cells), (SPRITE_FONT, 2, 2));
        assert_eq!((g[1].key.font, g[1].key.cells, g[1].cell), (SPRITE_FONT, 1, 2));
    }
}

#[test]
fn the_cache_gives_the_same_keys() {
    let f = Fonts::new("Menlo", 26.0);
    let r = run(&[("👨\u{200D}👩\u{200D}👧", 2), ("e\u{301}", 1), ("x\u{301}", 1), ("a", 1), ("👍🏽", 2)]);
    assert_eq!(f.shaped_clusters(), 0);
    let cold = shape(&f, &r, CLUSTERS);
    assert_eq!(f.shaped_clusters(), 4, "single characters don't go in the cache");
    let warm = shape(&f, &r, CLUSTERS);
    assert_eq!(cold, warm);
    assert_eq!(f.shaped_clusters(), 4);
    // Faces are part of the key.
    let mut bold = Vec::new();
    f.shape(&r, Style::Bold, CLUSTERS, &mut bold);
    assert_eq!(f.shaped_clusters(), 8);
    assert_ne!(bold[3].key, cold[3].key, "x in bold");
    f.clear_shaped_clusters();
    assert_eq!(shape(&f, &r, CLUSTERS), cold, "shaped again, the same");
}

#[test]
fn the_cache_keeps_to_its_cap() {
    let f = Fonts::new("Menlo", 13.0);
    let mut n = 0;
    let mut r = RunText::new();
    let mut out = Vec::new();
    // 52 letters × 112 combining marks, all different clusters.
    for base in ('a'..='z').chain('A'..='Z') {
        r.clear();
        for mark in '\u{300}'..='\u{36F}' {
            r.push(&format!("{base}{mark}"), 1);
            n += 1;
        }
        out.clear();
        f.shape(&r, Style::Regular, CLUSTERS, &mut out);
        assert!(f.shaped_clusters() <= 4096);
    }
    assert!(n > 4096 + 1000);
    assert_eq!(f.shaped_clusters(), 4096);
    // The newest are still there: shaping the last row again adds nothing.
    let before = out.clone();
    out.clear();
    f.shape(&r, Style::Regular, CLUSTERS, &mut out);
    assert_eq!(out, before);
    assert_eq!(f.shaped_clusters(), 4096);
}
