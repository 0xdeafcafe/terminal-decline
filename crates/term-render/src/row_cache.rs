//! What frame building keeps between frames (cells design §9.3): the
//! snapshot it passes back to term-core, a mirror of each screen's style
//! table, and the instances of each visible row, keyed by the row's
//! [`RowKey`].
//!
//! A cached row holds its backgrounds, glyphs and decorations where it was
//! last drawn, so a row that stayed is copied into the frame as it is, and
//! one that only moved (a scroll, a region scroll) is moved first. Nothing that changes without the row
//! changing is in it: the cursor, highlights and input-method text are
//! drawn over the rows each frame. Each glyph and decoration records its
//! column, so the block cursor can recolour the ones in its cell and
//! input-method text can hide the glyphs under it.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use term_core::{RowKey, ScreenId, Snapshot, Style, Terminal};

use crate::frame::{GlyphInstance, RectInstance, Theme};

/// The cell a decoration belongs to, and how the block cursor
/// recolours it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Mark {
    pub col: u16,
    pub flags: u8,
}

impl Mark {
    /// The cell is dim: its text colour has a lower alpha.
    pub const DIM: u8 = 1 << 0;
    /// The decoration is drawn in the text colour (overline, strike, an
    /// underline without a colour of its own), so it takes the cursor's
    /// text colour under a block cursor.
    pub const TEXT_COLOUR: u8 = 1 << 1;
}

/// The instances of one row, placed for a row top of `y`.
#[derive(Default)]
pub(crate) struct Entry {
    /// The row top, pixels, its instances are placed for. A row drawn
    /// somewhere else (it moved) is moved there first.
    pub y: i32,
    pub backgrounds: Vec<RectInstance>,
    pub glyphs: Vec<GlyphInstance>,
    pub decorations: Vec<RectInstance>,
    pub decoration_marks: Vec<Mark>,
    /// The frame that last showed it.
    used: u64,
}

impl Entry {
    fn clear(&mut self) {
        self.backgrounds.clear();
        self.glyphs.clear();
        self.decorations.clear();
        self.decoration_marks.clear();
    }
}

/// FxHash's mixing step: the keys are a counter or a line number and a
/// column, already well spread, and SipHash would cost ~20 ns a row.
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

impl KeyHasher {
    #[inline]
    fn mix(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl Hasher for KeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Eight bytes at a time: stretches of text are keys too.
        let (chunks, tail) = bytes.as_chunks::<8>();
        for c in chunks {
            self.mix(u64::from_le_bytes(*c));
        }
        let mut rest = [0u8; 8];
        if !tail.is_empty() {
            rest[..tail.len()].copy_from_slice(tail);
            self.mix(u64::from_le_bytes(rest) ^ (tail.len() as u64) << 59);
        }
    }
    #[inline]
    fn write_u8(&mut self, v: u8) {
        self.mix(v as u64);
    }
    #[inline]
    fn write_u16(&mut self, v: u16) {
        self.mix(v as u64);
    }
    #[inline]
    fn write_u32(&mut self, v: u32) {
        self.mix(v as u64);
    }
    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.mix(v);
    }
    #[inline]
    fn write_usize(&mut self, v: usize) {
        self.mix(v as u64);
    }
    #[inline]
    fn write_isize(&mut self, v: isize) {
        self.mix(v as u64);
    }
}

/// A copy of one screen's style table, kept from the snapshots' updates.
#[derive(Default)]
struct Mirror {
    /// The table's epoch it was filled from; `None` before the first.
    epoch: Option<u64>,
    styles: Vec<Style>,
}

/// What the cached instances depend on besides the rows' cells. When any
/// of it changes, every cached row is built again.
#[derive(Clone, PartialEq)]
pub(crate) struct Built {
    pub theme: Theme,
    /// [`term_font::Metrics`]' fields, which has no `PartialEq`.
    pub metrics: [u32; 6],
    /// Cached x positions include the left padding.
    pub padding_x: u32,
    /// [`GlyphSource::generation`](crate::GlyphSource::generation): glyph
    /// positions in the texture.
    pub glyphs: u64,
}

#[derive(Default)]
pub(crate) struct RowCache {
    /// Passed back to term-core each frame, so its buffers are reused and
    /// term-core can compare against what the renderer last saw.
    pub snap: Snapshot,
    index: HashMap<RowKey, u32, BuildHasherDefault<KeyHasher>>,
    entries: Vec<Entry>,
    free: Vec<u32>,
    frame: u64,
    built: Option<Built>,
    terminal_id: u64,
    mirrors: [Mirror; 2],
    /// The slot of each visible row, top to bottom.
    pub order: Vec<u32>,
}

/// Rows the renderer has no instances for: shouldn't happen (term-core
/// copies every row `have` says is missing), but drawn as nothing rather
/// than wrong.
pub(crate) const MISSING: u32 = u32::MAX;

impl RowCache {
    /// Before the snapshot: drop every cached row if what they were built
    /// with has changed.
    pub fn check(&mut self, built: &Built) {
        if self.built.as_ref() != Some(built) {
            self.forget_rows();
            self.built = Some(built.clone());
        }
    }

    /// Drop every cached row, keeping their buffers.
    pub fn forget_rows(&mut self) {
        self.free.clear();
        self.free.extend((0..self.entries.len() as u32).rev());
        self.index.clear();
    }

