//! WebP golden images: each fixture (made with cwebp and img2webp from
//! generated PNGs) is decoded with `term_image::decode`, at its own size
//! and smaller, and compared pixel by pixel with
//! `tests/golden/webp_<name>.png` (straight RGBA, colour conversion off
//! and on). Run with `BLESS=1` to write the PNGs after checking a change
//! by eye. The decode unit tests check the same images by hand, within a
//! tolerance for lossy ones, so blessing can't hide a mistake.

use term_image::VecPixels;
use term_image::decode::{Options, decode};

#[path = "support/golden.rs"]
mod golden;

const FIXTURES: &[(&str, &[u8])] = &[
    ("lossy", include_bytes!("fixtures/quadrants-64x48-lossy.webp")),
    ("lossless", include_bytes!("fixtures/quadrants-64x48-lossless.webp")),
    ("alpha_lossy", include_bytes!("fixtures/alpha-64x48-lossy.webp")),
    ("alpha_lossless", include_bytes!("fixtures/alpha-64x48-lossless.webp")),
    ("animated", include_bytes!("fixtures/animated-64x48.webp")),
];

#[test]
fn golden_images() {
    let mut images = Vec::new();
    for &(name, bytes) in FIXTURES {
        for ((w, h), p3) in [((64, 48), false), ((24, 18), false), ((64, 48), true)] {
            let mut t = VecPixels::new(w, h);
            let options = Options {
                display_p3: p3,
                ..Options::default()
            };
            decode(bytes, &mut t, &options).unwrap_or_else(|e| panic!("{name}: {e}"));
            let suffix = if p3 { "_p3".to_string() } else { format!("_{w}x{h}") };
            images.push((format!("webp_{name}{suffix}"), w, h, t.to_rgba()));
        }
    }
    golden::check(images);
}
