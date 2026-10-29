//! Regression tests for what an adversarial review of the decoders found:
//! images crafted to pass the memory cap or overflow the fixed-point
//! arithmetic. Allocations are counted (as in decode_memory.rs), so every
//! test here holds one lock: no other test's allocations run alongside.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use term_image::VecPixels;
use term_image::decode::{DecodeError, Options, decode};

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static LOCK: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let n = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(n, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Decode `bytes` into a `size` target; the result, the target and the
/// bytes allocated at the peak beyond what was allocated at the start.
fn decode_counted(bytes: &[u8], size: (u32, u32), options: &Options) -> (Result<bool, DecodeError>, VecPixels, usize) {
    let mut t = VecPixels::new(size.0, size.1);
    let start = NOW.load(Ordering::Relaxed);
    PEAK.store(start, Ordering::Relaxed);
    let r = decode(bytes, &mut t, options).map(|d| d.opaque);
    (r, t, PEAK.load(Ordering::Relaxed) - start)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A PNG of just its header: IHDR (8-bit, colour type `colour`), an
/// empty IDAT and IEND, with valid CRCs.
fn header_only_png(w: u32, h: u32, colour: u8) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, colour, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &[]);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// The scaler's own row is 4 bytes a source pixel, so a very wide grey
/// image (1 byte a pixel in the decoder) took 4 × the decoder's row on
/// top of it, unchecked: 300 MB from 57 bytes against a 64 MB cap.
#[test]
fn wide_grey_png_stays_within_the_cap() {
    let _g = lock();
    let png = header_only_png(60_000_000, 1, 0);
    assert_eq!(png.len(), 57);
    let (r, _, used) = decode_counted(&png, (1, 1), &Options::default());
    assert_eq!(r, Err(DecodeError::TooLarge));
    assert!(used <= 80 << 20, "{used} bytes at the peak");
}

/// Opaque white, premultiplied BGRA.
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// A real `w` × `h` PNG, every pixel `rgb`.
fn solid_rgb_png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.set_compression(png::Compression::Fast);
    e.set_filter(png::Filter::NoFilter);
    let mut wr = e.write_header().unwrap();
    let data: Vec<u8> = rgb.iter().copied().cycle().take(w as usize * h as usize * 3).collect();
    wr.write_image_data(&data).unwrap();
    wr.finish().unwrap();
    out
}

/// More than 2³² / 255 opaque pixels in one span overflowed the
/// scaler's 32-bit sums: a panic in debug builds, and in release wrong
/// colour and alpha while reporting the image opaque.
#[test]
fn wide_png_averages_without_overflow() {
    let _g = lock();
    let png = solid_rgb_png(17_000_000, 1, [255; 3]);
    // Its rows are past the default cap (see above), so a larger one.
    let options = Options {
        transient: 1 << 30,
        display_p3: false,
    };
    let (r, t, _) = decode_counted(&png, (1, 1), &options);
    assert_eq!(r, Ok(true));
    assert_eq!(t.pixel(0, 0), WHITE);
}

/// The same within every cap: a 65535 × 300 GIF (the largest width a
/// GIF can say), all white, 14 KB (made with the gif crate's encoder: a
/// white and a black palette entry, every index 0).
#[test]
fn large_gif_averages_without_overflow() {
    let _g = lock();
    let image = include_bytes!("fixtures/white-65535x300.gif");
    let options = Options {
        display_p3: false,
        ..Options::default()
    };
    let (r, t, _) = decode_counted(image, (1, 1), &options);
    assert_eq!(r, Ok(true));
    assert_eq!(t.pixel(0, 0), WHITE);
}

