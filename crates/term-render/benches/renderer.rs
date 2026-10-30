//! The renderer with real glyphs and a real GPU: building a window's frame
//! through the shared glyph cache and encoding it into an offscreen
//! target, for one 160×48 pane (today's window) and for the same window
//! split into four 80×24 panes (splits design §3.4).

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use objc2::rc::Retained;
use objc2_metal::{
    MTLCommandBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureUsage,
};
use std::hint::black_box;
use term_core::Term;
use term_font::Fonts;
use term_render::{CommandBuffer, Frame, FrameInput, PaneDraw, PaneRender, PxRect, RectInstance, Renderer, Theme, rgb};

fn screen(cols: usize, lines: usize) -> Term {
    let mut t = Term::new(cols, lines, 0, term_core::ImagePool::process());
    let mut s = String::new();
    for l in 0..lines {
        for c in 0..cols {
            if c % 7 == 0 {
                s.push_str(&format!("\x1b[38;5;{}m", (l + c) % 256));
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

fn frame_of(r: &mut Renderer) -> &mut Frame {
    &mut r.pane.frame
}

/// Time `encode` alone: the GPU's work is waited for outside the timing.
fn time_encode(n: u64, mut encode: impl FnMut() -> Retained<CommandBuffer>) -> Duration {
    let mut spent = Duration::ZERO;
    for _ in 0..n {
        let start = Instant::now();
        let cmd = encode();
        spent += start.elapsed();
        cmd.commit();
        cmd.waitUntilCompleted();
    }
    spent
}

fn bench(c: &mut Criterion) {
    if MTLCreateSystemDefaultDevice().is_none() {
        eprintln!("skipping: no Metal device");
        return;
    }
    let theme = Theme::default();
    let mut r = Renderer::new(Fonts::new("Menlo", 28.0));
    let m = r.metrics();
    let input = FrameInput {
        theme: &theme,
        metrics: m,
        padding: (0, 0),
        cursor_on: true,
        focused: true,
        preedit: None,
    };
    let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
    let (cols, lines) = (160, 48);
    let mut t = screen(cols, lines);
    let (w, h) = (cols * cw as usize, lines * ch as usize);
    let desc =
        unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::BGRA8Unorm, w, h, false) };
    desc.setUsage(MTLTextureUsage::RenderTarget);
    desc.setStorageMode(MTLStorageMode::Private);
    let target = r.device().newTextureWithDescriptor(&desc).expect("target");

    c.bench_function("renderer 1 pane 160x48 prepare", |b| {
        b.iter(|| {
            frame_of(&mut r).forget_rows();
            r.prepare(&mut t.state, black_box(&input));
        })
    });
    c.bench_function("renderer 1 pane 160x48 prepare unchanged", |b| {
        b.iter(|| r.prepare(&mut t.state, black_box(&input)))
    });
    c.bench_function("renderer 1 pane 160x48 encode", |b| {
        b.iter_custom(|n| time_encode(n, || r.encode(&target)))
    });
    // A spinner on the middle row: each frame built (not timed) and
    // encoded (timed), as when one row changes.
    let spinner: Vec<String> = "|/-\\".chars().map(|ch| format!("\x1b[{};1H{ch}", lines / 2)).collect();
    let mut i = 0;
    c.bench_function("renderer 1 pane 160x48 encode one row", |b| {
        b.iter_custom(|n| {
            let mut spent = Duration::ZERO;
            for _ in 0..n {
                i = (i + 1) % spinner.len();
                t.advance(spinner[i].as_bytes());
                r.prepare(&mut t.state, &input);
                let start = Instant::now();
                let cmd = r.encode(&target);
                spent += start.elapsed();
                cmd.commit();
                cmd.waitUntilCompleted();
            }
            spent
        })
    });

    // The same window split in four: 80×24 panes, each with its own
    // terminal, a 2×2 grid with a divider each way.
    let mut terms: Vec<Term> = (0..4).map(|_| screen(80, 24)).collect();
    let mut panes: Vec<PaneRender> = (0..4).map(|_| PaneRender::new()).collect();
    let origins: Vec<(i32, i32)> = (0..4).map(|i| ((i % 2) * 80 * cw, (i / 2) * 24 * ch)).collect();
    let dividers = [
        RectInstance {
            x: (80 * cw) as i16,
            y: 0,
            w: 1,
            h: h as u16,
            color: rgb(0x808080),
        },
        RectInstance {
            x: 0,
            y: (24 * ch) as i16,
            w: w as u16,
            h: 1,
            color: rgb(0x808080),
        },
    ];
    c.bench_function("renderer 4 panes 80x24 build", |b| {
        b.iter(|| {
            for (p, t) in panes.iter_mut().zip(&mut terms) {
                p.frame.forget_rows();
                r.build_pane(p, &mut t.state, black_box(&input));
            }
        })
    });
    c.bench_function("renderer 4 panes 80x24 build unchanged", |b| {
        b.iter(|| {
            for (p, t) in panes.iter_mut().zip(&mut terms) {
                r.build_pane(p, &mut t.state, black_box(&input));
            }
        })
    });
    let draws: Vec<PaneDraw> = panes
        .iter()
        .zip(&origins)
        .map(|(pane, &origin)| PaneDraw {
            pane,
            origin,
            scissor: PxRect {
                x: origin.0,
                y: origin.1,
                w: 80 * cw,
                h: 24 * ch,
            },
        })
        .collect();
    let clear = draws[0].pane.frame.clear;
    c.bench_function("renderer 4 panes 80x24 encode", |b| {
        b.iter_custom(|n| time_encode(n, || r.encode_panes(&target, clear, &draws, &dividers)))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
