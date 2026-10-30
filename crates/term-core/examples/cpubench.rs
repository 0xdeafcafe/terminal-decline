//! Parse cost in thread CPU time rather than wall time, so other load on
//! the machine counts less than in `cargo bench`: for comparing two builds
//! run turn about on a busy Mac, and for `sample`/Instruments (a filter
//! and many runs keep one input going).
//!
//! `cargo run --release -p term-core --example cpubench -- [filter] [runs] [scrollback]`
//!
//! Prints, per input of the `parse` bench and chunk size, the best and
//! median of `runs` (15) passes over 4 MB, in µs. `scrollback` (10000
//! lines) of 0 keeps no history, leaving the cost of writing the screen
//! alone.

use term_core::Term;

#[path = "../benches/inputs/mod.rs"]
mod inputs;

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
    let limit: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let inputs = inputs::all(4 << 20);
    for (group, chunk) in [("parse", 64 * 1024), ("parse_1k", 1024)] {
        for (name, data) in &inputs {
            let full = format!("{group}/{name}");
            if !full.contains(&filter) {
                continue;
            }
            let mut t = Term::new(160, 50, limit, term_core::ImagePool::process());
            // Twice through first, so history is full and its oldest
            // lines are being dropped, as in a long session.
            for _ in 0..2 {
                for piece in data.chunks(chunk) {
                    t.advance(std::hint::black_box(piece));
                }
            }
            let mut times = Vec::with_capacity(runs);
            for _ in 0..runs {
                let start = cpu_us();
                for piece in data.chunks(chunk) {
                    t.advance(std::hint::black_box(piece));
                }
                times.push(cpu_us() - start);
            }
            times.sort_by(f64::total_cmp);
            println!("{full} {:.0} {:.0}", times[0], times[times.len() / 2]);
        }
    }
}
