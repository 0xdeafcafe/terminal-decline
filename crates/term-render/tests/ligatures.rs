//! Ligatures drawn in pieces, and the shaping cache (ligatures design
//! §4, §6), with JetBrains Mono, checked in to term-font and registered
//! for this process.
//!
//! The frame tests use a glyph source with the real fonts (to shape) and
//! the real glyph sizes, placed side by side in a pretend texture: an
//! instance's `u` then says which glyph it is part of, and where in it.
//! The pixel test draws through the GPU and compares with the whole
//! glyphs drawn on the CPU at their grid positions.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Once;

use objc2_metal::{
    MTLCommandBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use term_core::{Point, Selection, Term, Terminal};
use term_font::{Features, FontConfig, Fonts, GlyphKey, Metrics, RunText, ShapedGlyph, Style, register_font_file};
use term_render::{Frame, FrameInput, GlyphInstance, GlyphSlot, GlyphSource, Renderer, Theme};

fn jetbrains() -> Fonts {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../term-font/tests/fonts/JetBrainsMono-Regular.ttf");
        assert!(register_font_file(Path::new(path)), "registering {path}");
    });
    let mut f = Fonts::with_config(&FontConfig::new("JetBrains Mono", 13.0), 2.0);
    assert_eq!(f.family(), "JetBrains Mono");
    f.allow_runs(true);
    f
}

/// The real fonts and glyph sizes; each glyph gets its own stretch of a
/// pretend texture, one pixel apart, in the order first asked for.
struct Source {
    fonts: Fonts,
    slots: HashMap<GlyphKey, Option<GlyphSlot>>,
    next: u16,
    key: u64,
}

impl Source {
    fn new(fonts: Fonts) -> Source {
        Source {
            fonts,
            slots: HashMap::new(),
            next: 0,
            key: 1,
        }
    }

    /// The glyph an instance is part of, by its `u`.
    fn owner(&self, g: &GlyphInstance) -> Option<(GlyphKey, GlyphSlot)> {
        self.slots
            .iter()
            .filter_map(|(k, s)| s.map(|s| (*k, s)))
            .find(|(_, s)| (s.u..s.u + s.w).contains(&g.u))
    }
}

impl GlyphSource for Source {
    fn glyph(&mut self, cp: u32, style: Style) -> Option<GlyphSlot> {
        let key = self.fonts.glyph_key(char::from_u32(cp)?, style)?;
        self.glyph_key(key)
    }

    fn glyph_key(&mut self, key: GlyphKey) -> Option<GlyphSlot> {
        if let Some(s) = self.slots.get(&key) {
            return *s;
        }
        let slot = self.fonts.rasterize_key(key).map(|r| {
            let s = GlyphSlot {
                u: self.next,
                v: 0,
                w: r.width as u16,
                h: r.height as u16,
                left: r.left as i16,
                top: r.top as i16,
                color: r.color,
            };
            self.next += r.width as u16 + 1;
            s
        });
        self.slots.insert(key, slot);
        slot
    }

    fn fonts(&self) -> Option<&Fonts> {
        Some(&self.fonts)
    }

    fn fonts_key(&self) -> u64 {
        self.key
    }

    /// New fonts are new glyph positions too, as in the renderer.
    fn generation(&self) -> u64 {
        self.key
    }
}

const PAD: (u32, u32) = (5, 3);

fn input<'a>(theme: &'a Theme, metrics: Metrics, preedit: Option<&'a str>) -> FrameInput<'a> {
    FrameInput {
        theme,
        metrics,
        padding: PAD,
        cursor_on: true,
        focused: true,
        preedit,
    }
}

/// Build `t` both ways, check the reference path gives the same glyphs,
/// and return the frame.
fn build(frame: &mut Frame, t: &mut Terminal, src: &mut Source, theme: &Theme, preedit: Option<&str>) {
    let input = input(theme, src.fonts.metrics, preedit);
    frame.build(t, &input, src);
    let mut live = Frame::default();
    live.build_live(t, &input, src);
    assert_eq!(frame.glyphs, live.glyphs, "snapshot and live glyphs differ\n{}", t.dump());
}

/// The glyphs of `text`'s first `cols` cells shaped as one run, the way
/// the cells are drawn: the per-character glyph, or the shaped glyphs
/// where they differ.
fn shaped(fonts: &Fonts, text: &str) -> Vec<ShapedGlyph> {
    let mut run = RunText::new();
    for ch in text.chars() {
        if ch == ' ' {
            run.push("", 1);
        } else {
            run.push_char(ch, 1);
        }
    }
    let mut out = Vec::new();
    fonts.shape(&run, Style::Regular, Features { ligatures: true }, &mut out);
    out
}

