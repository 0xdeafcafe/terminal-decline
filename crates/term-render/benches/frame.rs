//! CPU cost of turning a screen into GPU instances: the snapshot taken
//! under the lock plus building from it (cells design §9), when every row
//! changed, when none did, when one row did (a spinner) and when the
//! screen scrolled by a line; and the reference path that reads the live
//! terminal (`build_live`, what frames were built with before); and one
//! 160×48 pane against four 80×24 panes.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use term_core::Term;
use term_font::{Metrics, Style};
use term_render::{Frame, FrameInput, GlyphSlot, GlyphSource, Theme};

/// Every glyph is already in the texture, as in steady state.
struct Cached;
impl GlyphSource for Cached {
    #[inline]
    fn glyph(&mut self, cp: u32, _style: Style) -> Option<GlyphSlot> {
        Some(GlyphSlot {
            u: (cp % 64) as u16 * 16,
            v: 0,
            w: 14,
            h: 28,
            left: 1,
            top: 22,
            color: false,
        })
    }
}

/// How the bench screen is coloured.
#[derive(Clone, Copy, PartialEq)]
enum Colours {
    Plain,
    /// A new colour pair every 7 cells, like syntax-highlighted code.
    Coloured,
    /// Bold every other cell, plain in between: the style changes on every
    /// cell, but there are only two styles.
    Alternating,
    /// CJK, accented letters, emoji and grapheme clusters (combining
    /// marks, skin tones, flags, ZWJ sequences) among plain text.
    Unicode,
}

/// Pieces of the unicode screen, with their widths in cells.
const UNICODE: &[(&str, usize)] = &[
    ("漢字", 4),
    (" ", 1),
    ("cafe\u{301}", 4),
    (" ", 1),
    ("❤\u{FE0F}", 2),
    ("👍\u{1F3FD}", 2),
    (" ", 1),
    ("🇬🇧", 2),
    (" ", 1),
    ("👨\u{200D}👩\u{200D}👧", 2),
    (" naïve ", 7),
    ("Ελλάδα", 6),
    (" ", 1),
];

fn line(l: usize, cols: usize, colours: Colours, s: &mut String) {
    if colours == Colours::Unicode {
        let mut used = 0;
        for (piece, w) in UNICODE.iter().cycle().skip(l % UNICODE.len()) {
            if used + w > cols {
                break;
            }
            s.push_str(piece);
            used += w;
        }
        return;
    }
    for c in 0..cols {
        if colours == Colours::Coloured && c % 7 == 0 {
            s.push_str(&format!("\x1b[38;5;{};48;5;{}m", (l + c) % 256, (l * 3 + c) % 256));
        }
        if colours == Colours::Alternating {
            s.push_str(if c % 2 == 0 { "\x1b[1;31m" } else { "\x1b[0m" });
        }
        s.push((b'!' + ((l * cols + c) % 90) as u8) as char);
    }
}

fn screen(cols: usize, lines: usize, colours: Colours) -> Term {
    let mut t = Term::new(cols, lines, 0);
    let mut s = String::new();
    for l in 0..lines {
        line(l, cols, colours, &mut s);
        if l + 1 < lines {
            s.push_str("\r\n");
        }
    }
    t.advance(s.as_bytes());
    t
}

