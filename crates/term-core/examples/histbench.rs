//! History decode cost in thread CPU time, for comparing two builds turn
//! about on a busy Mac (see `cpubench`): expanding each of 10k stored lines
//! back into cells, for the kinds of lines the `history` bench stores.
//!
//! `cargo run --release -p term-core --example histbench -- [filter] [runs]`
//!
//! Prints, per kind, the best and median of `runs` (15) passes, in µs.

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

/// This thread's CPU time in µs (wall time where there is no such clock).
fn cpu_us() -> f64 {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        #[repr(C)]
        struct Timespec {
            sec: i64,
            nsec: i64,
        }
        unsafe extern "C" {
            fn clock_gettime(clock: i32, tp: *mut Timespec) -> i32;
        }
        // CLOCK_THREAD_CPUTIME_ID.
        const CLOCK: i32 = if cfg!(target_os = "macos") { 16 } else { 3 };
        let mut t = Timespec { sec: 0, nsec: 0 };
        // SAFETY: `t` is a valid timespec for the call to fill in.
        if unsafe { clock_gettime(CLOCK, &mut t) } == 0 {
            return t.sec as f64 * 1e6 + t.nsec as f64 / 1e3;
        }
    }
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1e6
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let filter = args.get(1).cloned().unwrap_or_default();
    let runs: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15).max(1);
    for kind in ["plain", "ls", "cjk", "emoji", "background", "gradient"] {
        let name = format!("decode/{kind}");
        if !name.contains(&filter) {
            continue;
        }
        let (rows, styles) = rows(kind);
        let mut sb = Scrollback::new(LINES);
        for row in rows.iter().cycle().take(LINES) {
            sb.push(row, &styles);
        }
        let (mut row, mut row_styles) = (Row::default(), Vec::new());
        let mut times = Vec::with_capacity(runs);
        // One pass more, to warm up.
        for _ in 0..=runs {
            let start = cpu_us();
            for i in 0..sb.len() {
                sb.get_into(i, &mut row, &mut row_styles);
                black_box(row.cells.len());
            }
            times.push(cpu_us() - start);
        }
        times.remove(0);
        times.sort_by(f64::total_cmp);
        println!("{name} {:.0} {:.0}", times[0], times[times.len() / 2]);
    }
}