/// Each cell its own colour: palette entry `16 + col`.
fn rainbow(text: &str) -> String {
    let mut s = String::from("\x1b[?25l");
    for (i, ch) in text.chars().enumerate() {
        s.push_str(&format!("\x1b[38;5;{}m{ch}", 16 + i));
    }
    s
}

fn term(cols: usize, lines: usize, bytes: &str) -> Term {
    let mut t = Term::new(cols, lines, 100, term_core::ImagePool::process());
    t.advance(bytes.as_bytes());
    t
}

const ARROWS: &str = "a->b => c===d !== e<=>f ::";

#[test]
fn pieces_join_into_the_glyph_each_in_its_cells_colour() {
    let theme = Theme::default();
    let mut src = Source::new(jetbrains());
    let cw = src.fonts.metrics.cell_width as i32;
    let mut t = term(40, 2, &rainbow(ARROWS));
    let mut frame = Frame::default();
    build(&mut frame, &mut t.state, &mut src, &theme, None);

    let glyphs = shaped(&src.fonts, ARROWS);
    let split: Vec<&ShapedGlyph> = glyphs.iter().filter(|g| g.split).collect();
    assert!(split.len() >= 6, "JetBrains Mono should ligate {ARROWS:?}: {glyphs:?}");
    let mut multi = 0;
    for g in &split {
        // Spacers have no ink.
        let Some(slot) = src.glyph_key(g.key) else { continue };
        let x0 = PAD.0 as i32 + g.cell as i32 * cw + g.dx as i32 + slot.left as i32;
        let x1 = x0 + slot.w as i32;
        let mut parts: Vec<GlyphInstance> = frame
            .glyphs
            .iter()
            .copied()
            .filter(|i| (slot.u..slot.u + slot.w).contains(&i.u))
            .collect();
        // Every copy of this glyph in the row: keep the ones in this one's span.
        parts.retain(|i| (x0..x1).contains(&(i.x as i32)));
        parts.sort_by_key(|i| i.x);
        assert!(!parts.is_empty(), "no pieces for {g:?}");
        // The leftmost piece starts where the glyph does, the rightmost
        // ends where it does, and they join with nothing between.
        assert_eq!(parts[0].x as i32, x0, "{g:?}");
        assert_eq!(parts.last().map(|p| p.x as i32 + p.w as i32), Some(x1), "{g:?}");
        for w in parts.windows(2) {
            assert_eq!(w[0].x + w[0].w as i16, w[1].x, "{g:?}: pieces don't join");
        }
        for p in &parts {
            assert_eq!(p.u as i32 - slot.u as i32, p.x as i32 - x0, "{g:?}: texture not where the piece is");
            assert_eq!((p.v, p.h), (slot.v, slot.h));
            // Inside one cell, and in that cell's colour.
            let col = (p.x as i32 + p.w as i32 / 2 - PAD.0 as i32).div_euclid(cw);
            let (l, r) = (PAD.0 as i32 + col * cw, PAD.0 as i32 + (col + 1) * cw);
            let open = p.x as i32 == x0 || p.x as i32 + p.w as i32 == x1;
            assert!(
                open || (p.x as i32 >= l && p.x as i32 + p.w as i32 <= r),
                "{p:?} crosses a cell edge"
            );
            assert_eq!(p.color, theme.palette[16 + col as usize], "{g:?}: piece in column {col}");
        }
        if parts.len() > 1 {
            multi += 1;
        }
    }
    assert!(multi >= 2, "some ligature ink reaches past its cell");
    // Cells under a ligature don't also draw their own character.
    let own: Vec<GlyphKey> = "-=>!".chars().filter_map(|c| src.fonts.glyph_key(c, Style::Regular)).collect();
    for g in &frame.glyphs {
        if let Some((key, _)) = src.owner(g) {
            assert!(!own.contains(&key) || !split.iter().any(|s| s.key == key), "{g:?}");
        }
    }
    let mut lone = [0usize; 4];
    for g in &frame.glyphs {
        if let Some((key, _)) = src.owner(g)
            && let Some(i) = own.iter().position(|k| *k == key)
        {
            lone[i] += 1;
        }
    }
    assert_eq!(lone, [0; 4], "-, =, > and ! all ligate in {ARROWS:?}");
}

#[test]
fn hidden_cells_drop_their_pieces() {
    let theme = Theme::default();
    let mut src = Source::new(jetbrains());
    let cw = src.fonts.metrics.cell_width as i32;
    let mut t = term(20, 2, "\x1b[?25la-\x1b[8m>\x1b[0mb =\x1b[8m=\x1b[0m=");
    let mut frame = Frame::default();
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    for g in &frame.glyphs {
        let col = (g.x as i32 + g.w as i32 / 2 - PAD.0 as i32).div_euclid(cw);
        assert!(col != 2 && col != 6, "a piece in hidden column {col}: {g:?}");
    }
    let shown: Vec<i32> = frame
        .glyphs
        .iter()
        .map(|g| (g.x as i32 + g.w as i32 / 2 - PAD.0 as i32).div_euclid(cw))
        .collect();
    assert!(shown.contains(&1) && shown.contains(&5) && shown.contains(&7), "{shown:?}");
}