fn bench(c: &mut Criterion) {
    let theme = Theme::default();
    let metrics = Metrics {
        cell_width: 16,
        cell_height: 34,
        baseline: 26,
        underline_offset: 3,
        underline_thickness: 2,
        strike_offset: 8,
    };
    let input = FrameInput {
        theme: &theme,
        metrics,
        padding: (12, 12),
        cursor_on: true,
        focused: true,
        preedit: None,
    };
    let cases = [
        ("80x24 plain", 80, 24, Colours::Plain),
        ("300x100 plain", 300, 100, Colours::Plain),
        ("300x100 coloured", 300, 100, Colours::Coloured),
        ("300x100 per-cell style", 300, 100, Colours::Alternating),
        ("300x100 unicode", 300, 100, Colours::Unicode),
    ];
    for (name, cols, lines, colours) in cases {
        let mut t = screen(cols, lines, colours);
        let mut frame = Frame::default();
        // Every row changed: all copied and built, as after a full redraw.
        c.bench_function(&format!("frame {name}"), |b| {
            b.iter(|| {
                frame.forget_rows();
                frame.build(&mut t.state, black_box(&input), &mut Cached);
                black_box(frame.glyphs.len())
            })
        });
        // Nothing changed (a cursor blink): every row from the cache.
        c.bench_function(&format!("frame {name} unchanged"), |b| {
            b.iter(|| {
                frame.build(&mut t.state, black_box(&input), &mut Cached);
                black_box(frame.glyphs.len())
            })
        });
        let mut live = Frame::default();
        c.bench_function(&format!("frame {name} live"), |b| {
            b.iter(|| {
                live.build_live(&t.state, black_box(&input), &mut Cached);
                black_box(live.glyphs.len())
            })
        });
    }
    // One window, one 160×48 pane or four 80×24 panes (splits design
    // §3.4): the same cells, four terminals and frames.
    let mut t = screen(160, 48, Colours::Coloured);
    let mut frame = Frame::default();
    c.bench_function("frame 1 pane 160x48 coloured", |b| {
        b.iter(|| {
            frame.forget_rows();
            frame.build(&mut t.state, black_box(&input), &mut Cached);
            black_box(frame.glyphs.len())
        })
    });
    c.bench_function("frame 1 pane 160x48 coloured unchanged", |b| {
        b.iter(|| {
            frame.build(&mut t.state, black_box(&input), &mut Cached);
            black_box(frame.glyphs.len())
        })
    });
    let mut terms: Vec<Term> = (0..4).map(|_| screen(80, 24, Colours::Coloured)).collect();
    let mut frames: Vec<Frame> = (0..4).map(|_| Frame::default()).collect();
    c.bench_function("frame 4 panes 80x24 coloured", |b| {
        b.iter(|| {
            for (f, t) in frames.iter_mut().zip(&mut terms) {
                f.forget_rows();
                f.build(&mut t.state, black_box(&input), &mut Cached);
            }
            black_box(frames[3].glyphs.len())
        })
    });
    c.bench_function("frame 4 panes 80x24 coloured unchanged", |b| {
        b.iter(|| {
            for (f, t) in frames.iter_mut().zip(&mut terms) {
                f.build(&mut t.state, black_box(&input), &mut Cached);
            }
            black_box(frames[3].glyphs.len())
        })
    });

    for (name, cols, lines, colours) in [cases[0], cases[2]] {
        // One row changes per frame: a spinner on the middle row.
        let mut t = screen(cols, lines, colours);
        let mut frame = Frame::default();
        let spinner: Vec<String> = "|/-\\".chars().map(|ch| format!("\x1b[{};1H{ch}", lines / 2)).collect();
        let mut i = 0;
        c.bench_function(&format!("frame {name} one row"), |b| {
            b.iter(|| {
                i = (i + 1) % spinner.len();
                t.advance(spinner[i].as_bytes());
                frame.build(&mut t.state, black_box(&input), &mut Cached);
                black_box(frame.glyphs.len())
            })
        });
        // The same through the reference path, which rebuilds every row.
        let mut live = Frame::default();
        c.bench_function(&format!("frame {name} one row live"), |b| {
            b.iter(|| {
                i = (i + 1) % spinner.len();
                t.advance(spinner[i].as_bytes());
                live.build_live(&t.state, black_box(&input), &mut Cached);
                black_box(live.glyphs.len())
            })
        });

        // A new line at the bottom: every row moves, one is new.
        let mut t = screen(cols, lines, colours);
        let lines_in: Vec<String> = (0..16)
            .map(|l| {
                let mut s = String::from("\r\n");
                line(l, cols, colours, &mut s);
                s
            })
            .collect();
        let mut frame = Frame::default();
        c.bench_function(&format!("frame {name} scrolled"), |b| {
            b.iter(|| {
                i = (i + 1) % lines_in.len();
                t.advance(lines_in[i].as_bytes());
                frame.build(&mut t.state, black_box(&input), &mut Cached);
                black_box(frame.glyphs.len())
            })
        });
        let mut live = Frame::default();
        c.bench_function(&format!("frame {name} scrolled live"), |b| {
            b.iter(|| {
                i = (i + 1) % lines_in.len();
                t.advance(lines_in[i].as_bytes());
                live.build_live(&t.state, black_box(&input), &mut Cached);
                black_box(live.glyphs.len())
            })
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
