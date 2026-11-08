//! The text settings drawn on the GPU and read back: thickening, the
//! baseline and underline adjustments, and the minimum contrast. Without a
//! Metal device the tests are skipped locally and fail on CI.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_metal::{
    MTLCommandBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use term_core::Term;
use term_font::{Adjust, FontConfig};
use term_render::{FrameInput, Renderer, Theme, contrast, rgb};

/// White on black, so a pixel's green is its ink.
fn theme() -> Theme {
    Theme::new([0; 16], 0xffffff, 0x000000, 0xffffff, 0x444444)
}

fn has_metal() -> bool {
    if MTLCreateSystemDefaultDevice().is_some() {
        return true;
    }
    assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
    eprintln!("skipping: no Metal device");
    false
}

struct Pixels {
    data: Vec<u8>,
    width: usize,
    height: usize,
}

impl Pixels {
    /// BGRA at `(x, y)`.
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        self.data[i..i + 4].try_into().unwrap()
    }

    /// The sum of every pixel's green: how much ink there is.
    fn ink(&self) -> u64 {
        self.data.chunks(4).map(|p| p[1] as u64).sum()
    }

    /// The rows with any ink.
    fn inked_rows(&self) -> Vec<usize> {
        (0..self.height)
            .filter(|&y| (0..self.width).any(|x| self.at(x, y)[1] > 128))
            .collect()
    }
}

/// `text` on a 4×1 grid in Menlo 24 as `edit` sets it, drawn and read back.
fn draw(edit: impl FnOnce(&mut FontConfig), theme: &Theme, text: &str) -> (Pixels, term_font::Metrics) {
    let mut config = FontConfig::new("Menlo", 24.0);
    edit(&mut config);
    let mut renderer = Renderer::with_font_config(&config, 1.0);
    let m = renderer.metrics();
    let mut t = Term::new(4, 1, 0, term_core::ImagePool::process());
    t.advance(format!("\x1b[?25l{text}").as_bytes());
    let input = FrameInput {
        theme,
        metrics: m,
        padding: (0, 0),
        cursor_on: true,
        focused: true,
        preedit: None,
        scale: 2.0,
    };
    renderer.prepare(&mut t.state, &input);
    let (width, height) = (4 * m.cell_width as usize, m.cell_height as usize);
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
    (Pixels { data, width, height }, m)
}

#[test]
fn thickened_text_has_more_ink() {
    if !has_metal() {
        return;
    }
    let (plain, _) = draw(|_| {}, &theme(), "il|M");
    let (thick, _) = draw(|c| c.thicken = 0.5, &theme(), "il|M");
    let (more, _) = draw(|c| c.thicken = 1.0, &theme(), "il|M");
    assert!(thick.ink() > plain.ink() * 11 / 10, "{} vs {}", thick.ink(), plain.ink());
    assert!(more.ink() > thick.ink(), "{} vs {}", more.ink(), thick.ink());
}

#[test]
fn the_baseline_moves_the_text() {
    if !has_metal() {
        return;
    }
    // 'M' sits on the baseline: its last inked row is just above it.
    let (plain, m) = draw(|_| {}, &theme(), "MMMM");
    let (up, m_up) = draw(|c| c.baseline = Adjust::Points(3.0), &theme(), "MMMM");
    let (down, m_down) = draw(|c| c.baseline = Adjust::Points(-2.0), &theme(), "MMMM");
    assert_eq!((m_up.baseline, m_down.baseline), (m.baseline - 3, m.baseline + 2));
    assert_eq!(m_up.cell_height, m.cell_height, "the cell keeps its height");
    let bottom = |p: &Pixels| *p.inked_rows().last().unwrap();
    assert_eq!(bottom(&up), bottom(&plain) - 3);
    assert_eq!(bottom(&down), bottom(&plain) + 2);
}

#[test]
fn underlines_move_and_thicken() {
    if !has_metal() {
        return;
    }
    // Underlined spaces: the only ink is the underline.
    let (plain, m) = draw(|_| {}, &theme(), "\x1b[4m    ");
    assert_eq!(plain.inked_rows().first(), Some(&((m.baseline + m.underline_offset) as usize)));
    let (moved, mm) = draw(
        |c| {
            c.underline_position = Adjust::Points(2.0);
            c.underline_thickness = Adjust::Points(1.0);
        },
        &theme(),
        "\x1b[4m    ",
    );
    assert_eq!(mm.underline_offset, m.underline_offset + 2);
    assert_eq!(mm.underline_thickness, m.underline_thickness + 1);
    let rows = moved.inked_rows();
    let top = (m.baseline + m.underline_offset + 2) as usize;
    assert_eq!(rows, (top..top + mm.underline_thickness as usize).collect::<Vec<_>>());
    // Never out of the cell, however far.
    let (_, far) = draw(|c| c.underline_position = Adjust::Points(100.0), &theme(), "\x1b[4m    ");
    assert!(far.baseline + far.underline_offset + far.underline_thickness <= far.cell_height);
}

#[test]
fn minimum_contrast_lifts_faint_text() {
    if !has_metal() {
        return;
    }
    // Grey #333333 on #222222: about 1.3:1.
    let text = "\x1b[38;2;51;51;51;48;2;34;34;34mMMMM";
    let brightest = |p: &Pixels| {
        (0..p.data.len() / 4)
            .map(|i| p.at(i % p.width, i / p.width))
            .max_by_key(|px| px[1])
            .unwrap()
    };
    let as_rgba = |px: [u8; 4]| rgb((px[2] as u32) << 16 | (px[1] as u32) << 8 | px[0] as u32);
    let bg = rgb(0x222222);
    let (faint, _) = draw(|_| {}, &theme(), text);
    assert!(contrast(as_rgba(brightest(&faint)), bg) < 1.5);
    let mut t = theme();
    t.minimum_contrast = 4.5;
    let (legible, _) = draw(|_| {}, &t, text);
    let got = contrast(as_rgba(brightest(&legible)), bg);
    assert!(got >= 4.4, "{got}");
}
