//! Code-drawn glyphs (box drawing, blocks, braille, Powerline, legacy
//! computing): every one draws at several cell sizes, bitmaps are exactly
//! the cell, lines reach the cell edges and neighbours join without seams.

use term_font::sprite::{self, SpriteId};
use term_font::{Fonts, Metrics, RasterGlyph, Style, is_code_drawn};

/// Cell sizes from small non-Retina to large Retina, odd and even.
const SIZES: [(u32, u32, u32); 5] = [(7, 15, 1), (8, 17, 1), (16, 34, 2), (19, 41, 2), (13, 28, 3)];

fn metrics(w: u32, h: u32, t: u32) -> Metrics {
    Metrics {
        cell_width: w,
        cell_height: h,
        baseline: h * 3 / 4,
        underline_offset: 2,
        underline_thickness: t,
        strike_offset: h / 4,
    }
}

fn covered() -> impl Iterator<Item = char> {
    [
        0x2500..=0x259F,
        0x2800..=0x28FF,
        0xE0A0..=0xE0A3,
        0xE0B0..=0xE0BF,
        0x1FB00..=0x1FB8B,
    ]
    .into_iter()
    .flatten()
    .map(|cp| char::from_u32(cp).unwrap())
}

fn draw(ch: char, m: &Metrics) -> RasterGlyph {
    sprite::rasterize(SpriteId::for_char(ch).unwrap(), m, 1).unwrap_or_else(|| panic!("{ch} drew nothing"))
}

fn at(g: &RasterGlyph, x: u32, y: u32) -> u8 {
    g.data[(y * g.width + x) as usize]
}

fn column(g: &RasterGlyph, x: u32) -> Vec<u8> {
    (0..g.height).map(|y| at(g, x, y)).collect()
}

fn row(g: &RasterGlyph, y: u32) -> Vec<u8> {
    (0..g.width).map(|x| at(g, x, y)).collect()
}

#[test]
fn every_covered_character_draws_at_every_size() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        for ch in covered() {
            assert!(is_code_drawn(ch), "{ch:?}");
            let id = SpriteId::for_char(ch).unwrap();
            assert_eq!(id.char(), Some(ch), "sprite id round trip");
            let Some(g) = sprite::rasterize(id, &m, 1) else {
                assert_eq!(ch, '\u{2800}', "only blank braille draws nothing ({ch:?} at {w}x{h})");
                continue;
            };
            assert_eq!((g.width, g.height), (w, h), "{ch:?} is the cell's size");
            assert_eq!(g.data.len(), (w * h) as usize);
            assert_eq!((g.left, g.top), (0, m.baseline as i32), "{ch:?} sits on the cell");
            assert!(!g.color);
            assert!(g.data.iter().any(|&p| p > 0), "{ch:?} at {w}x{h} has ink");
        }
        // Two cells wide, for wide characters later.
        let g = sprite::rasterize(SpriteId::for_char('─').unwrap(), &m, 2).unwrap();
        assert_eq!((g.width, g.height), (2 * w, h));
    }
}

#[test]
fn others_are_not_code_drawn() {
    for ch in [
        'A',
        ' ',
        '■',
        '→',
        '\u{24FF}',
        '\u{25A0}',
        '\u{27FF}',
        '\u{2900}',
        '\u{E0A4}',
        '\u{E0AF}',
        '\u{E0C0}',
        '\u{1FB8C}',
    ] {
        assert!(!is_code_drawn(ch), "{ch:?}");
    }
}

