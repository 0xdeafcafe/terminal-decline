//! The style table: the distinct styles one screen's cells use, each stored
//! once, with cells holding an index into it (§5.2 of the cells design).
//!
//! Ids are freed by a sweep rather than by reference counts. Only the
//! screen refers to ids (history stores styles by value), so a sweep marks
//! the ids the screen's cells and the pen's and erase templates use, and
//! frees the rest. It runs when interning a new style would grow the table
//! past `sweep_at`, so the write path does no counting. A sweep that finds
//! fewer than a quarter of the ids live renumbers them densely, and gives
//! the memory back.

use std::collections::VecDeque;

use crate::cell::{Cell, Row};
use crate::style::Style;

/// The smallest `sweep_at`, so small screens don't sweep all the time.
const MIN_SWEEP_AT: u32 = 1024;
/// The `new_ids` list holds at most half the table, but never needs to be
/// cut shorter than this.
const MIN_NEW_IDS: usize = 32;
/// An empty index slot. Entries are `tag << 24 | id` with id ≥ 1, so
/// never 0.
const EMPTY: u32 = 0;
/// Ids are below 2²⁴ (§4.1: the top byte of a cell's style id is 0): a
/// table is swept at 1.5 × the screen's cells, and holds at most the
/// screen's cells + 3 ids after a sweep.
const ID_BITS: u32 = 24;
const ID_MASK: u32 = (1 << ID_BITS) - 1;

/// A style that `lookup` didn't find: its hash, and the empty slot its
/// probe ended on, for `insert` (valid while the index is unchanged).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Miss {
    hash: u64,
    slot: usize,
    index_gen: u32,
}

#[inline]
fn slot_of(hash: u64, mask: usize) -> usize {
    (hash >> 32) as usize & mask
}

#[inline]
fn entry(hash: u64, id: u32) -> u32 {
    (hash as u32) << ID_BITS | id
}

pub(crate) struct StyleTable {
    /// By id. Freed ids keep their last style until reused; `live` says
    /// which ids are in use.
    styles: Vec<Style>,
    /// One bit per id: in use.
    live: Vec<u64>,
    /// Open addressing by hash of the style, a power of two long, at most
    /// half full. Each entry is a 24-bit id with the low byte of its
    /// style's hash as a tag, so a probe reads a style only when the tags
    /// match (4 bytes a slot keeps the index in cache); `EMPTY` for an empty
    /// slot. Id 0, the default style, is found without it.
    index: Vec<u32>,
    /// Bumped whenever the index is rebuilt, so a [`Miss`] knows its slot
    /// is stale.
    index_gen: u32,
    /// Ids freed by the last sweep, lowest last, so they're used first.
    free: Vec<u32>,
    /// Sweep before growing past this many ids.
    sweep_at: u32,
    /// A `sweep_at` set by a test, used instead of the one worked out from
    /// the screen size.
    sweep_at_test: Option<u32>,
    /// Set during resize, while rows are half-built.
    sweeps_blocked: bool,
    /// The last two styles interned, so alternating styles skip the hash.
    recent: [(Style, u32); 2],
    /// Ids interned since the last snapshot took them.
    new_ids: Vec<u32>,
    /// Too many new ids (or a renumbering, or a new table): the next
    /// snapshot takes the whole table.
    send_all: bool,
    /// From the terminal-wide counter, when made and at each renumbering.
    epoch: u64,
    /// Scratch for the sweep: one bit per id, marked when something uses it.
    marks: Vec<u64>,
    /// How many ids are live (the bits set in `live`).
    live_n: usize,
    sweeps: u64,
    renumberings: u64,
}

/// What the table holds, for tests and measuring.
#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StyleStats {
    /// Ids allocated, live or free (the default included).
    pub len: usize,
    /// Ids in use.
    pub live: usize,
    pub epoch: u64,
    pub sweep_at: usize,
    pub sweeps: u64,
    pub renumberings: u64,
    /// Bytes of heap the table holds.
    pub bytes: usize,
}

