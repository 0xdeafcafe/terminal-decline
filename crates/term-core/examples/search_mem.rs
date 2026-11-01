//! Searching compressed history (R.7, cells design §11.8 and §13.5):
//! `cargo run --release -p term-core --example search_mem [runs] [plain]`
//!
//! 100k lines of log output at 160 columns, compressed (as under memory
//! pressure), with filters built again as the idle job does. For each
//! query: the most heap the search job held above what was held before
//! it started (a counting allocator), and its thread CPU time (best and
//! median of `runs`, default 21). With `plain`, history is left as
//! written (filters built all the same).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use term_core::{Query, Term, WordMatch};

/// The system allocator, counting the bytes live and the most live.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: it hands every call to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: as the caller promised.
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        // SAFETY: as the caller promised.
        unsafe { System.dealloc(p, l) }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        // SAFETY: as the caller promised.
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            if new >= l.size() {
                let now = LIVE.fetch_add(new - l.size(), Ordering::Relaxed) + new - l.size();
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(l.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

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

const LINES: usize = 100_000;

/// The search bench's history (`benches/search.rs`), compressed unless
/// `plain`.
fn terminal(plain: bool) -> Term {
    let mut t = Term::new(160, 50, LINES, term_core::ImagePool::new(32 << 20));
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
    if !plain {
        t.state.shrink_history();
    }
    if let Some(mut job) = t.state.filter_job(usize::MAX) {
        job.run();
        t.state.add_filters(job);
    }
    t
}

fn main() {
    let runs: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(21).max(1);
    let plain = std::env::args().nth(2).is_some_and(|a| a == "plain");
    let mut t = terminal(plain);
    println!(
        "history: {} lines, {:.2} MB held ({}), filters built",
        t.state.scrollback_len(),
        t.state.scrollback_bytes() as f64 / 1e6,
        if plain { "as written" } else { "compressed" }
    );
    let cases = [
        ("rare", "needle", false),
        ("absent", "zebra crossing", false),
        ("common", "request", false),
        ("common_ignore_case", "Server", true),
    ];
    for (name, text, ignore_case) in cases {
        let q = Query {
            text: text.into(),
            ignore_case,
            word: WordMatch::Contains,
        };
        let (mut peak, mut times, mut hits) = (0usize, Vec::with_capacity(runs), 0usize);
        for _ in 0..runs {
            let mut job = t.state.start_search(q.clone());
            let base = LIVE.load(Ordering::Relaxed);
            PEAK.store(base, Ordering::Relaxed);
            let start = cpu_us();
            let mut n = 0;
            job.run(|batch| n += batch.hits().len());
            times.push(cpu_us() - start);
            peak = peak.max(PEAK.load(Ordering::Relaxed) - base);
            hits = n;
            drop(job);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{name:<20} {hits:>7} hits  peak {:>7.3} MB  cpu best {:>7.0} µs  median {:>7.0} µs",
            peak as f64 / 1e6,
            times[0],
            times[times.len() / 2],
        );
    }
}
