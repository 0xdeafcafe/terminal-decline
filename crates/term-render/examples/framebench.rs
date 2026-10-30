//! Frame building and encoding in thread CPU time rather than wall time,
//! so other load on the machine counts less than in `cargo bench`: for
//! comparing two builds run turn about on a busy Mac (as term-core's
//! `cpubench`). The cases are a subset of the `frame` and `renderer`
//! benches.
//!
//! `cargo run --release -p term-render --example framebench -- [filter] [runs]`
//!
//! Prints, per case, the best and median of `runs` (15) batches, in µs
//! per frame.

use std::hint::black_box;

use objc2_metal::{
    MTLCommandBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureUsage,
};
use term_core::Term;
use term_font::{Fonts, Metrics, Style};
use term_render::{Frame, FrameInput, GlyphSlot, GlyphSource, Renderer, Theme};

/// This thread's CPU time in µs.
fn cpu_us() -> f64 {
    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, tp: *mut Timespec) -> i32;
    }
    // CLOCK_THREAD_CPUTIME_ID on macOS.
    let mut t = Timespec { sec: 0, nsec: 0 };
    // SAFETY: `t` is a valid timespec for the call to fill in.
    unsafe { clock_gettime(16, &mut t) };
    t.sec as f64 * 1e6 + t.nsec as f64 / 1e3
}

/// Every glyph already in the texture, as in the `frame` bench.
struct Cached;
impl GlyphSource for Cached {
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

fn line(l: usize, cols: usize, coloured: bool, s: &mut String) {
    for c in 0..cols {
        if coloured && c % 7 == 0 {
            s.push_str(&format!("\x1b[38;5;{};48;5;{}m", (l + c) % 256, (l * 3 + c) % 256));
        }
        s.push((b'!' + ((l * cols + c) % 90) as u8) as char);
    }
}

fn screen(cols: usize, lines: usize, coloured: bool, scrollback: usize) -> Term {
    let mut t = Term::new(cols, lines, scrollback, term_core::ImagePool::process());
    let mut s = String::new();
    for l in 0..lines {
        line(l, cols, coloured, &mut s);
        if l + 1 < lines {
            s.push_str("\r\n");
        }
    }
    t.advance(s.as_bytes());
    t
}

fn sparse() -> Term {
    let (cols, lines) = (203, 50);
    let mut t = Term::new(cols, lines, 1000, term_core::ImagePool::process());
    let mut s = String::new();
    for l in 0..lines - 1 {
        match l % 3 {
            0 => s.push_str(&format!("\x1b[1;32m~/src/project\x1b[0m \x1b[34mmain\x1b[0m $ ls -la {l}")),
            1 => s.push_str("drwxr-xr-x  12 alex  staff   384 Oct 30 09:12 crates"),
            _ => s.push_str(&"-rw-r--r--   1 alex  staff  1078 Oct 30 09:12 Cargo.toml"[..(20 + l % 37)]),
        }
        s.push_str("\r\n");
    }
    s.push_str("\x1b[1;32m~/src/project\x1b[0m $ ");
    t.advance(s.as_bytes());
    t
}

fn history(cols: usize, lines: usize, coloured: bool, narrowed: bool) -> Term {
    let mut t = Term::new(cols, lines, 10_000, term_core::ImagePool::process());
    let mut s = String::new();
    for l in 0..lines * 3 {
        line(l, cols, coloured, &mut s);
        s.push_str("\x1b[0m\r\n");
    }
    t.advance(s.as_bytes());
    if narrowed {
        t.state.resize(cols - 7, lines);
    }
    t.state.scroll_view(lines as isize * 2);
    t
}

struct Runner {
    filter: String,
    runs: usize,
}

impl Runner {
    /// Time `f` (one frame) in batches of about 20 ms; `setup` runs
    /// before each frame, untimed.
    fn case(&self, name: &str, mut setup: impl FnMut(), mut f: impl FnMut()) {
        if !name.contains(&self.filter) {
            return;
        }
        // Warm up, and size the batch.
        let start = cpu_us();
        let mut n = 0;
        while cpu_us() - start < 50_000.0 {
            setup();
            f();
            n += 1;
        }
        let batch = (n / 3).max(1);
        let mut per: Vec<f64> = (0..self.runs)
            .map(|_| {
                let mut spent = 0.0;
                for _ in 0..batch {
                    setup();
                    let t = cpu_us();
                    f();
                    spent += cpu_us() - t;
                }
                spent / batch as f64
            })
            .collect();
        per.sort_by(f64::total_cmp);
        println!("{name}\t{:.2}\t{:.2}", per[0], per[per.len() / 2]);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = Runner {
        filter: args.get(1).cloned().unwrap_or_default(),
        runs: args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15).max(1),
    };
    let theme = Theme::default();
    let input = FrameInput {
        theme: &theme,
        metrics: Metrics {
            cell_width: 16,
            cell_height: 34,
            baseline: 26,
            underline_offset: 3,
            underline_thickness: 2,
            strike_offset: 8,
        },
        padding: (12, 12),
        cursor_on: true,
        focused: true,
        preedit: None,
    };

