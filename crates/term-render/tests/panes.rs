//! Several panes in one frame, and glyph textures shared between windows,
//! on the GPU (splits design §3, §9): each pane lands in its own rectangle
//! and nothing of it outside; a pane not rebuilt draws the same; a glyph
//! texture that grows keeps every pane's pixels without a rebuild, and one
//! emptied at its cap makes them rebuild; scissors are clamped to the
//! target; two windows draw correctly while glyphs are uploaded between
//! their frames; closing a window gives its glyph space back; a pane
//! built from its snapshot once the terminal is unlocked draws as one
//! built with it. Without a
//! Metal device the tests are skipped locally and fail on CI (the `CI` env
//! var is set).

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2_metal::{
    MTLCommandBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use term_core::Term;
use term_font::Fonts;
use term_render::{FrameInput, PaneDraw, PaneRender, PxRect, RectInstance, Renderer, Texture, Theme, rgb};

fn have_gpu() -> bool {
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return false;
    }
    true
}

#[derive(Clone, PartialEq)]
struct Image {
    data: Vec<u8>,
    width: usize,
    height: usize,
}

impl Image {
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        self.data[i..i + 4].try_into().unwrap()
    }

    /// Pixels in `x0..x1` × `y0..y1` that differ from `other`'s at the
    /// same place moved by `(dx, dy)`.
    fn differ(&self, other: &Image, (x0, x1): (usize, usize), (y0, y1): (usize, usize), (dx, dy): (usize, usize)) -> usize {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| self.at(x, y) != other.at(x - dx, y - dy))
            .count()
    }

    /// Where `self` and `other` (the same size) differ, for failure
    /// messages: how many pixels, the first few with both values, and the
    /// glyph ink in each.
    fn diff(&self, other: &Image) -> String {
        let (xs, ys) = ((0, self.width), (0, self.height));
        let differ: Vec<(usize, usize)> = (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .filter(|&(x, y)| self.at(x, y) != other.at(x, y))
            .collect();
        let first: Vec<String> = differ
            .iter()
            .take(8)
            .map(|&(x, y)| format!("({x},{y}) {:?} vs {:?}", self.at(x, y), other.at(x, y)))
            .collect();
        format!(
            "{} of {}x{} pixels differ, first: {}; ink {} vs {}",
            differ.len(),
            self.width,
            self.height,
            first.join(", "),
            self.inked(xs, ys),
            other.inked(xs, ys)
        )
    }

    /// Pixels in `x0..x1` × `y0..y1` brighter than the dark backgrounds
    /// used here: glyph ink.
    fn inked(&self, (x0, x1): (usize, usize), (y0, y1): (usize, usize)) -> usize {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| self.at(x, y)[1] > 100)
            .count()
    }
}

fn target(renderer: &Renderer, width: usize, height: usize) -> Retained<Texture> {
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::BGRA8Unorm, width, height, false)
    };
    desc.setUsage(MTLTextureUsage::RenderTarget);
    desc.setStorageMode(MTLStorageMode::Shared);
    renderer.device().newTextureWithDescriptor(&desc).expect("offscreen texture")
}

fn read(target: &Texture) -> Image {
    let (width, height) = (target.width(), target.height());
    let mut data = vec![0u8; width * height * 4];
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize { width, height, depth: 1 },
    };
    unsafe {
        target.getBytes_bytesPerRow_fromRegion_mipmapLevel(NonNull::new(data.as_mut_ptr() as *mut c_void).unwrap(), width * 4, region, 0);
    }
    Image { data, width, height }
}

/// Draw `panes` and `dividers` into a fresh `width` × `height` target.
fn draw(renderer: &mut Renderer, (width, height): (usize, usize), clear: u32, panes: &[PaneDraw], dividers: &[RectInstance]) -> Image {
    let t = target(renderer, width, height);
    let cmd = renderer.encode_panes(&t, clear, panes, dividers);
    cmd.commit();
    cmd.waitUntilCompleted();
    read(&t)
}

/// `pane` alone at the top-left of a target its terminal's size.
fn alone(renderer: &mut Renderer, pane: &PaneRender, size: (usize, usize)) -> Image {
    let all = PxRect {
        x: 0,
        y: 0,
        w: size.0 as i32,
        h: size.1 as i32,
    };
    let clear = pane.frame.clear;
    draw(
        renderer,
        size,
        clear,
        &[PaneDraw {
            pane,
            origin: (0, 0),
            scissor: all,
            scroll: None,
        }],
        &[],
    )
}

fn input<'a>(renderer: &Renderer, theme: &'a Theme) -> FrameInput<'a> {
    FrameInput {
        theme,
        metrics: renderer.metrics(),
        padding: (0, 0),
        cursor_on: true,
        focused: true,
        preedit: None,
    }
}

fn theme() -> Theme {
    Theme::new([0; 16], 0xffffff, 0x101010, 0xffffff, 0x444444)
}

