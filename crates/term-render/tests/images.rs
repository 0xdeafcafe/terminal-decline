//! Images on the GPU (images.md §6, §10): pixels in Metal buffers drawn
//! through textures over them, into an offscreen target read back. The
//! images are put in the snapshot by the test (`build_pane_with`) until
//! term-core's protocols make them; placeholder cells are written through
//! the terminal. Without a Metal device the tests are skipped locally and
//! fail on CI (the `CI` env var is set).

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2_metal::{
    MTLCommandBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use term_core::image::{Fit, ImageKey, Layer, SnapImage, SnapPlacement, SnapVirtual};
use term_core::{ImagePool, PixelBuffer, Snapshot, Term};
use term_font::Fonts;
use term_image::PixelTarget;
use term_render::{FrameInput, PaneDraw, PaneRender, PixelAlloc, PxRect, Renderer, Texture, Theme};

fn have_gpu() -> bool {
    if MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return false;
    }
    true
}

const BG: u32 = 0x101010;
const RED: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [255, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
/// The theme background in BGRA.
const BACK: [u8; 4] = [0x10, 0x10, 0x10, 0xFF];

struct Picture {
    data: Vec<u8>,
    width: usize,
}

impl Picture {
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        self.data[i..i + 4].try_into().unwrap()
    }

    fn near(&self, x: usize, y: usize, want: [u8; 4], what: &str) {
        let got = self.at(x, y);
        let ok = got.iter().zip(want).all(|(&g, w)| (g as i32 - w as i32).abs() <= 3);
        assert!(ok, "{what} at ({x}, {y}): got BGRA {got:?}, want {want:?}");
    }

    fn count(&self, (x0, x1): (usize, usize), (y0, y1): (usize, usize), f: impl Fn([u8; 4]) -> bool) -> usize {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| f(self.at(x, y)))
            .count()
    }
}

struct Setup {
    renderer: Renderer,
    alloc: PixelAlloc,
    pool: Arc<ImagePool>,
    theme: Theme,
    cw: usize,
    ch: usize,
}

fn setup() -> Setup {
    let renderer = Renderer::new(Fonts::new("Menlo", 24.0));
    let m = renderer.metrics();
    Setup {
        alloc: renderer.pixel_alloc(),
        renderer,
        pool: ImagePool::new(32 << 20),
        theme: Theme::new([0; 16], 0xffffff, BG, 0xffffff, 0x4060a0),
        cw: m.cell_width as usize,
        ch: m.cell_height as usize,
    }
}

