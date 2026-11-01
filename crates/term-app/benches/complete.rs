//! Completion and colouring at the prompt (docs/design/completion.md
//! §11): reading the command line into words, which runs on the main
//! thread on every key. Budget: reading and colouring a 200-character
//! line in 20 µs.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use terminal_decline_app::complete::lex;

/// A typical line, then the same kind of line grown to `len` bytes.
fn line(len: usize) -> String {
    const PIECES: &[&str] = &[
        "git commit -m \"fix the thing; again\"",
        " && ls -la ~/src/project",
        " | grep -v $HOME",
        " > ./out.txt 2>&1",
        " ; FOO=1 sudo -u root make -j8 all",
        " && echo \"${PATH}\" $(date +%s)",
    ];
    let mut s = String::new();
    while s.len() < len {
        s.push_str(PIECES[s.len() % PIECES.len()]);
    }
    let mut end = len.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("lex");
    for len in [20, 200, 4000] {
        let text = line(len);
        g.throughput(Throughput::Bytes(text.len() as u64));
        g.bench_with_input(BenchmarkId::from_parameter(len), &text, |b, text| {
            b.iter(|| black_box(lex::read(black_box(text), text.len())));
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
