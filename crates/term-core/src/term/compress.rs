//! Idle history compression (R.7, cells design §11.8) and giving memory
//! back under pressure (R.5), as the terminal offers them.

use super::Terminal;
use crate::scrollback::CompressJob;

impl Terminal {
    /// Under the lock: up to `max` sealed history blocks not compressed
    /// yet, oldest first, for [`CompressJob::run`] to compress on a
    /// background thread and [`Terminal::add_compressed`] to store. None
    /// if every sealed block is compressed. Never on the output path.
    pub fn compress_job(&self, max: usize) -> Option<CompressJob> {
        let job = self.scrollback.compress_job(max);
        (!job.is_empty()).then_some(job)
    }

    /// Under the lock: swap in the compressed blocks `job` built, for
    /// blocks still in history.
    pub fn add_compressed(&mut self, job: CompressJob) {
        self.scrollback.add_compressed(job);
    }

    /// Under memory pressure (R.5): compress every sealed block now,
    /// under the lock, drop the decompressed-block cache and the search
    /// filters (rebuilt in the background later). Returns the bytes
    /// history held before and after.
    pub fn shrink_history(&mut self) -> (usize, usize) {
        self.scrollback.shrink()
    }
}

impl Terminal {
    /// Measuring aid: decompress every compressed history block once,
    /// bypassing the cache. Returns how many.
    #[doc(hidden)]
    pub fn unpack_history_for_bench(&self) -> usize {
        self.scrollback.unpack_all()
    }
}
