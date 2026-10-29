//! Cost of `Fonts::shape`: one cluster cold (CTLine) and from the
//! shaped-cluster cache, and 160-cell rows of ASCII, of text with emoji
//! and combining marks, and through one CTLine as B.1's ligature rows will.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use term_font::{Features, Fonts, RunText, Style};

const CLUSTERS: Features = Features { ligatures: false };
const LINE: Features = Features { ligatures: true };

/// 160 cells of ASCII.
fn ascii() -> RunText {
    let mut r = RunText::new();
    for i in 0..160u8 {
        r.push_char((b'!' + i % 90) as char, 1);
    }
    r
}

/// 160 cells: ASCII with an emoji sequence or combining mark every 10
/// cells (16 clusters, 5 different).
fn mixed() -> RunText {
    let clusters = [
        ("👨\u{200D}👩\u{200D}👧\u{200D}👦", 2),
        ("👍🏽", 2),
        ("🇬🇧", 2),
        ("e\u{301}", 1),
        ("x\u{301}", 1),
    ];
    let mut r = RunText::new();
    let mut i = 0;
    while r.width() < 160 {
        if i % 10 == 9 {
            let (s, w) = clusters[(i / 10) % clusters.len()];
            r.push(s, w);
        } else {
            r.push_char((b'a' + (i % 26) as u8) as char, 1);
        }
        i += 1;
    }
    r
}

fn bench(c: &mut Criterion) {
    let f = Fonts::new("Menlo", 26.0);
    let mut out = Vec::with_capacity(256);
    let family = {
        let mut r = RunText::new();
        r.push("👨\u{200D}👩\u{200D}👧\u{200D}👦", 2);
        r
    };
    let combining = {
        let mut r = RunText::new();
        r.push("x\u{301}", 1);
        r
    };
    let mut g = c.benchmark_group("shape");
    for (name, run) in [("zwj family", &family), ("combining mark", &combining)] {
        g.bench_function(format!("{name}, cold"), |b| {
            b.iter(|| {
                f.clear_shaped_clusters();
                out.clear();
                f.shape(black_box(run), Style::Regular, CLUSTERS, &mut out);
            })
        });
        g.bench_function(format!("{name}, cached"), |b| {
            b.iter(|| {
                out.clear();
                f.shape(black_box(run), Style::Regular, CLUSTERS, &mut out);
            })
        });
    }
    let (ascii, mixed) = (ascii(), mixed());
    for (name, run) in [("160 ascii", &ascii), ("160 mixed", &mixed)] {
        g.bench_function(format!("{name}, clusters cached"), |b| {
            b.iter(|| {
                out.clear();
                f.shape(black_box(run), Style::Regular, CLUSTERS, &mut out);
            })
        });
        g.bench_function(format!("{name}, one CTLine"), |b| {
            b.iter(|| {
                out.clear();
                f.shape(black_box(run), Style::Regular, LINE, &mut out);
            })
        });
    }
    g.bench_function("160 mixed, clusters cold", |b| {
        b.iter(|| {
            f.clear_shaped_clusters();
            out.clear();
            f.shape(black_box(&mixed), Style::Regular, CLUSTERS, &mut out);
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
