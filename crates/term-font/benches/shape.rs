//! Cost of `Fonts::shape`: one cluster cold (CTLine) and from the
//! shaped-cluster cache, 160-cell rows of ASCII and of text with emoji and
//! combining marks both ways, and (ligatures design §5) rows of code at
//! 80, 160 and 300 cells in Menlo, which stays on the fast path, and in
//! fonts with ligatures, through one CTLine per row.
//!
//! JetBrains Mono is checked in. Fira Code, Cascadia Code, Iosevka and
//! Monaspace Neon are measured too when installed, or when
//! `TERM_FONT_BENCH_FONTS` names a folder of font files to register.
//! Before the timed cases it prints the one-off costs: building `Fonts`
//! with face detection, and a font's first CTLine, in the process and at a
//! new size.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;
use term_font::{Features, FontConfig, Fonts, RunText, Style, register_font_file};

const CLUSTERS: Features = Features { ligatures: false };
const LINE: Features = Features { ligatures: true };

const FAMILIES: [&str; 6] = ["Menlo", "JetBrains Mono", "Fira Code", "Cascadia Code", "Iosevka", "Monaspace Neon"];

/// Code as a terminal shows it (an editor, a diff, `cat`): Rust and
/// TypeScript, with the usual share of operators.
const CODE: &str = "    pub fn shape(&self, run: &RunText, face: Style, out: &mut Vec<ShapedGlyph>) -> bool { \
    if features.ligatures && self.shape_line(run, face, out) { return; } \
    for i in 0..run.len() { self.shape_cell(run, face, i, out); } } \
    const total = items.filter((x) => x.price >= 10 && x.id !== undefined).map((x) => x.price * 2); \
    // the cell's own glyph /* comment */ let next = (i + 1..n).find(|&c| own[c]).unwrap_or(n); \
    match kind[i] { Kind::Blank => {} Kind::Other => self.shape_cell(run, face, i, out), _ => {} } ";

/// `n` cells of `text`, from `skip` characters in.
fn row(text: &str, skip: usize, n: usize) -> RunText {
    let mut r = RunText::new();
    for ch in text.chars().cycle().skip(skip).take(n) {
        let mut buf = [0; 4];
        r.push(if ch == ' ' { "" } else { ch.encode_utf8(&mut buf) }, 1);
    }
    r
}

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

fn register() {
    let jb = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/JetBrainsMono-Regular.ttf");
    assert!(register_font_file(Path::new(jb)));
    if let Some(dir) = std::env::var_os("TERM_FONT_BENCH_FONTS") {
        for entry in std::fs::read_dir(dir).expect("TERM_FONT_BENCH_FONTS").flatten() {
            register_font_file(&entry.path());
        }
    }
}

/// Fonts for `family` at 13 pt × 2 with runs allowed, if installed.
fn fonts(family: &str, size: f64) -> Option<Fonts> {
    let mut f = Fonts::with_config(&FontConfig::new(family, size), 2.0);
    f.allow_runs(true);
    (f.family() == family).then_some(f)
}

fn us(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e6
}

/// One-off costs, printed: building `Fonts` (metrics, registry, face
/// detection), detection alone, the first 80-cell CTLine in the process
/// and the first at a new size (Cmd+=).
fn one_offs() {
    println!("one-off costs (µs): build, detection, first CTLine, first at a new size");
    let r = row(CODE, 0, 80);
    for family in FAMILIES {
        let t = Instant::now();
        let Some(f) = fonts(family, 13.0) else { continue };
        let build = us(t);
        let t = Instant::now();
        black_box(f.redetect_runs(&[]));
        let detect = us(t);
        let features = f.features(Style::Regular);
        let mut out = Vec::new();
        let t = Instant::now();
        f.shape(&r, Style::Regular, features, &mut out);
        let first = us(t);
        let g = fonts(family, 14.0).unwrap();
        let t = Instant::now();
        g.shape(&r, Style::Regular, features, &mut out);
        let resized = us(t);
        println!(
            "  {family:15} {build:8.1} {detect:8.1} {first:8.1} {resized:8.1}   runs: {}",
            features.ligatures
        );
    }
}

fn bench(c: &mut Criterion) {
    register();
    one_offs();
    let f = Fonts::new("Menlo", 26.0);
    let mut out = Vec::with_capacity(512);
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

    // Rows of code, each font's own way (`features`): Menlo per character,
    // the others one CTLine per row. Several rows in turn, as a frame
    // shapes them.
    let mut g = c.benchmark_group("ligatures");
    for family in FAMILIES {
        let Some(f) = fonts(family, 13.0) else { continue };
        let features = f.features(Style::Regular);
        for n in [80, 160, 300] {
            let rows: Vec<RunText> = (0..8).map(|k| row(CODE, k * 37, n)).collect();
            g.bench_function(format!("{family}, code {n}"), |b| {
                let mut k = 0;
                b.iter(|| {
                    out.clear();
                    k = (k + 1) % rows.len();
                    f.shape(black_box(&rows[k]), Style::Regular, features, &mut out);
                })
            });
        }
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