fn built(renderer: &mut Renderer, theme: &Theme, cols: usize, lines: usize, text: &str) -> (PaneRender, Term) {
    let mut t = Term::new(cols, lines, 0, term_core::ImagePool::process());
    t.advance(format!("\x1b[?25l{text}").as_bytes());
    let mut pane = PaneRender::new();
    let input = input(renderer, theme);
    renderer.build_pane(&mut pane, &mut t.state, &input);
    (pane, t)
}

/// Two panes side by side with a divider between them: each pane's pixels
/// are what it draws alone, moved to its rectangle; the divider is on top;
/// the pass's transparent clear shows nowhere.
#[test]
fn two_panes_land_in_their_rects() {
    if !have_gpu() {
        return;
    }
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r.metrics();
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let theme = theme();
    let (a, _ta) = built(&mut r, &theme, 4, 2, "left\r\n\x1b[41mAB\x1b[0m");
    let (b, _tb) = built(&mut r, &theme, 4, 2, "\x1b[1mWXYZ\r\n\x1b[42m12");
    let pane_size = (4 * cw, 2 * ch);
    let solo_a = alone(&mut r, &a, pane_size);
    let solo_b = alone(&mut r, &b, pane_size);

    let bx = 4 * cw + 1;
    let divider = RectInstance {
        x: 4 * cw as i16,
        y: 0,
        w: 1,
        h: 2 * ch as u16,
        color: rgb(0x808080),
        tag: 0,
    };
    let rect = |x: usize| PxRect {
        x: x as i32,
        y: 0,
        w: pane_size.0 as i32,
        h: pane_size.1 as i32,
    };
    let size = (8 * cw + 1, 2 * ch);
    let panes = [
        PaneDraw {
            pane: &a,
            origin: (0, 0),
            scissor: rect(0),
            scroll: None,
        },
        PaneDraw {
            pane: &b,
            origin: (bx as i32, 0),
            scissor: rect(bx),
            scroll: None,
        },
    ];
    let split = draw(&mut r, size, 0, &panes, &[divider]);
    assert_eq!(split.differ(&solo_a, (0, 4 * cw), (0, 2 * ch), (0, 0)), 0, "pane A");
    assert_eq!(split.differ(&solo_b, (bx, bx + 4 * cw), (0, 2 * ch), (bx, 0)), 0, "pane B");
    for y in 0..2 * ch {
        assert_eq!(split.at(4 * cw, y), [0x80, 0x80, 0x80, 0xFF], "divider at y {y}");
    }
    assert!(solo_a.inked((0, 4 * cw), (0, ch)) > 0 && solo_b.inked((0, 4 * cw), (0, ch)) > 0);

    // Drawn again without building: the same pixels.
    let again = draw(&mut r, size, 0, &panes, &[divider]);
    assert!(again == split, "a pane drawn again without a rebuild changed");
}

/// A pane's glyphs and backgrounds stop at its scissor: a pane drawn into
/// half its width leaves the other half to the pass's clear colour, while
/// drawn whole it inks there.
/// Smooth scrolling: a view scrolled back, with the row below it in its
/// snapshot, drawn `up` pixels higher inside its rows is the same frame
/// drawn whole, moved up by exactly `up` pixels; the padding below its
/// rows shows the background, not the row below.
#[test]
fn a_pane_scrolled_part_of_a_line_moves_by_the_pixels() {
    if !have_gpu() {
        return;
    }
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r.metrics();
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let theme = theme();
    let mut t = Term::new(4, 3, 100, term_core::ImagePool::process());
    t.advance(b"\x1b[?25l\x1b[41ma1\x1b[0m\r\n\x1b[42mb2\x1b[0m\r\nc3\r\n\x1b[44md4\x1b[0m\r\n\x1b[45me5\x1b[0m\r\nf6");
    t.state.set_view_overscan(true);
    t.state.scroll_view(2);
    let mut pane = PaneRender::new();
    let input = input(&r, &theme);
    r.build_pane(&mut pane, &mut t.state, &input);
    assert_eq!(pane.frame.last_snapshot().rows.len(), 4, "the view's rows and the one below");
    let w = 4 * cw;
    // All four rows, unmoved.
    let whole = draw(
        &mut r,
        (w, 4 * ch),
        pane.frame.clear,
        &[PaneDraw {
            pane: &pane,
            origin: (0, 0),
            scissor: PxRect {
                x: 0,
                y: 0,
                w: w as i32,
                h: 4 * ch as i32,
            },
            scroll: None,
        }],
        &[],
    );
    let pad = 5;
    let rows = PxRect {
        x: 0,
        y: 0,
        w: w as i32,
        h: 3 * ch as i32,
    };
    for up in [0, 1, ch / 3, ch / 2, ch - 1] {
        let moved = draw(
            &mut r,
            (w, 3 * ch + pad),
            pane.frame.clear,
            &[PaneDraw {
                pane: &pane,
                origin: (0, 0),
                scissor: PxRect {
                    h: (3 * ch + pad) as i32,
                    ..rows
                },
                scroll: Some(term_render::Scroll { up: up as i32, clip: rows }),
            }],
            &[],
        );
        let differ = (0..3 * ch)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| moved.at(x, y) != whole.at(x, y + up))
            .count();
        assert_eq!(differ, 0, "up {up}");
        let bg = moved.at(0, 3 * ch + pad - 1);
        assert!(
            (3 * ch..3 * ch + pad).all(|y| (0..w).all(|x| moved.at(x, y) == bg)),
            "up {up}: the padding is background only"
        );
        assert_ne!(whole.at(0, 3 * ch), bg, "the row below has its own colour");
    }
}

