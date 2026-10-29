//! Header sniffing and sizes, run under the terminal's lock (images.md §9:
//! under 1 µs expected).

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use term_image::probe::probe;

fn png() -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    v.extend_from_slice(&1624u32.to_be_bytes());
    v.extend_from_slice(&1018u32.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    v
}

/// A phone photo's header: JFIF, a 12 KB EXIF block with the orientation,
/// a 3 KB ICC profile, tables, then the frame header.
fn jpeg() -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
    v.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
    exif.resize(12 * 1024, 0);
    v.extend_from_slice(&[0xFF, 0xE1]);
    v.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    v.extend(exif);
    v.extend_from_slice(&[0xFF, 0xE2]);
    v.extend_from_slice(&(3002u16).to_be_bytes());
    v.extend(std::iter::repeat_n(0, 3000));
    for _ in 0..2 {
        v.extend_from_slice(&[0xFF, 0xDB, 0, 67]);
        v.extend(std::iter::repeat_n(1, 65));
    }
    v.extend_from_slice(&[0xFF, 0xC0, 0, 17, 8, 0x0B, 0xD0, 0x0F, 0xC0, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    v
}

fn gif() -> Vec<u8> {
    b"GIF89a\x40\x01\xf0\x00\xf7\0\0".to_vec()
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("probe");
    for (name, data) in [("png", png()), ("jpeg_exif", jpeg()), ("gif", gif())] {
        assert!(probe(&data).is_ok(), "{name}");
        g.bench_function(name, |b| b.iter(|| black_box(probe(black_box(&data)))));
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