/// A matrix/TRC ICC profile whose `rXYZ`, `gXYZ` and `bXYZ` are all
/// 5.5 × the D50 white, with a linear `rTRC` (a `curv` of no entries):
/// the conversion to Display P3 then has about 5.5 in every entry, under
/// the sanity check's 8, and 5.5 in Q12 times linear white overflowed
/// the matrix's 32-bit sums.
fn bright_profile() -> Vec<u8> {
    let mut p = vec![0u8; 128];
    p[16..20].copy_from_slice(b"RGB ");
    p[20..24].copy_from_slice(b"XYZ ");
    p[36..40].copy_from_slice(b"acsp");
    let tags: [&[u8; 4]; 4] = [b"rXYZ", b"gXYZ", b"bXYZ", b"rTRC"];
    p.extend_from_slice(&(tags.len() as u32).to_be_bytes());
    let data_at = p.len() + tags.len() * 12;
    let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
    let mut data = Vec::new();
    for (n, sig) in tags.iter().enumerate() {
        let at = data_at + data.len();
        let start = data.len();
        if n < 3 {
            data.extend_from_slice(b"XYZ \0\0\0\0");
            for v in [0.9642, 1.0, 0.8249] {
                data.extend_from_slice(&fixed(5.5 * v));
            }
        } else {
            data.extend_from_slice(b"curv\0\0\0\0\0\0\0\0");
        }
        p.extend_from_slice(*sig);
        p.extend_from_slice(&(at as u32).to_be_bytes());
        p.extend_from_slice(&((data.len() - start) as u32).to_be_bytes());
    }
    p.extend(data);
    p
}

#[test]
fn bright_profile_converts_without_overflow() {
    let _g = lock();
    let t = term_image::color::ToP3::from_icc(&bright_profile()).unwrap();
    assert_eq!(t.rgb([255, 255, 255]), [255, 255, 255]);
    assert_eq!(t.rgb([0, 0, 0]), [0, 0, 0]);
}

/// The same through decode(): a 1 × 1 white PNG carrying the profile.
#[test]
fn bright_profile_in_a_png_decodes_without_overflow() {
    let _g = lock();
    let zlib = |data: &[u8]| miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    let mut iccp = b"bright\0\0".to_vec();
    iccp.extend(zlib(&bright_profile()));
    chunk(&mut png, b"iCCP", &iccp);
    chunk(&mut png, b"IDAT", &zlib(&[0, 255, 255, 255]));
    chunk(&mut png, b"IEND", &[]);
    let (r, t, _) = decode_counted(&png, (1, 1), &Options::default());
    assert_eq!(r, Ok(true));
    assert_eq!(t.pixel(0, 0), WHITE);
}

/// A Sixel string through a decoder with the default limits: the image's
/// size, and the bytes allocated at the peak.
fn sixel(data: &[u8]) -> (Option<(u32, u32)>, usize) {
    use term_image::sixel::{Decoder, Limits, Palette};
    let start = NOW.load(Ordering::Relaxed);
    PEAK.store(start, Ordering::Relaxed);
    let mut d = Decoder::<VecPixels>::new(7, 1, Palette::vt340(), Limits::default());
    let mut alloc = |w: u32, h: u32| (w as usize * h as usize * 4 <= 64 << 20).then(|| VecPixels::new(w, h));
    d.put_slice(data, &mut alloc);
    let size = d.finish(&mut alloc).image.map(|s| (s.width, s.height));
    (size, PEAK.load(Ordering::Relaxed) - start)
}

// Pins for what the review tried and found holding.

/// Repeat counts past u32 clamp to the largest width.
#[test]
fn sixel_giant_repeat_clamps() {
    let _g = lock();
    let (size, _) = sixel(b"#1!99999999999999~");
    assert_eq!(size, Some((term_image::MAX_SIDE, 6)));
}

/// Raster attributes with Pad = 0 (a division, if taken as is).
#[test]
fn sixel_raster_with_zero_pad() {
    let _g = lock();
    let (size, _) = sixel(b"\"1;0;5;5#1~");
    assert_eq!(size, Some((5, 6)));
    let (size, _) = sixel(b"\"0;0#1~");
    assert_eq!(size, Some((1, 6)));
}

/// 300,000 graphics newlines: painting past the limits is clipped, and
/// nothing large is allocated.
#[test]
fn sixel_band_flood_stays_within_limits() {
    let _g = lock();
    let data = [&b"#1~"[..], &b"-".repeat(300_000), b"~"].concat();
    let (size, used) = sixel(&data);
    let (w, h) = size.unwrap();
    assert!(w <= term_image::MAX_SIDE && h <= term_image::MAX_SIDE, "{w} × {h}");
    assert!(used <= 1 << 20, "{used} bytes at the peak");
}
