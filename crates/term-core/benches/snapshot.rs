//! Cost of a snapshot under the lock (§9.2 of the cells design): a full
//! copy (a new renderer, or every row changed), a spinner (one row
//! changed), nothing changed, and a view scrolled into history.

use criterion::{Criterion, criterion_group, criterion_main};
use std::collections::HashSet;
use std::hint::black_box;
use term_core::{RowKey, Snapshot, Term};

/// A screen of coloured text, `lines` rows of history below the screen's
/// worth.
fn screen(cols: usize, lines: usize, history: usize) -> Term {
    let mut t = Term::new(cols, lines, history.max(1));
    let mut s = String::new();
    for l in 0..lines + history {
        for c in 0..cols {
            if c % 7 == 0 {
                s.push_str(&format!("\x1b[38;5;{}m", (l + c) % 256));
            }
            s.push((b'!' + ((l * cols + c) % 90) as u8) as char);
        }
        s.push_str("\r\n");
    }
    t.advance(s.as_bytes());
    t
}

fn bench(c: &mut Criterion) {
    for (cols, lines) in [(80usize, 24usize), (300, 100)] {
        let mut t = screen(cols, lines, 0);
        let mut snap = Snapshot::default();
        c.bench_function(&format!("snapshot {cols}x{lines} full copy"), |b| {
            b.iter(|| {
                t.state.snapshot(&mut snap, |_| false);
                black_box(snap.cells.len())
            })
        });

        // A renderer that caches every row it is given.
        let mut cache: HashSet<RowKey> = HashSet::new();
        let take = |t: &mut Term, snap: &mut Snapshot, cache: &mut HashSet<RowKey>| {
            t.state.snapshot(snap, |k| cache.contains(&k));
            cache.extend(snap.rows.iter().filter(|r| r.cells.is_some()).map(|r| r.key));
        };
        take(&mut t, &mut snap, &mut cache);
        c.bench_function(&format!("snapshot {cols}x{lines} unchanged"), |b| {
            b.iter(|| {
                take(&mut t, &mut snap, &mut cache);
                black_box(snap.cells.len())
            })
        });
        let frames = [&b"\x1b[5;1H|"[..], b"\x1b[5;1H/", b"\x1b[5;1H-", b"\x1b[5;1H\\"];
        let mut i = 0;
        c.bench_function(&format!("snapshot {cols}x{lines} spinner"), |b| {
            b.iter(|| {
                t.advance(frames[i % 4]);
                i += 1;
                take(&mut t, &mut snap, &mut cache);
                black_box(snap.cells.len())
            })
        });
    }

    let mut t = screen(300, 100, 200);
    t.state.scroll_view(150);
    let mut snap = Snapshot::default();
    c.bench_function("snapshot 300x100 history full copy", |b| {
        b.iter(|| {
            t.state.snapshot(&mut snap, |_| false);
            black_box(snap.cells.len())
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
