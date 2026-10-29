//! Glyph-texture packing (`ShelfPacker`): placed glyphs never overlap and
//! stay inside the texture, also across growing; a full texture grows by
//! doubling up to its cap, keeping what it holds; glyphs bigger than the
//! cap are skipped without marking the texture full.

use proptest::prelude::*;
use term_render::ShelfPacker;
use term_render::packer::{GAP, NoRoom};

type Placed = (u32, u32, u32, u32);

fn overlaps(a: Placed, b: Placed) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

proptest! {
    #[test]
    fn placements_never_overlap_and_stay_in_bounds(
        size in prop::sample::select(vec![16u32, 64, 256, 1024]),
        rects in prop::collection::vec((1u32..80, 1u32..80), 1..300),
    ) {
        let mut p = ShelfPacker::new(size, size * 4);
        let mut placed: Vec<Placed> = Vec::new();
        for (w, h) in rects {
            match p.insert(w, h) {
                Ok((x, y)) => {
                    prop_assert!(x + w <= p.size() && y + h <= p.size(), "{w}x{h} at {x},{y} outside {}", p.size());
                    // The gap is part of the contract: grow each rect by it.
                    let r = (x, y, w + GAP, h + GAP);
                    for q in &placed {
                        prop_assert!(!overlaps(r, *q), "{r:?} overlaps {q:?}");
                    }
                    placed.push(r);
                }
                Err(NoRoom::Full) => {
                    prop_assert!(p.is_full());
                    // Grow as the renderer does; earlier placements stay.
                    let old = p.size();
                    if p.grow() {
                        prop_assert_eq!(p.size(), (old * 2).min(p.max()));
                        prop_assert!(!p.is_full());
                    } else {
                        prop_assert_eq!(p.size(), p.max());
                    }
                }
                Err(NoRoom::TooBig) => prop_assert!(w > p.max() || h > p.max()),
            }
        }
    }

    #[test]
    fn growth_keeps_doubling_to_the_cap(start in 4u32..9, steps in 0usize..8) {
        let size = 1u32 << start;
        let max = 1u32 << 10;
        let mut p = ShelfPacker::new(size, max);
        let mut expected = size;
        for _ in 0..steps {
            // Fill it.
            while p.insert(size, size).is_ok() {}
            prop_assert!(p.is_full());
            let grew = p.grow();
            prop_assert_eq!(grew, expected < max);
            expected = (expected * 2).min(max);
            prop_assert_eq!(p.size(), expected);
        }
    }
}

#[test]
fn full_then_grow_then_cap() {
    let mut p = ShelfPacker::new(64, 256);
    // 30×30 with a 1-px gap: two per shelf, two shelves in 64 px.
    for _ in 0..4 {
        p.insert(30, 30).unwrap();
    }
    assert_eq!(p.insert(30, 30), Err(NoRoom::Full));
    assert!(p.is_full());

    assert!(p.grow(), "a full texture grows");
    assert_eq!(p.size(), 128);
    assert!(!p.is_full());
    assert_eq!(p.insert(30, 30), Ok((65, 0)), "growing keeps the old square and fills beside it");

    while p.insert(30, 30).is_ok() {}
    assert!(p.grow());
    assert_eq!(p.size(), 256);

    while p.insert(30, 30).is_ok() {}
    assert!(!p.grow(), "at the cap it doesn't grow");
    assert_eq!(p.size(), 256);
    assert!(p.is_full());
    p.clear();
    assert!(!p.is_full());
    assert_eq!(p.size(), 256);
    assert_eq!(p.insert(30, 30), Ok((0, 0)), "clearing empties it");
}

#[test]
fn growing_fills_the_right_strip_then_the_bottom() {
    let mut p = ShelfPacker::new(64, 128);
    p.insert(64, 64).unwrap();
    assert!(p.grow());
    // Beside the old square, then below it across the full width.
    assert_eq!(p.insert(63, 64), Ok((65, 0)));
    assert_eq!(p.insert(100, 10), Ok((0, 65)));
    // The old square and the right strip are full: the bottom shelf.
    assert_eq!(p.insert(10, 10), Ok((101, 65)));
}

#[test]
fn clear_keeps_the_size() {
    let mut p = ShelfPacker::new(64, 256);
    p.insert(10, 10).unwrap();
    p.clear();
    assert_eq!(p.size(), 64);
    assert_eq!(p.insert(10, 10), Ok((0, 0)));
}

/// Regression: a glyph bigger than the current texture (but within the cap)
/// used to be rejected without marking the texture full, so it never grew
/// and the renderer rebuilt both textures three times every frame. Colour
/// emoji hit this from about 109 pt at 2× scale, as that texture starts at
/// 256 px.
#[test]
fn glyph_bigger_than_texture_asks_it_to_grow() {
    let mut p = ShelfPacker::new(256, 2048);
    assert_eq!(p.insert(300, 150), Err(NoRoom::Full));
    assert!(p.is_full());
    assert!(p.grow());
    assert_eq!(p.size(), 512);
    assert_eq!(p.insert(300, 150), Ok((0, 257)), "below the old square, the only place that fits");
}

#[test]
fn glyph_bigger_than_the_cap_is_skipped() {
    let mut p = ShelfPacker::new(256, 2048);
    assert_eq!(p.insert(2049, 10), Err(NoRoom::TooBig));
    assert_eq!(p.insert(10, 4000), Err(NoRoom::TooBig));
    assert!(!p.is_full(), "an impossible glyph must not trigger a rebuild");
    assert_eq!(p.insert(10, 10), Ok((0, 0)), "nothing was used up");
}

#[test]
fn exact_fit_and_shelf_wrap() {
    let mut p = ShelfPacker::new(64, 64);
    assert_eq!(p.insert(64, 10), Ok((0, 0)));
    assert_eq!(
        p.insert(5, 5),
        Ok((0, 11)),
        "next shelf starts after the tallest glyph plus the gap"
    );
    assert_eq!(p.insert(5, 20), Ok((6, 11)));
    assert_eq!(p.insert(60, 5), Ok((0, 32)));
}
