//! Grapheme clusters and ambiguous-width characters drawn on the GPU with
//! Menlo (cells design §6.3, §6.5): a cluster cell is shaped whole, not
//! drawn as its first character, and a one-cell character in a two-cell
//! cell is centred (font glyphs) or drawn across both cells (code-drawn
//! ones). Each frame is drawn from a snapshot and from the live terminal,
//! which must agree. Without a Metal device the tests are skipped locally
//! and fail on CI (the `CI` env var is set).

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_metal::{
    MTLCommandBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use term_core::Term;
use term_font::Fonts;
use term_render::{FrameInput, GlyphInstance, Renderer, Theme};

const COLS: usize = 6;

struct Pixels {
    data: Vec<u8>,
    width: usize,
    cell: (usize, usize),
}

impl Pixels {
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        self.data[i..i + 4].try_into().unwrap()
    }

    /// Pixels that aren't the black background.
    fn inked(&self, x: usize, y: usize) -> bool {
        let p = self.at(x, y);
        p[0] > 40 || p[1] > 40 || p[2] > 40
    }

    /// Pixels in colour, not grey: emoji, not white text.
    fn coloured(&self, x: usize, y: usize) -> bool {
        let p = self.at(x, y);
        let (lo, hi) = (p[..3].iter().min().unwrap(), p[..3].iter().max().unwrap());
        hi - lo > 60
    }

    /// Columns of pixels with ink in them.
    fn ink_columns(&self) -> Vec<usize> {
        let h = self.data.len() / 4 / self.width;
        (0..self.width).filter(|&x| (0..h).any(|y| self.inked(x, y))).collect()
    }

    fn count(&self, cols: std::ops::Range<usize>, f: impl Fn(&Pixels, usize, usize) -> bool) -> usize {
        let (cw, ch) = self.cell;
        (0..ch)
            .flat_map(|y| (cols.start * cw..cols.end * cw).map(move |x| (x, y)))
            .filter(|&(x, y)| f(self, x, y))
            .count()
    }

    fn differ(&self, other: &Pixels) -> usize {
        self.data.chunks(4).zip(other.data.chunks(4)).filter(|(a, b)| a != b).count()
    }
}

fn metal() -> bool {
    if MTLCreateSystemDefaultDevice().is_some() {
        return true;
    }
    assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
    eprintln!("skipping: no Metal device");
    false
}

fn renderer() -> Renderer {
    Renderer::new(Fonts::new("Menlo", 24.0))
}

/// White on black, so ink and colour are easy to tell apart.
fn theme() -> Theme {
    Theme::new([0; 16], 0xffffff, 0x000000, 0xffffff, 0x444444)
}

/// `text` on one row of a fresh terminal, cursor hidden, with ambiguous
/// characters `ambiguous` cells wide: drawn from a snapshot, checked
/// against the live terminal's frame, and the snapshot frame's glyphs.
fn render(renderer: &mut Renderer, text: &str, ambiguous: u8) -> (Pixels, Vec<GlyphInstance>) {
    let theme = theme();
    let mut t = Term::new(COLS, 1, 0);
    t.state.set_ambiguous_width(ambiguous);
    t.advance(format!("\x1b[?25l{text}").as_bytes());
    let input = FrameInput {
        theme: &theme,
        metrics: renderer.metrics(),
        padding: (0, 0),
        cursor_on: true,
        focused: true,
        preedit: None,
    };
    renderer.prepare(&mut t.state, &input);
    let glyphs = renderer.pane.frame.glyphs.clone();
    let px = draw(renderer, &t);
    renderer.build_live(&t.state, &input);
    let live = draw(renderer, &t);
    assert_eq!(px.differ(&live), 0, "{text:?}: snapshot and live frames differ");
    (px, glyphs)
}

