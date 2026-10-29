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
    let mut t = Term::new(COLS, LINES, HISTORY, term_core::ImagePool::process());
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

    // Scrolling history after a width change (§12.3): 100k lines of 400
    // characters written at 300 columns (two stored lines each), read at
    // 300 (a row per stored line) and at 200 (laid out again). Each
    // iteration scrolls a page back or forward and takes a snapshot of
    // the whole view, nothing cached by the renderer.
    let mut g = c.benchmark_group("history_at_width");
    g.sample_size(20);
    let mut t = Term::new(COLS, LINES, HISTORY, term_core::ImagePool::process());
    let mut out = Vec::new();
    for i in 0..HISTORY / 2 {
        out.extend_from_slice(format!("{i:06} ").as_bytes());
        for _ in 0..9 {
            out.extend_from_slice(b"the quick brown fox jumps over the lazy dog ");
        }
        out.extend_from_slice(b"\r\n");
    }
    t.advance(&out);
    for cols in [COLS, 200] {
        t.state.resize(cols, LINES);
        t.state.scroll_view(HISTORY as isize / 2);
        let mut snap = term_core::Snapshot::default();
        let mut back = true;
        let mut n = 0;
        g.bench_function(format!("scroll_page_at_{cols}"), |b| {
            b.iter(|| {
                n += 1;
                if n % 50 == 0 {
                    back = !back;
                }
                t.state.scroll_view(if back { LINES as isize } else { -(LINES as isize) });
                t.state.snapshot(&mut snap, |_| false);
                black_box(snap.rows.len());
            })
        });
        g.bench_function(format!("display_offset_at_{cols}"), |b| {
            b.iter(|| black_box(t.state.display_offset()))
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