/// Styles interned since the last call, for a mirror of the table (§9.2).
/// The snapshot takes them with [`StyleTable::drain_updates`]; this form
/// is for the table's own tests.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub struct StyleUpdates {
    pub epoch: u64,
    /// `styles` is the whole table: drop the mirror first.
    pub all: bool,
    pub styles: Vec<(u32, Style)>,
}

#[inline]
fn hash(s: &Style) -> u64 {
    // FxHash over the style's 20 bytes, as three words.
    const K: u64 = 0xf135_7aea_2e62_a9c5;
    let a = s.fg.0 as u64 | (s.bg.0 as u64) << 32;
    let b = s.underline_color.0 as u64 | (s.link as u64) << 32;
    let c = s.attrs.bits() as u64 | (s.underline as u64) << 16 | (s.image_id_high as u64) << 24;
    let mut h = a.wrapping_mul(K);
    h = (h.rotate_left(26) ^ b).wrapping_mul(K);
    h = (h.rotate_left(26) ^ c).wrapping_mul(K);
    h
}

#[inline]
fn bit(bits: &[u64], id: u32) -> bool {
    bits.get(id as usize >> 6).is_some_and(|w| w >> (id & 63) & 1 != 0)
}

#[inline]
fn set_bit(bits: &mut [u64], id: u32) {
    bits[id as usize >> 6] |= 1 << (id & 63);
}

impl StyleTable {
    /// A table holding only the default style, for a screen of `cells`
    /// cells.
    pub fn new(epoch: u64, cells: usize, sweep_at_test: Option<u32>) -> StyleTable {
        let mut t = StyleTable {
            styles: vec![Style::DEFAULT],
            live: vec![1],
            index: vec![EMPTY; 16],
            index_gen: 0,
            free: Vec::new(),
            sweep_at: 0,
            sweep_at_test,
            sweeps_blocked: false,
            recent: [(Style::DEFAULT, 0); 2],
            new_ids: Vec::new(),
            send_all: true,
            epoch,
            marks: Vec::new(),
            live_n: 1,
            sweeps: 0,
            renumberings: 0,
        };
        t.set_cells(cells);
        t
    }

    /// The screen now has `cells` cells: `sweep_at` follows it.
    pub fn set_cells(&mut self, cells: usize) {
        let auto = (cells.saturating_mul(3) / 2).clamp(MIN_SWEEP_AT as usize, 1 << 24) as u32;
        self.sweep_at = self.sweep_at_test.unwrap_or(auto);
    }

    pub fn set_sweep_at_test(&mut self, n: Option<u32>, cells: usize) {
        self.sweep_at_test = n.map(|n| n.max(2));
        self.set_cells(cells);
    }

