//! The grapheme clusters of one row (§6.1 of the cells design). A cluster
//! cell's content is a slot here; the slot holds the cluster's text.
//!
//! Each slot belongs to exactly one cell, so overwriting or erasing that
//! cell frees it on the spot: no reference counts and no sweep. Rows move
//! (scrolling, insert/delete lines) with their store, so slots never need
//! fixing up.

/// At most this many scalars in one cluster; further ones are dropped.
pub const MAX_SCALARS: usize = 32;
/// At most this many bytes of UTF-8 in one cluster.
pub const MAX_BYTES: usize = 128;
/// At most this many bytes of live cluster text in one row.
pub const MAX_ROW_BYTES: usize = 8 * 1024;

/// No slot: the end of the free list.
const NONE: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: u32,
    /// 0 for a free slot (a live cluster has at least two scalars).
    len: u16,
    next_free: u16,
}

/// The clusters of one row: their UTF-8 back to back, with holes where
/// freed or moved clusters were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clusters {
    text: String,
    spans: Vec<Span>,
    /// First free slot, chained through `next_free`.
    free: u16,
    /// Bytes of `text` no live slot uses.
    garbage: u32,
}

impl Default for Clusters {
    fn default() -> Self {
        Clusters {
            text: String::new(),
            spans: Vec::new(),
            free: NONE,
            garbage: 0,
        }
    }
}

impl Clusters {
    /// The text in `slot`; empty for a slot that isn't live.
    #[inline]
    pub fn get(&self, slot: u32) -> &str {
        match self.spans.get(slot as usize) {
            Some(s) if s.len > 0 => self.text.get(s.start as usize..s.start as usize + s.len as usize).unwrap_or(""),
            _ => "",
        }
    }

    /// Store `text` (a whole cluster, within the caps) in a new slot. None
    /// if the row is out of room.
    pub fn insert(&mut self, text: &str) -> Option<u32> {
        debug_assert!(text.len() >= 2 && text.len() <= MAX_BYTES);
        if !self.make_room(text.len()) {
            return None;
        }
        let slot = if self.free != NONE {
            let slot = self.free;
            self.free = self.spans[slot as usize].next_free;
            slot
        } else {
            if self.spans.len() >= NONE as usize {
                return None;
            }
            self.spans.push(Span {
                start: 0,
                len: 0,
                next_free: NONE,
            });
            (self.spans.len() - 1) as u16
        };
        self.spans[slot as usize] = Span {
            start: self.text.len() as u32,
            len: text.len() as u16,
            next_free: NONE,
        };
        self.text.push_str(text);
        Some(slot as u32)
    }

    /// Add `c` to the end of the cluster in `slot`. False (and nothing
    /// changes) if that would pass the caps or the row is out of room.
    pub fn push(&mut self, slot: u32, c: char) -> bool {
        let Some(&span) = self.spans.get(slot as usize) else { return false };
        let (start, len) = (span.start as usize, span.len as usize);
        let add = c.len_utf8();
        // 32 scalars take at least 32 bytes: shorter clusters aren't counted.
        if len == 0 || len + add > MAX_BYTES || (len >= MAX_SCALARS && self.text[start..start + len].chars().count() >= MAX_SCALARS) {
            return false;
        }
        if start + len == self.text.len() {
            // The last cluster written: grow it in place.
            if self.live_bytes() + add > MAX_ROW_BYTES {
                return false;
            }
            self.text.push(c);
            self.spans[slot as usize].len += add as u16;
            return true;
        }
        // Copy it to the end; its old bytes become garbage.
        let mut buf = [0u8; MAX_BYTES];
        buf[..len].copy_from_slice(&self.text.as_bytes()[start..start + len]);
        c.encode_utf8(&mut buf[len..len + add]);
        let grown = std::str::from_utf8(&buf[..len + add]).expect("a cluster and a scalar are UTF-8");
        self.free(slot);
        // Freeing may have compacted, but the freed slot is the first free
        // one, so `insert` takes it back.
        let again = self.insert(grown);
        debug_assert!(again.is_none_or(|s| s == slot));
        if again.is_none() {
            // Out of room: put it back as it was (that fits: it just left).
            let back = std::str::from_utf8(&buf[..len]).expect("UTF-8");
            let s = self.insert(back);
            debug_assert_eq!(s, Some(slot));
            return false;
        }
        true
    }

    /// Free `slot` (it must be live).
    pub fn free(&mut self, slot: u32) {
        let Some(span) = self.spans.get_mut(slot as usize) else { return };
        if span.len == 0 {
            return;
        }
        self.garbage += span.len as u32;
        span.len = 0;
        span.next_free = self.free;
        self.free = slot as u16;
        if self.garbage as usize == self.text.len() {
            // Nothing live: start the text again.
            self.text.clear();
            self.garbage = 0;
        }
    }

    /// Free every slot, keeping the allocations.
    pub fn clear(&mut self) {
        self.text.clear();
        self.spans.clear();
        self.free = NONE;
        self.garbage = 0;
    }

    /// Bytes allocated, for deciding whether to keep the store on reset.
    pub fn capacity_bytes(&self) -> usize {
        self.text.capacity() + self.spans.capacity() * std::mem::size_of::<Span>()
    }

    /// Bytes of text live slots use.
    #[inline]
    fn live_bytes(&self) -> usize {
        self.text.len() - self.garbage as usize
    }

    /// Make sure `add` more bytes fit, compacting when over half the text
    /// is garbage. False if the row's cap would be passed.
    fn make_room(&mut self, add: usize) -> bool {
        if self.live_bytes() + add > MAX_ROW_BYTES {
            return false;
        }
        if self.garbage as usize > self.text.len() / 2 && self.text.len() > 256 {
            self.compact();
        }
        true
    }

