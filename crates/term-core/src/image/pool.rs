//! The process-wide image budget (images.md §5): one [`ImagePool`] shared
//! by every terminal, with its own lock, taken after a terminal's and never
//! before. Poisoning is ignored (a panic elsewhere doesn't break other
//! windows), and `used` only changes once an update is complete.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use super::PixelBuffer;

/// An entry in the pool, unique in the process.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PoolId(u64);

/// The request doesn't fit, even after evicting what may be evicted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Refused;

/// Encoded image bytes (PNG, JPEG, …, or an upload so far), in 64 KB
/// pieces so growing never copies.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Encoded {
    pieces: Vec<Vec<u8>>,
    len: usize,
}

impl std::fmt::Debug for Encoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Encoded({} bytes)", self.len)
    }
}

impl Encoded {
    pub const PIECE: usize = 64 * 1024;

    pub fn new() -> Encoded {
        Encoded::default()
    }

    pub fn push(&mut self, mut bytes: &[u8]) {
        self.len += bytes.len();
        while !bytes.is_empty() {
            if self.pieces.last().is_none_or(|p| p.len() == Self::PIECE) {
                self.pieces.push(Vec::with_capacity(Self::PIECE));
            }
            let piece = self.pieces.last_mut().unwrap();
            let n = (Self::PIECE - piece.len()).min(bytes.len());
            piece.extend_from_slice(&bytes[..n]);
            bytes = &bytes[n..];
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The pieces in order.
    pub fn pieces(&self) -> impl Iterator<Item = &[u8]> {
        self.pieces.iter().map(|p| p.as_slice())
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.pieces.concat()
    }

    /// What it holds, counted as allocated.
    fn bytes(&self) -> usize {
        self.pieces.len() * Self::PIECE
    }
}

/// The smallest budget: 32 MB (Alex's decision).
pub const MIN_LIMIT: usize = 32 << 20;

struct Entry {
    owner: u64,
    pixels: Option<Arc<dyn PixelBuffer>>,
    encoded: Option<Arc<Encoded>>,
    /// Reserved and not yet holding anything: an upload so far, a decode's
    /// output and headroom (§5.1).
    reserved: usize,
    /// What is counted in `used` for it.
    bytes: usize,
    /// Clock when made, or when last drawn.
    last_used: u64,
    /// The frame it was last drawn in (0: never).
    drawn_at: u64,
    placed: bool,
    busy: bool,
    volatile: bool,
    generation: u32,
    /// Its key in the order (class, last used), when not busy.
    indexed: Option<(u8, u64)>,
}

impl Entry {
    fn holding(&self) -> usize {
        self.pixels.as_ref().map_or(0, |p| p.allocated_size()) + self.encoded.as_ref().map_or(0, |e| e.bytes()) + self.reserved
    }
}

struct PoolInner {
    limit: usize,
    used: usize,
    entries: HashMap<PoolId, Entry>,
    /// (eviction class, last used, id) of every entry not busy (§5.3).
    order: BTreeSet<(u8, u64, PoolId)>,
    /// Per owner: is its window shown (unknown owners are).
    shown: HashMap<u64, bool>,
    /// Per owner: the clock value of its latest frame.
    latest_frame: HashMap<u64, u64>,
    /// Per owner: entries evicted since it last asked.
    evicted: HashMap<u64, Vec<PoolId>>,
    /// Ticks once per frame of any window.
    clock: u64,
}

/// The process-wide image budget: who holds what, and what goes first when
/// a new image doesn't fit.
pub struct ImagePool {
    inner: Mutex<PoolInner>,
    /// Whether this pool's terminals handle images: the hidden switch at
    /// first, then the app's `images` setting ([`ImagePool::set_enabled`]).
    enabled: std::sync::atomic::AtomicBool,
}

static POOL_IDS: AtomicU64 = AtomicU64::new(1);

impl ImagePool {
    /// A pool with images on (for tests and tools).
    pub fn new(limit: usize) -> Arc<ImagePool> {
        Arc::new(ImagePool::with_switch(limit, true))
    }

    fn with_switch(limit: usize, enabled: bool) -> ImagePool {
        ImagePool {
            inner: Mutex::new(PoolInner {
                limit,
                used: 0,
                entries: HashMap::new(),
                order: BTreeSet::new(),
                shown: HashMap::new(),
                latest_frame: HashMap::new(),
                evicted: HashMap::new(),
                clock: 1,
            }),
            enabled: std::sync::atomic::AtomicBool::new(enabled),
        }
    }

    /// The pool of this process, made on first use with the smallest
    /// budget and the hidden switch ([`super::images_switch`]).
    pub fn process() -> Arc<ImagePool> {
        static POOL: OnceLock<Arc<ImagePool>> = OnceLock::new();
        POOL.get_or_init(|| Arc::new(ImagePool::with_switch(MIN_LIMIT, super::images_switch())))
            .clone()
    }

    /// The budget for windows whose largest is `width` × `height` device
    /// pixels: the larger of 32 MB and two images that size (§12, 1).
    pub fn budget_for(width: u32, height: u32) -> usize {
        MIN_LIMIT.max(2 * width as usize * height as usize * 4)
    }

    /// Whether this pool's terminals handle images at all.
    pub fn enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Turn images on or off for this pool's terminals, from now on:
    /// what is held stays until evicted.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    fn lock(&self) -> MutexGuard<'_, PoolInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn limit(&self) -> usize {
        self.lock().limit
    }

    pub fn used(&self) -> usize {
        self.lock().used
    }

    /// Change the budget; lowering it evicts at once.
    pub fn set_limit(&self, limit: usize) {
        let mut p = self.lock();
        p.limit = limit;
        p.evict(0);
    }

    /// Reserve `bytes` for a new entry of `owner`, busy until
    /// [`ImagePool::set_busy`] says otherwise. The entry being replaced gives
    /// its bytes back first; then others are evicted until it fits.
    pub fn reserve(&self, owner: u64, bytes: usize, replacing: Option<PoolId>) -> Result<PoolId, Refused> {
        let mut p = self.lock();
        if let Some(old) = replacing {
            p.remove(old);
        }
        if bytes > p.limit || !p.evict(bytes) {
            return Err(Refused);
        }
        let id = PoolId(POOL_IDS.fetch_add(1, Ordering::Relaxed));
        let last_used = p.clock;
        p.entries.insert(
            id,
            Entry {
                owner,
                pixels: None,
                encoded: None,
                reserved: bytes,
                bytes,
                last_used,
                drawn_at: 0,
                placed: false,
                busy: true,
                volatile: false,
                generation: 0,
                indexed: None,
            },
        );
        p.used += bytes;
        Ok(id)
    }

    /// Reserve `more` bytes for an entry (an upload growing).
    pub fn grow(&self, id: PoolId, more: usize) -> Result<(), Refused> {
        let mut p = self.lock();
        let Some(e) = p.entries.get(&id) else { return Err(Refused) };
        if e.bytes + more > p.limit {
            return Err(Refused);
        }
        // Not itself.
        p.unindex(id);
        let fits = p.evict(more);
        p.reindex(id);
        if !fits {
            return Err(Refused);
        }
        p.update(id, |e| e.reserved += more);
        Ok(())
    }

    /// The entry's pixels are ready: what was reserved for them turns into
    /// them, and the generation moves on. Returns the generation.
    pub fn set_pixels(&self, id: PoolId, pixels: Arc<dyn PixelBuffer>) -> u32 {
        let mut p = self.lock();
        let size = pixels.allocated_size();
        let mut generation = 0;
        p.update(id, |e| {
            e.reserved = e.reserved.saturating_sub(size);
            e.pixels = Some(pixels);
            e.volatile = false;
            e.generation += 1;
            generation = e.generation;
        });
        p.evict(0);
        generation
    }

    /// Keep (or drop) the entry's encoded copy (§5.4).
    pub fn set_encoded(&self, id: PoolId, encoded: Option<Arc<Encoded>>) {
        let mut p = self.lock();
        let size = encoded.as_ref().map_or(0, |e| e.bytes());
        p.update(id, |e| {
            e.reserved = e.reserved.saturating_sub(size);
            e.encoded = encoded;
        });
        p.evict(0);
    }

    /// Busy entries (receiving, or a job running) are never evicted. When
    /// an entry stops being busy, what it reserved and didn't use goes back.
    pub fn set_busy(&self, id: PoolId, busy: bool) {
        let mut p = self.lock();
        p.update(id, |e| {
            e.busy = busy;
            if !busy {
                e.reserved = 0;
            }
        });
    }

    /// Whether the image has a placement or placeholder in use.
    pub fn set_placed(&self, id: PoolId, placed: bool) {
        self.lock().update(id, |e| e.placed = placed);
    }

    /// Called once per frame by the snapshot with the images it is about
    /// to draw: they are protected while that owner is shown and this is
    /// its latest frame.
    pub fn drawn(&self, owner: u64, ids: &[PoolId]) {
        let mut p = self.lock();
        p.clock += 1;
        let clock = p.clock;
        p.latest_frame.insert(owner, clock);
        for &id in ids {
            p.update(id, |e| {
                e.last_used = clock;
                e.drawn_at = clock;
            });
        }
    }

    /// A window was hidden or shown (§5.5). Hidden, its entries with an
    /// encoded copy are made purgeable; shown again, those macOS took
    /// become lost-with-source.
    pub fn set_shown(&self, owner: u64, shown: bool) {
        let mut p = self.lock();
        p.shown.insert(owner, shown);
        let ids: Vec<PoolId> = p.entries.iter().filter(|(_, e)| e.owner == owner).map(|(id, _)| *id).collect();
        let mut taken = Vec::new();
        for id in ids {
            p.update(id, |e| {
                let Some(px) = e.pixels.clone() else { return };
                if !shown && e.encoded.is_some() && !e.volatile {
                    px.set_volatile(true);
                    e.volatile = true;
                } else if shown && e.volatile {
                    e.volatile = false;
                    if !px.set_volatile(false) {
                        e.pixels = None;
                        taken.push(id);
                    }
                }
            });
        }
        if !taken.is_empty() {
            p.evicted.entry(owner).or_default().extend(taken);
        }
    }

    /// Free an entry.
    pub fn release(&self, id: PoolId) {
        self.lock().remove(id);
    }

    /// Free everything a terminal owned: when it is dropped, including
    /// after a caught panic.
    pub fn release_owner(&self, owner: u64) {
        let mut p = self.lock();
        let ids: Vec<PoolId> = p.entries.iter().filter(|(_, e)| e.owner == owner).map(|(id, _)| *id).collect();
        for id in ids {
            p.remove(id);
        }
        p.shown.remove(&owner);
        p.latest_frame.remove(&owner);
        p.evicted.remove(&owner);
    }

    /// The entries of `owner` evicted (wholly or their pixels) since the
    /// last call.
    pub fn take_evicted(&self, owner: u64, out: &mut Vec<PoolId>) {
        if let Some(v) = self.lock().evicted.remove(&owner) {
            out.extend(v);
        }
    }

    pub fn contains(&self, id: PoolId) -> bool {
        self.lock().entries.contains_key(&id)
    }

    /// The entry's pixels and their generation, if it still has them.
    pub fn pixels(&self, id: PoolId) -> Option<(Arc<dyn PixelBuffer>, u32)> {
        let p = self.lock();
        let e = p.entries.get(&id)?;
        Some((e.pixels.clone()?, e.generation))
    }

    /// The entry's encoded copy, if kept.
    pub fn encoded(&self, id: PoolId) -> Option<Arc<Encoded>> {
        self.lock().entries.get(&id)?.encoded.clone()
    }

    /// Check `used` and the index agree with the entries.
    pub fn check(&self) -> Result<(), String> {
        let p = self.lock();
        let sum: usize = p.entries.values().map(|e| e.bytes).sum();
        if sum != p.used {
            return Err(format!("used {} but entries hold {sum}", p.used));
        }
        for (id, e) in &p.entries {
            if e.bytes != e.holding() {
                return Err(format!("{id:?} counted {} but holds {}", e.bytes, e.holding()));
            }
            let want = (!e.busy).then(|| (p.class(e), e.last_used));
            if e.indexed != want || want.is_some_and(|(c, l)| !p.order.contains(&(c, l, *id))) {
                return Err(format!("{id:?} indexed as {:?}, should be {want:?}", e.indexed));
            }
        }
        let idle = p.entries.values().filter(|e| !e.busy).count();
        if idle != p.order.len() {
            return Err(format!("{} in the order, {idle} idle entries", p.order.len()));
        }
        Ok(())
    }
}

impl PoolInner {
    /// Eviction class: hidden windows' first, then never placed, then the
    /// rest.
    fn class(&self, e: &Entry) -> u8 {
        if self.shown.get(&e.owner) == Some(&false) {
            0
        } else if !e.placed {
            1
        } else {
            2
        }
    }

