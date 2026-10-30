//! History work done while a pane is idle: search filters (cells design
//! §13.6), then compressing history blocks (§11.8, roadmap R.7).
//!
//! A block of history searches faster once it has a filter, but nothing
//! needs one: a block without is scanned. Likewise a block takes less
//! memory once compressed, but nothing needs it to be. So both are done
//! only in spare time, never while output is arriving. Once a window's
//! display link has paused and stayed paused for [`IDLE_BEFORE`] (no
//! output, no frame), the view hands each pane's terminal to the one
//! `filters` thread. It takes [`BATCH`] blocks with `Terminal::filter_job`
//! under the lock, builds their filters off it and stores them with
//! `Terminal::add_filters` under the lock, until no block is left; then
//! the same with `Terminal::compress_job`, `CompressJob::run` and
//! `Terminal::add_compressed`. It stops as soon as the pane is busy again.
//! Filters go first: a compressed block would have to be decompressed to
//! build its filter.
//!
//! Idle cost: nothing here polls. The only timer is the one the view sets
//! when its display link pauses (one shot per pause, like the drawable
//! trim), and the thread sleeps on a channel; with every block filtered
//! and compressed a pause costs one timer and two peeks at the history,
//! then nothing runs. The `nothing_left_wakes_nothing` test checks the
//! loop stops at once.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use term_core::Term;

use crate::session;

/// How long a pane must have had no output and no frame.
pub const IDLE_BEFORE: Duration = Duration::from_secs(2);
/// Blocks per job: small, so the terminal's lock is only held briefly and
/// output that arrives stops the work within a few blocks.
pub const BATCH: usize = 4;

/// Whether a pane is idle enough to build filters: quiet for
/// [`IDLE_BEFORE`] and no frame wanted.
pub fn due(quiet_for: Duration, frame_pending: bool) -> bool {
    !frame_pending && quiet_for >= IDLE_BEFORE
}

/// A pane's output counter as it was when the view saw it idle: the pane
/// is still idle while it hasn't moved.
pub struct Quiet {
    counter: Arc<AtomicU64>,
    seen: u64,
}

impl Quiet {
    pub fn new(counter: Arc<AtomicU64>) -> Quiet {
        let seen = counter.load(Ordering::Relaxed);
        Quiet { counter, seen }
    }

    pub fn still(&self) -> bool {
        self.counter.load(Ordering::Relaxed) == self.seen
    }
}

/// Build filters for `term`'s history, then compress its blocks, in
/// batches while `idle` holds; the number of batches done. Only the lock
/// is shared with the pane: the filters are built and the blocks
/// compressed without it.
pub fn build(term: &Mutex<Term>, idle: impl Fn() -> bool) -> usize {
    let mut done = 0;
    let mut filters = true;
    while idle() {
        if filters {
            // Bound first: the guard must go before the job runs.
            let job = session::lock(term).state.filter_job(BATCH);
            if let Some(mut job) = job {
                job.run();
                session::lock(term).state.add_filters(job);
                done += 1;
                continue;
            }
            filters = false;
        }
        let Some(mut job) = session::lock(term).state.compress_job(BATCH) else {
            break;
        };
        job.run();
        session::lock(term).state.add_compressed(job);
        done += 1;
    }
    done
}

type Request = (Arc<Mutex<Term>>, Quiet);

static BUSY: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

/// Ask the filters thread to build `term`'s filters while `quiet` holds.
/// A terminal already being worked on is not queued again.
pub fn request(term: Arc<Mutex<Term>>, quiet: Quiet) {
    static WORKER: OnceLock<mpsc::Sender<Request>> = OnceLock::new();
    let key = Arc::as_ptr(&term) as usize;
    let first = BUSY.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default().insert(key);
    if !first {
        return;
    }
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("filters".into())
            .spawn(move || {
                background_priority();
                for (term, quiet) in rx {
                    build(&term, || quiet.still());
                    let key = Arc::as_ptr(&term) as usize;
                    if let Some(set) = BUSY.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                        set.remove(&key);
                    }
                }
            })
            .expect("filters thread starts");
        tx
    });
    let _ = tx.send((term, quiet));
}