fn draw(renderer: &mut Renderer, t: &Term) -> Pixels {
    let m = renderer.metrics();
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let (width, height) = (t.state.cols() * cw, t.state.lines() * ch);
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::BGRA8Unorm, width, height, false)
    };
    desc.setUsage(MTLTextureUsage::RenderTarget);
    desc.setStorageMode(MTLStorageMode::Shared);
    let target = renderer.device().newTextureWithDescriptor(&desc).expect("offscreen texture");
    let cmd = renderer.encode(&target);
    cmd.commit();
    cmd.waitUntilCompleted();
    let mut data = vec![0u8; width * height * 4];
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize { width, height, depth: 1 },
    };
    unsafe {
        target.getBytes_bytesPerRow_fromRegion_mipmapLevel(NonNull::new(data.as_mut_ptr() as *mut c_void).unwrap(), width * 4, region, 0);
    }
    Pixels {
        data,
        width,
        cell: (cw, ch),
    }
}

/// An emoji sequence is one colour glyph across its two cells, and not
/// what its first character alone draws. (Not every emoji is colourful:
/// Apple's families are grey silhouettes.)
fn one_emoji(sequence: &str, first: &str) {
    let mut r = renderer();
    let (px, glyphs) = render(&mut r, sequence, 1);
    let (alone, _) = render(&mut r, first, 1);
    assert_eq!(glyphs.len(), 1, "{sequence:?}: {glyphs:?}");
    assert_eq!(glyphs[0].is_color & GlyphInstance::COLOR, 1, "{sequence:?} isn't a colour glyph");
    let cw = px.cell.0;
    for col in 0..2 {
        let n = px.count(col..col + 1, Pixels::inked);
        assert!(n > cw * 4, "{sequence:?}: cell {col} has {n} inked pixels");
    }
    assert_eq!(px.count(2..COLS, Pixels::inked), 0, "{sequence:?} reaches past its cells");
    let differ = px.differ(&alone);
    assert!(differ > cw * 4, "{sequence:?} draws like {first:?} ({differ} pixels differ)");
}

#[test]
fn family_emoji_draws_whole() {
    if metal() {
        one_emoji("👨\u{200D}👩\u{200D}👧", "👨");
    }
}

#[test]
fn flag_draws_whole() {
    if metal() {
        one_emoji("🇬🇧", "🇬");
        let (px, _) = render(&mut renderer(), "🇬🇧", 1);
        assert!(px.count(0..2, Pixels::coloured) > 50, "the flag isn't in colour");
    }
}

#[test]
fn skin_tone_draws_whole() {
    if metal() {
        one_emoji("👍\u{1F3FD}", "👍");
    }
}

#[test]
fn keycap_draws_whole() {
    if metal() {
        // `1` alone is Menlo's white digit: no colour at all.
        one_emoji("1\u{FE0F}\u{20E3}", "1");
        let (px, _) = render(&mut renderer(), "1\u{FE0F}\u{20E3}", 1);
        assert!(px.count(0..2, Pixels::coloured) > 50, "the keycap isn't in colour");
    }
}

#[test]
fn combining_acute_draws_like_the_precomposed_letter() {
    if !metal() {
        return;
    }
    let mut r = renderer();
    let (combined, _) = render(&mut r, "e\u{301}", 1);
    let (precomposed, _) = render(&mut r, "\u{E9}", 1);
    let (plain, _) = render(&mut r, "e", 1);
    // Menlo shapes e + U+0301 into its own é.
    assert_eq!(combined.differ(&precomposed), 0, "e + U+0301 isn't drawn as é");
    assert!(combined.differ(&plain) > 10, "the accent is missing");
    // A mark Menlo has no precomposed letter for still lands on its base
    // cell: x + U+0301, two glyphs, both in cell 0.
    let (x_acute, glyphs) = render(&mut r, "x\u{301}", 1);
    let (x, _) = render(&mut r, "x", 1);
    assert_eq!(glyphs.len(), 2, "{glyphs:?}");
    assert!(x_acute.differ(&x) > 10, "the accent on x is missing");
    let cw = x.cell.0;
    assert!(
        x_acute.ink_columns().iter().all(|&c| c < cw + 2),
        "x + U+0301 reaches into the next cell"
    );
}

