//! CPU cost of turning a full screen into GPU instances.

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
}

fn screen(cols: usize, lines: usize, colours: Colours) -> Term {
    let mut t = Term::new(cols, lines, 0);
    let mut s = String::new();
    for l in 0..lines {
        for c in 0..cols {
            if colours == Colours::Coloured && c % 7 == 0 {
                s.push_str(&format!("\x1b[38;5;{};48;5;{}m", (l + c) % 256, (l * 3 + c) % 256));
            }
            if colours == Colours::Alternating {
                s.push_str(if c % 2 == 0 { "\x1b[1;31m" } else { "\x1b[0m" });
            }
            s.push((b'!' + ((l * cols + c) % 90) as u8) as char);
        }
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
    for (name, cols, lines, colours) in [
        ("80x24 plain", 80, 24, Colours::Plain),
        ("300x100 plain", 300, 100, Colours::Plain),
        ("300x100 coloured", 300, 100, Colours::Coloured),
        ("300x100 per-cell style", 300, 100, Colours::Alternating),
    ] {
        let t = screen(cols, lines, colours);
        let mut frame = Frame::default();
        c.bench_function(&format!("frame {name}"), |b| {
            b.iter(|| {
                let input = FrameInput {
                    term: &t.state,
                    theme: &theme,
                    metrics,
                    padding: (12, 12),
                    cursor_on: true,
                    focused: true,
                    preedit: None,
                };
                frame.build(black_box(&input), &mut Cached);
                black_box(frame.glyphs.len())
            })
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
