//! Curly, dotted and dashed underline sprites: each draws at every cell
//! size inside the cell, below the baseline, and tiles across cells without
//! a seam.

use term_font::sprite::{self, SpriteId};
use term_font::{Fonts, GlyphKey, Metrics, RasterGlyph};

const PATTERNS: [SpriteId; 3] = [SpriteId::UNDERLINE_CURLY, SpriteId::UNDERLINE_DOTTED, SpriteId::UNDERLINE_DASHED];

/// Cell width, height, underline thickness: small non-Retina to large
/// Retina, odd and even widths.
const SIZES: [(u32, u32, u32); 8] = [
    (6, 13, 1),
    (7, 15, 1),
    (8, 17, 1),
    (13, 28, 3),
    (16, 34, 2),
    (17, 36, 2),
    (19, 41, 2),
    (24, 50, 3),
];

fn metrics(w: u32, h: u32, t: u32) -> Metrics {
    Metrics {
        cell_width: w,
        cell_height: h,
        baseline: h * 4 / 5,
        underline_offset: 1 + t / 2,
        underline_thickness: t,
        strike_offset: h / 4,
    }
}

fn draw(id: SpriteId, m: &Metrics, cells: u32) -> RasterGlyph {
    sprite::rasterize(id, m, cells).unwrap_or_else(|| panic!("{id:?} drew nothing"))
}

fn at(g: &RasterGlyph, x: u32, y: u32) -> u8 {
    g.data[(y * g.width + x) as usize]
}

/// First row of the bitmap, counted from the cell's top.
fn first_row(g: &RasterGlyph, m: &Metrics) -> i32 {
    m.baseline as i32 - g.top
}

/// Weighted mean row of column `x`'s ink.
fn centroid(g: &RasterGlyph, x: u32) -> f64 {
    let (mut sum, mut ink) = (0.0, 0.0);
    for y in 0..g.height {
        let a = at(g, x, y) as f64;
        sum += a * y as f64;
        ink += a;
    }
    sum / ink
}

/// Lengths of the alternating ink and gap runs along row `y` of `cells`
/// cells, starting at the first ink and wrapping around, so the gap across
/// the strip's end counts like any other.
fn runs(g: &RasterGlyph, y: u32) -> (Vec<u32>, Vec<u32>) {
    let row: Vec<bool> = (0..g.width).map(|x| at(g, x, y) > 0).collect();
    let start = row.iter().position(|&b| b).expect("ink in the row");
    let (mut ink, mut gaps) = (vec![], vec![]);
    let mut len = 0;
    let mut cur = true;
    for i in 0..row.len() {
        let b = row[(start + i) % row.len()];
        if b == cur {
            len += 1;
        } else {
            if cur {
                ink.push(len)
            } else {
                gaps.push(len)
            }
            cur = b;
            len = 1;
        }
    }
    if cur {
        ink.push(len)
    } else {
        gaps.push(len)
    }
    (ink, gaps)
}

#[test]
fn ids_name_no_character() {
    for id in PATTERNS {
        assert!(id.0 >= 0x1000);
        assert_eq!(id.char(), None);
    }
    assert!(sprite::rasterize(SpriteId(0x1003), &metrics(8, 17, 1), 1).is_none());
}

#[test]
fn every_pattern_draws_inside_the_cell_below_the_baseline() {
    for (w, h, t) in SIZES {
        // Also with the underline pushed to (and past) the cell's bottom.
        for m in [
            metrics(w, h, t),
            Metrics {
                baseline: h - 2,
                underline_offset: 3,
                ..metrics(w, h, t)
            },
        ] {
            for id in PATTERNS {
                for cells in [1, 2, 3] {
                    let g = draw(id, &m, cells);
                    assert!(!g.color);
                    assert_eq!((g.width, g.left), (w * cells, 0), "{id:?} at {w}x{h}");
                    assert_eq!(g.data.len(), (g.width * g.height) as usize);
                    let y0 = first_row(&g, &m);
                    // Below the baseline when there's room for the pattern
                    // there; otherwise the cell wins.
                    if h - m.baseline >= g.height {
                        assert!(y0 >= m.baseline as i32, "{id:?} at {w}x{h} starts above the baseline: {y0}");
                    }
                    assert!(y0 + g.height as i32 <= h as i32, "{id:?} at {w}x{h} leaves the cell");
                }
            }
        }
    }
}

#[test]
fn dotted_and_dashed_sit_on_the_single_underline() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        for id in [SpriteId::UNDERLINE_DOTTED, SpriteId::UNDERLINE_DASHED] {
            let g = draw(id, &m, 1);
            assert_eq!(first_row(&g, &m), (m.baseline + m.underline_offset) as i32, "{id:?} at {w}x{h}");
            assert_eq!(g.height, t, "{id:?} is the underline's thickness");
            assert!(g.data.iter().all(|&a| a == 0 || a == 255), "{id:?} is whole pixels");
            for y in 1..g.height {
                assert!((0..w).all(|x| at(&g, x, y) == at(&g, x, 0)), "{id:?} rows alike");
            }
        }
    }
}