/// The glyph instances of `frame`, and their colours, in cell `col` of
/// row `line`.
fn in_cell(frame: &Frame, cw: i32, ch: i32, line: i32, col: i32) -> Vec<GlyphInstance> {
    frame
        .glyphs
        .iter()
        .copied()
        .filter(|g| {
            let c = (g.x as i32 + g.w as i32 / 2 - PAD.0 as i32).div_euclid(cw);
            let l = (g.y as i32 + g.h as i32 / 2 - PAD.1 as i32).div_euclid(ch);
            c == col && l == line
        })
        .collect()
}

#[test]
fn block_cursor_recolours_only_its_cells_pieces() {
    let theme = Theme::default();
    let mut src = Source::new(jetbrains());
    let (cw, ch) = (src.fonts.metrics.cell_width as i32, src.fonts.metrics.cell_height as i32);
    let text = "x => y === z";
    let mut t = term(20, 2, &format!("\x1b[2 q{}", &rainbow(text)[6..]));
    let mut frame = Frame::default();
    for col in 0..text.len() {
        t.advance(format!("\x1b[1;{}H", col + 1).as_bytes());
        build(&mut frame, &mut t.state, &mut src, &theme, None);
        for c in 0..text.len() as i32 {
            for g in in_cell(&frame, cw, ch, 0, c) {
                let want = if c == col as i32 {
                    theme.cursor_text
                } else {
                    theme.palette[16 + c as usize]
                };
                assert_eq!(g.color, want, "cursor at {col}, piece in {c}: {g:?}");
            }
        }
        // `=` and `>` cells have pieces of their ligature to recolour.
        if [2, 3, 7, 8, 9].contains(&col) {
            assert!(
                !in_cell(&frame, cw, ch, 0, col as i32).is_empty(),
                "cursor at {col}: nothing to invert"
            );
        }
    }
}

#[test]
fn selection_and_input_method_text_over_half_a_ligature() {
    let theme = Theme::default();
    let mut src = Source::new(jetbrains());
    let (cw, ch) = (src.fonts.metrics.cell_width as i32, src.fonts.metrics.cell_height as i32);
    let mut t = term(20, 2, "\x1b[?25lx === y\r\n");
    let mut frame = Frame::default();
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    let plain = frame.glyphs.clone();
    // A selection is a background: the glyphs don't change.
    let line = t.state.point_at(0, 0).line;
    t.state
        .set_selection(Some(Selection::new(Point { line, col: 0 }, Point { line, col: 3 })));
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    assert_eq!(frame.glyphs, plain);
    t.state.set_selection(None);
    // Input-method text at column 3 covers the last two `=`: their pieces
    // go, the first `=`'s stay.
    t.advance(b"\x1b[?25h\x1b[1;4H");
    build(&mut frame, &mut t.state, &mut src, &theme, Some("あ"));
    assert!(!in_cell(&frame, cw, ch, 0, 2).is_empty(), "the first = keeps its piece");
    let covered: Vec<GlyphInstance> = in_cell(&frame, cw, ch, 0, 3)
        .into_iter()
        .chain(in_cell(&frame, cw, ch, 0, 4))
        .collect();
    assert!(
        covered.iter().all(|g| src
            .owner(g)
            .is_none_or(|(k, _)| Some(k) == src.fonts.glyph_key('あ', Style::Regular))),
        "{covered:?}"
    );
}