#[test]
fn lines_reach_the_cell_edges() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        for ch in ['─', '━', '═'] {
            let g = draw(ch, &m);
            let inked: Vec<u32> = (0..h).filter(|&y| at(&g, 0, y) == 255).collect();
            assert!(!inked.is_empty(), "{ch} at {w}x{h}");
            // Around the middle, the same rows at both edges.
            assert!(
                inked.iter().all(|&y| y.abs_diff(h / 2) <= 2 * t + 1),
                "{ch} at {w}x{h}: rows {inked:?}"
            );
            assert_eq!(column(&g, 0), column(&g, w - 1), "{ch} at {w}x{h}");
            // Unbroken from edge to edge.
            for &y in &inked {
                assert!(row(&g, y).iter().all(|&p| p == 255), "{ch} at {w}x{h}: row {y} has a gap");
            }
        }
        for ch in ['│', '┃', '║'] {
            let g = draw(ch, &m);
            let inked: Vec<u32> = (0..w).filter(|&x| at(&g, x, 0) == 255).collect();
            assert!(!inked.is_empty(), "{ch} at {w}x{h}");
            assert!(
                inked.iter().all(|&x| x.abs_diff(w / 2) <= 2 * t + 1),
                "{ch} at {w}x{h}: columns {inked:?}"
            );
            assert_eq!(row(&g, 0), row(&g, h - 1), "{ch} at {w}x{h}");
            for &x in &inked {
                assert!(column(&g, x).iter().all(|&p| p == 255), "{ch} at {w}x{h}: column {x} has a gap");
            }
        }
    }
}

#[test]
fn light_heavy_and_double_thickness() {
    let m = metrics(16, 34, 2);
    let rows = |ch| (0..34).filter(|&y| at(&draw(ch, &m), 0, y) == 255).count();
    assert_eq!(rows('─'), 2, "light = the underline thickness");
    assert_eq!(rows('━'), 4, "heavy = twice that");
    assert_eq!(rows('═'), 4, "double = two light strokes");
}

/// Horizontal neighbours: the right edge of `a` against the left edge of
/// `b`, which must be the same pixels as a straight line's.
fn joins_across(a: char, b: char, m: &Metrics) {
    let (ga, gb) = (draw(a, m), draw(b, m));
    let line = column(&draw('─', m), 0);
    assert_eq!(
        column(&ga, ga.width - 1),
        line,
        "{a} right edge at {}x{}",
        m.cell_width,
        m.cell_height
    );
    assert_eq!(column(&gb, 0), line, "{b} left edge at {}x{}", m.cell_width, m.cell_height);
}

fn joins_down(a: char, b: char, m: &Metrics) {
    let (ga, gb) = (draw(a, m), draw(b, m));
    let line = row(&draw('│', m), 0);
    assert_eq!(
        row(&ga, ga.height - 1),
        line,
        "{a} bottom edge at {}x{}",
        m.cell_width,
        m.cell_height
    );
    assert_eq!(row(&gb, 0), line, "{b} top edge at {}x{}", m.cell_width, m.cell_height);
}

#[test]
fn neighbours_join_without_seams() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        for (a, b) in [
            ('┼', '─'),
            ('─', '┼'),
            ('├', '─'),
            ('─', '┤'),
            ('╭', '─'),
            ('─', '╮'),
            ('╰', '┴'),
            ('┬', '╯'),
            ('┌', '┐'),
        ] {
            joins_across(a, b, &m);
        }
        for (a, b) in [('┼', '│'), ('│', '┼'), ('╭', '│'), ('│', '╰'), ('┬', '┴'), ('├', '└'), ('╮', '┤')] {
            joins_down(a, b, &m);
        }
        // Double and heavy lines against their own straight pieces.
        let (dh, dv) = (column(&draw('═', &m), 0), row(&draw('║', &m), 0));
        for ch in ['╔', '╦', '╠', '╬', '╚', '╩', '╤', '╪'] {
            assert_eq!(column(&draw(ch, &m), w - 1), dh, "{ch} right edge at {w}x{h}");
        }
        for ch in ['╔', '╦', '╠', '╬', '╗', '╣', '╥', '╫'] {
            assert_eq!(row(&draw(ch, &m), h - 1), dv, "{ch} bottom edge at {w}x{h}");
        }
        let hh = column(&draw('━', &m), 0);
        for ch in ['┏', '┳', '┣', '╋', '┍', '┿', '╺'] {
            assert_eq!(column(&draw(ch, &m), w - 1), hh, "{ch} right edge at {w}x{h}");
        }
    }
}

#[test]
fn corners_are_closed() {
    // The corner pixel where the two arms of '┌' meet is inked.
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        let g = draw('┌', &m);
        let x = (0..w).find(|&x| at(&g, x, h - 1) == 255).unwrap();
        let y = (0..h).find(|&y| at(&g, w - 1, y) == 255).unwrap();
        assert_eq!(at(&g, x, y), 255, "┌ at {w}x{h}");
        assert_eq!(at(&g, x.saturating_sub(1), y), 0, "┌ at {w}x{h} doesn't overshoot");
    }
}

