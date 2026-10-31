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

/// Typing at a zsh prompt, read by read: each key's echo, alone or after
/// the shell's line report (OSC 7701, docs/design/completion.md §3.1),
/// which zsh writes just before it. One iteration is a prompt, 40 keys
/// and the command running.
fn typing(c: &mut Criterion) {
    const TOKEN: &[u8; term_core::TOKEN_LEN] = b"0123456789abcdef0123456789abcdef";
    let line = "git commit -m \"fix the thing\" && ls -la";
    let session = |reports: bool| -> Vec<Vec<u8>> {
        let mut reads = vec![b"\x1b]133;D;0\x07\x1b]133;A\x07~/src/project main \xe2\x9d\xaf \x1b]133;B\x07".to_vec()];
        for (i, ch) in line.char_indices() {
            let end = i + ch.len_utf8();
            if reports {
                let text = line[..end].replace('%', "%25").replace(';', "%3B");
                let tok = std::str::from_utf8(TOKEN).unwrap();
                reads.push(format!("\x1b]7701;line;{tok};main;{end};{text}\x07").into_bytes());
            }
            reads.push(ch.to_string().into_bytes());
        }
        reads.push(b"\r\n\x1b]133;C\x07[main 1a2b3c4] fix the thing\r\n 1 file changed\r\n".to_vec());
        reads
    };
    let mut g = c.benchmark_group("typing");
    for (name, reports) in [("echo", false), ("echo+report", true)] {
        let reads = session(reports);
        g.throughput(Throughput::Elements(line.chars().count() as u64));
        g.bench_function(name, |b| {
            let mut t = Term::new(160, 50, 10_000, term_core::ImagePool::process());
            t.state.set_report_token(reports.then_some(*TOKEN));
            b.iter(|| {
                for read in &reads {
                    t.advance(black_box(read));
                    black_box(t.state.take_report_seen());
                }
                black_box(t.state.take_commands());
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench, typing);
criterion_main!(benches);
