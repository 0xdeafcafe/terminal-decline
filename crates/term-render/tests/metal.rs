//! Real GPU render: a 4×2-cell grid drawn into an offscreen BGRA8 texture
//! and read back. Checks the background colour's byte order, premultiplied
//! blending of a half-transparent background over a half-transparent window,
//! and that a glyph draws something. Without a Metal device the test is
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
    let mut t = Term::new(COLS, LINES, 0);
    t.advance(b"\x1b[?25l\x1b[41m \x1b[48;2;17;34;51m \x1b[0m\r\nM");

    let input = FrameInput {
        term: &t.state,
        theme: &theme,
        metrics: m,
        padding: (0, 0),
        cursor_on: true,
        focused: true,
        preedit: None,
    };
    renderer.prepare(&input);
    assert_eq!(renderer.frame.glyphs.len(), 1);

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
