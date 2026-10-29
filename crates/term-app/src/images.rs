//! Images in the app (images.md §4.2, §5.5, §11 I.5a): the image host
//! ([`host::AppHost`]), the decode queue ([`queue::DecodeQueue`]), and the
//! plumbing between them and the sessions and windows:
//! - jobs are taken from a terminal after each parse slice (on the reader
//!   thread) and after each snapshot (on the main thread, so an idle
//!   terminal still runs its re-decodes), and run on the queue with no
//!   lock held;
//! - a finished job locks its terminal briefly to hand the pixels over,
//!   then wakes the window;
//! - a session's reader waits before reading more output while its
//!   queued encoded bytes pass a quarter of the budget (the hold-back), so
//!   a loop of `imgcat`s is slowed by the PTY instead of filling memory;
//! - windows tell the pool when they hide (after the renderer dropped its
//!   textures) and show (before drawing), and the budget follows the
//!   largest window.
//!
//! All of it is off unless images are on (`TD_IMAGES=1`, the hidden
//! switch): [`enabled`].

pub mod host;
pub mod queue;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use term_core::image::{ImageError, ImageHost, ImageJob, JobDone, run_job};
use term_core::{ImagePool, Term, Terminal};

use host::AppHost;
use queue::{DecodeQueue, Task};

/// Whether this process handles images (the hidden switch).
pub fn enabled() -> bool {
    ImagePool::process().enabled()
}

/// The process's host, made on first use.
pub fn process_host() -> Arc<AppHost> {
    static HOST: OnceLock<Arc<AppHost>> = OnceLock::new();
    HOST.get_or_init(|| Arc::new(AppHost::new(ImagePool::process(), true))).clone()
}

/// The process's decode queue.
pub fn process_queue() -> DecodeQueue<JobDone> {
    static QUEUE: OnceLock<DecodeQueue<JobDone>> = OnceLock::new();
    QUEUE.get_or_init(DecodeQueue::new).clone()
}

/// A job failed by a panic in its decoder: the same as a decode error.
fn panicked(terminal_id: u64, job: &ImageJob) -> JobDone {
    let (id, image, generation) = match job {
        ImageJob::Decode {
            job, image, generation, ..
        }
        | ImageJob::Read {
            job, image, generation, ..
        } => (*job, *image, *generation),
    };
    JobDone {
        job: id,
        terminal_id,
        image,
        generation,
        result: Err(ImageError::BadData("the decoder failed")),
        opaque: false,
    }
}

/// How long the reader waits for the queue at most before reading on.
const HOLD_BACK_WAIT: Duration = Duration::from_secs(2);

static OWNERS: AtomicU64 = AtomicU64::new(1);

/// One session's side of the queue: where its jobs' results go.
pub struct SessionImages {
    /// The session's key in the queue (a terminal's id changes when it is
    /// replaced after a panic; this doesn't).
    owner: u64,
    term: Weak<Mutex<Term>>,
    pool: Arc<ImagePool>,
    dirty: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
    host: Arc<dyn ImageHost>,
    queue: DecodeQueue<JobDone>,
    /// The terminal's id when last seen, for the pool's shown state.
    terminal_id: AtomicU64,
}

impl SessionImages {
    pub fn new(
        term: &Arc<Mutex<Term>>,
        terminal_id: u64,
        pool: Arc<ImagePool>,
        dirty: Arc<AtomicBool>,
        wake: Arc<dyn Fn() + Send + Sync>,
        host: Arc<dyn ImageHost>,
        queue: DecodeQueue<JobDone>,
    ) -> Arc<SessionImages> {
        Arc::new(SessionImages {
            owner: OWNERS.fetch_add(1, Ordering::Relaxed),
            term: Arc::downgrade(term),
            pool,
            dirty,
            wake,
            host,
            queue,
            terminal_id: AtomicU64::new(terminal_id),
        })
    }

