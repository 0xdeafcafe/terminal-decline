//! The pure-Rust PNG, JPEG and GIF decoding (term-image) on arbitrary
//! bytes, into a target of a size taken from the input: never panics, and
//! stays within its transient limit (the decoders' own checks; a crash
//! here is also a bug to report upstream).
//!
//! Run with `cargo +nightly fuzz run decode`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use term_image::VecPixels;
use term_image::decode::{Limits, decode};

fuzz_target!(|input: &[u8]| {
    let Some((&size, bytes)) = input.split_first() else { return };
    let (w, h) = (1 + (size & 0x0F) as u32 * 8, 1 + (size >> 4) as u32 * 8);
    let mut t = VecPixels::new(w, h);
    let _ = decode(bytes, &mut t, &Limits { transient: 16 << 20 });
});
