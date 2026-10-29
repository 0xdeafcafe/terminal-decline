//! The streaming paths of a kitty transmission (images.md §9: ≥ 500 MB/s
//! of payload expected for raw RGBA): base64 alone, base64 then RGBA into
//! pixels in 4 KB chunks as kitty sends them, and base64 then inflate.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use term_image::VecPixels;
use term_image::base64::Base64;
use term_image::convert::{Converter, Layout};
use term_image::inflate::Inflate;

fn encode(data: &[u8]) -> Vec<u8> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len() / 3 * 4 + 4);
    for c in data.chunks(3) {
        let v = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(alphabet[(v >> 18) as usize & 63]);
        out.push(alphabet[(v >> 12) as usize & 63]);
        out.push(if c.len() > 1 { alphabet[(v >> 6) as usize & 63] } else { b'=' });
        out.push(if c.len() > 2 { alphabet[v as usize & 63] } else { b'=' });
    }
    out
}

fn rgba(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&[
                (x % 256) as u8,
                (y % 256) as u8,
                ((x + y) % 256) as u8,
                if x % 7 == 0 { 128 } else { 255 },
            ]);
        }
    }
    v
}

fn bench(c: &mut Criterion) {
    // A full-HD RGBA image, 8 MB, in kitty's 4096-byte base64 chunks.
    let (w, h) = (1920, 1080);
    let raw = rgba(w, h);
    let chunks: Vec<Vec<u8>> = raw.chunks(3072).map(encode).collect();
    let payload: usize = chunks.iter().map(Vec::len).sum();

    let mut g = c.benchmark_group("stream");
    g.throughput(Throughput::Bytes(payload as u64));
    g.bench_function("base64_8mb", |b| {
        let mut out = Vec::with_capacity(raw.len());
        b.iter(|| {
            out.clear();
            for chunk in &chunks {
                let mut d = Base64::new();
                d.feed(chunk, &mut out).unwrap();
                d.finish(&mut out).unwrap();
            }
            black_box(out.len())
        })
    });
    g.bench_function("kitty_rgba_8mb", |b| {
        let mut target = VecPixels::new(w, h);
        b.iter(|| {
            let mut conv = Converter::new(Layout::Rgba, w, h);
            for chunk in &chunks {
                let mut d = Base64::new();
                d.feed_to(chunk, &mut |bytes| conv.feed(bytes, &mut target)).unwrap();
                d.finish_to(&mut |bytes| conv.feed(bytes, &mut target)).unwrap();
            }
            black_box(conv.finish().unwrap())
        })
    });

    // A screenshot-like image compressed: base64, inflate, convert.
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let zchunks: Vec<Vec<u8>> = z.chunks(3072).map(encode).collect();
    let zpayload: usize = zchunks.iter().map(Vec::len).sum();
    g.throughput(Throughput::Bytes(zpayload as u64));
    g.bench_function("kitty_zlib_rgba", |b| {
        let mut target = VecPixels::new(w, h);
        b.iter(|| {
            let mut conv = Converter::new(Layout::Rgba, w, h);
            let mut z = Inflate::new(raw.len());
            for chunk in &zchunks {
                let mut d = Base64::new();
                let mut bytes = Vec::with_capacity(3072);
                d.feed(chunk, &mut bytes).unwrap();
                d.finish(&mut bytes).unwrap();
                z.feed_to(&bytes, &mut |out| conv.feed(out, &mut target)).unwrap();
            }
            z.finish().unwrap();
            black_box(conv.finish().unwrap())
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