/// The thread runs as background work, behind everything the user sees.
fn background_priority() {
    #[cfg(target_os = "macos")]
    // SAFETY: sets the calling thread's own quality of service.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn term_with_history() -> Mutex<Term> {
        let mut t = Term::new(40, 5, 100_000, term_core::ImagePool::process());
        let mut out = String::new();
        for i in 0..30_000 {
            out.push_str(&format!("line {i} of some output to fill blocks\r\n"));
        }
        t.advance(out.as_bytes());
        Mutex::new(t)
    }

    fn unfiltered(t: &Mutex<Term>) -> usize {
        // Count by asking for batches without storing them.
        let mut n = 0;
        let mut probe = 1;
        while session::lock(t).state.filter_job(probe).is_some_and(|j| !j.is_empty()) && probe < 4096 {
            n = probe;
            probe *= 2;
        }
        n
    }

    #[test]
    fn due_needs_quiet_and_no_frame() {
        assert!(!due(Duration::from_millis(1999), false));
        assert!(due(IDLE_BEFORE, false));
        assert!(!due(Duration::from_secs(60), true));
    }

    #[test]
    fn quiet_ends_when_output_arrives() {
        let n = Arc::new(AtomicU64::new(5));
        let q = Quiet::new(n.clone());
        assert!(q.still());
        n.fetch_add(1, Ordering::Relaxed);
        assert!(!q.still());
    }

    #[test]
    fn builds_every_filter_then_stops() {
        let term = term_with_history();
        assert!(unfiltered(&term) > 0, "history has blocks to filter");
        assert!(build(&term, || true) > 0);
        assert!(session::lock(&term).state.filter_job(1).is_none());
    }

    #[test]
    fn compresses_every_block_after_the_filters() {
        let term = term_with_history();
        let blocks = session::lock(&term).state.history_stats().sealed_blocks;
        assert!(blocks > BATCH, "history has several batches of blocks");
        let batches = build(&term, || true);
        // Every block's filter in batches of BATCH, then (as far as there
        // is anything to compress) its compression.
        assert!(batches >= blocks.div_ceil(BATCH), "{batches} batches for {blocks} blocks");
        let t = session::lock(&term);
        assert!(t.state.filter_job(1).is_none() && t.state.compress_job(1).is_none());
    }

    #[test]
    fn output_between_batches_stops_the_compressing() {
        let term = term_with_history();
        // Filters first, then one compression batch at most.
        let filter_batches = build(&term, || session::lock(&term).state.filter_job(1).is_some());
        assert!(filter_batches > 0);
        let allowed = Cell::new(1);
        let batches = build(&term, || {
            let ok = allowed.get() > 0;
            allowed.set(allowed.get() - 1);
            ok
        });
        assert!(batches <= 1);
    }

    #[test]
    fn nothing_left_wakes_nothing() {
        let term = term_with_history();
        build(&term, || true);
        // Done: a second run checks once and returns without a batch.
        let asked = Cell::new(0);
        let batches = build(&term, || {
            asked.set(asked.get() + 1);
            true
        });
        assert_eq!((batches, asked.get()), (0, 1));
    }

    #[test]
    fn stops_within_a_batch_when_output_arrives() {
        let term = term_with_history();
        let allowed = Cell::new(1);
        let batches = build(&term, || {
            let ok = allowed.get() > 0;
            allowed.set(allowed.get() - 1);
            ok
        });
        assert_eq!(batches, 1);
        assert!(session::lock(&term).state.filter_job(1).is_some(), "the rest wait");
    }

    #[test]
    fn a_busy_pane_builds_nothing() {
        let term = term_with_history();
        assert_eq!(build(&term, || false), 0);
    }
}
