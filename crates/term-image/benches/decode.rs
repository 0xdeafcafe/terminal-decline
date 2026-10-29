//! Pure-Rust decoding of images from programs (images.md §9: a 1624 × 1018
//! screenshot PNG in 5–15 ms expected; it runs on the decode queue, off
//! the lock): PNG at its own size and halved, JPEG at its own size and at
//! a quarter (DCT scaling), GIF.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use term_image::VecPixels;
use term_image::decode::{Options, decode};

/// A screenshot-like 1624 × 1018 RGBA PNG: flat areas with text-like
/// stripes.
fn screenshot() -> Vec<u8> {
    let (w, h) = (1624u32, 1018u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let text = (y / 4) % 5 == 1 && (x / 3 + y) % 7 < 4 && x % 400 < 330;
            let px = if x < 260 {
                [40, 44, 52, 255]
            } else if text {
                [200, 200, (x % 256) as u8, 255]
            } else {
                [30, 30, 30, 255]
            };
            data.extend_from_slice(&px);
        }
    }
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&data).unwrap();
    }
    out
}

fn bench(c: &mut Criterion) {
    let png = screenshot();
    let jpeg = include_bytes!("../tests/fixtures/photo-1600x1200.jpg");
    let gif = include_bytes!("../tests/fixtures/quadrants-64x48.gif");
    let l = Options::default();
    let mut g = c.benchmark_group("decode");
    g.sample_size(20);
    for (name, bytes, (w, h)) in [
        ("png_1624x1018", &png[..], (1624, 1018)),
        ("png_1624x1018_to_812x509", &png[..], (812, 509)),
        ("jpeg_1600x1200", &jpeg[..], (1600, 1200)),
        ("jpeg_1600x1200_to_400x300", &jpeg[..], (400, 300)),
        ("gif_64x48", &gif[..], (64, 48)),
    ] {
        let mut t = VecPixels::new(w, h);
        decode(bytes, &mut t, &l).unwrap();
        g.bench_function(name, |b| b.iter(|| black_box(decode(bytes, &mut t, &l).unwrap())));
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
