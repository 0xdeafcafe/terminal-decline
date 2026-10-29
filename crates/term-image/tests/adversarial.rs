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
