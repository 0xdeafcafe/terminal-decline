//! The decode queue (images.md §4.2): one background thread per process
//! at utility QoS, running image jobs one at a time (decoding is
//! memory-bound; two at once would only raise the peak). Each job runs
//! inside `catch_unwind`, so a bad image fails that image only and the
//! queue keeps going.
//!
//! The queue counts the encoded bytes each owner (a session) has waiting,
//! so its reader can hold back while decoding lags ([`DecodeQueue::wait_below`]).

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Work for the queue: `work` runs inside `catch_unwind`; `done` gets its
/// result, or `None` if it panicked. Neither runs with the queue locked.
pub struct Task<T> {
    /// Who queued it: cancelled together, counted together.
    pub owner: u64,
    /// Encoded bytes it holds while waiting, for the hold-back.
    pub bytes: usize,
    pub work: Box<dyn FnOnce() -> T + Send>,
    pub done: Box<dyn FnOnce(Option<T>) + Send>,
}

struct State<T> {
    tasks: VecDeque<Task<T>>,
    /// Per owner: bytes of its tasks waiting or running.
    queued: HashMap<u64, usize>,
    /// The thread is started on the first task.
    started: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    /// A task was added.
    work: Condvar,
    /// A task finished or was cancelled.
    drained: Condvar,
}

/// See the module documentation.
pub struct DecodeQueue<T: Send + 'static> {
    shared: Arc<Shared<T>>,
}

impl<T: Send + 'static> Clone for DecodeQueue<T> {
    fn clone(&self) -> Self {
        DecodeQueue {
            shared: self.shared.clone(),
        }
    }
}

impl<T: Send + 'static> Default for DecodeQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<S>(m: &Mutex<S>) -> MutexGuard<'_, S> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<T: Send + 'static> DecodeQueue<T> {
    /// A queue with its own thread (started when the first task comes).
    pub fn new() -> DecodeQueue<T> {
        DecodeQueue {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    tasks: VecDeque::new(),
                    queued: HashMap::new(),
                    started: false,
                }),
                work: Condvar::new(),
                drained: Condvar::new(),
            }),
        }
    }

    /// Queue `task`, after the others.
    pub fn push(&self, task: Task<T>) {
        let mut s = lock(&self.shared.state);
        *s.queued.entry(task.owner).or_default() += task.bytes;
        s.tasks.push_back(task);
        if !s.started {
            let shared = self.shared.clone();
            let spawned = std::thread::Builder::new().name("image-decode".into()).spawn(move || run(shared));
            if let Err(e) = spawned {
                // No thread: the tasks fail rather than wait forever.
                eprintln!("terminal-decline: could not start the image decoder: {e}");
                let tasks: Vec<Task<T>> = s.tasks.drain(..).collect();
                s.queued.clear();
                drop(s);
                for t in tasks {
                    (t.done)(None);
                }
                self.shared.drained.notify_all();
                return;
            }
            s.started = true;
        }
        drop(s);
        self.shared.work.notify_one();
    }

    /// Drop `owner`'s tasks still waiting (its session closed). The one
    /// running, if any, finishes; its `done` should find nothing to do.
    pub fn cancel_owner(&self, owner: u64) {
        let mut s = lock(&self.shared.state);
        let mut gone = 0;
        s.tasks.retain(|t| {
            let keep = t.owner != owner;
            if !keep {
                gone += t.bytes;
            }
            keep
        });
        if let Some(q) = s.queued.get_mut(&owner) {
            *q -= gone;
            if *q == 0 {
                s.queued.remove(&owner);
            }
        }
        drop(s);
        self.shared.drained.notify_all();
    }

    /// Bytes of `owner`'s tasks waiting or running.
    pub fn queued(&self, owner: u64) -> usize {
        lock(&self.shared.state).queued.get(&owner).copied().unwrap_or(0)
    }

    /// Wait until `owner` has at most `limit` bytes queued, or `timeout`
    /// passes (so a stuck decode can't stop a program's output for
    /// good). Returns whether it is below.
    pub fn wait_below(&self, owner: u64, limit: usize, timeout: Duration) -> bool {
        let s = lock(&self.shared.state);
        let (s, _) = self
            .shared
            .drained
            .wait_timeout_while(s, timeout, |s| s.queued.get(&owner).is_some_and(|&q| q > limit))
            .unwrap_or_else(PoisonError::into_inner);
        s.queued.get(&owner).is_none_or(|&q| q <= limit)
    }

    /// Wait until no task is waiting or running (tests).
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let s = lock(&self.shared.state);
        let (s, _) = self
            .shared
            .drained
            .wait_timeout_while(s, timeout, |s| !s.queued.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        s.queued.is_empty()
    }
}

