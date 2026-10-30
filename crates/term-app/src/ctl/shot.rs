//! Screenshots: a session's frame built with term-render's renderer (its
//! fonts, theme and scale), drawn into an offscreen BGRA texture on the
//! GPU, read back and written as PNG. Renderers keep their glyph caches
//! on one thread of their own, kept per font settings.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use objc2_metal::{
    MTLCommandBuffer, MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture, MTLTextureDescriptor,
    MTLTextureUsage,
};
use term_render::{FrameInput, Renderer};

use super::headless::{Headless, ShotTimes};

/// An RGBA image, not premultiplied.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

type Job = Box<dyn FnOnce(&mut HashMap<String, Renderer>) + Send>;

/// The render thread's queue, started on first use.
fn queue() -> &'static Mutex<Sender<Job>> {
    static QUEUE: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("ctl-render".into())
            .spawn(move || {
                let mut renderers = HashMap::new();
                for job in rx {
                    job(&mut renderers);
                }
            })
            .expect("render thread");
        Mutex::new(tx)
    })
}

/// Draw `h`'s screen as it is now.
pub fn capture(h: &Arc<Headless>) -> Result<(Image, ShotTimes), String> {
    let (tx, rx) = channel();
    let h2 = h.clone();
    let job: Job = Box::new(move |renderers| {
        let _ = tx.send(crate::crash::contain(|| draw(&h2, renderers)).unwrap_or_else(|_| Err("drawing the screenshot panicked".into())));
    });
    queue()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .send(job)
        .map_err(|_| "the render thread has stopped")?;
    rx.recv().map_err(|_| "the render thread has stopped".to_string())?
}

fn draw(h: &Headless, renderers: &mut HashMap<String, Renderer>) -> Result<(Image, ShotTimes), String> {
    let t0 = Instant::now();
    let look = &h.look;
    let fc = look.config.font_config(look.config.font_size);
    let key = format!("{fc:?}@{}", look.scale);
    let renderer = renderers.entry(key).or_insert_with(|| Renderer::with_font_config(&fc, look.scale));
    let m = renderer.metrics();
    let pad = crate::layout::padding(look.config.padding, 0.0, look.scale);
    let (width, height) = {
        let mut t = h.term();
        let theme = crate::config::program_theme(&look.theme, &t.state);
        let input = FrameInput {
            theme: theme.as_ref().unwrap_or(&look.theme),
            metrics: m,
            padding: (pad.x, pad.top),
            cursor_on: true,
            focused: true,
            preedit: None,
        };
        renderer.prepare(&mut t.state, &input);
        (
            t.state.cols() as u32 * m.cell_width + 2 * pad.x,
            t.state.lines() as u32 * m.cell_height + pad.top + pad.bottom,
        )
    };
    let build = t0.elapsed();
    let (w, hgt) = (width as usize, height as usize);
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::BGRA8Unorm, w, hgt, false)
    };
    desc.setUsage(MTLTextureUsage::RenderTarget);
    desc.setStorageMode(MTLStorageMode::Shared);
    let target = renderer.device().newTextureWithDescriptor(&desc).ok_or("no offscreen texture")?;
    let gpu0 = Instant::now();
    let cmd = renderer.encode(&target);
    cmd.commit();
    cmd.waitUntilCompleted();
    let gpu = gpu0.elapsed();
    let mut data = vec![0u8; w * hgt * 4];
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width: w,
            height: hgt,
            depth: 1,
        },
    };
    unsafe {
        target.getBytes_bytesPerRow_fromRegion_mipmapLevel(
            NonNull::new(data.as_mut_ptr() as *mut c_void).expect("a buffer"),
            w * 4,
            region,
            0,
        );
    }
    // BGRA, premultiplied → RGBA.
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        let un = |c: u8| {
            if a == 0 || a == 255 {
                c
            } else {
                ((c as u32 * 255 + a / 2) / a).min(255) as u8
            }
        };
        let (b, g, r) = (un(px[0]), un(px[1]), un(px[2]));
        px[0] = r;
        px[1] = g;
        px[2] = b;
    }
    let times = ShotTimes {
        build,
        gpu,
        total: t0.elapsed(),
        width,
        height,
    };
    Ok((Image { width, height, rgba: data }, times))
}

/// Write `img` to `path` as PNG.
pub fn write_png(img: &Image, path: &str) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    // Screenshots are for reading back, not keeping: fast over small.
    enc.set_compression(png::Compression::Fast);
    let mut w = enc.write_header().map_err(|e| format!("{path}: {e}"))?;
    w.write_image_data(&img.rgba).map_err(|e| format!("{path}: {e}"))?;
    w.finish().map_err(|e| format!("{path}: {e}"))
}
