//! Parse throughput for the kinds of output terminals actually see.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use term_core::Term;

mod inputs;

fn bench(c: &mut Criterion) {
    const SIZE: usize = 4 << 20;
    let inputs = inputs::all(SIZE);
    // `parse`: 64 KB per `advance` (the reader's slice size). `parse_1k`:
    // 1 KB per call, what a macOS PTY returns per read, so per-call costs
    // and runs cut at chunk ends count.
    for (group, chunk) in [("parse", 64 * 1024), ("parse_1k", 1024)] {
        let mut g = c.benchmark_group(group);
        g.sample_size(20);
        for (name, data) in &inputs {
            g.throughput(Throughput::Bytes(data.len() as u64));
            g.bench_with_input(BenchmarkId::from_parameter(name), data, |b, data| {
                // A full scrollback, so trimming cost is included.
                let mut t = Term::new(160, 50, 10_000, term_core::ImagePool::process());
                b.iter(|| {
                    for piece in data.chunks(chunk) {
                        t.advance(black_box(piece));
                    }
                });
            });
        }
        g.finish();
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