#[test]
fn several_cells_are_the_one_cell_sprite_repeated() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        for id in PATTERNS {
            let one = draw(id, &m, 1);
            for cells in [2, 3] {
                let many = draw(id, &m, cells);
                assert_eq!((many.height, many.top), (one.height, one.top));
                for y in 0..one.height {
                    for x in 0..many.width {
                        assert_eq!(at(&many, x, y), at(&one, x % w, y), "{id:?} {cells} cells at {w}x{h}");
                    }
                }
            }
        }
    }
}

/// Dots: as many as fit with gaps at least a dot wide, all the same size,
/// and the gap across a cell edge within a pixel of the gaps inside.
#[test]
fn dotted_tiles_without_a_seam() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        let g = draw(SpriteId::UNDERLINE_DOTTED, &m, 3);
        let (dots, gaps) = runs(&g, 0);
        let d = dots[0];
        assert!(dots.iter().all(|&n| n == d), "dots alike at {w}x{h}: {dots:?}");
        assert_eq!(d, t.min(w / 2), "a dot is as wide as the line is thick");
        assert!(dots.len() >= 3 && dots.len() % 3 == 0, "{dots:?}");
        let (lo, hi) = (*gaps.iter().min().unwrap(), *gaps.iter().max().unwrap());
        assert!(lo >= d && hi - lo <= 1, "even gaps at {w}x{h}, across edges too: {gaps:?}");
    }
}

/// Dashes: one per cell, longer than the gaps, and every gap (all of them
/// across a cell edge) the same.
#[test]
fn dashed_tiles_without_a_seam() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        let g = draw(SpriteId::UNDERLINE_DASHED, &m, 3);
        let (dashes, gaps) = runs(&g, 0);
        assert_eq!(dashes.len(), 3, "one dash per cell at {w}x{h}");
        assert!(dashes.iter().all(|&n| n == dashes[0]));
        assert!(gaps.iter().all(|&n| n == gaps[0]), "{gaps:?}");
        assert!(dashes[0] > gaps[0] && gaps[0] >= 1, "{dashes:?} {gaps:?} at {w}x{h}");
        assert_eq!(dashes[0] + gaps[0], w);
    }
}

/// The wave: ink in every column, about a stroke's worth, lowest at the
/// cell edges and highest in the middle, and no bigger a step between the
/// two columns either side of a cell edge than between any two inside.
#[test]
fn curly_tiles_without_a_seam() {
    for (w, h, t) in SIZES {
        let m = metrics(w, h, t);
        let g = draw(SpriteId::UNDERLINE_CURLY, &m, 3);
        for x in 0..g.width {
            let ink: u32 = (0..g.height).map(|y| at(&g, x, y) as u32).sum();
            assert!(ink as f64 >= 0.8 * 255.0 * t as f64, "column {x} at {w}x{h} has {ink}");
            assert!(ink as f64 <= 2.0 * 255.0 * t as f64, "column {x} at {w}x{h} has {ink}");
        }
        let c: Vec<f64> = (0..g.width).map(|x| centroid(&g, x)).collect();
        // Lowest at the edges, highest in the middle, by at least a pixel.
        assert!(c[0] - c[(w / 2) as usize] >= 1.0, "a wave at {w}x{h}: {c:?}");
        let step = |x: usize| (c[x + 1] - c[x]).abs();
        let inner = (0..w as usize - 1).map(step).fold(0.0, f64::max);
        for edge in [w as usize - 1, 2 * w as usize - 1] {
            assert!(step(edge) <= inner + 1e-9, "seam at {w}x{h}: {} vs {inner}", step(edge));
        }
        // The two columns either side of an edge mirror each other (the
        // wave is flat there), to within one subsample column.
        for y in 0..g.height {
            let (l, r) = (at(&g, w - 1, y), at(&g, w, y));
            assert!(l.abs_diff(r) <= 255 / 6 + 1, "edge columns at {w}x{h} row {y}: {l} vs {r}");
        }
    }
}

#[test]
fn fonts_draw_underline_sprites_from_their_metrics() {
    for size in [13.0, 26.0, 37.0] {
        let f = Fonts::new("Menlo", size);
        let m = f.metrics;
        for id in PATTERNS {
            let g = f.rasterize_key(GlyphKey::sprite(id, 1)).expect("an underline");
            assert_eq!(g.width, m.cell_width);
            let y0 = first_row(&g, &m);
            assert!(
                y0 >= m.baseline as i32 && y0 + g.height as i32 <= m.cell_height as i32,
                "{id:?} at {size}: {m:?}"
            );
            assert_eq!(g.data, draw(id, &m, 1).data);
        }
    }
}