/// The middle of the columns with ink.
fn ink_centre(px: &Pixels) -> f64 {
    let cols = px.ink_columns();
    assert!(!cols.is_empty(), "nothing drawn");
    (cols[0] + cols[cols.len() - 1] + 1) as f64 / 2.0
}

#[test]
fn ambiguous_characters_at_two_cells_are_centred_in_them() {
    if !metal() {
        return;
    }
    let mut r = renderer();
    let cw = r.metrics().cell_width as f64;
    // ① comes from a fallback font whose glyph is wider than Menlo's
    // cell: it moves by half a cell like the others, but isn't centred in
    // one cell to begin with.
    for text in ["①", "§", "±", "§\u{301}"] {
        let (narrow, _) = render(&mut r, &format!("{text}x"), 1);
        let (wide, _) = render(&mut r, &format!("{text}x"), 2);
        // Only the character itself: the `x` after it moved a cell right.
        let (narrow_alone, _) = render(&mut r, text, 1);
        let (wide_alone, _) = render(&mut r, text, 2);
        assert_eq!(narrow.count(2..3, Pixels::inked), 0, "{text:?} at 1: x is in cell 1");
        assert!(wide.count(2..3, Pixels::inked) > 0, "{text:?} at 2: x isn't in cell 2");
        let (a, b) = (ink_centre(&narrow_alone), ink_centre(&wide_alone));
        assert!(
            (b - a - cw / 2.0).abs() <= 1.0,
            "{text:?} moved from {a} to {b}, not by half a cell ({cw})"
        );
        if text != "①" {
            assert!((a - cw / 2.0).abs() <= 2.0, "{text:?} at 1 is centred at {a}, cell {cw}");
            assert!(
                (b - cw).abs() <= 2.0,
                "{text:?} at 2 is centred at {b}, not in its two cells ({cw} each)"
            );
        }
    }
}

#[test]
fn code_drawn_ambiguous_characters_at_two_cells_span_both() {
    if !metal() {
        return;
    }
    let mut r = renderer();
    let cw = r.metrics().cell_width as usize;
    let (px, glyphs) = render(&mut r, "─", 2);
    assert_eq!(glyphs.len(), 1, "{glyphs:?}");
    assert_eq!(
        px.ink_columns(),
        (0..2 * cw).collect::<Vec<_>>(),
        "the line doesn't cover both cells"
    );
    let (px, _) = render(&mut r, "─", 1);
    assert_eq!(px.ink_columns(), (0..cw).collect::<Vec<_>>());
}

#[test]
fn patterned_underlines_draw_through_their_sprites() {
    if !metal() {
        return;
    }
    let mut r = renderer();
    let m = r.metrics();
    let (cw, baseline) = (m.cell_width as usize, m.baseline as usize);
    for (sgr, name) in [("4:3", "curly"), ("4:4", "dotted"), ("4:5", "dashed")] {
        let (px, glyphs) = render(&mut r, &format!("\x1b[{sgr}m   "), 1);
        assert_eq!(glyphs.len(), 3, "{name}: {glyphs:?}");
        let h = px.data.len() / 4 / px.width;
        for col in 0..3 {
            let ink: Vec<(usize, usize)> = (0..h)
                .flat_map(|y| (col * cw..(col + 1) * cw).map(move |x| (x, y)))
                .filter(|&(x, y)| px.inked(x, y))
                .collect();
            assert!(!ink.is_empty(), "{name}: no underline in cell {col}");
            assert!(
                ink.iter().all(|&(_, y)| y >= baseline),
                "{name}: ink above the baseline in cell {col}"
            );
        }
        assert_eq!(px.count(3..COLS, Pixels::inked), 0, "{name}: underline past the cells");
    }
}
