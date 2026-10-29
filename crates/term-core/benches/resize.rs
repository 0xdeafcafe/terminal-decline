//! Resize cost (§12.4 of the cells design): a 300×100 terminal with 100k
//! lines of history, made one column narrower and back (the screen is
//! reflowed, the line it starts in pulled back from history), and ten
//! rows shorter and back (rows pushed into history and pulled back). Each
//! iteration is two resizes.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use term_core::Term;

const COLS: usize = 300;
const LINES: usize = 100;
const HISTORY: usize = 100_000;

/// History of short lines, then a screen of long lines that wrap, the
/// first of them starting in history.
fn terminal(wide: bool) -> Term {
    let mut t = Term::new(COLS, LINES, HISTORY);
    let mut out = Vec::new();
    for i in 0..HISTORY {
        out.extend_from_slice(format!("line {i}: the quick brown fox jumps over the lazy dog\r\n").as_bytes());
    }
    t.advance(&out);
    out.clear();
    let word = if wide {
        "漢字かな交じり "
    } else {
        "lorem ipsum dolor sit amet "
    };
    for i in 0..60 {
        out.extend_from_slice(format!("\x1b[3{}m{i} ", i % 8).as_bytes());
        for _ in 0..(i % 7) * 4 + 3 {
            out.extend_from_slice(word.as_bytes());
        }
        out.extend_from_slice(b"\x1b[0m\r\n");
    }
    t.advance(&out);
    t
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("resize");
    for (name, wide) in [("ascii", false), ("cjk", true)] {
        let mut t = terminal(wide);
        g.bench_function(format!("width_300x100_{name}"), |b| {
            b.iter(|| {
                t.state.resize(COLS - 1, LINES);
                t.state.resize(COLS, LINES);
                black_box(t.state.cursor().row());
            })
        });
        let mut t = terminal(wide);
        g.bench_function(format!("height_300x100_{name}"), |b| {
            b.iter(|| {
                t.state.resize(COLS, LINES - 10);
                t.state.resize(COLS, LINES);
                black_box(t.state.cursor().row());
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