    pub fn set_sweeps_blocked(&mut self, on: bool) {
        self.sweeps_blocked = on;
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The styles by id, freed ids included (nothing on the screen uses
    /// those).
    #[inline]
    pub fn styles(&self) -> &[Style] {
        &self.styles
    }

    #[inline]
    pub fn get(&self, id: u32) -> &Style {
        &self.styles[id as usize]
    }

    #[inline]
    fn template(&self, id: u32) -> Cell {
        Cell::template(id, self.styles[id as usize].shows_empty())
    }

    /// The template of `s`, if it is in the table; if not, where to
    /// [`StyleTable::insert`] it.
    #[inline]
    pub fn lookup(&mut self, s: &Style) -> Result<Cell, Miss> {
        if *s == Style::DEFAULT {
            return Ok(Cell::BLANK);
        }
        if self.recent[0].0 == *s {
            return Ok(self.template(self.recent[0].1));
        }
        if self.recent[1].0 == *s {
            self.recent.swap(0, 1);
            return Ok(self.template(self.recent[0].1));
        }
        let id = self.probe(s)?;
        self.recent[1] = std::mem::replace(&mut self.recent[0], (*s, id));
        Ok(self.template(id))
    }

    #[inline]
    fn probe(&self, s: &Style) -> Result<u32, Miss> {
        let h = hash(s);
        let mask = self.index.len() - 1;
        let tag = entry(h, 0);
        let mut i = slot_of(h, mask);
        loop {
            let e = self.index[i];
            if e == EMPTY {
                return Err(Miss {
                    hash: h,
                    slot: i,
                    index_gen: self.index_gen,
                });
            }
            if e & !ID_MASK == tag && self.styles[(e & ID_MASK) as usize] == *s {
                return Ok(e & ID_MASK);
            }
            i = (i + 1) & mask;
        }
    }

    /// Interning a style not in the table would grow it past `sweep_at`:
    /// sweep first.
    #[inline]
    pub fn needs_sweep(&self) -> bool {
        !self.sweeps_blocked && self.free.is_empty() && self.styles.len() >= self.sweep_at as usize
    }

    /// Add `s`, which [`StyleTable::lookup`] didn't find (`miss`), and
    /// return its template.
    pub fn insert(&mut self, s: Style, miss: Miss) -> Cell {
        debug_assert!(s != Style::DEFAULT && self.probe(&s).is_err());
        let id = match self.free.pop() {
            Some(id) => {
                self.styles[id as usize] = s;
                id
            }
            None => {
                debug_assert!(self.styles.len() < ID_MASK as usize, "style ids past 2^24");
                self.styles.push(s);
                if self.live.len() * 64 < self.styles.len() {
                    self.live.push(0);
                }
                (self.styles.len() - 1) as u32
            }
        };
        set_bit(&mut self.live, id);
        self.live_n += 1;
        if self.live_n * 2 > self.index.len() {
            self.rebuild_index(self.index.len() * 2);
        } else if miss.index_gen == self.index_gen {
            self.index[miss.slot] = entry(miss.hash, id);
        } else {
            self.index_insert(id);
        }
        self.recent[1] = std::mem::replace(&mut self.recent[0], (s, id));
        if !self.send_all {
            self.new_ids.push(id);
            if self.new_ids.len() > self.new_ids_cap() {
                self.new_ids.clear();
                self.send_all = true;
            }
        }
        self.template(id)
    }

    fn new_ids_cap(&self) -> usize {
        (self.styles.len() / 2).max(MIN_NEW_IDS)
    }

    fn index_insert(&mut self, id: u32) {
        let h = hash(&self.styles[id as usize]);
        let mask = self.index.len() - 1;
        let mut i = slot_of(h, mask);
        while self.index[i] != EMPTY {
            i = (i + 1) & mask;
        }
        self.index[i] = entry(h, id);
    }

    /// Make the index `size` slots long (a power of two) and fill it with
    /// the live ids.
    fn rebuild_index(&mut self, size: usize) {
        self.index_gen = self.index_gen.wrapping_add(1);
        self.index.clear();
        self.index.resize(size, EMPTY);
        for id in 1..self.styles.len() as u32 {
            if bit(&self.live, id) {
                self.index_insert(id);
            }
        }
    }

    fn live_count(&self) -> usize {
        self.live.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Free every id that no cell of `rows` and no template uses, and
    /// with `give_back`, renumber the ids if fewer than a quarter are left
    /// in use and shrink the table to them. The renumbering rewrites `rows`
    /// and `templates`, and takes its epoch from `next_epoch`. Returns
    /// whether it renumbered.
    ///
    /// Ordinary sweeps (the table is full) never renumber: a program
    /// painting a new style per cell sweeps over and over, and each
    /// renumbering would change every row and the whole table for the
    /// snapshot (§9.2). The terminal gives back when the whole screen is
    /// cleared.
    pub fn sweep(
        &mut self,
        rows: &mut VecDeque<Row>,
        templates: &mut [&mut Option<Cell>],
        next_epoch: impl FnOnce() -> u64,
        give_back: bool,
    ) -> bool {
        self.sweeps += 1;
        let words = self.styles.len().div_ceil(64);
        self.marks.clear();
        self.marks.resize(words, 0);
        self.marks[0] = 1;
        for row in rows.iter() {
            // Neighbouring cells mostly share a style: mark each run once.
            let mut last = 0u32;
            for c in &row.cells {
                let id = c.style_id();
                if id != last {
                    set_bit(&mut self.marks, id);
                    last = id;
                }
            }
        }
        for t in templates.iter().filter_map(|t| t.as_ref()) {
            set_bit(&mut self.marks, t.style_id());
        }
        std::mem::swap(&mut self.live, &mut self.marks);
        self.free.clear();
        for id in (1..self.styles.len() as u32).rev() {
            if !bit(&self.live, id) {
                self.free.push(id);
            }
        }
        // `recent` may name a freed id.
        self.recent = [(Style::DEFAULT, 0); 2];
        let live = &self.live;
        self.new_ids.retain(|&id| bit(live, id));

        let live = self.styles.len() - self.free.len();
        self.live_n = live;
        if give_back && live * 4 < self.styles.len() {
            self.renumber(rows, templates, next_epoch());
            return true;
        }
        let size = (live * 2).next_power_of_two().max(16);
        self.rebuild_index(size.max(self.index.len()));
        false
    }

    /// Give the live ids the numbers 0..live in their old order, rewrite
    /// every holder, and shrink the vectors to what is left.
    fn renumber(&mut self, rows: &mut VecDeque<Row>, templates: &mut [&mut Option<Cell>], epoch: u64) {
        self.renumberings += 1;
        let len = self.styles.len();
        let mut map = vec![u32::MAX; len];
        let mut n = 0;
        for (id, new) in map.iter_mut().enumerate() {
            if bit(&self.live, id as u32) {
                *new = n as u32;
                self.styles[n] = self.styles[id];
                n += 1;
            }
        }
        self.styles.truncate(n);
        self.live.clear();
        self.live.resize(n.div_ceil(64), 0);
        for id in 0..n as u32 {
            set_bit(&mut self.live, id);
        }
        self.styles.shrink_to_fit();
        self.live.shrink_to_fit();
        self.free = Vec::new();
        self.marks = Vec::new();
        self.new_ids = Vec::new();
        self.index = Vec::new();
        self.rebuild_index((n * 2).next_power_of_two().max(16));
        let remap = |c: Cell| Cell::template(map[c.style_id() as usize], c.shows_empty());
        for row in rows.iter_mut() {
            for c in &mut row.cells {
                if c.style_id() != 0 {
                    *c = c.with_style(remap(*c));
                }
            }
        }
        for c in templates.iter_mut().filter_map(|t| t.as_mut()) {
            *c = remap(*c);
        }
        self.recent = [(Style::DEFAULT, 0); 2];
        self.new_ids.clear();
        self.send_all = true;
        self.epoch = epoch;
    }

    /// Ids allocated, live or free.
    #[inline]
    pub fn len(&self) -> usize {
        self.styles.len()
    }

    /// The styles interned since the last call (all of them after a
    /// renumbering, for a new table, or when there were too many).
    #[cfg(test)]
    pub fn take_updates(&mut self) -> StyleUpdates {
        let mut styles = Vec::new();
        let all = self.drain_updates(&mut styles);
        StyleUpdates {
            epoch: self.epoch,
            all,
            styles,
        }
    }

    /// [`StyleTable::take_updates`] appended to `out`, keeping the list's
    /// memory, so a snapshot allocates nothing in steady state. Returns
    /// whether `out` got the whole table.
    pub fn drain_updates(&mut self, out: &mut Vec<(u32, Style)>) -> bool {
        let all = std::mem::take(&mut self.send_all);
        if all {
            self.new_ids.clear();
            out.extend(
                (0..self.styles.len() as u32)
                    .filter(|&id| bit(&self.live, id))
                    .map(|id| (id, self.styles[id as usize])),
            );
        } else {
            out.extend(self.new_ids.drain(..).map(|id| (id, self.styles[id as usize])));
        }
        all
    }

    pub fn is_live(&self, id: u32) -> bool {
        bit(&self.live, id)
    }

    pub fn stats(&self) -> StyleStats {
        StyleStats {
            len: self.styles.len(),
            live: self.live_n,
            epoch: self.epoch,
            sweep_at: self.sweep_at as usize,
            sweeps: self.sweeps,
            renumberings: self.renumberings,
            bytes: self.styles.capacity() * size_of::<Style>()
                + self.live.capacity() * 8
                + self.index.capacity() * 4
                + self.free.capacity() * 4
                + self.new_ids.capacity() * 4
                + self.marks.capacity() * 8,
        }
    }

    /// Check the table on its own: id 0 is the default; the index finds
    /// every live id and nothing else; `free` holds exactly the dead ids;
    /// no two live ids have equal styles; `recent` names live ids with
    /// their styles; `new_ids` is within its cap and names live ids.
    pub fn check(&self) -> Result<(), String> {
        if self.styles.first() != Some(&Style::DEFAULT) || !self.is_live(0) {
            return Err("id 0 isn't the live default style".into());
        }
        if self.live.len() * 64 < self.styles.len() {
            return Err("live bits too short".into());
        }
        if self.live_n != self.live_count() {
            return Err(format!("live count {} but {} live bits", self.live_n, self.live_count()));
        }
        for id in self.styles.len()..self.live.len() * 64 {
            if bit(&self.live, id as u32) {
                return Err(format!("id {id} live past the end of the table"));
            }
        }
        let mut seen = std::collections::HashMap::new();
        for id in 1..self.styles.len() as u32 {
            if !self.is_live(id) {
                continue;
            }
            let s = &self.styles[id as usize];
            if *s == Style::DEFAULT {
                return Err(format!("id {id} holds the default style"));
            }
            if let Some(other) = seen.insert(*s, id) {
                return Err(format!("ids {other} and {id} hold the same style"));
            }
            if self.probe(s).ok() != Some(id) {
                return Err(format!("the index doesn't find id {id}"));
            }
        }
        for (slot, &e) in self.index.iter().enumerate() {
            let id = e & ID_MASK;
            if e != EMPTY && (id as usize >= self.styles.len() || e != entry(hash(&self.styles[id as usize]), id)) {
                return Err(format!("index slot {slot} holds {e:#x}, not its style's tag"));
            }
        }
        let in_index: Vec<u32> = self.index.iter().filter(|&&e| e != EMPTY).map(|&e| e & ID_MASK).collect();
        if in_index.len() != seen.len() || in_index.iter().any(|&id| !self.is_live(id)) {
            return Err(format!("index holds {} ids for {} live ones", in_index.len(), seen.len()));
        }
        if !self.index.len().is_power_of_two() || in_index.len() * 2 > self.index.len() {
            return Err(format!("index of {} slots for {} ids", self.index.len(), in_index.len()));
        }
        let mut free = self.free.clone();
        free.sort_unstable();
        let dead: Vec<u32> = (1..self.styles.len() as u32).filter(|&id| !self.is_live(id)).collect();
        if free != dead {
            return Err(format!("free list {free:?} isn't the dead ids {dead:?}"));
        }
        for (s, id) in &self.recent {
            if !self.is_live(*id) || self.styles[*id as usize] != *s {
                return Err(format!("recent names id {id}, which isn't live with that style"));
            }
        }
        if self.new_ids.len() > self.new_ids_cap() {
            return Err(format!("{} new ids in a table of {}", self.new_ids.len(), self.styles.len()));
        }
        if let Some(id) = self.new_ids.iter().find(|&&id| !self.is_live(id)) {
            return Err(format!("new ids name dead id {id}"));
        }
        if self.send_all && !self.new_ids.is_empty() {
            return Err("new ids kept while the whole table is to be sent".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Color;

    fn style(i: u32) -> Style {
        Style {
            fg: Color::rgb((i >> 16) as u8, (i >> 8) as u8, i as u8),
            ..Style::DEFAULT
        }
    }

    fn intern(t: &mut StyleTable, s: Style) -> Cell {
        t.lookup(&s).unwrap_or_else(|miss| t.insert(s, miss))
    }

    #[test]
    fn interning_gives_one_id_per_style() {
        let mut t = StyleTable::new(1, 100, None);
        assert_eq!(intern(&mut t, Style::DEFAULT), Cell::BLANK);
        let a = intern(&mut t, style(1));
        let b = intern(&mut t, style(2));
        assert_ne!(a, b);
        for _ in 0..3 {
            assert_eq!(intern(&mut t, style(1)), a);
            assert_eq!(intern(&mut t, style(2)), b);
        }
        // Past the two recent entries, through the index.
        let ids: Vec<Cell> = (1..500).map(|i| intern(&mut t, style(i))).collect();
        for i in 1..500 {
            assert_eq!(intern(&mut t, style(i)), ids[i as usize - 1]);
        }
        assert_eq!(t.stats().live, 500);
        t.check().unwrap();
        // SE follows the style.
        let bg = Style {
            bg: Color::indexed(1),
            ..Style::DEFAULT
        };
        assert!(intern(&mut t, bg).shows_empty());
        assert!(!a.shows_empty());
    }

    #[test]
    fn sweep_frees_unused_ids_and_renumbers_when_few_are_left() {
        let mut t = StyleTable::new(1, 10, Some(12));
        let mut rows: VecDeque<Row> = (0..2).map(|_| Row::new(4, Cell::BLANK)).collect();
        let ids: Vec<Cell> = (1..=11).map(|i| intern(&mut t, style(i))).collect();
        assert!(t.needs_sweep());
        rows[0].cells[0] = ids[10].with_char('a');
        rows[1].cells[3] = ids[2].with_char('b');
        let mut pen = Some(ids[4]);
        let mut erase = None;
        let mut epoch = 1;
        let renumbered = t.sweep(
            &mut rows,
            &mut [&mut pen, &mut erase],
            || {
                epoch += 1;
                epoch
            },
            false,
        );
        // 4 live of 12: no renumbering.
        assert!(!renumbered);
        assert_eq!(t.stats().live, 4);
        assert!(!t.needs_sweep(), "freed ids are reused first");
        t.check().unwrap();
        // Freed ids come back lowest first.
        assert_eq!(intern(&mut t, style(100)).style_id(), 1);
        t.check().unwrap();

        // Now only the pen is left: renumber.
        rows[0].cells[0] = Cell::BLANK;
        rows[1].cells[3] = Cell::BLANK;
        let old_pen = pen;
        let renumbered = t.sweep(
            &mut rows,
            &mut [&mut pen, &mut erase],
            || {
                epoch += 1;
                epoch
            },
            true,
        );
        assert!(renumbered);
        assert_eq!(t.epoch(), 2);
        assert_eq!(t.stats().len, 2, "ids are dense");
        assert_eq!(pen.unwrap().style_id(), 1);
        assert_eq!(
            t.get(1),
            &style(5),
            "the pen keeps its style: was id {}",
            old_pen.unwrap().style_id()
        );
        t.check().unwrap();
        let u = t.take_updates();
        assert!(u.all);
        assert_eq!(u.styles, vec![(0, Style::DEFAULT), (1, style(5))]);
    }

    #[test]
    fn new_ids_are_capped() {
        let mut t = StyleTable::new(1, 100_000, None);
        let _ = t.take_updates();
        intern(&mut t, style(1));
        intern(&mut t, style(2));
        let u = t.take_updates();
        assert!(!u.all);
        assert_eq!(u.styles.len(), 2);
        for i in 3..10_000 {
            intern(&mut t, style(i));
            if i % 1000 == 0 {
                t.check().unwrap_or_else(|e| panic!("{i}: {e}"));
            }
        }
        assert!(t.take_updates().all, "too many new ids: the whole table");
    }
}
