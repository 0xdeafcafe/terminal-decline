//! The WebP decoder's allocations, counted: image-webp doesn't check its
//! memory limit for its pixel buffers, so `decode` checks an estimate
//! first (images.md §6.1). Here the real peak is measured against that
//! estimate, so the cap holds.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use term_image::VecPixels;
use term_image::decode::{Options, decode};

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

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

/// Bytes allocated at the peak of decoding `bytes` into a target of
/// `size` (allocated before), beyond what was allocated at the start.
fn peak(bytes: &[u8], size: (u32, u32)) -> usize {
    let mut t = VecPixels::new(size.0, size.1);
    let options = Options::default();
    let start = NOW.load(Ordering::Relaxed);
    PEAK.store(start, Ordering::Relaxed);
    decode(bytes, &mut t, &options).unwrap();
    PEAK.load(Ordering::Relaxed) - start
}

/// Name, file, size, output bytes a pixel, the decoder's own bytes a pixel.
type Case<'a> = (&'a str, &'a [u8], (u32, u32), usize, usize);

/// One test, so no other test's allocations run alongside.
#[test]
fn webp_stays_within_its_estimate() {
    // A 1000 × 800 lossless image with no alpha (decoded as RGBA, then
    // copied to RGB).
    let (w, h) = (1000u32, 800u32);
    let rgb: Vec<u8> = (0..w * h)
        .flat_map(|i| [(i % w) as u8, (i / w) as u8, (i % 7) as u8 * 30])
        .collect();
    let mut lossless = Vec::new();
    image_webp::WebPEncoder::new(&mut lossless)
        .encode(&rgb, w, h, image_webp::ColorType::Rgb8)
        .unwrap();
    let cases: [Case; 5] = [
        ("lossy photo", include_bytes!("fixtures/photo-1600x1200.webp"), (1600, 1200), 3, 4),
        ("lossless, no alpha", &lossless, (1000, 800), 3, 4),
        (
            "lossy with alpha",
            include_bytes!("fixtures/alpha-64x48-lossy.webp"),
            (64, 48),
            4,
            4,
        ),
        (
            "lossless with alpha",
            include_bytes!("fixtures/alpha-64x48-lossless.webp"),
            (64, 48),
            4,
            4,
        ),
        ("animated", include_bytes!("fixtures/animated-64x48.webp"), (64, 48), 4, 12),
    ];
    for (name, bytes, (w, h), out, own) in cases {
        let px = w as usize * h as usize;
        // The estimate decode() checks, plus the colour tables and the
        // scaler's rows (a few KB, fixed or per row).
        let estimate = px * (out + own) + 64 * 1024 + w as usize * 16;
        let used = peak(bytes, (w / 4, h / 4));
        assert!(used <= estimate, "{name}: {used} bytes at the peak, estimate {estimate}");
        eprintln!("{name}: {used} of {estimate} bytes");
    }
}