    /// Move every live cluster to the front, in slot order.
    fn compact(&mut self) {
        let mut text = String::with_capacity(self.live_bytes());
        for s in self.spans.iter_mut().filter(|s| s.len > 0) {
            let start = s.start as usize;
            s.start = text.len() as u32;
            text.push_str(&self.text[start..start + s.len as usize]);
        }
        self.text = text;
        self.garbage = 0;
    }

    /// The whole text, holes included, and each slot's (offset, length)
    /// in it, a free slot's length 0: for copying the store in one go.
    pub fn raw(&self) -> (&str, impl ExactSizeIterator<Item = (u32, u16)> + '_) {
        (&self.text, self.spans.iter().map(|s| (s.start, s.len)))
    }

    /// The live slots, in order.
    pub fn live_slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.spans.iter().enumerate().filter(|(_, s)| s.len > 0).map(|(i, _)| i as u32)
    }

    /// Check the store on its own: spans inside the text and on character
    /// boundaries, clusters of 2–32 scalars and at most 128 bytes, the free
    /// list holding exactly the free slots, and `garbage` right.
    pub fn check(&self) -> Result<(), String> {
        let mut live = 0usize;
        for (i, s) in self.spans.iter().enumerate().filter(|(_, s)| s.len > 0) {
            let (start, end) = (s.start as usize, s.start as usize + s.len as usize);
            let Some(text) = self.text.get(start..end) else {
                return Err(format!("slot {i} ({start}..{end}) isn't inside the text on character boundaries"));
            };
            let n = text.chars().count();
            if !(2..=MAX_SCALARS).contains(&n) || text.len() > MAX_BYTES {
                return Err(format!("slot {i} holds {n} scalars in {} bytes: {text:?}", text.len()));
            }
            live += text.len();
        }
        if live > MAX_ROW_BYTES {
            return Err(format!("{live} bytes of clusters, over the row's cap"));
        }
        if self.text.len() - live != self.garbage as usize {
            return Err(format!(
                "garbage is {}, but {} bytes aren't live",
                self.garbage,
                self.text.len() - live
            ));
        }
        let mut free = 0;
        let mut at = self.free;
        while at != NONE {
            let Some(s) = self.spans.get(at as usize) else {
                return Err(format!("free list names slot {at}, past the end"));
            };
            if s.len != 0 {
                return Err(format!("free list names live slot {at}"));
            }
            free += 1;
            if free > self.spans.len() {
                return Err("free list loops".into());
            }
            at = s.next_free;
        }
        let dead = self.spans.iter().filter(|s| s.len == 0).count();
        if free != dead {
            return Err(format!("{dead} free slots, {free} on the free list"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_free_reuse() {
        let mut c = Clusters::default();
        let a = c.insert("e\u{301}").unwrap();
        let b = c.insert("👍🏽").unwrap();
        assert_eq!((c.get(a), c.get(b)), ("e\u{301}", "👍🏽"));
        c.check().unwrap();
        c.free(a);
        assert_eq!(c.get(a), "");
        c.check().unwrap();
        // The freed slot is taken first.
        assert_eq!(c.insert("🇬🇧"), Some(a));
        assert_eq!(c.get(a), "🇬🇧");
        assert_eq!(c.live_slots().collect::<Vec<_>>(), vec![0, 1]);
        c.free(a);
        c.free(b);
        // Nothing live: the text starts again.
        assert_eq!(c.text.len(), 0);
        c.check().unwrap();
    }

    #[test]
    fn push_grows_in_place_or_moves() {
        let mut c = Clusters::default();
        let a = c.insert("👨\u{200d}").unwrap();
        assert!(c.push(a, '👩'));
        assert_eq!(c.get(a), "👨\u{200d}👩");
        let b = c.insert("e\u{301}").unwrap();
        // `a` is no longer last: it moves to the end.
        assert!(c.push(a, '\u{200d}'));
        assert!(c.push(a, '👧'));
        assert_eq!(c.get(a), "👨\u{200d}👩\u{200d}👧");
        assert_eq!(c.get(b), "e\u{301}");
        c.check().unwrap();
    }

    #[test]
    fn caps() {
        let mut c = Clusters::default();
        let a = c.insert("a\u{301}").unwrap();
        for _ in 2..MAX_SCALARS {
            assert!(c.push(a, '\u{301}'));
        }
        assert!(!c.push(a, '\u{301}'), "33 scalars");
        assert_eq!(c.get(a).chars().count(), MAX_SCALARS);
        // Bytes: 4-byte scalars reach 128 bytes before 32 scalars.
        let b = c.insert("😀\u{1f3fd}").unwrap();
        let mut n = 8;
        while c.push(b, '\u{e0100}') {
            n += 4;
        }
        assert_eq!(n, MAX_BYTES);
        c.check().unwrap();
        // The row: 8 KB of live text.
        let mut c = Clusters::default();
        let mut stored = 0;
        while c.insert(&"\u{1f600}".repeat(32)).is_some() {
            stored += 128;
        }
        assert_eq!(stored, MAX_ROW_BYTES);
        c.check().unwrap();
    }

    #[test]
    fn rewriting_in_place_compacts() {
        // A spinner rewriting one emoji cell: the text doesn't grow.
        let mut c = Clusters::default();
        let keep = c.insert("e\u{301}").unwrap();
        for _ in 0..10_000 {
            let s = c.insert("❤\u{fe0f}").unwrap();
            c.free(s);
        }
        assert!(c.text.len() < 600, "{}", c.text.len());
        assert_eq!(c.get(keep), "e\u{301}");
        c.check().unwrap();
    }
}
