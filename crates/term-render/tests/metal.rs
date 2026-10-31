//! Real GPU render: a 4×2-cell grid drawn into an offscreen BGRA8 texture
//! and read back. Checks the background colour's byte order, premultiplied
//! blending of a half-transparent background over a half-transparent window,
//! that a glyph draws something, and that frames built from snapshots look
//! like frames built from the live terminal. Without a Metal device the test is
//! skipped locally and fails on CI (the `CI` env var is set).

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_metal::{
    MTLCommandBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use term_core::Term;
use term_font::Fonts;
use term_render::{FrameInput, Renderer, Theme, rgb};

const COLS: usize = 4;
const LINES: usize = 2;

struct Pixels {
    data: Vec<u8>,
    width: usize,
    cell: (usize, usize),
}

impl Pixels {
    /// BGRA bytes at the centre of a cell.
    fn centre(&self, col: usize, line: usize) -> [u8; 4] {
        let (x, y) = (col * self.cell.0 + self.cell.0 / 2, line * self.cell.1 + self.cell.1 / 2);
        self.at(x, y)
    }

    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        self.data[i..i + 4].try_into().unwrap()
    }

    fn cell_pixels(&self, col: usize, line: usize) -> impl Iterator<Item = [u8; 4]> + '_ {
        let (cw, ch) = self.cell;
        (line * ch..(line + 1) * ch).flat_map(move |y| (col * cw..(col + 1) * cw).map(move |x| self.at(x, y)))
    }
}

fn close(got: [u8; 4], want: [f64; 4], what: &str) {
    let ok = got.iter().zip(want).all(|(&g, w)| (g as f64 - w * 255.0).abs() <= 2.0);
    let want: Vec<u8> = want.iter().map(|w| (w * 255.0).round() as u8).collect();
    assert!(ok, "{what}: got BGRA {got:?}, want about {want:?}");
}

#[test]
fn offscreen_render_reads_back() {
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let mut renderer = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = renderer.metrics();
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let (width, height) = (COLS * cw, LINES * ch);

    // Window background 0x204060 at half opacity; ANSI red at half alpha.
    let mut theme = Theme::new([0; 16], 0xffffff, 0x204060, 0xffffff, 0x444444);
    theme.background_opacity = 128;
    theme.palette[1] = rgb(0xff0000) & 0x00FF_FFFF | 0x80 << 24;
    // Cell (0,0): half-transparent red; (1,0): opaque #112233; (0,1): 'M'.
    let mut t = Term::new(COLS, LINES, 0, term_core::ImagePool::process());
    t.advance(b"\x1b[?25l\x1b[41m \x1b[48;2;17;34;51m \x1b[0m\r\nM");

    let input = FrameInput {
        theme: &theme,
        metrics: m,
        padding: (0, 0),
        cursor_on: true,
        focused: true,
        preedit: None,
    };
    renderer.prepare(&mut t.state, &input);
    assert_eq!(renderer.pane.frame.glyphs.len(), 1);

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
    let px = Pixels {
        data,
        width,
        cell: (cw, ch),
    };

    // Opaque truecolour background: exact, in BGRA order.
    assert_eq!(px.centre(1, 0), [0x33, 0x22, 0x11, 0xFF], "byte order");

    // Empty cells show the clear colour, premultiplied by its alpha.
    let a = 128.0 / 255.0;
    let clear = [0x60 as f64 / 255.0 * a, 0x40 as f64 / 255.0 * a, 0x20 as f64 / 255.0 * a, a];
    close(px.centre(3, 0), clear, "clear colour");
    close(px.centre(3, 1), clear, "clear colour");

    // Half-transparent red over it: premultiplied "source over".
    let sa = 128.0 / 255.0;
    let src = [0.0, 0.0, sa, sa];
    let over: Vec<f64> = (0..4).map(|i| src[i] + clear[i] * (1.0 - sa)).collect();
    close(px.centre(0, 0), over.try_into().unwrap(), "blended background");

    // The glyph cell has text-coloured pixels, not just background.
    let inked = px.cell_pixels(0, 1).filter(|p| p[3] > 230 && p[1] > 200).count();
    assert!(inked > 10, "glyph area looks like background ({inked} inked pixels)");
    let blank = px.cell_pixels(2, 1).filter(|p| p[3] > 200).count();
    assert_eq!(blank, 0, "an empty cell has no ink");
}

