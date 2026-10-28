//! Parse throughput for the kinds of output terminals actually see.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use term_core::Term;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Plain text lines, like `cat` of source code.
fn plain(size: usize) -> Vec<u8> {
    let mut r = Rng(1);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let len = 20 + (r.next() % 80) as usize;
        for _ in 0..len {
            out.push(b' ' + (r.next() % 94) as u8);
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Coloured output, like `ls --color` or compiler errors.
fn coloured(size: usize) -> Vec<u8> {
    let mut r = Rng(2);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        for _ in 0..6 {
            let c = r.next() % 256;
            out.extend_from_slice(format!("\x1b[38;5;{c}m").as_bytes());
            for _ in 0..(3 + r.next() % 10) {
                out.push(b'a' + (r.next() % 26) as u8);
            }
            out.extend_from_slice(b"\x1b[0m ");
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Full-screen redraws with cursor moves and truecolour, like vim or htop.
fn tui(size: usize) -> Vec<u8> {
    let mut r = Rng(3);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let row = 1 + r.next() % 50;
        let col = 1 + r.next() % 150;
        let (cr, cg, cb) = (r.next() % 256, r.next() % 256, r.next() % 256);
        out.extend_from_slice(format!("\x1b[{row};{col}H\x1b[48;2;{cr};{cg};{cb}m").as_bytes());
        for _ in 0..(5 + r.next() % 40) {
            out.push(b'a' + (r.next() % 26) as u8);
        }
        out.extend_from_slice(b"\x1b[0m\x1b[K");
    }
    out
}

/// Wide characters and emoji.
fn unicode(size: usize) -> Vec<u8> {
    let s = "漢字かなカナ한글 😀 Привет мир ┌─┐│└┘ ";
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        out.extend_from_slice(s.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

fn bench(c: &mut Criterion) {
    const SIZE: usize = 4 << 20;
    let inputs = [
        ("plain", plain(SIZE)),
        ("coloured", coloured(SIZE)),
        ("tui", tui(SIZE)),
        ("unicode", unicode(SIZE)),
    ];
    let mut g = c.benchmark_group("parse");
    g.sample_size(20);
    for (name, data) in &inputs {
        g.throughput(Throughput::Bytes(data.len() as u64));
        g.bench_with_input(BenchmarkId::from_parameter(name), data, |b, data| {
            // A full scrollback, so trimming cost is included.
            let mut t = Term::new(160, 50, 10_000);
            b.iter(|| {
                for chunk in data.chunks(64 * 1024) {
                    t.advance(black_box(chunk));
                }
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
