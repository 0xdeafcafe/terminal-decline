//! Inputs on which the nightly `decode` fuzz target crashed, committed in
//! `fuzz/regressions/parse` (where the fuzz issues ask for them), fed to
//! [`decode`] as the target does: the first byte picks the target's size,
//! the rest is the image. Each must be refused cleanly, not panic.

use std::path::Path;
use term_image::VecPixels;
use term_image::decode::{DecodeError, Options, decode};

fn decode_input(name: &str) -> Result<(), DecodeError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/regressions/parse")
        .join(name);
    let input = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (&size, bytes) = input.split_first().expect("an empty input");
    let (w, h) = (1 + (size & 0x0F) as u32 * 8, 1 + (size >> 4) as u32 * 8);
    let mut t = VecPixels::new(w, h);
    decode(
        bytes,
        &mut t,
        &Options {
            transient: 16 << 20,
            ..Options::default()
        },
    )
    .map(|_| ())
}

#[test]
fn webp_animation_frames_with_a_picture_of_another_size_are_refused() {
    let cases = [
        // #5: an ALPH frame 64 × 48 whose VP8 picture is 4672 × 0
        // (image-webp's yuv.rs:90).
        "5-webp-alph-frame-wider-than-vp8",
        // #9: an ALPH frame 64 × 48 whose VP8 picture is 0 × 0
        // (image-webp's yuv.rs:230).
        "9-webp-alph-frame-empty-vp8",
    ];
    for name in cases {
        let r = decode_input(name);
        assert!(matches!(r, Err(DecodeError::Corrupt(_))), "{name}: {r:?}");
    }
}
