//! Sixel decode throughput (images.md §9: 150–300 MB/s expected), on a
//! 1000 × 1000 photo-like image encoded as img2sixel and chafa encode
//! (raster attributes, a pass per colour per band, repeats).

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use term_image::VecPixels;
use term_image::sixel::{Decoder, Limits, Palette};

#[path = "../tests/support/sixel_encode.rs"]
mod sixel_encode;

fn bench(c: &mut Criterion) {
    let (w, h) = (1000, 1000);
    let data = sixel_encode::encode(&sixel_encode::photo(w, h), w, h);
    let mut g = c.benchmark_group("sixel");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("photo_1000x1000", |b| {
        b.iter(|| {
            let mut d = Decoder::<VecPixels>::new(0, 1, Palette::vt340(), Limits::default());
            let mut alloc = |w, h| Some(VecPixels::new(w, h));
            // In the 64 KB slices the reader thread uses.
            for slice in data.chunks(64 * 1024) {
                d.put_slice(slice, &mut alloc);
            }
            black_box(d.finish(&mut alloc).image.unwrap().width)
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