#[test]
fn a_pane_draws_nothing_outside_its_scissor() {
    if !have_gpu() {
        return;
    }
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r.metrics();
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let theme = theme();
    // Italic glyphs lean past their cell's right edge.
    let (a, _t) = built(&mut r, &theme, 4, 1, "MM\x1b[3mff");
    let size = (4 * cw, ch);
    let whole = alone(&mut r, &a, size);
    assert!(
        whole.inked((2 * cw, 4 * cw), (0, ch)) > 0,
        "the right half has ink when drawn whole"
    );

    let half = PxRect {
        x: 0,
        y: 0,
        w: 2 * cw as i32,
        h: ch as i32,
    };
    let cut = draw(
        &mut r,
        size,
        0,
        &[PaneDraw {
            pane: &a,
            origin: (0, 0),
            scissor: half,
            scroll: None,
        }],
        &[],
    );
    assert_eq!(cut.differ(&whole, (0, 2 * cw), (0, ch), (0, 0)), 0, "inside the scissor");
    for y in 0..ch {
        for x in 2 * cw..4 * cw {
            assert_eq!(cut.at(x, y), [0; 4], "outside the scissor at {x},{y}");
        }
    }

    // An italic glyph in a pane's last column, a neighbour to its right:
    // the neighbour shows none of it.
    let (left, _t) = built(&mut r, &theme, 2, 1, "\x1b[3mWf");
    let (right, _t) = built(&mut r, &theme, 2, 1, "");
    let lone = alone(&mut r, &left, (3 * cw, ch));
    let overhang = lone.inked((2 * cw, 3 * cw), (0, ch));
    let pane = |pane, x: usize| PaneDraw {
        pane,
        origin: (x as i32, 0),
        scissor: PxRect {
            x: x as i32,
            y: 0,
            w: 2 * cw as i32,
            h: ch as i32,
        },
        scroll: None,
    };
    let both = draw(&mut r, (4 * cw, ch), 0, &[pane(&left, 0), pane(&right, 2 * cw)], &[]);
    assert!(overhang > 0, "the italic glyph reaches past its pane");
    assert_eq!(
        both.inked((2 * cw, 4 * cw), (0, ch)),
        0,
        "neighbour inked (the glyph alone reaches {overhang} pixels past its pane)"
    );
}

/// Scissors reaching past the target (a pane laid out for a bigger
/// drawable during a live resize) are clamped; a pane wholly outside is
/// skipped.
#[test]
fn scissors_are_clamped_to_the_target() {
    let r = PxRect {
        x: -5,
        y: 10,
        w: 100,
        h: 100,
    };
    assert_eq!(r.clamped(50, 60), Some(PxRect { x: 0, y: 10, w: 50, h: 50 }));
    assert_eq!(PxRect { x: 50, y: 0, w: 10, h: 10 }.clamped(50, 60), None);
    assert_eq!(PxRect { x: 0, y: 0, w: -3, h: 10 }.clamped(50, 60), None);
    assert_eq!(
        PxRect {
            x: i32::MAX - 1,
            y: 0,
            w: i32::MAX,
            h: 1
        }
        .clamped(50, 60),
        None
    );
    if !have_gpu() {
        return;
    }
    let mut renderer = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = renderer.metrics();
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    let theme = theme();
    let (a, _t) = built(&mut renderer, &theme, 4, 2, "abcd\r\nefgh");
    let size = (4 * cw, 2 * ch);
    let solo = alone(&mut renderer, &a, size);
    // Laid out for a drawable twice the size; drawn into the real one.
    let big = PxRect {
        x: 0,
        y: 0,
        w: 8 * cw as i32,
        h: 4 * ch as i32,
    };
    let outside = PxRect {
        x: 4 * cw as i32,
        y: 0,
        w: 4 * cw as i32,
        h: 2 * ch as i32,
    };
    let clear = a.frame.clear;
    let got = draw(
        &mut renderer,
        size,
        clear,
        &[
            PaneDraw {
                pane: &a,
                origin: (0, 0),
                scissor: big,
                scroll: None,
            },
            PaneDraw {
                pane: &a,
                origin: (4 * cw as i32, 0),
                scissor: outside,
                scroll: None,
            },
        ],
        &[],
    );
    assert!(got == solo);
}