/// Draw `t` with `renderer` into an offscreen texture and read it back.
fn render(renderer: &mut Renderer, t: &mut Term, theme: &Theme) -> Pixels {
    let input = FrameInput {
        theme,
        metrics: renderer.metrics(),
        padding: (0, 0),
        cursor_on: true,
        focused: true,
        preedit: None,
    };
    renderer.prepare(&mut t.state, &input);
    draw(renderer, t)
}

/// Draw the frame `renderer` has prepared for `t` and read it back.
fn draw(renderer: &mut Renderer, t: &Term) -> Pixels {
    draw_padded(renderer, t, (0, 0))
}

/// [`draw`], for a frame prepared with `padding` around the grid.
fn draw_padded(renderer: &mut Renderer, t: &Term, padding: (usize, usize)) -> Pixels {
    let m = renderer.metrics();
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let (width, height) = (t.state.cols() * cw + 2 * padding.0, t.state.lines() * ch + 2 * padding.1);
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

/// Frames built from snapshots, with rows reused from the row cache and the
/// cursor, selection and input-method text drawn over them, look exactly
/// like frames built from the live terminal (the path they replaced), on
/// the GPU with real glyphs, frame after frame.
#[test]
fn snapshot_frames_draw_like_the_live_terminal() {
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let mut renderer = Renderer::new(Fonts::new("Menlo", 24.0));
    let theme = Theme::default();
    let mut t = Term::new(12, 4, 100, term_core::ImagePool::process());
    let steps: [(&str, Option<&str>, bool); 9] = [
        ("\x1b[41mred\x1b[0m \x1b[4;2munder\x1b[0m\r\n\x1b[1mbold\x1b[7m inv", None, true),
        // The cursor moves onto a dim underlined cell: rows unchanged.
        ("\x1b[1;6H", None, true),
        // A wide character under the cursor, then one line changes.
        ("\x1b[3;1H漢字\x1b[3;1H", None, true),
        ("\x1b[4;1Hspinner |\x1b[4;9H", Some("あい"), true),
        // Scrolled: every row moves, none changes.
        ("\r\n\r\n\r\nmore", None, false),
        ("\x1b[?1049h\x1b[44mfull screen\x1b[6 q", None, true),
        ("\x1b[?1049l\x1b[2 q", None, true),
        // Clusters and patterned underlines (one in its own colour) with
        // the block cursor on them, then input-method text over them.
        (
            "\x1b[2J\x1b[H\x1b[4:3;58:2::255:0:0me\u{301}x\x1b[4:4;59m\u{1F1EC}\u{1F1E7}\x1b[0m 1\u{FE0F}\u{20E3}\x1b[1;1H",
            None,
            true,
        ),
        ("\x1b[1;2H", Some("\u{3042}"), true),
    ];
    for (i, (bytes, preedit, focused)) in steps.into_iter().enumerate() {
        t.advance(bytes.as_bytes());
        if i == 2 {
            let line = t.state.point_at(1, 0).line;
            t.state.set_selection(Some(term_core::Selection::new(
                term_core::Point { line, col: 2 },
                term_core::Point { line: line + 1, col: 3 },
            )));
        }
        let input = FrameInput {
            theme: &theme,
            metrics: renderer.metrics(),
            padding: (0, 0),
            cursor_on: true,
            focused,
            preedit,
        };
        renderer.prepare(&mut t.state, &input);
        let from_snapshot = draw(&mut renderer, &t);
        renderer.build_live(&t.state, &input);
        let live = draw(&mut renderer, &t);
        let differ = from_snapshot
            .data
            .chunks(4)
            .zip(live.data.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(differ, 0, "step {i}: {differ} pixels differ\n{}", t.state.dump());
    }
}

/// A tmux-style border of code-drawn glyphs through the real glyph cache
/// and GPU: every border line runs unbroken across cell edges, at several
/// font sizes.
#[test]
fn border_has_no_gaps_between_cells() {
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let theme = Theme::new([0; 16], 0xffffff, 0x000000, 0xffffff, 0x444444);
    // White on black: full coverage is pure white.
    let ink = |p: [u8; 4]| p[0] == 255 && p[1] == 255 && p[2] == 255;
    for size in [15.0, 24.0, 26.0, 31.0, 37.0] {
        let mut renderer = Renderer::new(Fonts::new("Menlo", size));
        let mut t = Term::new(7, 4, 0, term_core::ImagePool::process());
        t.advance("\x1b[?25l╭──┬──╮\r\n│ab│cd│\r\n├──┼──┤\r\n╰──┴──╯".as_bytes());
        let px = render(&mut renderer, &mut t, &theme);
        let (cw, ch) = px.cell;
        // Horizontal lines on border lines 0, 2 and 3: the rows inked in the
        // middle of column 1 are inked from column 1 to the start of column 6.
        for line in [0, 2, 3] {
            let rows: Vec<usize> = (line * ch..(line + 1) * ch).filter(|&y| ink(px.at(cw + cw / 2, y))).collect();
            assert!(!rows.is_empty(), "size {size}: no line on row {line}");
            for y in rows {
                for x in cw..6 * cw {
                    assert!(ink(px.at(x, y)), "size {size}: gap at x {x}, y {y} (line {line}, cell {})", x / cw);
                }
            }
        }
        // Vertical lines in columns 0, 3 and 6, from line 1 into line 3.
        for col in [0, 3, 6] {
            let cols: Vec<usize> = (col * cw..(col + 1) * cw).filter(|&x| ink(px.at(x, ch + ch / 2))).collect();
            assert!(!cols.is_empty(), "size {size}: no line in column {col}");
            for x in cols {
                for y in ch..3 * ch + 2 {
                    assert!(ink(px.at(x, y)), "size {size}: gap at x {x}, y {y} (column {col}, line {})", y / ch);
                }
            }
        }
    }
}

/// Frames drawn one after another by one renderer, which copies only
/// the instances that changed into its buffers (used by turns), draw
/// like each frame drawn afresh: rows changing at their length and not,
/// the cursor moving over them, a selection and input-method text coming
/// and going, scrolling and clearing.
#[test]
fn kept_instances_draw_like_fresh_frames() {
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let mut kept = Renderer::new(Fonts::new("Menlo", 24.0));
    let mut fresh = Renderer::new(Fonts::new("Menlo", 24.0));
    let theme = Theme::default();
    let mut t = Term::new(16, 5, 100, term_core::ImagePool::process());
    let steps: [(&str, Option<&str>); 16] = [
        ("\x1b[41mred\x1b[0m \x1b[4munder\x1b[0m\r\nplain text\r\n\x1b[32mgreen", None),
        ("|", None),
        ("\x08/", None),
        ("\x08-", None),
        ("\x1b[1;1Hx", None),
        ("\x1b[2;3H\x1b[44mblue\x1b[0m", None),
        ("\x1b[5;1Hlonger and longer", None),
        ("\x1b[2;1H", Some("あ")),
        ("", None),
        ("\r\n\r\nscrolled", None),
        ("\x1b[3;5H", None),
        ("\x1b[2K", None),
        ("\x1b[4 q", None),
        ("\x1b[?1049h\x1b[44mfull screen", None),
        ("\x1b[?1049l\x1b[2 q", None),
        ("\x1b[2J\x1b[Hdone", None),
    ];
    for (i, (bytes, preedit)) in steps.into_iter().enumerate() {
        t.advance(bytes.as_bytes());
        if i == 4 || i == 6 {
            let line = t.state.point_at(0, 0).line;
            let sel =
                (i == 4).then(|| term_core::Selection::new(term_core::Point { line, col: 2 }, term_core::Point { line: line + 1, col: 3 }));
            t.state.set_selection(sel);
        }
        let input = FrameInput {
            theme: &theme,
            metrics: kept.metrics(),
            padding: (0, 0),
            cursor_on: true,
            focused: true,
            preedit,
        };
        kept.prepare(&mut t.state, &input);
        let a = draw(&mut kept, &t);
        fresh.pane.forget();
        fresh.prepare(&mut t.state, &input);
        let b = draw(&mut fresh, &t);
        let differ = a.data.chunks(4).zip(b.data.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "step {i}: {differ} pixels differ\n{}", t.state.dump());
    }
}

/// A theme fade half way: the background and a palette colour are drawn
/// at the midpoint of the old and new themes', and a truecolour cell stays
/// exactly as it is. Only the uniforms change: the frame built with the
/// new theme is drawn again as it is (`draw` doesn't build).
#[test]
fn fade_draws_the_midpoint_at_half_way() {
    use term_render::transition::{Blend, Kind, slots};
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let mut renderer = Renderer::new(Fonts::new("Menlo", 24.0));
    let mut old = Theme::new([0; 16], 0xffffff, 0x000000, 0xffffff, 0x444444);
    old.palette[1] = rgb(0x00ff00);
    let mut new = Theme::new([0; 16], 0x000000, 0xffffff, 0x000000, 0x444444);
    new.palette[1] = rgb(0xff0000);
    let mut t = Term::new(COLS, LINES, 0, term_core::ImagePool::process());
    // (0,0): ANSI red background; (1,0): truecolour #112233.
    t.advance(b"\x1b[?25l\x1b[41m \x1b[48;2;17;34;51m \x1b[0m");

    let end = render(&mut renderer, &mut t, &new);
    assert_eq!(end.centre(3, 1), [0xff, 0xff, 0xff, 0xff], "the new background, with no transition");
    let blend = |progress| Blend::single(Kind::Fade, progress, slots(&old, rgb(0x333333)), slots(&new, rgb(0x333333)));
    renderer.set_blend(Some(blend(0.5)));
    let half = draw(&mut renderer, &t);
    close(half.centre(3, 1), [0.5, 0.5, 0.5, 1.0], "background half way");
    // BGRA: green to red half way.
    close(half.centre(0, 0), [0.0, 0.5, 0.5, 1.0], "ANSI red half way from green");
    assert_eq!(half.centre(1, 0), [0x33, 0x22, 0x11, 0xFF], "truecolour stays");
    renderer.set_blend(Some(blend(0.0)));
    let start = draw(&mut renderer, &t);
    assert_eq!(start.centre(3, 1), [0, 0, 0, 0xff], "the old background at the start");
    assert_eq!(start.centre(0, 0), [0, 0xff, 0, 0xff], "the old red at the start");
    renderer.set_blend(None);
    assert_eq!(draw(&mut renderer, &t).centre(3, 1), [0xff, 0xff, 0xff, 0xff]);
}

/// Tokyo Night to Solarized Dark, two shipped themes (term-app/themes):
/// Tokyo Night's background is Solarized's ANSI 8 and its foreground is
/// its cursor, which a match by colour mixed up. At the start of a
/// transition every theme colour is the old theme's, as drawn without
/// one; at the end the new theme's; and truecolour equal to a new theme
/// colour (the background, the text colour) stays as it is throughout.
#[test]
fn transitions_start_at_the_old_theme_and_leave_literals_alone() {
    use term_render::transition::{Blend, Kind, slots};
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let (tokyo, solarized) = (tokyo_night(), solarized_dark());
    let mut renderer = Renderer::new(Fonts::new("Menlo", 24.0));
    let mut t = Term::new(COLS, LINES, 0, term_core::ImagePool::process());
    // Row 0: the default background, ANSI 8's, truecolour #002b36
    // (Solarized's background), inverse (the text colour as background).
    // Row 1: default text, truecolour #839496 text (Solarized's text
    // colour), then the block cursor.
    t.advance(b" \x1b[100m \x1b[48;2;0;43;54m \x1b[0;7m \x1b[0m\r\na\x1b[38;2;131;148;150mb\x1b[0m");
    let divider = rgb(0x333333);
    for (old, new) in [(&tokyo, &solarized), (&solarized, &tokyo)] {
        let before = render(&mut renderer, &mut t, old);
        let after = render(&mut renderer, &mut t, new);
        let blend = |progress| Blend::single(Kind::Fade, progress, slots(old, divider), slots(new, divider));
        let (cw, ch) = before.cell;
        let cell_of = |x: usize, y: usize| (x / cw, y / ch);
        let text = [0x96, 0x94, 0x83, 0xff]; // #839496 as BGRA
        for (progress, want) in [(0.0, &before), (1.0, &after)] {
            renderer.set_blend(Some(blend(progress)));
            let got = draw(&mut renderer, &t);
            let height = got.data.len() / 4 / got.width;
            for y in 0..height {
                for x in 0..got.width {
                    let (cell, ink) = (cell_of(x, y), after.at(x, y) == text);
                    let w = match cell {
                        // The truecolour background.
                        (2, 0) => after.at(x, y),
                        // The truecolour text: where it is all ink (its
                        // edges show the background through).
                        (1, 1) if ink => after.at(x, y),
                        (1, 1) => continue,
                        _ => want.at(x, y),
                    };
                    let g = got.at(x, y);
                    assert!(
                        g.iter().zip(w).all(|(&a, b)| a.abs_diff(b) <= 1),
                        "at progress {progress}, pixel ({x}, {y}): got {g:?}, want {w:?}"
                    );
                }
            }
        }
        renderer.set_blend(None);
    }
    // The literals really are Solarized's colours.
    let px = render(&mut renderer, &mut t, &solarized);
    assert_eq!(px.centre(2, 0), px.centre(0, 0), "truecolour #002b36 is Solarized's background");
    assert!(
        px.cell_pixels(1, 1).filter(|&p| p == [0x96, 0x94, 0x83, 0xff]).count() > 10,
        "solid text pixels"
    );
}

/// Tokyo Night as shipped (term-app/themes/tokyo-night.toml).
fn tokyo_night() -> Theme {
    Theme::new(
        [
            0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, //
            0x414868, 0xff899d, 0x9fe044, 0xfaba4a, 0x8db0ff, 0xc7a9ff, 0xa4daff, 0xc0caf5,
        ],
        0xc0caf5,
        0x1a1b26,
        0xc0caf5,
        0x283457,
    )
}

/// Solarized Dark as shipped (term-app/themes/solarized-dark.toml).
fn solarized_dark() -> Theme {
    Theme::new(
        [
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, //
            0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
        ],
        0x839496,
        0x002b36,
        0x93a1a1,
        0x073642,
    )
}

/// A wave, glimmer or pulse cut short by another theme change: the frame
/// drawn just after the change is the frame drawn just before it, pixel
/// for pixel (the one playing goes on under the new one), on the GPU.
/// Half way through a wave from black to white the top left is white and
/// the bottom right still black; they stay so.
#[test]
fn an_interrupted_transition_carries_on_from_the_screen() {
    use std::time::{Duration, Instant};
    use term_render::transition::{Kind, Transition, slots};
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let black = Theme::new([0; 16], 0xffffff, 0x000000, 0xffffff, 0x444444);
    let white = Theme::new([0xffffff; 16], 0x000000, 0xffffff, 0x000000, 0x444444);
    let red = Theme::new([0x00ff00; 16], 0xffffff, 0xff0000, 0xffffff, 0x444444);
    let divider = rgb(0x333333);
    let (t0, ms) = (Instant::now(), Duration::from_millis);
    let mut renderer = Renderer::new(Fonts::new("Menlo", 24.0));
    let mut t = Term::new(COLS, LINES, 0, term_core::ImagePool::process());
    t.advance(b"\x1b[?25lab\x1b[41m c\x1b[0m\r\nd");
    for kind in [Kind::Wave, Kind::Glimmer, Kind::Pulse] {
        let first = Transition::start(None, slots(&black, divider), slots(&white, divider), kind, ms(300), t0).unwrap();
        let half = t0 + ms(150);
        render(&mut renderer, &mut t, &white);
        renderer.set_blend(Some(first.blend(half)));
        let before = draw(&mut renderer, &t);
        if kind == Kind::Wave {
            let (w, h) = (before.width, before.data.len() / 4 / before.width);
            assert_eq!(before.at(0, 0), [0xff, 0xff, 0xff, 0xff], "top left white");
            assert_eq!(before.at(w - 1, h - 1), [0, 0, 0, 0xff], "bottom right black");
        }
        // Red comes in: rows built with it, from where the first got to.
        let second = Transition::start(
            Some(&first),
            slots(&white, divider),
            slots(&red, divider),
            Kind::Fade,
            ms(300),
            half,
        )
        .unwrap();
        render(&mut renderer, &mut t, &red);
        renderer.set_blend(Some(second.blend(half)));
        let after = draw(&mut renderer, &t);
        let height = before.data.len() / 4 / before.width;
        for y in 0..height {
            for x in 0..before.width {
                let (a, b) = (before.at(x, y), after.at(x, y));
                assert!(
                    a.iter().zip(b).all(|(&p, q)| p.abs_diff(q) <= 1),
                    "{kind:?}: pixel ({x}, {y}) jumped from {a:?} to {b:?}"
                );
            }
        }
        // And it ends at red, as drawn without a transition.
        renderer.set_blend(Some(second.blend(half + ms(300))));
        let end = draw(&mut renderer, &t);
        renderer.set_blend(None);
        assert_eq!(end.data, draw(&mut renderer, &t).data, "{kind:?} ends at the new theme");
        assert_eq!(end.centre(3, 1), [0, 0, 0xff, 0xff], "{kind:?}: red background");
    }
}

/// A prompt whose command exited non-zero gets a thin bar in the
/// theme's red in the left padding beside its row, and only that one:
/// not the prompt of a command that succeeded, nor the cells.
#[test]
fn failed_commands_are_marked_in_the_padding() {
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let mut ansi = [0; 16];
    ansi[1] = 0xff0000;
    let theme = Theme::new(ansi, 0xffffff, 0x000000, 0xffffff, 0x444444);
    let mut renderer = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = renderer.metrics();
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let pad = (cw, 4);
    let mut t = Term::new(6, 5, 0, term_core::ImagePool::process());
    let session = |cmd: &str, status: u32| format!("\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07out\r\n\x1b]133;D;{status}\x07");
    t.advance(format!("\x1b[?25l{}{}\x1b]133;A\x07$ ", session("ok", 0), session("no", 1)).as_bytes());
    let input = FrameInput {
        theme: &theme,
        metrics: m,
        padding: (pad.0 as u32, pad.1 as u32),
        cursor_on: true,
        focused: true,
        preedit: None,
    };
    renderer.prepare(&mut t.state, &input);
    let px = draw_padded(&mut renderer, &t, pad);
    let red = |p: [u8; 4]| p[2] > 200 && p[1] < 40 && p[0] < 40;
    // Red pixels in the left padding, by view row.
    let reds = |line: usize| {
        (pad.1 + line * ch..pad.1 + (line + 1) * ch)
            .flat_map(|y| (0..pad.0).map(move |x| (x, y)))
            .filter(|&(x, y)| red(px.at(x, y)))
            .count()
    };
    // Rows: `$ ok`, out, `$ no`, out, `$ `.
    assert_eq!(reds(0), 0, "a command that succeeded");
    assert!(reds(2) >= ch, "the failed command's prompt: {} red pixels", reds(2));
    for line in [1, 3, 4] {
        assert_eq!(reds(line), 0, "row {line}");
    }
    // The bar is the row's full height and stays out of the cells.
    let x = (0..pad.0).find(|&x| red(px.at(x, pad.1 + 2 * ch + ch / 2))).expect("a red column");
    assert!((pad.1 + 2 * ch..pad.1 + 3 * ch).all(|y| red(px.at(x, y))));
    let in_cells = (pad.1..pad.1 + 5 * ch)
        .flat_map(|y| (pad.0..pad.0 + 6 * cw).map(move |x| (x, y)))
        .filter(|&(x, y)| red(px.at(x, y)))
        .count();
    assert_eq!(in_cells, 0);
}
