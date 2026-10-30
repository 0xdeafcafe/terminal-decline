//! The per-block filter (§13.6 of the cells design): a 4 KB set of the
//! 4-byte grams in a sealed block's text, ASCII lower-cased and other
//! bytes as they are, one bit per gram and one hash function. A search
//! skips a block whose filter lacks one of its query's grams: the filter
//! never wrongly skips a block that holds the text.
//!
//! Filters are built only in the background ([`FilterJob`]), never on the
//! output path or while someone waits for a search; a block without one
//! is simply scanned.

use std::sync::Arc;

use crate::scrollback::Body;

/// Bits in a filter: 4 KB.
const BITS: usize = 32_768;
const WORDS: usize = BITS / 64;

/// One block's filter (§13.6).
pub struct Filter {
    bits: Box<[u64; WORDS]>,
}

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set: u32 = self.bits.iter().map(|w| w.count_ones()).sum();
        write!(f, "Filter({set} of {BITS} bits set)")
    }
}

/// A gram's bit: Fibonacci hashing down to 15 bits.
#[inline]
fn bit(gram: u32) -> usize {
    (gram.wrapping_mul(0x9E37_79B1) >> 17) as usize
}

impl Filter {
    /// Bytes a filter holds.
    pub const BYTES: usize = BITS / 8;

    /// The filter of `text` (about 100 µs for a full 64 KB block).
    pub fn build(text: &[u8]) -> Filter {
        let mut bits = Box::new([0u64; WORDS]);
        let mut gram = 0u32;
        for (i, &b) in text.iter().enumerate() {
            gram = gram << 8 | b.to_ascii_lowercase() as u32;
            if i >= 3 {
                let k = bit(gram);
                bits[k / 64] |= 1 << (k % 64);
            }
        }
        Filter { bits }
    }

    /// Whether the text may hold every one of `grams` (from [`grams`]).
    #[inline]
    pub fn may_contain(&self, grams: &[u32]) -> bool {
        grams.iter().all(|&g| {
            let k = bit(g);
            self.bits[k / 64] & 1 << (k % 64) != 0
        })
    }
}

/// The grams of `bytes` (ASCII lower-cased), each once. With `ascii_only`,
/// only those whose four bytes are all ASCII: a non-ASCII query ignoring
/// case matches text whose non-ASCII bytes differ, but whose ASCII ones
/// are the query's (no non-ASCII character folds to ASCII).
pub(crate) fn grams(bytes: &[u8], ascii_only: bool) -> Vec<u32> {
    let mut out: Vec<u32> = bytes
        .windows(4)
        .filter(|w| !ascii_only || w.is_ascii())
        .map(|w| w.iter().fold(0u32, |g, &b| g << 8 | b.to_ascii_lowercase() as u32))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Filters to build off the lock (§13.6): taken under the lock by
/// [`crate::Terminal::filter_job`] for sealed blocks without one, built
/// by [`FilterJob::run`] on a background thread, and stored by
/// [`crate::Terminal::add_filters`] under the lock. Blocks freed in
/// between are left out then.
#[derive(Debug, Default)]
pub struct FilterJob {
    pub(crate) blocks: Vec<Body>,
    pub(crate) built: Vec<Arc<Filter>>,
}

impl FilterJob {
    /// Blocks it builds filters for.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Build the filters (off the lock).
    pub fn run(&mut self) {
        // A compressed block is decompressed here, off the lock.
        self.built = self.blocks.iter().map(|b| Arc::new(Filter::build(b.unpack().text()))).collect();
    }
}