    /// Take the snapshot, copying only the rows not cached.
    pub fn snapshot(&mut self, term: &mut Terminal) {
        let RowCache { snap, index, .. } = self;
        term.snapshot(snap, |k| index.contains_key(&k));
    }

    /// After the snapshot: apply its style updates and the rules for
    /// dropping rows (§9.1), mark the rows it shows as used and drop the
    /// others. Returns the rows to build, as indexes into `snap.rows`.
    pub fn take(&mut self, to_build: &mut Vec<usize>) {
        if self.snap.terminal_id != self.terminal_id {
            // A new terminal: its counters and line numbers mean nothing
            // here, and its style ids index other tables. term-core copied
            // every row.
            self.terminal_id = self.snap.terminal_id;
            self.mirrors = Default::default();
            self.forget_rows();
        }
        let snap = &self.snap;
        let mirror = &mut self.mirrors[screen_index(snap.screen)];
        if mirror.epoch != Some(snap.styles_epoch) {
            mirror.epoch = Some(snap.styles_epoch);
            mirror.styles.clear();
        }
        for &(id, style) in &snap.style_updates {
            let id = id as usize;
            if id >= mirror.styles.len() {
                mirror.styles.resize(id + 1, Style::DEFAULT);
            }
            mirror.styles[id] = style;
        }

        self.frame += 1;
        let frame = self.frame;
        to_build.clear();
        for (i, row) in snap.rows.iter().enumerate() {
            if let Some(&slot) = self.index.get(&row.key) {
                self.entries[slot as usize].used = frame;
            }
            if row.cells.is_some() {
                to_build.push(i);
            }
        }
        let (entries, free) = (&self.entries, &mut self.free);
        self.index.retain(|_, slot| {
            let keep = entries[*slot as usize].used == frame;
            if !keep {
                free.push(*slot);
            }
            keep
        });
    }

    /// The styles a copied row's cells index.
    pub fn styles_for(&self, history: bool) -> &[Style] {
        if history {
            &self.snap.history_styles
        } else {
            &self.mirrors[screen_index(self.snap.screen)].styles
        }
    }

    /// An empty entry for `key`: its old one, or a free one.
    pub fn entry_for(&mut self, key: RowKey) -> u32 {
        let slot = match self.index.get(&key) {
            Some(&slot) => slot,
            None => {
                let slot = self.free.pop().unwrap_or_else(|| {
                    self.entries.push(Entry::default());
                    self.entries.len() as u32 - 1
                });
                self.index.insert(key, slot);
                slot
            }
        };
        let e = &mut self.entries[slot as usize];
        e.clear();
        e.used = self.frame;
        slot
    }

    pub fn entry(&self, slot: u32) -> &Entry {
        &self.entries[slot as usize]
    }

    pub fn entry_mut(&mut self, slot: u32) -> &mut Entry {
        &mut self.entries[slot as usize]
    }

    /// Move each visible row's instances to its row top, `top(line)`.
    /// Rows too far down to move exactly (their positions may have been
    /// clamped, see [`FAR`]) are dropped after this frame, so they are
    /// copied and built again.
    pub fn move_rows(&mut self, top: impl Fn(usize) -> i32) {
        for (line, &slot) in self.order.iter().enumerate() {
            if slot == MISSING {
                continue;
            }
            let y = top(line);
            let e = &mut self.entries[slot as usize];
            if e.y != y {
                let dy = y - e.y;
                e.y = y;
                shift(&mut e.backgrounds, dy);
                shift(&mut e.glyphs, dy);
                shift(&mut e.decorations, dy);
            }
        }
    }

    /// Drop rows whose top is at or past `far`: instances there may be
    /// clamped to the i16 range, and can't be moved exactly.
    pub fn drop_far(&mut self, far: i32) {
        let (entries, free) = (&self.entries, &mut self.free);
        if !self.order.iter().any(|&s| s != MISSING && entries[s as usize].y >= far) {
            return;
        }
        self.index.retain(|_, slot| {
            let keep = entries[*slot as usize].y < far;
            if !keep {
                free.push(*slot);
            }
            keep
        });
    }

    /// Fill `order` with each visible row's slot.
    pub fn place(&mut self) {
        self.order.clear();
        for row in &self.snap.rows {
            let slot = self.index.get(&row.key).copied();
            debug_assert!(slot.is_some(), "no instances for {:?}", row.key);
            self.order.push(slot.unwrap_or(MISSING));
        }
    }

    /// Rows cached now.
    pub fn len(&self) -> usize {
        self.index.len()
    }
}

/// Move instances `dy` pixels down, clamped as positions are when built.
fn shift<T: Placed>(v: &mut [T], dy: i32) {
    for i in v {
        let y = i.y();
        *y = (*y as i32 + dy).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }
}

/// An instance with a y position.
pub(crate) trait Placed {
    fn y(&mut self) -> &mut i16;
}

impl Placed for RectInstance {
    #[inline(always)]
    fn y(&mut self) -> &mut i16 {
        &mut self.y
    }
}

impl Placed for GlyphInstance {
    #[inline(always)]
    fn y(&mut self) -> &mut i16 {
        &mut self.y
    }
}

fn screen_index(s: ScreenId) -> usize {
    match s {
        ScreenId::Primary => 0,
        ScreenId::Alternate => 1,
    }
}
