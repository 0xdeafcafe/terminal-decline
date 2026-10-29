//! The Sixel decoder (term-image) on arbitrary data after `DCS P1;P2 q`,
//! with a 1 MB budget: it must never panic, never ask for a buffer past
//! the budget (it is refused, and the decoder must stop), and an image
//! it returns must fit its buffer.
//!
//! Run with `cargo +nightly fuzz run sixel`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use term_image::sixel::{Decoder, Limits, Palette};
use term_image::{PixelTarget, VecPixels};

const BUDGET: usize = 1 << 20;

fuzz_target!(|input: &[u8]| {
    let Some((&head, data)) = input.split_first() else { return };
    let (p1, p2) = ((head & 0x0F) as u16, (head >> 4) as u16 % 3);
    let mut d = Decoder::<VecPixels>::new(p1, p2, Palette::vt340(), Limits::default());
    let mut held = 0usize;
    let mut alloc = |w: u32, h: u32| {
        let bytes = w as usize * h as usize * 4;
        if bytes > BUDGET {
            return None;
        }
        held = held.max(bytes);
        Some(VecPixels::new(w, h))
    };
    // In a few slices, as the reader hands them over.
    for slice in data.chunks(97) {
        d.put_slice(slice, &mut alloc);
    }
    let failed = d.failed();
    let f = d.finish(&mut alloc);
    if let Some(img) = f.image {
        assert!(!failed);
        assert!(img.width <= img.pixels.width() && img.height <= img.pixels.height());
        assert!(img.width > 0 && img.height > 0);
    }
    assert!(held <= BUDGET);
});