/// Growing a glyph texture copies it into the bigger one: a pane built
/// before draws the same pixels without being built again.
#[test]
fn a_texture_grow_keeps_pixels_without_a_rebuild() {
    if !have_gpu() {
        return;
    }
    // Small textures, so a second font's glyphs make them grow.
    term_render::set_glyph_texture_sizes((256, 4096), (64, 2048));
    let theme = theme();
    let mut r1 = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r1.metrics();
    let size = (6 * m.cell_width as usize, m.cell_height as usize);
    let (a, _t) = built(&mut r1, &theme, 6, 1, "abc\x1b[1mxyz");
    let before = alone(&mut r1, &a, size);
    let (mask, _) = r1.glyph_texture_sizes();

    // Another window with big fonts fills the texture.
    let mut r2 = Renderer::new(Fonts::new("Menlo", 64.0));
    let text: String = (0x21u8..0x7F).map(|c| c as char).collect();
    let (_b, _t) = built(&mut r2, &theme, 94, 1, &format!("\x1b[1m{text}\x1b[3m{text}"));
    assert!(r2.glyph_texture_sizes().0 > mask, "the coverage texture grew");

    assert!(r1.is_current(&a), "growing keeps slots");
    let after = alone(&mut r1, &a, size);
    let device = r1.device();
    assert!(
        after == before,
        "pixels changed after the texture grew ({}, unified memory {}): after vs before: {}",
        device.name(),
        device.hasUnifiedMemory(),
        after.diff(&before)
    );
}

/// Emptying a texture at its cap makes every pane built before stale: it
/// draws no glyphs until built again, and then draws as before.
#[test]
fn emptying_a_texture_at_its_cap_makes_panes_rebuild() {
    if !have_gpu() {
        return;
    }
    term_render::set_glyph_texture_sizes((256, 256), (64, 64));
    let theme = theme();
    let mut r1 = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r1.metrics();
    let size = (6 * m.cell_width as usize, m.cell_height as usize);
    let (mut a, mut ta) = built(&mut r1, &theme, 6, 1, "abc\x1b[48;2;0;0;128mxyz");
    let before = alone(&mut r1, &a, size);
    assert!(before.inked((0, size.0), (0, size.1)) > 0);

    let mut r2 = Renderer::new(Fonts::new("Menlo", 64.0));
    let text: String = (0x21u8..0x7F).map(|c| c as char).collect();
    let (_b, _t) = built(&mut r2, &theme, 94, 1, &text);
    assert_eq!(r2.glyph_texture_sizes().0, 256, "at the cap");

    assert!(!r1.is_current(&a), "emptied: built against glyphs since forgotten");
    let stale = alone(&mut r1, &a, size);
    assert_eq!(stale.inked((0, size.0), (0, size.1)), 0, "a stale pane draws no glyphs");
    assert_eq!(stale.at(4 * m.cell_width as usize, 1), [0x80, 0, 0, 0xFF], "but its backgrounds");

    let input = input(&r1, &theme);
    r1.build_pane(&mut a, &mut ta.state, &input);
    assert!(r1.is_current(&a));
    assert!(alone(&mut r1, &a, size) == before, "rebuilt, it draws as before");
}

/// After a panic while building, the glyphs are forgotten: every pane is
/// stale, draws its backgrounds only, and once built again draws as before.
#[test]
fn forgetting_glyphs_makes_every_pane_rebuild() {
    if !have_gpu() {
        return;
    }
    let theme = theme();
    let mut r1 = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r1.metrics();
    let size = (6 * m.cell_width as usize, m.cell_height as usize);
    let (mut a, mut ta) = built(&mut r1, &theme, 6, 1, "abc\x1b[48;2;0;0;128mxyz");
    let mut r2 = Renderer::new(Fonts::new("Menlo", 24.0));
    let (b, _tb) = built(&mut r2, &theme, 6, 1, "abc");
    let before = alone(&mut r1, &a, size);

    r1.forget_glyphs();
    assert!(!r1.is_current(&a) && !r2.is_current(&b), "every window's panes are stale");
    let stale = alone(&mut r1, &a, size);
    assert_eq!(stale.inked((0, size.0), (0, size.1)), 0, "a stale pane draws no glyphs");
    assert_eq!(stale.at(4 * m.cell_width as usize, 1), [0x80, 0, 0, 0xFF], "but its backgrounds");
    let input = input(&r1, &theme);
    r1.build_pane(&mut a, &mut ta.state, &input);
    assert!(r1.is_current(&a));
    assert!(alone(&mut r1, &a, size) == before, "rebuilt, it draws as before");
}

