//! Allocations made and heap kept while parsing the `parse` bench's
//! inputs, for the perf watch (scripts/perf-watch):
//! `cargo run --release -p term-core --example alloc_count -- [megabytes]`
//!
//! For each input, a 160x50 terminal with 10k lines of history is fed
//! `megabytes` (default 4) in 64 KiB chunks, as the reader thread hands
//! them over. A counting allocator gives the number of allocations
//! (reallocations included), the bytes asked for, the most heap held
//! above what was held before the terminal was made, and the heap the
//! terminal still holds at the end. Same input, same counts: these are
//! counts, not times, so they don't depend on the machine's load.
//!
//! Prints one line of JSON per input.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use term_core::Term;

#[path = "../benches/inputs/mod.rs"]
mod inputs;

/// The system allocator, counting calls, bytes asked for, bytes live and
/// the most live.
struct Counting;

static CALLS: AtomicUsize = AtomicUsize::new(0);
static ASKED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

// SAFETY: it hands every call to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: as the caller promised.
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            ASKED.fetch_add(l.size(), Ordering::Relaxed);
            grew(l.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        // SAFETY: as the caller promised.
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            ASKED.fetch_add(l.size(), Ordering::Relaxed);
            grew(l.size());
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
            CALLS.fetch_add(1, Ordering::Relaxed);
            if new >= l.size() {
                ASKED.fetch_add(new - l.size(), Ordering::Relaxed);
                grew(new - l.size());
            } else {
                LIVE.fetch_sub(l.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const CHUNK: usize = 64 << 10;

fn main() {
    let megabytes: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4).max(1);
    for (name, input) in inputs::all(megabytes << 20) {
        let base = LIVE.load(Ordering::Relaxed);
        PEAK.store(base, Ordering::Relaxed);
        let calls = CALLS.load(Ordering::Relaxed);
        let asked = ASKED.load(Ordering::Relaxed);
        let mut t = Term::new(160, 50, 10_000, term_core::ImagePool::new(32 << 20));
        for chunk in input.chunks(CHUNK) {
            t.advance(chunk);
        }
        let retained = LIVE.load(Ordering::Relaxed).saturating_sub(base);
        println!(
            "{{\"input\": \"{name}\", \"bytes_in\": {}, \"allocs\": {}, \"bytes_allocated\": {}, \"peak_heap\": {}, \"retained_heap\": {}, \"history_lines\": {}}}",
            input.len(),
            CALLS.load(Ordering::Relaxed) - calls,
            ASKED.load(Ordering::Relaxed) - asked,
            PEAK.load(Ordering::Relaxed) - base,
            retained,
            t.state.scrollback_len(),
        );
        drop(t);
    }
}