#[test]
fn rows_are_shaped_once_and_only_while_on_screen() {
    let theme = Theme::default();
    let mut src = Source::new(jetbrains());
    let mut t = term(30, 4, "\x1b[?25la -> b\r\nc => d\r\nplain\r\n!=");
    let mut frame = Frame::default();
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    let (kept, rows, _) = frame.shaping();
    assert_eq!((kept, rows), (4, 4));
    // Nothing changed: the row cache has every row, nothing is shaped.
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    assert_eq!(frame.shaping().1, 4);
    // Every row built again (the theme changed): shaped glyphs are kept.
    let mut other = theme.clone();
    other.foreground ^= 0x10;
    build(&mut frame, &mut t.state, &mut src, &other, None);
    assert_eq!(frame.shaping().1, 4);
    // One row changes: it alone is shaped.
    t.advance(b"\x1b[3;1Hx <= y");
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    assert_eq!(frame.shaping().1, 5);
    // A new line: rows that moved aren't shaped again.
    t.advance(b"\x1b[4;1H\r\nz |> w");
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    assert_eq!(frame.shaping().1, 6);
    assert_eq!(frame.shaping().0, 4, "only visible rows are kept");
    // Another width: shaped again.
    t.state.resize(25, 4);
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    assert_eq!(frame.shaping().1, 10);
    // Other fonts: shaped again.
    src.key += 1;
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    assert_eq!(frame.shaping().1, 14);
    // Another terminal in the same frame: shaped again.
    let mut u = term(25, 4, "\x1b[?25la -> b");
    build(&mut frame, &mut u.state, &mut src, &theme, None);
    assert_eq!(frame.shaping().1, 18);
    // A second pane keeps its own.
    let mut pane = Frame::default();
    build(&mut pane, &mut t.state, &mut src, &theme, None);
    assert_eq!(pane.shaping().1, 4);
    build(&mut frame, &mut u.state, &mut src, &theme, None);
    assert_eq!(frame.shaping().1, 18);

    // After 10k lines, only the visible rows are kept.
    let mut s = String::new();
    for i in 0..10_000 {
        s.push_str(&format!("line {i} -> {} != {}\r\n", i * 2, i + 1));
    }
    u.advance(s.as_bytes());
    build(&mut frame, &mut u.state, &mut src, &theme, None);
    assert!(frame.shaping().0 <= 4, "{} rows kept", frame.shaping().0);
}

#[test]
fn menlo_shapes_nothing() {
    let theme = Theme::default();
    let mut f = Fonts::new("Menlo", 26.0);
    f.allow_runs(true);
    let mut src = Source::new(f);
    let mut t = term(30, 2, "a -> b != c === d");
    let mut frame = Frame::default();
    build(&mut frame, &mut t.state, &mut src, &theme, None);
    assert_eq!(frame.shaping(), (0, 0, 0));
}

// ---- pixels ---------------------------------------------------------------

/// `-> => !==` in JetBrains Mono through the GPU, against each glyph of
/// the run's CTLine drawn whole on the CPU at its grid position (cell ×
/// cell width + offset) with `rasterize_key`'s bitmaps: the pieces put
/// every pixel back where the whole glyph has it.
#[test]
fn ligatures_draw_like_whole_glyphs_on_the_grid() {
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let text = "-> => !== a<=b";
    let mut renderer = Renderer::new(jetbrains());
    let fonts = jetbrains();
    let m = fonts.metrics;
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let (cols, lines) = (16, 1);
    let (width, height) = (cols * cw, lines * ch);
    let theme = Theme::new([0; 16], 0xffffff, 0x000000, 0xffffff, 0x444444);
    let mut t = Term::new(cols, lines, 0, term_core::ImagePool::process());
    t.advance(format!("\x1b[?25l{text}").as_bytes());
    let input = FrameInput {
        theme: &theme,
        metrics: m,
        padding: (0, 0),
        cursor_on: true,
        focused: true,
        preedit: None,
    };
    renderer.prepare(&mut t.state, &input);
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

    // The reference: every glyph of the run, whole, white "over" black.
    let mut clear = vec![1.0f64; width * height];
    let glyphs = shaped(&fonts, text);
    assert!(glyphs.iter().filter(|g| g.split).count() >= 4, "{glyphs:?}");
    for g in &glyphs {
        let Some(r) = fonts.rasterize_key(g.key) else { continue };
        let x0 = g.cell as i32 * cw as i32 + g.dx as i32 + r.left;
        let y0 = m.baseline as i32 + g.dy as i32 - r.top;
        for y in 0..r.height as i32 {
            for x in 0..r.width as i32 {
                let (px, py) = (x0 + x, y0 + y);
                if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                    continue;
                }
                let a = r.data[(y * r.width as i32 + x) as usize] as f64 / 255.0;
                clear[py as usize * width + px as usize] *= 1.0 - a;
            }
        }
    }
    let mut diff = 0.0;
    let mut worst = 0u8;
    for (i, c) in clear.iter().enumerate() {
        let want = ((1.0 - c) * 255.0).round() as i32;
        let got = data[i * 4 + 1] as i32; // green
        let d = (got - want).unsigned_abs() as u8;
        worst = worst.max(d);
        diff += d as f64;
    }
    let mean = diff / clear.len() as f64;
    let inked = clear.iter().filter(|&&c| c < 0.5).count();
    assert!(inked > 200, "the reference has ink ({inked} pixels)");
    assert!(mean <= 1.0, "mean difference {mean:.3}/255 (worst {worst})");
    assert!(worst <= 3, "worst difference {worst}/255, mean {mean:.3}");
}
