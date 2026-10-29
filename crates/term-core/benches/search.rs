//! Searching 100k lines of history (§13.7): starting a search under the
//! lock, and the whole job (every block scanned, hits turned into
//! places), case-sensitive and ignoring case, for a rare word, a common
//! one and a non-ASCII query. History's filters (§13.6) are built first,
//! as the background job would (`scan/` is without them), and building
//! them is measured too.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use term_core::{Query, Term, WordMatch};

const LINES: usize = 100_000;

/// 100k lines of log-like output at 160 columns, one rare word in it.
fn terminal() -> Term {
    let mut t = Term::new(160, 50, LINES, term_core::ImagePool::process());
    let mut out = String::new();
    for i in 0..LINES {
        let word = if i == LINES / 3 { "needle" } else { "request" };
        out.push_str(&format!(
            "2026-10-29T12:{:02}:{:02}Z INFO server: handled {word} id={i} path=/api/v1/items status=200 took={}ms\r\n",
            (i / 60) % 60,
            i % 60,
            i % 97
        ));
        if out.len() > 1 << 20 {
            t.advance(out.as_bytes());
            out.clear();
        }
    }
    t.advance(out.as_bytes());
    t
}

fn bench(c: &mut Criterion) {
    let mut t = terminal();
    let mut plain = terminal();
    let mut g = c.benchmark_group("search_100k");
    g.sample_size(20);
    g.bench_function("filters/build_all", |b| {
        b.iter(|| {
            let mut job = t.state.filter_job(usize::MAX).expect("blocks without filters");
            job.run();
            black_box(job)
        });
    });
    let mut job = t.state.filter_job(usize::MAX).expect("blocks without filters");
    let blocks = job.len();
    job.run();
    t.state.add_filters(job);
    eprintln!("{blocks} blocks filtered");
    let cases = [
        ("rare", "needle", false),
        ("rare_ignore_case", "NEEDLE", true),
        ("common_ignore_case", "Server", true),
        ("absent", "zebra crossing", false),
        ("absent_ignore_case", "Zebra Crossing", true),
        ("non_ascii_ignore_case", "Σigma", true),
    ];
    for (name, text, ignore_case) in cases {
        let q = Query {
            text: text.into(),
            ignore_case,
            word: WordMatch::Contains,
        };
        g.bench_function(format!("start/{name}"), |b| {
            b.iter(|| black_box(t.state.start_search(q.clone())));
        });
        g.bench_function(format!("job/{name}"), |b| {
            b.iter(|| {
                let mut job = t.state.start_search(q.clone());
                let mut n = 0;
                job.run(|batch| n += batch.hits().len());
                black_box(n)
            });
        });
        // The same before the filters are built: every block scanned.
        g.bench_function(format!("scan/{name}"), |b| {
            b.iter(|| {
                let mut job = plain.state.start_search(q.clone());
                let mut n = 0;
                job.run(|batch| n += batch.hits().len());
                black_box(n)
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