    for (name, cols, lines, coloured) in [
        ("80x24 plain", 80, 24, false),
        ("300x100 plain", 300, 100, false),
        ("300x100 coloured", 300, 100, true),
    ] {
        let mut t = screen(cols, lines, coloured, 0);
        let mut frame = Frame::default();
        r.case(
            &format!("frame {name}"),
            || {},
            || {
                frame.forget_rows();
                frame.build(&mut t.state, black_box(&input), &mut Cached)
            },
        );
        let mut frame = Frame::default();
        r.case(
            &format!("frame {name} unchanged"),
            || {},
            || frame.build(&mut t.state, black_box(&input), &mut Cached),
        );
        let spinner: Vec<String> = "|/-\\".chars().map(|ch| format!("\x1b[{};1H{ch}", lines / 2)).collect();
        let mut i = 0;
        let mut frame = Frame::default();
        r.case(
            &format!("frame {name} one row"),
            || {},
            || {
                i = (i + 1) % spinner.len();
                t.advance(spinner[i].as_bytes());
                frame.build(&mut t.state, black_box(&input), &mut Cached)
            },
        );
        let mut t = screen(cols, lines, coloured, 0);
        let lines_in: Vec<String> = (0..16)
            .map(|l| {
                let mut s = String::from("\r\n");
                line(l, cols, coloured, &mut s);
                s
            })
            .collect();
        let mut frame = Frame::default();
        r.case(
            &format!("frame {name} scrolled"),
            || {},
            || {
                i = (i + 1) % lines_in.len();
                t.advance(lines_in[i].as_bytes());
                frame.build(&mut t.state, black_box(&input), &mut Cached)
            },
        );
    }

    let mut t = sparse();
    let mut frame = Frame::default();
    r.case(
        "frame 203x50 sparse",
        || {},
        || {
            frame.forget_rows();
            frame.build(&mut t.state, black_box(&input), &mut Cached)
        },
    );
    let mut frame = Frame::default();
    r.case(
        "frame 203x50 sparse unchanged",
        || {},
        || frame.build(&mut t.state, black_box(&input), &mut Cached),
    );
    let mut i = 0usize;
    let mut frame = Frame::default();
    r.case(
        "frame 203x50 sparse one row",
        || {},
        || {
            i += 1;
            if i.is_multiple_of(40) {
                t.advance(b"\r\x1b[K\x1b[1;32m~/src/project\x1b[0m $ ");
            } else {
                t.advance(&[b'a' + (i % 26) as u8]);
            }
            frame.build(&mut t.state, black_box(&input), &mut Cached)
        },
    );

    for (name, coloured) in [("plain", false), ("coloured", true)] {
        for narrowed in [false, true] {
            let mut t = history(300, 100, coloured, narrowed);
            let mut frame = Frame::default();
            let what = if narrowed {
                "history reflowed unchanged"
            } else {
                "history unchanged"
            };
            r.case(
                &format!("frame 300x100 {name} {what}"),
                || {},
                || frame.build(&mut t.state, black_box(&input), &mut Cached),
            );
        }
    }

    if MTLCreateSystemDefaultDevice().is_none() {
        return;
    }
    let mut ren = Renderer::new(Fonts::new("Menlo", 28.0));
    let m = ren.metrics();
    let input = FrameInput {
        metrics: m,
        padding: (0, 0),
        ..input
    };
    let (cols, lines) = (160, 48);
    let mut t = screen(cols, lines, true, 0);
    let (w, h) = (cols * m.cell_width as usize, lines * m.cell_height as usize);
    let desc =
        unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::BGRA8Unorm, w, h, false) };
    desc.setUsage(MTLTextureUsage::RenderTarget);
    desc.setStorageMode(MTLStorageMode::Private);
    let target = ren.device().newTextureWithDescriptor(&desc).expect("target");
    ren.prepare(&mut t.state, &input);
    // The GPU's work is waited for in the next frame's untimed setup.
    let mut pending = None;
    let wait = |p: &mut Option<objc2::rc::Retained<term_render::CommandBuffer>>| {
        if let Some(cmd) = p.take() {
            cmd.waitUntilCompleted();
        }
    };
    {
        let ren = &mut ren;
        let pending = std::cell::RefCell::new(&mut pending);
        r.case(
            "renderer 1 pane 160x48 encode",
            || wait(&mut pending.borrow_mut()),
            || {
                let cmd = ren.encode(&target);
                cmd.commit();
                **pending.borrow_mut() = Some(cmd);
            },
        );
        wait(&mut pending.borrow_mut());
    }
    let spinner: Vec<String> = "|/-\\".chars().map(|ch| format!("\x1b[{};1H{ch}", lines / 2)).collect();
    let mut i = 0;
    {
        let ren = std::cell::RefCell::new(&mut ren);
        let pending = std::cell::RefCell::new(&mut pending);
        r.case(
            "renderer 1 pane 160x48 encode one row",
            || {
                wait(&mut pending.borrow_mut());
                i = (i + 1) % spinner.len();
                t.advance(spinner[i].as_bytes());
                ren.borrow_mut().prepare(&mut t.state, &input);
            },
            || {
                let cmd = ren.borrow_mut().encode(&target);
                cmd.commit();
                **pending.borrow_mut() = Some(cmd);
            },
        );
        wait(&mut pending.borrow_mut());
    }
}