impl Setup {
    /// A `w` × `h` image, pixel (x, y) from `f` (BGRA, premultiplied).
    fn image(&self, w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> SnapImage {
        let mut px = self.alloc.alloc_metal(w, h).expect("pixels");
        let pitch = px.row_pitch();
        let bytes = px.bytes_mut();
        for y in 0..h {
            for x in 0..w {
                let i = y as usize * pitch + x as usize * 4;
                bytes[i..i + 4].copy_from_slice(&f(x, y));
            }
        }
        let id = self.pool.reserve(1, px.allocated_size(), None).expect("room");
        let pixels: Arc<dyn PixelBuffer> = Arc::new(px);
        let generation = self.pool.set_pixels(id, pixels.clone());
        SnapImage {
            pool: id,
            generation,
            pixels,
            width: w,
            height: h,
        }
    }

    /// One colour, `w` × `h`.
    fn solid(&self, w: u32, h: u32, c: [u8; 4]) -> SnapImage {
        self.image(w, h, |_, _| c)
    }

    /// Red, green / blue, white.
    fn four(&self) -> SnapImage {
        self.image(2, 2, |x, y| [[RED, GREEN], [BLUE, WHITE]][y as usize][x as usize])
    }

    fn term(&self, cols: usize, lines: usize, text: &str) -> Term {
        let mut t = Term::new(cols, lines, 0, self.pool.clone());
        t.advance(format!("\x1b[?25l{text}").as_bytes());
        t
    }

    /// Build `t` with `inject` and draw it into a `cols` × `lines` target,
    /// inside `scissor` (the whole target if `None`).
    fn draw(&mut self, t: &mut Term, scissor: Option<PxRect>, inject: impl FnOnce(&mut Snapshot)) -> (Picture, PaneRender) {
        let input = FrameInput {
            theme: &self.theme,
            metrics: self.renderer.metrics(),
            padding: (0, 0),
            cursor_on: true,
            focused: true,
            preedit: None,
        };
        let mut pane = PaneRender::new();
        self.renderer.build_pane_with(&mut pane, &mut t.state, &input, inject);
        let (width, height) = (t.state.cols() * self.cw, t.state.lines() * self.ch);
        let target = target(&self.renderer, width, height);
        let all = PxRect {
            x: 0,
            y: 0,
            w: width as i32,
            h: height as i32,
        };
        let cmd = self.renderer.encode_panes(
            &target,
            pane.frame.clear,
            &[PaneDraw {
                pane: &pane,
                origin: (0, 0),
                scissor: scissor.unwrap_or(all),
            }],
            &[],
        );
        cmd.commit();
        cmd.waitUntilCompleted();
        (read(&target), pane)
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

fn read(target: &Texture) -> Picture {
    let (width, height) = (target.width(), target.height());
    let mut data = vec![0u8; width * height * 4];
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize { width, height, depth: 1 },
    };
    unsafe {
        target.getBytes_bytesPerRow_fromRegion_mipmapLevel(NonNull::new(data.as_mut_ptr() as *mut c_void).unwrap(), width * 4, region, 0);
    }
    Picture { data, width }
}

fn placed(image: SnapImage, (col, row): (u16, i32), (cols, rows): (u16, u16), z: i32, serial: u64) -> SnapPlacement {
    SnapPlacement {
        image,
        view_row: row,
        col,
        cols,
        rows,
        offset: (0, 0),
        src: [0, 0, 0, 0],
        layer: Layer::of_z(z),
        z,
        serial,
    }
}

#[test]
fn an_image_is_drawn_stretched_over_its_cells() {
    if !have_gpu() {
        return;
    }
    let mut s = setup();
    let (cw, ch) = (s.cw, s.ch);
    let mut t = s.term(4, 3, "");
    let img = s.four();
    // Two cells by two from column 1, row 0.
    let (px, pane) = s.draw(&mut t, None, |snap| snap.placements.push(placed(img, (1, 0), (2, 2), 0, 1)));
    assert_eq!(pane.frame.images.instances.len(), 1);
    // Each texel fills a quarter; near the corners the colour is exact.
    px.near(cw + 1, 1, RED, "top left");
    px.near(3 * cw - 2, 1, GREEN, "top right");
    px.near(cw + 1, 2 * ch - 2, BLUE, "bottom left");
    px.near(3 * cw - 2, 2 * ch - 2, WHITE, "bottom right");
    // Nothing outside its cells.
    px.near(cw - 1, ch, BACK, "left of it");
    px.near(3 * cw, ch, BACK, "right of it");
    px.near(2 * cw, 2 * ch, BACK, "under it");
    // The pool heard it was drawn: the window keeps its texture.
    assert_eq!(s.renderer.image_textures(), 1);
}

#[test]
fn a_source_rectangle_takes_nothing_from_outside_it() {
    if !have_gpu() {
        return;
    }
    let mut s = setup();
    let (cw, ch) = (s.cw, s.ch);
    let mut t = s.term(4, 2, "");
    // Left half red, right half green; only the red half, stretched over
    // three cells: bilinear filtering must not pull green in at its edge.
    let img = s.image(4, 4, |x, _| if x < 2 { RED } else { GREEN });
    let mut p = placed(img, (0, 0), (3, 2), 0, 1);
    p.src = [0, 0, 2, 4];
    let (px, _) = s.draw(&mut t, None, |snap| snap.placements.push(p));
    for y in [1, ch, 2 * ch - 2] {
        px.near(3 * cw - 1, y, RED, "right edge of the source");
        px.near(0, y, RED, "left edge");
    }
}

#[test]
fn the_pane_scissor_clips_an_image() {
    if !have_gpu() {
        return;
    }
    let mut s = setup();
    let (cw, ch) = (s.cw, s.ch);
    let mut t = s.term(4, 2, "");
    let img = s.solid(8, 8, GREEN);
    // Over the whole grid, the scissor its left half.
    let scissor = PxRect {
        x: 0,
        y: 0,
        w: 2 * cw as i32,
        h: 2 * ch as i32,
    };
    let (px, _) = s.draw(&mut t, Some(scissor), |snap| {
        snap.placements.push(placed(img, (0, 0), (4, 2), 0, 1))
    });
    px.near(cw, ch, GREEN, "inside the scissor");
    // Outside, the pass's clear colour (the theme background).
    px.near(3 * cw, ch, BACK, "outside the scissor");
}

#[test]
fn layers_put_images_under_or_over_the_text() {
    if !have_gpu() {
        return;
    }
    let white = |p: [u8; 4]| p[0] > 200 && p[1] > 200 && p[2] > 200;
    for (z, text_shows) in [(-1, true), (0, false), (-(1 << 30) - 1, true)] {
        let mut s = setup();
        let (cw, ch) = (s.cw, s.ch);
        let mut t = s.term(2, 1, "M");
        let img = s.solid(4, 4, GREEN);
        let (px, _) = s.draw(&mut t, None, |snap| snap.placements.push(placed(img, (0, 0), (1, 1), z, 1)));
        let inked = px.count((0, cw), (0, ch), white);
        assert_eq!(inked > 10, text_shows, "z {z}: {inked} text pixels");
        let green = px.count((0, cw), (0, ch), |p| p == GREEN);
        assert!(green > cw * ch / 3, "z {z}: the image shows ({green} green pixels)");
    }
}

#[test]
fn an_image_under_the_backgrounds_is_hidden_by_a_coloured_cell() {
    if !have_gpu() {
        return;
    }
    let mut s = setup();
    let (cw, ch) = (s.cw, s.ch);
    // Cell 0 has a blue background, cell 1 the default.
    let mut t = s.term(2, 1, "\x1b[44m \x1b[0m ");
    s.theme.palette[4] = term_render::rgb(0x0000ff);
    let img = s.solid(4, 4, GREEN);
    let (px, _) = s.draw(&mut t, None, |snap| {
        snap.placements.push(placed(img, (0, 0), (2, 1), -(1 << 30) - 1, 1))
    });
    px.near(cw / 2, ch / 2, BLUE, "coloured cell");
    px.near(cw + cw / 2, ch / 2, GREEN, "default background shows the image");
}

#[test]
fn z_orders_images_in_a_layer() {
    if !have_gpu() {
        return;
    }
    for (red_z, green_z, want) in [(1, 2, GREEN), (2, 1, RED), (5, 5, GREEN)] {
        let mut s = setup();
        let (cw, ch) = (s.cw, s.ch);
        let mut t = s.term(2, 1, "");
        let red = s.solid(2, 2, RED);
        let green = s.solid(2, 2, GREEN);
        let (px, pane) = s.draw(&mut t, None, |snap| {
            snap.placements.push(placed(red, (0, 0), (1, 1), red_z, 1));
            snap.placements.push(placed(green, (0, 0), (1, 1), green_z, 2));
        });
        assert_eq!(pane.frame.images.draws.len(), 2);
        px.near(cw / 2, ch / 2, want, &format!("red z {red_z}, green z {green_z}"));
    }
}

#[test]
fn shrinking_averages_texels() {
    if !have_gpu() {
        return;
    }
    let mut s = setup();
    let (cw, ch) = (s.cw, s.ch);
    let mut t = s.term(1, 1, "");
    // 8 texels a pixel, each pixel half white in its middle: one bilinear
    // sample at its centre would be white, the 2×2 box is mid-grey.
    let w = cw as u32 * 8;
    let img = s.image(
        w,
        ch as u32 * 8,
        |x, _| if (2..6).contains(&(x % 8)) { WHITE } else { [0, 0, 0, 255] },
    );
    let (px, pane) = s.draw(&mut t, None, |snap| snap.placements.push(placed(img, (0, 0), (1, 1), 0, 1)));
    assert!(pane.frame.images.instances[0].scale > 7.9);
    let grey = px.count((1, cw - 1), (1, ch - 1), |p| (90..=165).contains(&p[1]));
    assert_eq!(grey, (cw - 2) * (ch - 2), "every pixel about mid-grey");
}

/// Two placeholder cells (row 0, columns 0 and 1) of kitty image 7.
const PLACEHOLDERS: &str = "\x1b[38;5;7m\u{10EEEE}\u{10EEEE}\x1b[0m";

fn virtual_of(image: SnapImage, cols: u16, rows: u16, fit: Fit) -> SnapVirtual {
    SnapVirtual {
        key: (ImageKey::Kitty(7), 0),
        image,
        cols,
        rows,
        fit,
    }
}

#[test]
fn placeholder_cells_draw_their_pieces_merged() {
    if !have_gpu() {
        return;
    }
    let mut s = setup();
    let (cw, ch) = (s.cw, s.ch);
    let mut t = s.term(3, 1, PLACEHOLDERS);
    let img = s.image(2, 1, |x, _| if x == 0 { RED } else { GREEN });
    let v = virtual_of(img, 2, 1, Fit::Stretch);
    let (px, pane) = s.draw(&mut t, None, |snap| snap.virtual_placements.push(v.clone()));
    assert_eq!(pane.frame.images.instances.len(), 1, "one quad for the run");
    assert!(pane.frame.glyphs.is_empty(), "a placeholder's content isn't a character");
    px.near(2, ch / 2, RED, "first piece");
    px.near(2 * cw - 3, ch / 2, GREEN, "second piece");
    px.near(2 * cw + cw / 2, ch / 2, BACK, "past the image");

    // The second cell overwritten: only the first piece is drawn.
    t.advance(b"\x1b[1;2H ");
    let (px, pane) = s.draw(&mut t, None, |snap| snap.virtual_placements.push(v));
    assert_eq!(pane.frame.images.instances.len(), 1);
    assert_eq!(pane.frame.images.instances[0].dst[2] as usize, cw);
    px.near(2, ch / 2, RED, "first piece");
    px.near(cw + cw / 2, ch / 2, BACK, "overwritten cell");
}

#[test]
fn placeholders_without_their_image_draw_nothing() {
    if !have_gpu() {
        return;
    }
    let mut s = setup();
    let (cw, ch) = (s.cw, s.ch);
    let mut t = s.term(3, 1, PLACEHOLDERS);
    let (px, pane) = s.draw(&mut t, None, |_| {});
    assert!(pane.frame.images.instances.is_empty());
    assert!(pane.frame.glyphs.is_empty());
    assert_eq!(px.count((0, 3 * cw), (0, ch), |p| p != BACK), 0);
}

#[test]
fn a_selection_over_an_image_is_a_tint() {
    if !have_gpu() {
        return;
    }
    let mut s = setup();
    let (cw, ch) = (s.cw, s.ch);
    let mut t = s.term(3, 1, PLACEHOLDERS);
    let img = s.solid(2, 1, GREEN);
    let v = virtual_of(img, 2, 1, Fit::Stretch);
    let (px, _) = s.draw(&mut t, None, |snap| {
        snap.virtual_placements.push(v);
        let at = snap.highlights.len() as u32;
        snap.highlights.push(term_core::Highlight {
            cols: 0..3,
            kind: term_core::HighlightKind::Selection,
        });
        snap.rows[0].highlights = at..at + 1;
    });
    // Over the image: green shows through the selection colour.
    let over = px.at(cw / 2, ch / 2);
    assert!(over[1] > 0x60 && over[1] < 0xF0, "tinted, not covered: {over:?}");
    // Past it: the selection colour itself (#4060a0).
    px.near(2 * cw + cw / 2, ch / 2, [0xa0, 0x60, 0x40, 0xFF], "plain selection");
}