#[test]
fn curves_and_diagonals_are_anti_aliased() {
    let m = metrics(16, 34, 2);
    for ch in ['╭', '╱', '\u{E0B0}', '\u{E0B4}', '\u{1FB3C}'] {
        let g = draw(ch, &m);
        assert!(g.data.iter().any(|&p| p > 0 && p < 255), "{ch} has partial coverage");
    }
    // The Powerline triangle fills its left edge, so it meets the cell before it.
    let g = draw('\u{E0B0}', &m);
    assert!(column(&g, 0)[2..32].iter().all(|&p| p == 255));
}

#[test]
fn blocks_tile() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        let full = draw('█', &m);
        assert!(full.data.iter().all(|&p| p == 255), "█ at {w}x{h}");
        // Complementary halves cover each pixel exactly once.
        for (a, b) in [('▀', '▄'), ('▌', '▐'), ('▘', '▟'), ('▚', '▞'), ('▔', '▇'), ('▕', '▉')] {
            let (ga, gb) = (draw(a, &m), draw(b, &m));
            for i in 0..ga.data.len() {
                assert_eq!(ga.data[i] as u32 + gb.data[i] as u32, 255, "{a}{b} at {w}x{h}, pixel {i}");
            }
        }
        // Shades are even, so they tile at any size.
        for (ch, level) in [('░', 0x40), ('▒', 0x80), ('▓', 0xC0)] {
            assert!(draw(ch, &m).data.iter().all(|&p| p == level), "{ch} at {w}x{h}");
        }
        // Lower eighths grow by whole rows.
        let inked = |ch| draw(ch, &m).data.iter().filter(|&&p| p == 255).count() as u32 / w;
        let heights: Vec<u32> = "▁▂▃▄▅▆▇█".chars().map(inked).collect();
        assert!(heights.windows(2).all(|p| p[0] < p[1] || h < 8), "{heights:?} at {w}x{h}");
        assert_eq!(heights[7], h);
    }
}

#[test]
fn braille_and_sextant_bits() {
    // Dot 1 alone, all eight dots: eight times the ink (square dots at
    // small sizes; round ones vary by a pixel's worth of coverage).
    let ink = |ch: char, m: &Metrics| draw(ch, m).data.iter().map(|&p| p as u32).sum::<u32>();
    let small = metrics(8, 17, 1);
    assert_eq!(ink('\u{28FF}', &small), 8 * ink('\u{2801}', &small));
    let m = metrics(16, 34, 2);
    let (all, one) = (ink('\u{28FF}', &m), ink('\u{2801}', &m));
    assert!(all.abs_diff(8 * one) < 255 * 2, "{all} vs 8 × {one}");
    // 🬀 is the upper-left sixth; 🬻 (sextant-23456) is all but that one.
    let one = draw('\u{1FB00}', &m);
    assert_eq!(at(&one, 0, 0), 255);
    assert_eq!(at(&one, 15, 0), 0);
    let five = draw('\u{1FB3B}', &m);
    assert_eq!(at(&five, 0, 0), 0);
    assert_eq!(at(&five, 15, 33), 255);
}

#[test]
fn fonts_draw_code_drawn_glyphs_at_the_cell_size() {
    for size in [13.0, 26.0, 37.0] {
        let f = Fonts::new("Menlo", size);
        let m = f.metrics;
        for style in [Style::Regular, Style::Bold] {
            let g = f.rasterize('─', style).unwrap();
            assert_eq!(
                (g.width, g.height, g.left, g.top),
                (m.cell_width, m.cell_height, 0, m.baseline as i32)
            );
            assert_eq!(g.data, draw('─', &m).data, "same in every style");
        }
        // Braille and Powerline need no font that has them.
        assert!(f.rasterize('\u{E0B0}', Style::Regular).is_some());
        assert!(f.rasterize('\u{28FF}', Style::Regular).is_some());
        assert!(f.rasterize('\u{2800}', Style::Regular).is_none(), "blank braille is like a space");
    }
}