/// A pane snapshotted with its terminal locked is built after the lock
/// is let go, as `build_pane` builds it; when another window empties the
/// glyph textures in between, building from the snapshot refuses, and the
/// pane is built again with the terminal.
#[test]
fn panes_build_from_a_snapshot_unless_the_textures_were_emptied() {
    if !have_gpu() {
        return;
    }
    term_render::set_glyph_texture_sizes((256, 256), (64, 64));
    let theme = theme();
    let mut r1 = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r1.metrics();
    let size = (6 * m.cell_width as usize, m.cell_height as usize);
    let (mut a, mut ta) = built(&mut r1, &theme, 6, 1, "abc");
    let input = input(&r1, &theme);

    ta.advance(b"xyz");
    r1.snapshot_pane(&mut a, &mut ta.state, &input);
    assert!(r1.build_from_snapshot(&mut a, &input));
    assert!(r1.is_current(&a));
    let (mut b, _tb) = built(&mut r1, &theme, 6, 1, "abcxyz");
    assert!(alone(&mut r1, &a, size) == alone(&mut r1, &b, size), "as build_pane builds it");

    ta.advance(b"\rABC");
    r1.snapshot_pane(&mut a, &mut ta.state, &input);
    let mut r2 = Renderer::new(Fonts::new("Menlo", 64.0));
    let text: String = (0x21u8..0x7F).map(|c| c as char).collect();
    let (_c, _tc) = built(&mut r2, &theme, 94, 1, &text);
    assert!(!r1.build_from_snapshot(&mut a, &input), "emptied since the snapshot");
    r1.build_pane(&mut a, &mut ta.state, &input);
    assert!(r1.is_current(&a));
    let mut tb = Term::new(6, 1, 0, term_core::ImagePool::process());
    tb.advance(b"\x1b[?25lABCxyz");
    r1.build_pane(&mut b, &mut tb.state, &input);
    assert!(
        alone(&mut r1, &a, size) == alone(&mut r1, &b, size),
        "built again, it draws its new text"
    );
}

/// With a glyph budget (roadmap R.1) a build draws without the new glyphs
/// past it and says so; built again until it has them all (at least one
/// new glyph a build), it draws as a build without a budget.
#[test]
fn a_glyph_budget_leaves_new_glyphs_for_later_builds() {
    if !have_gpu() {
        return;
    }
    let theme = theme();
    let text = "αβγδεζ";
    // Fonts of their own each, so no glyph of `text` is in the textures.
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let mut whole = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r.metrics();
    let size = (6 * m.cell_width as usize, m.cell_height as usize);
    r.set_glyph_budget(Some(std::time::Duration::ZERO));
    let (mut pane, mut t) = built(&mut r, &theme, 6, 1, text);
    let input = input(&r, &theme);
    let mut builds = 1;
    while pane.glyphs_deferred() {
        assert!(builds <= text.chars().count(), "a build that made no progress");
        r.snapshot_pane(&mut pane, &mut t.state, &input);
        assert!(r.build_from_snapshot(&mut pane, &input));
        builds += 1;
    }
    assert!(builds > 1, "a zero budget leaves glyphs for later");
    let (all, _ta) = built(&mut whole, &theme, 6, 1, text);
    assert!(!all.glyphs_deferred(), "no budget, no glyph left");
    assert!(
        alone(&mut r, &pane, size) == alone(&mut whole, &all, size),
        "every glyph drawn in the end"
    );
}

/// With the glyph thread on (roadmap R.1) a build draws without glyphs
/// it hasn't seen and asks the thread for them; once the thread wakes the
/// app, the next build has them and draws as a build that found them
/// itself. ASCII and glyphs found before never wait.
#[test]
fn a_glyph_from_the_glyph_thread_appears_on_a_later_build() {
    if !have_gpu() {
        return;
    }
    let theme = theme();
    let text = "漢字😀αβ";
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let mut whole = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r.metrics();
    let size = (8 * m.cell_width as usize, m.cell_height as usize);
    let (woke, wakes) = std::sync::mpsc::channel::<()>();
    let woke = std::sync::Mutex::new(woke);
    r.set_glyph_waker(Some(std::sync::Arc::new(move || {
        let _ = woke.lock().unwrap().send(());
    })));
    let (ascii, _t) = built(&mut r, &theme, 8, 1, "abc");
    assert!(!ascii.glyphs_deferred(), "ASCII is found on this thread");
    let (mut pane, mut t) = built(&mut r, &theme, 8, 1, text);
    assert!(pane.glyphs_deferred(), "new glyphs go to the glyph thread");
    let (blank, _tb) = built(&mut whole, &theme, 8, 1, "");
    assert!(
        alone(&mut r, &pane, size) == alone(&mut whole, &blank, size),
        "drawn without them for now"
    );
    let input = input(&r, &theme);
    let mut builds = 1;
    while pane.glyphs_deferred() {
        wakes
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("the glyph thread wakes the app");
        r.snapshot_pane(&mut pane, &mut t.state, &input);
        assert!(r.build_from_snapshot(&mut pane, &input));
        builds += 1;
        assert!(builds < 20, "the glyphs never came");
    }
    assert!(!r.glyphs_pending());
    let (all, _ta) = built(&mut whole, &theme, 8, 1, text);
    assert!(
        alone(&mut r, &pane, size) == alone(&mut whole, &all, size),
        "every glyph drawn in the end"
    );
    // Another pane with the same glyphs has them at once.
    let (again, _t2) = built(&mut r, &theme, 8, 1, text);
    assert!(!again.glyphs_deferred(), "glyphs found before don't wait");
}