fn run<T: Send + 'static>(shared: Arc<Shared<T>>) {
    // Decoding is background work: utility QoS, as a dispatch queue would.
    // SAFETY: only changes this thread's own scheduling class.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
    loop {
        let task = {
            let mut s = lock(&shared.state);
            loop {
                if let Some(t) = s.tasks.pop_front() {
                    break t;
                }
                s = shared.work.wait(s).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let Task { owner, bytes, work, done } = task;
        let result = catch_unwind(AssertUnwindSafe(work)).ok();
        // A panic in `done` (it locks a terminal and updates it) is caught
        // too: the queue serves every window.
        let _ = catch_unwind(AssertUnwindSafe(move || done(result)));
        let mut s = lock(&shared.state);
        if let Some(q) = s.queued.get_mut(&owner) {
            *q = q.saturating_sub(bytes);
            if *q == 0 {
                s.queued.remove(&owner);
            }
        }
        drop(s);
        shared.drained.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    const WAIT: Duration = Duration::from_secs(10);

    fn task<T: Send + 'static>(
        owner: u64,
        bytes: usize,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(Option<T>) + Send + 'static,
    ) -> Task<T> {
        Task {
            owner,
            bytes,
            work: Box::new(work),
            done: Box::new(done),
        }
    }

    #[test]
    fn a_panicking_job_fails_alone() {
        let q = DecodeQueue::<u32>::new();
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        let tx3 = tx.clone();
        q.push(task(1, 10, || panic!("a bad image"), move |r| tx.send(("bad", r)).unwrap()));
        q.push(task(1, 10, || 7, move |r| tx2.send(("good", r)).unwrap()));
        q.push(task(2, 10, || panic!("another"), move |r| tx3.send(("bad too", r)).unwrap()));
        let got: Vec<_> = (0..3).map(|_| rx.recv_timeout(WAIT).unwrap()).collect();
        assert_eq!(got, vec![("bad", None), ("good", Some(7)), ("bad too", None)]);
        assert!(q.wait_idle(WAIT));
        // And it keeps going.
        let (tx, rx) = mpsc::channel();
        q.push(task(1, 0, || 8, move |r| tx.send(r).unwrap()));
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), Some(8));
    }

    #[test]
    fn a_panicking_completion_doesnt_stop_the_queue() {
        let q = DecodeQueue::<u32>::new();
        q.push(task(1, 0, || 1, |_| panic!("completion")));
        let (tx, rx) = mpsc::channel();
        q.push(task(1, 0, || 2, move |r| tx.send(r).unwrap()));
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), Some(2));
    }

    #[test]
    fn cancelled_jobs_never_run() {
        let q = DecodeQueue::<()>::new();
        // Hold the thread on a first job so the others wait.
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let (started_tx, started_rx) = mpsc::channel::<()>();
        q.push(task(
            9,
            1,
            move || {
                started_tx.send(()).unwrap();
                go_rx.recv_timeout(WAIT).unwrap();
            },
            |_| {},
        ));
        started_rx.recv_timeout(WAIT).unwrap();
        let ran = Arc::new(AtomicUsize::new(0));
        for owner in [1, 2, 1, 2] {
            let (ran, done) = (ran.clone(), ran.clone());
            q.push(task(
                owner,
                100,
                move || {
                    ran.fetch_add(1, Ordering::SeqCst);
                },
                move |_| {
                    done.fetch_add(10, Ordering::SeqCst);
                },
            ));
        }
        assert_eq!(q.queued(1), 200);
        q.cancel_owner(1);
        assert_eq!(q.queued(1), 0);
        assert_eq!(q.queued(2), 200);
        go_tx.send(()).unwrap();
        assert!(q.wait_idle(WAIT));
        // Owner 2's two ran and completed; owner 1's neither.
        assert_eq!(ran.load(Ordering::SeqCst), 22);
    }

    #[test]
    fn the_producer_waits_until_the_queue_drains() {
        let q = DecodeQueue::<()>::new();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        q.push(task(
            3,
            1000,
            move || {
                go_rx.recv_timeout(WAIT).unwrap();
            },
            |_| {},
        ));
        q.push(task(3, 1000, || {}, |_| {}));
        assert_eq!(q.queued(3), 2000);
        // Over the limit: waiting gives up after the timeout.
        assert!(!q.wait_below(3, 500, Duration::from_millis(20)));
        // Another owner isn't held back.
        assert!(q.wait_below(4, 500, Duration::from_millis(20)));
        let q2 = q.clone();
        let waiter = std::thread::spawn(move || q2.wait_below(3, 500, WAIT));
        go_tx.send(()).unwrap();
        assert!(waiter.join().unwrap());
        assert_eq!(q.queued(3), 0);
    }
}
