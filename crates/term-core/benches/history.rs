//! History (scrollback) cost per line: storing a row as it scrolls off the
//! screen, and expanding a stored line back into cells, for the kinds of
//! lines `scrollback_mem` measures (160 columns).

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use term_core::scrollback::Scrollback;
use term_core::{Attrs, Cell, Color, Row, Style, Width};

const COLS: usize = 160;
const LINES: usize = 10_000;

fn template(styles: &[Style], id: usize) -> Cell {
    Cell::template(id as u32, styles[id].shows_empty())
}

/// Write `text` from column `at` in style `id`; wide characters get a
/// spacer. Returns the column after it.
fn put(row: &mut Row, styles: &[Style], at: usize, id: usize, text: &str) -> usize {
    let t = template(styles, id);
    let mut col = at;
    for ch in text.chars() {
        if unicode_width::UnicodeWidthChar::width(ch) == Some(2) {
            row.cells[col] = t.with_char(ch).with_width(Width::Wide);
            row.cells[col + 1] = t.with_width(Width::Spacer);
            col += 2;
        } else {
            row.cells[col] = t.with_char(ch);
            col += 1;
        }
    }
    col
}

/// 16 rows of one kind of output, and the styles their ids index.
fn rows(kind: &str) -> (Vec<Row>, Vec<Style>) {
    let bold_blue = Style {
        fg: Color::indexed(4),
        attrs: Attrs::BOLD,
        ..Style::DEFAULT
    };
    let fg = |i| Style {
        fg: Color::indexed(i),
        ..Style::DEFAULT
    };
    let mut styles = vec![Style::DEFAULT, bold_blue, fg(2), fg(1)];
    let mut out = Vec::new();
    for i in 0..16 {
        let mut row = Row::new(COLS, Cell::BLANK);
        match kind {
            "plain" => {
                put(&mut row, &styles, 0, 0, &format!("{i:08} the quick brown fox jumps over"));
            }
            "ls" => {
                let mut c = put(&mut row, &styles, 0, 1, &format!("dir{i:05}"));
                c = put(&mut row, &styles, c, 0, "  ");
                c = put(&mut row, &styles, c, 2, "build.sh");
                c = put(&mut row, &styles, c, 0, "  notes.txt  ");
                c = put(&mut row, &styles, c, 1, "src");
                c = put(&mut row, &styles, c, 0, "  ");
                put(&mut row, &styles, c, 3, "archive.tar");
            }
            "cjk" => {
                put(&mut row, &styles, 0, 0, &format!("{i:05} 漢字かなカナ한글 混合テキスト 中文字符"));
            }
            "emoji" => {
                let c = put(&mut row, &styles, 0, 0, &format!("{i:05} done "));
                let t = template(&styles, 0);
                // Four two-cell clusters: a skin tone, a flag, VS16, a ZWJ family.
                for (k, cluster) in ["👍🏽", "🇬🇧", "❤\u{fe0f}", "👨\u{200d}👩\u{200d}👧"].iter().enumerate() {
                    let col = c + k * 3;
                    row.cells[col] = t.with_char(cluster.chars().next().unwrap()).with_width(Width::Wide);
                    row.cells[col + 1] = t.with_width(Width::Spacer);
                    row.set_cluster(col, cluster);
                }
            }
            "background" => {
                if styles.len() == 4 {
                    styles.push(Style {
                        bg: Color::indexed(22),
                        ..Style::DEFAULT
                    });
                }
                let t = template(&styles, 4);
                row.cells.fill(t);
                put(&mut row, &styles, 0, 4, &format!("+{i:05}"));
            }
            "gradient" => {
                // A different RGB foreground in every cell.
                if styles.len() == 4 {
                    for n in 0..COLS * 16 {
                        styles.push(Style {
                            fg: Color::rgb((n >> 8) as u8, n as u8, 128),
                            ..Style::DEFAULT
                        });
                    }
                }
                for c in 0..COLS {
                    let id = 4 + i * COLS + c;
                    row.cells[c] = template(&styles, id).with_char('█');
                }
            }
            _ => unreachable!(),
        }
        out.push(row);
    }
    (out, styles)
}

fn bench(c: &mut Criterion) {
    let kinds = ["plain", "ls", "cjk", "emoji", "background", "gradient"];
    let mut push = c.benchmark_group("history push");
    for kind in kinds {
        let (rows, styles) = rows(kind);
        push.throughput(Throughput::Elements(LINES as u64));
        push.bench_with_input(BenchmarkId::from_parameter(kind), &rows, |b, rows| {
            // Full, so freeing the oldest lines is included.
            let mut sb = Scrollback::new(LINES);
            b.iter(|| {
                for row in rows.iter().cycle().take(LINES) {
                    sb.push(black_box(row), &styles);
                }
            });
        });
    }
    // As the terminal stores rows: one style table, so the slot cache is
    // kept from line to line.
    for kind in kinds {
        let (rows, styles) = rows(kind);
        push.bench_with_input(BenchmarkId::new("keyed", kind), &rows, |b, rows| {
            let mut sb = Scrollback::new(LINES);
            b.iter(|| {
                for row in rows.iter().cycle().take(LINES) {
                    sb.push_keyed(black_box(row), &styles, [1, 0]);
                }
            });
        });
    }
    push.finish();

    let mut decode = c.benchmark_group("history decode");
    for kind in kinds {
        let (rows, styles) = rows(kind);
        let mut sb = Scrollback::new(LINES);
        for row in rows.iter().cycle().take(LINES) {
            sb.push(row, &styles);
        }
        decode.throughput(Throughput::Elements(LINES as u64));
        decode.bench_function(BenchmarkId::from_parameter(kind), |b| {
            let (mut row, mut row_styles) = (Row::default(), Vec::new());
            b.iter(|| {
                for i in 0..sb.len() {
                    sb.get_into(i, &mut row, &mut row_styles);
                    black_box(row.cells.len());
                }
            });
        });
    }
    decode.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