/// JetBrains Mono (checked in to term-font), whose runs are shaped.
fn jetbrains() -> Fonts {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../term-font/tests/fonts/JetBrainsMono-Regular.ttf");
        assert!(term_font::register_font_file(std::path::Path::new(path)), "registering {path}");
    });
    Fonts::with_config(&term_font::FontConfig::new("JetBrains Mono", 13.0), 2.0)
}

/// A waker that sends on a channel.
fn waker() -> (term_render::GlyphWaker, std::sync::mpsc::Receiver<()>) {
    let (woke, wakes) = std::sync::mpsc::channel::<()>();
    let woke = std::sync::Mutex::new(woke);
    let waker: term_render::GlyphWaker = std::sync::Arc::new(move || {
        let _ = woke.lock().unwrap().send(());
    });
    (waker, wakes)
}

/// Build `pane` again each time the glyph thread wakes, until it has
/// every glyph.
fn until_complete(r: &mut Renderer, pane: &mut PaneRender, t: &mut Term, theme: &Theme, wakes: &std::sync::mpsc::Receiver<()>) {
    let input = input(r, theme);
    let mut builds = 1;
    while pane.glyphs_deferred() {
        wakes
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("the glyph thread wakes the app");
        r.snapshot_pane(pane, &mut t.state, &input);
        if !r.build_from_snapshot(pane, &input) {
            r.build_pane(pane, &mut t.state, &input);
        }
        builds += 1;
        assert!(builds < 30, "the glyphs never came");
    }
}

/// Clusters (combining marks, emoji sequences) and ligature runs not
/// shaped before are shaped on the glyph thread too: the first build
/// leaves them out and asks, and once the thread wakes the app the pane
/// draws as one shaped on this thread. Shaped once, they don't wait.
#[test]
fn clusters_and_ligatures_are_shaped_on_the_glyph_thread() {
    if !have_gpu() {
        return;
    }
    let theme = theme();
    // Plain letters and spaces first, so only the shaping is new.
    let plain = "a b c x y = - > ! e";
    let text = "x -> e\u{301} \u{1F44D}\u{1F3FD} \u{1F469}\u{200D}\u{1F4BB} != y";
    let mut r = Renderer::new(jetbrains());
    let mut whole = Renderer::new(jetbrains());
    let m = r.metrics();
    let size = (24 * m.cell_width as usize, m.cell_height as usize);
    let (waker, wakes) = waker();
    r.set_glyph_waker(Some(waker));
    let (mut warm, mut tw) = built(&mut r, &theme, 24, 1, plain);
    until_complete(&mut r, &mut warm, &mut tw, &theme, &wakes);
    let (mut pane, mut t) = built(&mut r, &theme, 24, 1, text);
    assert!(pane.glyphs_deferred(), "clusters and stretches go to the glyph thread");
    until_complete(&mut r, &mut pane, &mut t, &theme, &wakes);
    let (all, _ta) = built(&mut whole, &theme, 24, 1, text);
    assert!(
        alone(&mut r, &pane, size) == alone(&mut whole, &all, size),
        "drawn as shaped on this thread"
    );
    let (again, _t2) = built(&mut r, &theme, 24, 1, text);
    assert!(!again.glyphs_deferred(), "runs shaped before don't wait");
}

/// A pane dropped while waiting for glyphs wants nothing any more.
#[test]
fn a_pane_dropped_while_waiting_wants_nothing() {
    if !have_gpu() {
        return;
    }
    let theme = theme();
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let (waker, _wakes) = waker();
    r.set_glyph_waker(Some(waker));
    let (pane, _t) = built(&mut r, &theme, 8, 1, "\u{4E01}\u{4E03}");
    assert!(pane.glyphs_deferred());
    assert_eq!(r.glyphs_wanted(), 2);
    drop(pane);
    assert_eq!(r.glyphs_wanted(), 0, "its wants went with it");
}