    fn protected(&self, e: &Entry) -> bool {
        e.busy || self.shown.get(&e.owner) != Some(&false) && e.drawn_at != 0 && self.latest_frame.get(&e.owner) == Some(&e.drawn_at)
    }

    /// Take `id` out of the order.
    fn unindex(&mut self, id: PoolId) {
        if let Some(e) = self.entries.get_mut(&id)
            && let Some((class, last)) = e.indexed.take()
        {
            self.order.remove(&(class, last, id));
        }
    }

    /// Put `id` back in the order, unless it is busy.
    fn reindex(&mut self, id: PoolId) {
        let Some(e) = self.entries.get(&id) else { return };
        if e.busy || e.indexed.is_some() {
            return;
        }
        let key = (self.class(e), e.last_used);
        self.order.insert((key.0, key.1, id));
        self.entries.get_mut(&id).unwrap().indexed = Some(key);
    }

    /// Change an entry, keeping `used` and the order right.
    fn update(&mut self, id: PoolId, f: impl FnOnce(&mut Entry)) {
        if !self.entries.contains_key(&id) {
            return;
        }
        self.unindex(id);
        let e = self.entries.get_mut(&id).unwrap();
        f(e);
        let bytes = e.holding();
        self.used = self.used - e.bytes + bytes;
        e.bytes = bytes;
        self.reindex(id);
    }

    fn remove(&mut self, id: PoolId) {
        self.unindex(id);
        if let Some(e) = self.entries.remove(&id) {
            self.used -= e.bytes;
        }
    }

    /// Evict until `more` bytes fit: pixels first, cheapest class and
    /// oldest first, then encoded copies. False if it still doesn't fit.
    fn evict(&mut self, more: usize) -> bool {
        for pixels in [true, false] {
            while self.used + more > self.limit {
                let victim = self.order.iter().map(|k| k.2).find(|id| {
                    let e = &self.entries[id];
                    !self.protected(e)
                        && if pixels {
                            e.pixels.is_some()
                        } else {
                            e.encoded.is_some() || e.pixels.is_none()
                        }
                });
                let Some(id) = victim else { break };
                let owner = self.entries[&id].owner;
                if pixels && self.entries[&id].encoded.is_some() {
                    self.update(id, |e| {
                        e.pixels = None;
                        e.reserved = 0;
                    });
                } else {
                    self.remove(id);
                }
                self.evicted.entry(owner).or_default().push(id);
            }
        }
        self.used + more <= self.limit
    }
}

#[cfg(test)]
mod tests;