    /// The process's pool, host and queue.
    pub fn for_process(
        term: &Arc<Mutex<Term>>,
        terminal_id: u64,
        dirty: Arc<AtomicBool>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Arc<SessionImages> {
        SessionImages::new(
            term,
            terminal_id,
            ImagePool::process(),
            dirty,
            wake,
            process_host(),
            process_queue(),
        )
    }

    /// The terminal's id when its jobs were last taken.
    pub fn terminal_id(&self) -> u64 {
        self.terminal_id.load(Ordering::Relaxed)
    }

    /// Take `t`'s queued jobs (it is locked) and put them on the queue.
    pub fn take_jobs(&self, t: &mut Terminal) {
        self.terminal_id.store(t.terminal_id(), Ordering::Relaxed);
        if !t.images_enabled() {
            return;
        }
        let mut jobs = Vec::new();
        t.take_image_jobs(&mut jobs);
        let terminal_id = t.terminal_id();
        for job in jobs {
            self.submit(terminal_id, job);
        }
    }

    fn submit(&self, terminal_id: u64, job: ImageJob) {
        let bytes = match &job {
            ImageJob::Decode { bytes, .. } => bytes.len(),
            ImageJob::Read { medium, .. } => medium.len as usize,
        };
        let failed = panicked(terminal_id, &job);
        let host = self.host.clone();
        let (term, dirty, wake) = (self.term.clone(), self.dirty.clone(), self.wake.clone());
        self.queue.push(Task {
            owner: self.owner,
            bytes,
            work: Box::new(move || run_job(&*host, terminal_id, job)),
            done: Box::new(move |r| {
                let f = r.unwrap_or(failed);
                // The session may have gone meanwhile.
                let Some(term) = term.upgrade() else { return };
                let redraw = crate::session::lock(&term).state.image_job_done(f);
                if redraw && !dirty.swap(true, Ordering::AcqRel) {
                    wake();
                }
            }),
        });
    }

    /// Wait (no lock held) while this session's queued bytes pass a
    /// quarter of the budget.
    pub fn hold_back(&self) {
        let limit = self.pool.limit() / 4;
        if self.queue.queued(self.owner) > limit {
            self.queue.wait_below(self.owner, limit, HOLD_BACK_WAIT);
        }
    }

    /// Drop the jobs still waiting: the session closed.
    pub fn cancel(&self) {
        self.queue.cancel_owner(self.owner);
    }

    pub fn queue(&self) -> &DecodeQueue<JobDone> {
        &self.queue
    }
}

impl Drop for SessionImages {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Each window's drawable size in pixels, for the budget.
fn windows() -> &'static Mutex<HashMap<usize, (u32, u32)>> {
    static W: OnceLock<Mutex<HashMap<usize, (u32, u32)>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Window `id` is now `width` × `height` device pixels (0 × 0: closed).
/// The budget follows the largest window: the larger of 32 MB and two
/// images its size (§12, question 1).
pub fn window_size(pool: &ImagePool, id: usize, width: u32, height: u32) {
    if !pool.enabled() {
        return;
    }
    let mut w = windows().lock().unwrap_or_else(PoisonError::into_inner);
    if width == 0 || height == 0 {
        w.remove(&id);
    } else {
        w.insert(id, (width, height));
    }
    let largest = w.values().copied().max_by_key(|&(w, h)| w as u64 * h as u64).unwrap_or((0, 0));
    let budget = ImagePool::budget_for(largest.0, largest.1);
    drop(w);
    if pool.limit() != budget {
        pool.set_limit(budget);
    }
}

/// A window hid or showed: tell the pool for each of its terminals. Call
/// after the renderer dropped its textures on hide, and before drawing on
/// show (§5.5).
pub fn set_shown(pool: &ImagePool, terminals: impl IntoIterator<Item = u64>, shown: bool) {
    if !pool.enabled() {
        return;
    }
    for id in terminals {
        pool.set_shown(id, shown);
    }
}

#[cfg(test)]
mod tests;