/// With split panes, one pane's complete build keeps the bitmaps another
/// pane is waiting for: that pane draws them on its next build without
/// asking again.
#[test]
fn a_complete_build_keeps_what_another_pane_waits_for() {
    if !have_gpu() {
        return;
    }
    let theme = theme();
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let (waker, wakes) = waker();
    r.set_glyph_waker(Some(waker));
    let (mut a, mut ta) = built(&mut r, &theme, 8, 1, "\u{4E08}");
    let (mut b, mut tb) = built(&mut r, &theme, 8, 1, "\u{4E08}\u{4E09}");
    assert!(a.glyphs_deferred() && b.glyphs_deferred());
    // Woken with nothing back drawn yet, as the app is: what came back
    // is taken in, and what no pane waits for would go.
    while r.glyphs_pending() {
        wakes.recv_timeout(std::time::Duration::from_secs(20)).expect("woken");
        r.drop_unused_glyphs();
    }
    let input = input(&r, &theme);
    r.snapshot_pane(&mut a, &mut ta.state, &input);
    assert!(r.build_from_snapshot(&mut a, &input));
    assert!(!a.glyphs_deferred(), "the first pane has its glyph");
    r.snapshot_pane(&mut b, &mut tb.state, &input);
    assert!(r.build_from_snapshot(&mut b, &input));
    assert!(!b.glyphs_deferred(), "the second pane's glyphs were kept for it");
}

/// Glyphs from the glyph thread that find no room in a texture make it
/// grow, as building does, and are all drawn.
#[test]
fn glyphs_from_the_glyph_thread_grow_a_full_texture() {
    if !have_gpu() {
        return;
    }
    term_render::set_glyph_texture_sizes((256, 4096), (64, 2048));
    let theme = theme();
    let text = "😀😁😂😃😄😅😆😇😈😉";
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let mut whole = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r.metrics();
    let size = (20 * m.cell_width as usize, m.cell_height as usize);
    let (woke, wakes) = std::sync::mpsc::channel::<()>();
    let woke = std::sync::Mutex::new(woke);
    r.set_glyph_waker(Some(std::sync::Arc::new(move || {
        let _ = woke.lock().unwrap().send(());
    })));
    let (mut pane, mut t) = built(&mut r, &theme, 20, 1, text);
    let (_, colour) = r.glyph_texture_sizes();
    let input = input(&r, &theme);
    let mut builds = 1;
    while pane.glyphs_deferred() {
        wakes
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("the glyph thread wakes the app");
        r.snapshot_pane(&mut pane, &mut t.state, &input);
        if !r.build_from_snapshot(&mut pane, &input) {
            r.build_pane(&mut pane, &mut t.state, &input);
        }
        builds += 1;
        assert!(builds < 30, "the glyphs never came");
    }
    assert!(r.glyph_texture_sizes().1 > colour, "the colour texture grew");
    let (all, _ta) = built(&mut whole, &theme, 20, 1, text);
    assert!(alone(&mut r, &pane, size) == alone(&mut whole, &all, size), "every glyph drawn");
}

/// Once idle, textures a flood grew go back to their starting sizes: the
/// pane is stale, and built again it gets every glyph at once, on this
/// thread though the glyph thread is on, and draws as before. Growing
/// back while filling in doesn't count: the next shrink does nothing.
#[test]
fn grown_textures_shrink_and_panes_fill_in_at_once() {
    if !have_gpu() {
        return;
    }
    term_render::set_glyph_texture_sizes((256, 4096), (64, 2048));
    let theme = theme();
    let text = "😀😁😂😃😄😅😆😇😈😉";
    let mut r = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r.metrics();
    let size = (20 * m.cell_width as usize, m.cell_height as usize);
    assert!(!r.shrink_glyph_textures(), "nothing grew yet");
    let (woke, wakes) = std::sync::mpsc::channel::<()>();
    let woke = std::sync::Mutex::new(woke);
    r.set_glyph_waker(Some(std::sync::Arc::new(move || {
        let _ = woke.lock().unwrap().send(());
    })));
    let (_, colour) = r.glyph_texture_sizes();
    let (mut pane, mut t) = built(&mut r, &theme, 20, 1, text);
    let input = input(&r, &theme);
    let mut builds = 1;
    while pane.glyphs_deferred() {
        wakes
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("the glyph thread wakes the app");
        r.build_pane(&mut pane, &mut t.state, &input);
        builds += 1;
        assert!(builds < 30, "the glyphs never came");
    }
    assert!(r.glyph_texture_sizes().1 > colour, "the colour texture grew");
    let before = alone(&mut r, &pane, size);

    assert!(r.shrink_glyph_textures());
    assert_eq!(r.glyph_texture_sizes().1, colour, "back to its starting size");
    assert!(!r.is_current(&pane), "its glyphs were forgotten");
    r.build_pane(&mut pane, &mut t.state, &input);
    assert!(!pane.glyphs_deferred(), "every glyph at once");
    assert!(r.is_current(&pane));
    assert!(alone(&mut r, &pane, size) == before, "drawn as before");
    assert!(!r.shrink_glyph_textures(), "filling in isn't growing");
}

/// Two windows with their own fonts share the glyph textures: each draws
/// its own text while the other uploads new glyphs between their frames,
/// and each frame matches the same frame drawn with the GPU idle.
#[test]
fn two_windows_share_the_glyph_textures() {
    if !have_gpu() {
        return;
    }
    let theme = theme();
    let mut r1 = Renderer::new(Fonts::new("Menlo", 24.0));
    let mut r2 = Renderer::new(Fonts::new("Menlo", 30.0));
    let size = |r: &Renderer| {
        let m = r.metrics();
        (8 * m.cell_width as usize, 2 * m.cell_height as usize)
    };
    let (s1, s2) = (size(&r1), size(&r2));
    let (t1, t2) = (target(&r1, s1.0, s1.1), target(&r2, s2.0, s2.1));
    let steps = ["plain", "\x1b[1mbold é", "\x1b[3mitalic ü", "漢字 ✓ ▒", "→ λ ∑ ß"];
    for (i, text) in steps.iter().enumerate() {
        let (p1, _a) = built(&mut r1, &theme, 8, 2, text);
        let encode = |r: &mut Renderer, p: &PaneRender, t: &Texture, s: (usize, usize)| {
            let all = PxRect {
                x: 0,
                y: 0,
                w: s.0 as i32,
                h: s.1 as i32,
            };
            r.encode_panes(
                t,
                p.frame.clear,
                &[PaneDraw {
                    pane: p,
                    origin: (0, 0),
                    scissor: all,
                    scroll: None,
                }],
                &[],
            )
        };
        let c1 = encode(&mut r1, &p1, &t1, s1);
        c1.commit();
        // New glyphs for the other window's fonts, while frame 1 may be
        // on the GPU.
        let (p2, _b) = built(&mut r2, &theme, 8, 2, &format!("{text}{i}"));
        let c2 = encode(&mut r2, &p2, &t2, s2);
        c2.commit();
        c1.waitUntilCompleted();
        c2.waitUntilCompleted();
        let (got1, got2) = (read(&t1), read(&t2));
        assert!(
            got1.inked((0, s1.0), (0, s1.1)) > 0 && got2.inked((0, s2.0), (0, s2.1)) > 0,
            "step {i}"
        );
        assert!(alone(&mut r1, &p1, s1) == got1, "step {i}: window 1");
        assert!(alone(&mut r2, &p2, s2) == got2, "step {i}: window 2");
        assert!(r1.is_current(&p1) && r2.is_current(&p2));
    }
}

/// Closing a window whose fonts no other window uses empties the shared
/// textures back to their starting size, as its own textures were freed
/// before they were shared; the other window's panes are built again and
/// draw as before.
#[test]
fn closing_a_window_gives_its_glyph_space_back() {
    if !have_gpu() {
        return;
    }
    term_render::set_glyph_texture_sizes((256, 4096), (64, 2048));
    let theme = theme();
    let mut r1 = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = r1.metrics();
    let size = (6 * m.cell_width as usize, m.cell_height as usize);
    let (mut a, mut ta) = built(&mut r1, &theme, 6, 1, "abcxyz");
    let before = alone(&mut r1, &a, size);
    let start = r1.glyph_texture_sizes();

    let mut r2 = Renderer::new(Fonts::new("Menlo", 64.0));
    let text: String = (0x21u8..0x7F).map(|c| c as char).collect();
    let (_b, _t) = built(&mut r2, &theme, 94, 1, &text);
    assert!(r2.glyph_texture_sizes().0 > start.0, "the second window's glyphs made it grow");
    drop(r2);

    // Back to the starting sizes, emptied.
    assert_eq!(r1.glyph_texture_sizes(), (256, 64));
    assert!(!r1.is_current(&a));
    let input = input(&r1, &theme);
    r1.build_pane(&mut a, &mut ta.state, &input);
    assert!(alone(&mut r1, &a, size) == before);
}

/// Under memory pressure the glyph textures start again, empty, at their
/// starting sizes: panes built before are stale, and built again they
/// draw as before. A second purge with nothing drawn in between gives
/// nothing more back.
#[test]
fn a_purge_gives_glyph_space_back_and_redraws_the_same() {
    if !have_gpu() {
        return;
    }
    term_render::set_glyph_texture_sizes((256, 4096), (64, 2048));
    let theme = theme();
    let mut r1 = Renderer::new(Fonts::new("Menlo", 64.0));
    let m = r1.metrics();
    let text: String = (0x21u8..0x7F).map(|c| c as char).collect();
    let size = (94 * m.cell_width as usize, m.cell_height as usize);
    let (mut a, mut ta) = built(&mut r1, &theme, 94, 1, &format!("\x1b[1m{text}"));
    let before = alone(&mut r1, &a, size);
    assert!(r1.glyph_texture_sizes().0 > 256, "the coverage texture grew");

    assert!(r1.purge() > 0);
    assert_eq!(r1.glyph_texture_sizes(), (256, 64));
    assert!(!r1.is_current(&a));
    assert_eq!(r1.purge(), 0, "already given back");
    let input = input(&r1, &theme);
    r1.build_pane(&mut a, &mut ta.state, &input);
    assert!(alone(&mut r1, &a, size) == before);
}
