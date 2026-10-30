//! History (scrollback v2, §11 of the cells design): every line that
//! scrolled off the primary screen, stored compactly and expanded back
//! into cells only when shown, copied or popped.
//!
//! Lines are kept in blocks. Each block keeps its lines' **text
//! contiguous**, in one array, apart from the rest of each line (its
//! flags, style runs, cluster list and width exceptions, in `meta`), so a
//! search can scan the text directly. A block also has its own style
//! dictionary: lines name styles by slot, so history holds no screen
//! style ids and the screen's table never has to keep an id alive for it.
//!
//! The block being written is an [`OpenBlock`] of growable parts. It is
//! sealed into a [`BlockData`], immutable and shared through an `Arc`, when
//! the next line would take a part past 64 KB (so every offset is 16 bits),
//! or when the ambiguous-width setting changes. History's own record of a
//! sealed block is a [`BlockEntry`]: the shared data, and how many of its
//! lines are still in use (`pop()` lowers that, never the data).
//!
//! Limits: `limit` lines, plus a 32 MB backstop counting every byte the
//! blocks hold. Both free whole blocks from the oldest end; lines over the
//! line limit in a block that can't be freed yet are hidden.
//!
//! A stored line, in `meta` and `text` (§11.3):
//!
//! ```text
//! meta:  flags:u8 [tlen:varint] [runs] [clusters] [widths]
//! text:  the line's UTF-8, then 0x0A if the line ends its logical line
//! ```
//!
//! - `tlen`: the text length, for WRAPPED lines (their text has no end
//!   byte) and, behind the `extra` byte's TLEN bit, for unwrapped lines
//!   with 128 bytes of text or more. Wrapped lines keep every cell, trailing blanks as spaces;
//!   others stop at the last cell with content.
//! - text: one entry per cell: a scalar, a cluster's scalars, a space for
//!   an empty cell; nothing for spacers.
//! - `runs`: `count`, then per run `varint(cells << 3 | slot)` and, when
//!   slot is 7, `varint(index)`: the dictionary entry (0 = default). Runs
//!   count columns; columns after the last run are in the default style.
//! - `clusters`: `count`, then per cluster `gap` (scalars since the last
//!   one ended) and `extra << 1 | wide` (its scalars after the first, and
//!   whether it takes two cells: a cluster's width is stored, not worked
//!   out again).
//! - `widths`: `count`, then per exception `gap` (text entries since the
//!   last one) and the width (1 or 2): only single-scalar cells whose
//!   width isn't what the Unicode table and the block's ambiguous setting
//!   give.
//! - `FILL`: the row's last column was an empty cell whose style shows,
//!   so the last run's style continues to the end of the row at any width.

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use crate::cell::{Cell, Color, ColorKind, Mark, MarkKind, Row, Width};
use crate::image::ImageKey;
use crate::links::LinkValue;
use crate::search::Filter;
use crate::style::{Attrs, Style, Underline};
use crate::unicode;

/// The most bytes in one part of a block (meta, text, the dictionary), so
/// that every offset fits in 16 bits.
const PART: usize = u16::MAX as usize;
/// A checkpoint (a text offset) every this many lines.
const CHECKPOINT: usize = 16;
/// The hidden backstop: history never holds more than this many bytes (it
/// frees whole blocks from the oldest end until it doesn't, each time a
/// block is sealed).
pub const BYTE_LIMIT: usize = 32 << 20;
/// Rows wider than this are cut to it when stored (§11.2: a line fits in
/// an empty block).
const MAX_CELLS: usize = 4096;
/// Ends an unwrapped line's text.
const END: u8 = 0x0A;
/// Smallest allocation for a part of the open block.
const MIN_PART: usize = 256;

/// Bits of a stored line's flags byte.
mod flag {
    /// The logical line continues on the next stored line; `tlen` follows.
    pub const WRAPPED: u8 = 1 << 0;
    /// The last column was an edge spacer.
    pub const EDGE: u8 = 1 << 1;
    /// Style runs follow.
    pub const STYLED: u8 = 1 << 2;
    /// A cluster list follows.
    pub const CLUSTERS: u8 = 1 << 3;
    /// Width exceptions follow.
    pub const WIDTHS: u8 = 1 << 4;
    /// The last run's style fills to the end of the row.
    pub const FILL: u8 = 1 << 5;
    /// Prompt marks follow (OSC 133).
    pub const MARKS: u8 = 1 << 6;
    /// An `extra` byte follows the flags.
    pub const EXTRA: u8 = 1 << 7;
}

/// Bits of a stored line's `extra` byte. Bits 1–2 are double width and
/// height (later).
mod extra {
    /// A placeholder list follows the marks (§11.3).
    pub const PLACEHOLDERS: u8 = 1 << 0;
    /// An unwrapped line's text length follows too, as for wrapped lines:
    /// written for text of [`super::LONG_TEXT`] bytes or more, so walking
    /// from a checkpoint past long lines (a gradient's 480 bytes) needs no
    /// scan for their end byte.
    pub const TLEN: u8 = 1 << 3;
}

/// Flags bits search reads (§13.5): a line with either can't take the
/// all-ASCII shortcut from byte offsets to columns.
pub(crate) const SEARCH_SLOW_FLAGS: u8 = flag::CLUSTERS | flag::WIDTHS;

/// Unwrapped lines with this much text or more record its length.
const LONG_TEXT: usize = 128;

/// A stored line's flags, its text length if recorded (`tlen`), and where
/// its sections start in its meta.
#[inline]
fn head(meta: &[u8]) -> (u8, Option<usize>, usize) {
    let flags = meta[0];
    let mut pos = 1;
    let mut has_tlen = flags & flag::WRAPPED != 0;
    if flags & flag::EXTRA != 0 {
        has_tlen |= meta.get(1).is_some_and(|x| x & extra::TLEN != 0);
        pos = 2;
    }
    let tlen = has_tlen.then(|| get_varint(meta, &mut pos) as usize);
    (flags, tlen, pos)
}

/// A stored line, as counting its rows at another width needs it (§12.3),
/// read from its flags, style runs and text without decoding its cells.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape {
    /// It soft-wraps onto the next stored line.
    pub(crate) wrapped: bool,
    /// The cells it decodes to, when every one is narrow and nothing but
    /// its blank tail is trimmed (ASCII text, and no clusters, width
    /// exceptions, edge spacer, marks or `FILL`).
    pub(crate) cells: Option<usize>,
    /// How many cells at its end are blank in the default style, the
    /// ones trimming a logical line's tail drops, if that is known.
    pub(crate) blank_tail: Option<usize>,
}

/// What a sealed block holds. It never changes, and is shared as
/// `Arc<BlockData>`: a search or a decode cache can hold one without the
/// lock, and a block freed from history lives until they let it go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockData {
    /// Each line's flags and sections.
    meta: Box<[u8]>,
    /// Offset of each line in `meta`.
    starts: Box<[u16]>,
    /// Every line's text back to back; 0x0A ends each logical line.
    text: Box<[u8]>,
    /// Offset in `text` of every 16th line.
    checkpoints: Box<[u16]>,
    /// The style dictionary, encoded (§11.5).
    styles: Box<[u8]>,
    /// Offset of each dictionary entry (slot 1 onwards) in `styles`.
    style_starts: Box<[u16]>,
    /// The block's own hyperlinks (OSC 8), which dictionary entries name
    /// by index (1 onwards). Shared with the screen's link table by `Arc`.
    links: Box<[Arc<LinkValue>]>,
    /// Stable line number of the first line.
    first: u64,
    /// The width every line here was written at, 0 if mixed.
    width: u16,
    /// The ambiguous-width setting its lines were written with.
    ambiguous: u8,
    /// The first line continues a logical line from the block before.
    head_continues: bool,
    /// Some line here has prompt marks (OSC 133).
    has_marks: bool,
    /// Decline's own images its dictionary names, each once (images.md
    /// §4.3): they are needed until the block is freed.
    images: Box<[ImageKey]>,
}

/// Bytes a block counts for one of its links.
fn link_bytes(l: &LinkValue) -> usize {
    std::mem::size_of::<Arc<LinkValue>>() + std::mem::size_of::<LinkValue>() + 16 + l.uri.len() + l.id.as_ref().map_or(0, |s| s.len())
}

impl BlockData {
    fn view(&self, len: usize) -> View<'_> {
        View {
            meta: &self.meta,
            starts: &self.starts[..len],
            text: &self.text,
            checkpoints: &self.checkpoints,
            styles: &self.styles,
            style_starts: &self.style_starts,
            links: &self.links,
            ambiguous: self.ambiguous,
        }
    }

    /// Every byte it holds, for the backstop.
    fn bytes(&self) -> usize {
        std::mem::size_of::<BlockData>()
            + 2 * std::mem::size_of::<usize>() // the Arc's counts
            + self.meta.len()
            + self.starts.len() * 2
            + self.text.len()
            + self.checkpoints.len() * 2
            + self.styles.len()
            + self.style_starts.len() * 2
            + self.links.iter().map(|l| link_bytes(l)).sum::<usize>()
            + self.images.len() * std::mem::size_of::<ImageKey>()
    }

    /// Decline's own images this block's lines name.
    pub fn images(&self) -> &[ImageKey] {
        &self.images
    }

    /// Stable line number of the first line.
    pub fn first(&self) -> u64 {
        self.first
    }

    /// Lines written to it (a [`BlockEntry`] may use fewer).
    pub fn lines(&self) -> usize {
        self.starts.len()
    }

    /// All its lines' text, back to back.
    pub fn text(&self) -> &[u8] {
        &self.text
    }

    /// The width its lines were written at, 0 if mixed.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// The ambiguous-width setting its lines were written with.
    pub fn ambiguous(&self) -> u8 {
        self.ambiguous
    }

    /// Its first line continues a logical line from the block before.
    pub fn head_continues(&self) -> bool {
        self.head_continues
    }

    /// Some line here has prompt marks.
    pub fn has_marks(&self) -> bool {
        self.has_marks
    }
}

/// History's own record of a sealed block, changed only under the lock.
#[derive(Debug)]
struct BlockEntry {
    data: Arc<BlockData>,
    /// Lines still in use: `pop()` lowers it, the shared data is untouched.
    len: u32,
    /// Its search filter (§13.6), built in the background; None = scan it.
    filter: Option<Arc<Filter>>,
}

impl BlockEntry {
    /// Bytes it counts against the backstop: its data and its filter.
    fn bytes(&self) -> usize {
        self.data.bytes() + self.filter.as_ref().map_or(0, |_| Filter::BYTES)
    }
}

/// The block being written: the parts of a [`BlockData`] as growable
/// vectors, plus the encoder's style dictionary and slot cache.
#[derive(Debug)]
struct OpenBlock {
    meta: Vec<u8>,
    starts: Vec<u16>,
    text: Vec<u8>,
    checkpoints: Vec<u16>,
    styles: Vec<u8>,
    style_starts: Vec<u16>,
    first: u64,
    /// `u16::MAX` until a line is written.
    width: u16,
    ambiguous: u8,
    head_continues: bool,
    has_marks: bool,
    /// Style (its link an index in `links`) → dictionary slot, for this
    /// block only.
    dict: HashMap<Key, u32, BuildHasherDefault<FxHasher>>,
    links: Vec<Arc<LinkValue>>,
    /// Link → its index in `links` (1 onwards).
    link_index: HashMap<Arc<LinkValue>, u32>,
    /// Bytes the links count.
    link_bytes: usize,
    /// Screen style id → slot, indexed by `id & 255`, in front of `dict`:
    /// the common case is one compare per run. Reset with the block, and
    /// whenever the screen's table may have reused ids (a sweep, another
    /// table).
    cache: [(u32, u32); CACHE],
    /// Entries are tagged `id | gen << 24` (style ids are below 2^24):
    /// dropping the cache is a new generation, not a 2 KB memset.
    cache_gen: u32,
    /// Decline's own images the dictionary names.
    images: Vec<ImageKey>,
}

const NO_ENTRY: (u32, u32) = (u32::MAX, 0);

/// Entries in the slot cache. 256 rather than §11.2's 64: a gradient
/// line has a new style in each of its 160 cells, and 2 KB per terminal
/// is cheap.
const CACHE: usize = 256;

/// A style as the dictionary's key, hashed as three words (as the
/// screen's style table hashes it) rather than field by field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key(Style);

impl std::hash::Hash for Key {
    #[inline]
    fn hash<H: Hasher>(&self, h: &mut H) {
        let s = &self.0;
        h.write_u64(s.fg.0 as u64 | (s.bg.0 as u64) << 32);
        h.write_u64(s.underline_color.0 as u64 | (s.link as u64) << 32);
        h.write_u64(s.attrs.bits() as u64 | (s.underline as u64) << 16 | (s.image_id_high as u64) << 24);
    }
}

/// FxHash, for the dictionary's map: a style is a few small words, and
/// SipHash's resistance to chosen keys buys nothing here.
#[derive(Default)]
struct FxHasher(u64);

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.add(b as u64);
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn write_isize(&mut self, i: isize) {
        self.add(i as u64);
    }
}

impl OpenBlock {
    fn new(first: u64, ambiguous: u8) -> OpenBlock {
        OpenBlock {
            meta: Vec::new(),
            starts: Vec::new(),
            text: Vec::new(),
            checkpoints: Vec::new(),
            styles: Vec::new(),
            style_starts: Vec::new(),
            first,
            width: u16::MAX,
            ambiguous,
            head_continues: false,
            has_marks: false,
            dict: HashMap::default(),
            links: Vec::new(),
            link_index: HashMap::new(),
            link_bytes: 0,
            cache: [NO_ENTRY; CACHE],
            cache_gen: 1,
            images: Vec::new(),
        }
    }

    fn lines(&self) -> usize {
        self.starts.len()
    }

    fn view(&self) -> View<'_> {
        View {
            meta: &self.meta,
            starts: &self.starts,
            text: &self.text,
            checkpoints: &self.checkpoints,
            styles: &self.styles,
            style_starts: &self.style_starts,
            links: &self.links,
            ambiguous: self.ambiguous,
        }
    }

    /// Bytes allocated, spare capacity included.
    fn bytes(&self) -> usize {
        std::mem::size_of::<OpenBlock>()
            + self.meta.capacity()
            + self.starts.capacity() * 2
            + self.text.capacity()
            + self.checkpoints.capacity() * 2
            + self.styles.capacity()
            + self.style_starts.capacity() * 2
            + self.dict.capacity() * (std::mem::size_of::<(Key, u32)>() + 1)
            + self.link_bytes
            + self.link_index.capacity() * (std::mem::size_of::<(Arc<LinkValue>, u32)>() + 1)
    }

    fn reset_cache(&mut self) {
        self.cache_gen += 1;
        if self.cache_gen == 256 {
            self.cache = [NO_ENTRY; CACHE];
            self.cache_gen = 1;
        }
    }

    /// Turn it into a sealed block, leaving an empty one starting where it
    /// ended.
    fn seal(&mut self) -> BlockData {
        let next = OpenBlock::new(self.first + self.lines() as u64, self.ambiguous);
        let mut b = std::mem::replace(self, next);
        // The next block takes over the dictionary's map, emptied: a
        // gradient fills one with ~13k styles a block, and growing it
        // again from nothing rehashes it a dozen times. One much bigger
        // than this block needed is shrunk.
        let used = b.dict.len();
        let mut dict = std::mem::take(&mut b.dict);
        dict.clear();
        if dict.capacity() > 4 * used.max(16) {
            dict.shrink_to(2 * used);
        }
        self.dict = dict;
        BlockData {
            meta: b.meta.into_boxed_slice(),
            starts: b.starts.into_boxed_slice(),
            text: b.text.into_boxed_slice(),
            checkpoints: b.checkpoints.into_boxed_slice(),
            styles: b.styles.into_boxed_slice(),
            style_starts: b.style_starts.into_boxed_slice(),
            links: b.links.into_boxed_slice(),
            first: b.first,
            width: if b.width == u16::MAX { 0 } else { b.width },
            ambiguous: b.ambiguous,
            head_continues: b.head_continues,
            has_marks: b.has_marks,
            images: b.images.into_boxed_slice(),
        }
    }

    /// The dictionary slot of screen style `id` (0 = default), adding the
    /// style to the dictionary when it is new.
    #[inline]
    fn slot(&mut self, id: u32, styles: &[Style], links: &[Option<Arc<LinkValue>>]) -> u32 {
        if id == 0 {
            return 0;
        }
        let e = self.cache[(id as usize) & (CACHE - 1)];
        if e.0 == id | self.cache_gen << 24 && id < (1 << 24) - 1 {
            return e.1;
        }
        self.slot_slow(id, styles, links)
    }

    #[cold]
    #[inline(never)]
    fn slot_slow(&mut self, id: u32, styles: &[Style], links: &[Option<Arc<LinkValue>>]) -> u32 {
        let mut s = styles.get(id as usize).copied().unwrap_or_default();
        // The link moves to the block's own table.
        s.link = match links.get(s.link as usize).and_then(|l| l.as_ref()) {
            Some(l) => self.link(l),
            None => 0,
        };
        let slot = if s == Style::DEFAULT {
            0
        } else {
            // One hash, whether the style is new or not.
            match self.dict.entry(Key(s)) {
                std::collections::hash_map::Entry::Occupied(e) => *e.get(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    room(&mut self.style_starts, 1);
                    room(&mut self.styles, 15);
                    self.style_starts.push(self.styles.len() as u16);
                    put_style(&mut self.styles, &s);
                    if s.attrs.contains(Attrs::OWN_IMAGE) {
                        let key = crate::image::placeholder_key(&s);
                        if !self.images.contains(&key) {
                            self.images.push(key);
                        }
                    }
                    *e.insert(self.style_starts.len() as u32)
                }
            }
        };
        if id < (1 << 24) - 1 {
            self.cache[(id as usize) & (CACHE - 1)] = (id | self.cache_gen << 24, slot);
        }
        slot
    }

    /// The block's index for `link`, adding it when it is new.
    fn link(&mut self, link: &Arc<LinkValue>) -> u32 {
        if let Some(&i) = self.link_index.get(link) {
            return i;
        }
        self.links.push(link.clone());
        self.link_bytes += link_bytes(link);
        let i = self.links.len() as u32;
        self.link_index.insert(link.clone(), i);
        i
    }

    /// Store `row`. False, with nothing changed, if a part would pass
    /// 64 KB.
    fn push(&mut self, row: &Row, styles: &[Style], links: &[Option<Arc<LinkValue>>], scratch: &mut Scratch) -> bool {
        let before = (self.meta.len(), self.text.len(), self.styles.len(), self.style_starts.len());
        let n = self.lines();
        room(&mut self.starts, 1);
        self.starts.push(before.0 as u16);
        let checkpoint = n.is_multiple_of(CHECKPOINT);
        if checkpoint {
            room(&mut self.checkpoints, 1);
            self.checkpoints.push(before.1 as u16);
        }
        self.encode(row, styles, links, scratch);
        if self.meta.len() <= PART && self.text.len() <= PART && self.styles.len() <= PART {
            let w = row.cells.len() as u16;
            self.width = if n == 0 || self.width == w { w } else { 0 };
            self.has_marks |= row.has_marks();
            return true;
        }
        // Undo it: the caller seals the block (or, for an empty block,
        // cuts the row) and writes the line again.
        self.meta.truncate(before.0);
        self.text.truncate(before.1);
        self.styles.truncate(before.2);
        self.style_starts.truncate(before.3);
        // The line may have grown a part past 64 KB: give that back.
        self.meta.shrink_to(PART + 1);
        self.text.shrink_to(PART + 1);
        self.styles.shrink_to(PART + 1);
        self.starts.pop();
        if checkpoint {
            self.checkpoints.pop();
        }
        let slots = before.3 as u32;
        self.dict.retain(|_, s| *s <= slots);
        // Links the dropped entries added stay: unused, and freed with the
        // block.
        self.reset_cache();
        false
    }

    fn encode(&mut self, row: &Row, styles: &[Style], links: &[Option<Arc<LinkValue>>], scratch: &mut Scratch) {
        let cells = &row.cells[..];
        let cols = cells.len();
        let wrapped = row.wrapped;

        // The cells up to the last that draws anything: blank cells after
        // it are dropped (a wrapped row keeps them all). Most rows end in
        // blanks, so this one backward scan is most of the work for short
        // lines; everything after looks at these cells only.
        let len = if wrapped {
            scratch.zero_from = cols;
            cols
        } else {
            let (len, zero_from) = blank_tail(cells);
            scratch.zero_from = zero_from;
            len
        };

        // Common case: every cell is narrow ASCII (or empty) in the
        // default style, so the text is a byte copy and there are no runs.
        let or = cells[..len].iter().fold(0, |a, c| a | c.bits());
        if or & !0x7F == 0 && !row.has_marks() {
            let end = if wrapped { cols } else { trim_spaces(&cells[..len]) };
            room(&mut self.meta, 7);
            if wrapped {
                self.meta.push(flag::WRAPPED);
                put_varint(&mut self.meta, end as u32);
            } else if end >= LONG_TEXT {
                self.meta.extend_from_slice(&[flag::EXTRA, extra::TLEN]);
                put_varint(&mut self.meta, end as u32);
            } else {
                self.meta.push(0);
            }
            room(&mut self.text, end + 1);
            self.text
                .extend(cells[..end].iter().map(|c| if c.bits() == 0 { b' ' } else { c.bits() as u8 }));
            if !wrapped {
                self.text.push(END);
            }
            return;
        }

        let edge = wrapped && cells[cols - 1].width() == Width::EdgeSpacer;
        let text_cols = cols - edge as usize;
        // Runs go up to the last cell that would draw something (blank
        // cells after it are dropped), the text up to the last one with
        // content other than a space.
        let run_end = if wrapped {
            cols
        } else {
            // A wide cell's spacer is in its style, even when blank.
            if len > 0 && len < cols && cells[len - 1].is_wide() {
                len + 1
            } else {
                len
            }
        };
        // Every stored cell narrow ASCII (or empty), in any style: the
        // text is a byte copy, with no clusters, width exceptions or
        // placeholders (`ls --color`, compiler output).
        let ascii = or & (Cell::CONTENT & !0x7F | Cell::KIND | Cell::WIDTH) == 0;
        let text_end = if wrapped {
            text_cols
        } else if ascii {
            let mut end = len;
            while end > 0 && matches!(cells[end - 1].bits() & 0x7F, 0 | 0x20) {
                end -= 1;
            }
            end
        } else {
            text_end(&cells[..run_end])
        };
        let last = cells[cols - 1];
        let fill = !wrapped && last.shows_empty() && last.bits() & (Cell::CONTENT | Cell::KIND) == 0 && !last.is_spacer();

        // The text, and the clusters and width exceptions it needs.
        scratch.clusters.clear();
        scratch.widths.clear();
        scratch.placeholders.clear();
        let tlen = if ascii {
            room(&mut self.text, text_end + 1);
            self.text.extend(
                cells[..text_end]
                    .iter()
                    .map(|c| if c.bits() as u8 == 0 { b' ' } else { c.bits() as u8 }),
            );
            text_end
        } else {
            self.encode_text(row, text_end, scratch)
        };
        if !wrapped {
            self.text.push(END);
        }

        // Runs, by column, merged when neighbours share a slot; the
        // default tail needs none.
        scratch.runs.clear();
        let cells_run = &cells[..run_end];
        let mut i = 0;
        while i < cells_run.len() {
            // Cells of one style id, then their slot once. (Two ids can
            // share a slot: styles differing only in a link that
            // isn't kept.)
            let id = cells_run[i].style_id();
            let n = cells_run[i..]
                .iter()
                .position(|c| c.style_id() != id)
                .unwrap_or(cells_run.len() - i);
            let slot = self.slot(id, styles, links);
            match scratch.runs.last_mut() {
                Some((m, s)) if *s == slot => *m += n as u32,
                _ => scratch.runs.push((n as u32, slot)),
            }
            i += n;
        }
        while scratch.runs.last().is_some_and(|r| r.1 == 0) {
            scratch.runs.pop();
        }

        let mut flags = 0;
        flags |= if wrapped { flag::WRAPPED } else { 0 };
        flags |= if edge { flag::EDGE } else { 0 };
        flags |= if scratch.runs.is_empty() { 0 } else { flag::STYLED };
        flags |= if scratch.clusters.is_empty() { 0 } else { flag::CLUSTERS };
        flags |= if scratch.widths.is_empty() { 0 } else { flag::WIDTHS };
        flags |= if fill && !scratch.runs.is_empty() { flag::FILL } else { 0 };
        flags |= if row.has_marks() { flag::MARKS } else { 0 };
        let long = !wrapped && tlen >= LONG_TEXT;
        let extra_bits = if long { extra::TLEN } else { 0 } | if scratch.placeholders.is_empty() { 0 } else { extra::PLACEHOLDERS };
        flags |= if extra_bits != 0 { flag::EXTRA } else { 0 };
        room(
            &mut self.meta,
            6 + 6
                + scratch.runs.len() * 8
                + scratch.clusters.len() * 10
                + scratch.widths.len() * 6
                + 4 * 12
                + 3
                + scratch.placeholders.len() * 9,
        );
        self.meta.push(flags);
        if extra_bits != 0 {
            self.meta.push(extra_bits);
        }
        if wrapped || long {
            put_varint(&mut self.meta, tlen as u32);
        }
        if !scratch.runs.is_empty() {
            put_varint(&mut self.meta, scratch.runs.len() as u32);
            for &(n, slot) in &scratch.runs {
                if slot < 7 {
                    put_varint(&mut self.meta, n << 3 | slot);
                } else {
                    put_varint(&mut self.meta, n << 3 | 7);
                    put_varint(&mut self.meta, slot);
                }
            }
        }
        if !scratch.clusters.is_empty() {
            put_varint(&mut self.meta, scratch.clusters.len() as u32);
            for &(gap, extra) in &scratch.clusters {
                put_varint(&mut self.meta, gap);
                put_varint(&mut self.meta, extra);
            }
        }
        if !scratch.widths.is_empty() {
            put_varint(&mut self.meta, scratch.widths.len() as u32);
            for &(gap, w) in &scratch.widths {
                put_varint(&mut self.meta, gap);
                self.meta.push(w);
            }
        }
        if flags & flag::MARKS != 0 {
            // count, then per mark: kind (bit 7: an exit status follows),
            // column, status.
            let at = self.meta.len();
            self.meta.push(0);
            for m in row.marks() {
                self.meta[at] += 1;
                self.meta.push(m.kind as u8 | if m.status.is_some() { 0x80 } else { 0 });
                put_varint(&mut self.meta, m.col as u32);
                if let Some(status) = m.status {
                    put_varint(&mut self.meta, status);
                }
            }
        }
        if !scratch.placeholders.is_empty() {
            // count, then per stretch: gap (cells since the last one
            // ended), cells, row, first column.
            put_varint(&mut self.meta, scratch.placeholders.len() as u32);
            let mut end = 0;
            for &(start, n, r, col) in &scratch.placeholders {
                put_varint(&mut self.meta, start - end);
                put_varint(&mut self.meta, n);
                put_varint(&mut self.meta, r as u32);
                put_varint(&mut self.meta, col as u32);
                end = start + n;
            }
        }
    }

    /// The text of `row`'s cells up to `text_end`, with the
    /// clusters, width exceptions and placeholders it needs, into the
    /// scratch lists. Returns the text's length.
    fn encode_text(&mut self, row: &Row, text_end: usize, scratch: &mut Scratch) -> usize {
        let cells = &row.cells[..];
        let cluster_bytes = row.clusters().map_or(0, |x| x.raw().0.len());
        // The text is written in place into room for its longest form
        // (4 bytes a cell, plus cluster text), then cut to length: no
        // capacity check or `memcpy` call per character.
        let most = text_end * 4 + cluster_bytes;
        room(&mut self.text, most + 1);
        let text_start = self.text.len();
        self.text.resize(text_start + most, 0);
        let buf = &mut self.text[text_start..];
        let mut at = 0;
        let amb = self.ambiguous;
        let (mut scalars, mut cluster_end, mut entry, mut width_next) = (0u32, 0u32, 0u32, 0u32);
        // The last scalar looked up in the Unicode table and its width.
        let mut last = ('\0', 1);
        for cell in &cells[..text_end] {
            let c = *cell;
            if c.is_spacer() {
                continue;
            }
            let bits = c.bits();
            let predicted;
            if bits & (Cell::CONTENT & !0x7F | Cell::KIND | Cell::WIDTH) == 0 {
                // Narrow ASCII (or empty), not a cluster: the usual case.
                buf[at] = if bits as u8 == 0 { b' ' } else { bits as u8 };
                at += 1;
                scalars += 1;
                entry += 1;
                continue;
            } else if c.is_cluster() {
                let s = row.clusters().map_or("", |x| x.get(c.codepoint()));
                let n = s.chars().count() as u32;
                if n == 0 {
                    buf[at] = b' ';
                    at += 1;
                    predicted = 1;
                    scalars += 1;
                } else {
                    buf[at..at + s.len()].copy_from_slice(s.as_bytes());
                    at += s.len();
                    if n > 1 {
                        // A cluster's width goes in its entry: no need to
                        // segment it again when it is read back.
                        let wide = c.is_wide() as u32;
                        scratch.clusters.push((scalars - cluster_end, (n - 1) << 1 | wide));
                        cluster_end = scalars + n;
                        predicted = 1 + wide as u8;
                    } else {
                        predicted = single_width(s.chars().next().unwrap_or(' '), amb);
                    }
                    scalars += n;
                }
            } else if c.is_placeholder() {
                // 0x00 in the text (never matches a search, copied as a
                // space); the piece goes in the line's placeholder list.
                buf[at] = 0;
                at += 1;
                scalars += 1;
                predicted = 1;
                // Its column, from where it is (no counter in the loop for
                // the rare case).
                let x = (std::ptr::from_ref(cell).addr() - cells.as_ptr().addr()) / std::mem::size_of::<Cell>();
                add_placeholder(&mut scratch.placeholders, x as u32, c);
            } else {
                let ch = match c.codepoint() {
                    0 => ' ',
                    cp => char::from_u32(cp).unwrap_or(' '),
                };
                at += ch.encode_utf8(&mut buf[at..at + 4]).len();
                predicted = if scratch.widths_as_written && ch != ' ' {
                    actual(c)
                } else {
                    if ch != last.0 {
                        last = (ch, scratch.memo.width(ch, amb));
                    }
                    last.1
                };
                scalars += 1;
            }
            let actual = actual(c);
            if actual != predicted {
                scratch.widths.push((entry - width_next, actual));
                width_next = entry + 1;
            }
            entry += 1;
        }
        self.text.truncate(text_start + at);
        if self.text.capacity() > PART + 1 && self.text.len() <= PART {
            // The room for the longest form went past 64 KB and the text
            // itself didn't: give the rest back.
            self.text.shrink_to(PART + 1);
        }
        at
    }

    /// Drop the newest line.
    fn pop(&mut self) {
        let n = self.lines() - 1;
        let (at, _) = self.view().text_span(n);
        self.meta.truncate(self.starts[n] as usize);
        self.text.truncate(at);
        self.starts.pop();
        if n.is_multiple_of(CHECKPOINT) {
            self.checkpoints.pop();
        }
    }
}

/// Add the placeholder `c` at column `x` to a line's stretches.
#[cold]
#[inline(never)]
fn add_placeholder(stretches: &mut Vec<(u32, u32, u16, u16)>, x: u32, c: Cell) {
    let Some((r, col)) = c.placeholder() else { return };
    match stretches.last_mut() {
        Some((start, n, pr, pc)) if *start + *n == x && *pr == r && *pc as u32 + *n == col as u32 => *n += 1,
        _ => stretches.push((x, 1, r, col)),
    }
}

/// Buffers the encoder reuses, so storing a line allocates nothing.
#[derive(Debug, Default)]
struct Scratch {
    /// (gap, extra scalars) per cluster.
    clusters: Vec<(u32, u32)>,
    /// (gap, width) per width exception.
    widths: Vec<(u32, u8)>,
    /// (cells, slot) per run.
    runs: Vec<(u32, u32)>,
    /// (first column, cells, image row, first image column) per stretch
    /// of placeholders.
    placeholders: Vec<(u32, u32, u16, u16)>,
    memo: WidthMemo,
    /// Where the last row stored ends in cells that are all
    /// [`Cell::BLANK`] (its length if unknown).
    zero_from: usize,
    /// Every single-scalar cell has the width the Unicode table gives at
    /// the open block's setting ([`Scrollback::set_widths_as_written`]),
    /// so storing skips the check.
    widths_as_written: bool,
}

/// A cell's width in cells (a wide cell's spacer aside).
#[inline(always)]
fn actual(c: Cell) -> u8 {
    if c.is_wide() { 2 } else { 1 }
}

/// The widths of the last scalars looked up, direct-mapped by scalar:
/// lines that scroll by repeat their characters (box drawing, Cyrillic,
/// symbols), so the store looks most of them up in the table once.
#[derive(Debug)]
struct WidthMemo([(u32, u8); 64]);

impl Default for WidthMemo {
    fn default() -> Self {
        WidthMemo([(0, 1); 64])
    }
}

impl WidthMemo {
    #[inline]
    fn width(&mut self, c: char, ambiguous: u8) -> u8 {
        // The setting is part of the key, so a change needs no reset.
        let key = c as u32 | (ambiguous as u32) << 22;
        let e = &mut self.0[(c as usize) & 63];
        if e.0 != key {
            *e = (key, single_width(c, ambiguous));
        }
        e.1
    }
}

/// Make room for `add` more elements, growing by doubling but never past
/// a part's 64 KB: a line that doesn't fit then grows the vector itself,
/// and the block is sealed (and shrunk) straight after.
#[inline]
fn room<T>(v: &mut Vec<T>, add: usize) {
    let need = v.len() + add;
    if need > v.capacity() {
        let cap = (v.capacity() * 2).max(need).clamp(MIN_PART, PART + 1);
        if cap > v.capacity() {
            v.reserve_exact(cap - v.len());
        }
    }
}

/// Scripts that are two cells wide whatever the ambiguous-width setting
/// (none of their characters is ambiguous): CJK ideographs, kana and
/// Hangul syllables. Checked against the table for every scalar in them.
const ALWAYS_WIDE: [(u32, u32); 5] = [
    (0x3041, 0x3096),
    (0x30A1, 0x30FA),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xAC00, 0xD7A3),
];

/// The width a single scalar's cell is given when written (§6.3): 2 for
/// wide characters (and ambiguous ones with the setting at 2), else 1.
/// ASCII and the common wide scripts need no table lookup.
#[inline]
fn single_width(c: char, ambiguous: u8) -> u8 {
    let cp = c as u32;
    if cp < 0x80 {
        return 1;
    }
    if (0x3041..=0xD7A3).contains(&cp) && ALWAYS_WIDE.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)) {
        return 2;
    }
    if unicode::props(c).width_with(ambiguous) == 2 { 2 } else { 1 }
}

/// `cells` (narrow ASCII, ending in a cell that isn't blank) without the
/// typed spaces at their end.
#[inline]
fn trim_spaces(cells: &[Cell]) -> usize {
    let mut len = cells.len();
    while len > 0 && matches!(cells[len - 1].bits(), 0 | 0x20) {
        len -= 1;
    }
    len
}

/// Cells up to and including the last that isn't blank ([`Cell::is_blank`]),
/// and where the tail of cells that are all [`Cell::BLANK`] starts (found
/// 16 cells at a time, so it may start later than it could).
#[inline]
fn blank_tail(cells: &[Cell]) -> (usize, usize) {
    // Skip trailing blanks 16 at a time (an OR over 128 bytes, which
    // vectorises, with no bounds checks), then one at a time.
    let mut len = cells.len();
    let mut zero_from = cells.len();
    for chunk in cells.rchunks_exact(16) {
        let or = chunk.iter().fold(0, |a, c| a | c.bits());
        if or == 0 && zero_from == len {
            zero_from -= 16;
        }
        if or & Cell::NOT_BLANK != 0 {
            break;
        }
        len -= 16;
    }
    while len > 0 && cells[len - 1].is_blank() {
        len -= 1;
    }
    (len, zero_from)
}

/// Columns of text to store for an unwrapped row: up to the last cell with
/// content other than a space, or a wide one (and its spacer).
pub(crate) fn text_end(cells: &[Cell]) -> usize {
    // An empty wide cell keeps its width only through the text.
    let has_text = |c: &Cell| {
        c.is_wide() || c.bits() & (Cell::CONTENT | Cell::KIND) != 0 && (c.bits() & Cell::KIND != 0 || c.codepoint() != ' ' as u32)
    };
    match cells.iter().rposition(has_text) {
        Some(i) if cells[i].is_wide() => (i + 2).min(cells.len()),
        Some(i) => i + 1,
        None => 0,
    }
}

/// A block's parts, borrowed: the open block's or a sealed one's, read the
/// same way.
#[derive(Clone, Copy)]
struct View<'a> {
    meta: &'a [u8],
    starts: &'a [u16],
    text: &'a [u8],
    checkpoints: &'a [u16],
    styles: &'a [u8],
    style_starts: &'a [u16],
    links: &'a [Arc<LinkValue>],
    ambiguous: u8,
}

/// Where a line's marks start in its meta: after its runs, clusters and
/// width exceptions, from `pos` (the runs).
fn marks_at(meta: &[u8], flags: u8, mut pos: usize) -> usize {
    if flags & flag::STYLED != 0 {
        for _ in 0..get_varint(meta, &mut pos) {
            if get_varint(meta, &mut pos) & 7 == 7 {
                get_varint(meta, &mut pos);
            }
        }
    }
    if flags & flag::CLUSTERS != 0 {
        for _ in 0..get_varint(meta, &mut pos) * 2 {
            get_varint(meta, &mut pos);
        }
    }
    if flags & flag::WIDTHS != 0 {
        for _ in 0..get_varint(meta, &mut pos) {
            get_varint(meta, &mut pos);
            pos += 1;
        }
    }
    pos
}

/// Where a line's placeholder list starts: after its marks, which start
/// at `pos`.
fn placeholders_at(meta: &[u8], flags: u8, mut pos: usize) -> usize {
    if flags & flag::MARKS != 0 {
        let count = meta.get(pos).copied().unwrap_or(0);
        pos += 1;
        for _ in 0..count {
            let k = meta.get(pos).copied().unwrap_or(0);
            pos += 1;
            get_varint(meta, &mut pos);
            if k & 0x80 != 0 {
                get_varint(meta, &mut pos);
            }
        }
    }
    pos
}

/// Whether a line's meta has a placeholder list.
#[inline]
fn has_placeholders(meta: &[u8]) -> bool {
    meta[0] & flag::EXTRA != 0 && meta.get(1).is_some_and(|x| x & extra::PLACEHOLDERS != 0)
}

/// Make the cells a line's placeholder list names placeholders again
/// (their text was 0x00, so they were decoded as blanks in their style).
fn read_placeholders(meta: &[u8], mut pos: usize, row: &mut Row) {
    let mut x = 0usize;
    for _ in 0..get_varint(meta, &mut pos) {
        x += get_varint(meta, &mut pos) as usize;
        let n = get_varint(meta, &mut pos) as usize;
        let r = get_varint(meta, &mut pos) as u16;
        let col = get_varint(meta, &mut pos) as u16;
        for k in 0..n {
            if let Some(c) = row.cells.get_mut(x + k) {
                *c = c.with_placeholder(r, col + k as u16);
            }
        }
        x += n;
    }
}

/// A line's marks, from where they start in its meta.
fn read_marks(meta: &[u8], mut pos: usize, mut f: impl FnMut(Mark)) {
    let count = meta.get(pos).copied().unwrap_or(0);
    pos += 1;
    for _ in 0..count {
        let k = meta.get(pos).copied().unwrap_or(0);
        pos += 1;
        let col = get_varint(meta, &mut pos);
        let status = (k & 0x80 != 0).then(|| get_varint(meta, &mut pos));
        let kind = match k & 0x7F {
            1 => MarkKind::Prompt,
            2 => MarkKind::Input,
            3 => MarkKind::Output,
            _ => MarkKind::End,
        };
        f(Mark {
            kind,
            col: col.min(u16::MAX as u32) as u16,
            status,
        });
    }
}

impl<'a> View<'a> {
    /// Line `n`'s [`Shape`], its text starting at `at`, and where the
    /// next line's text starts.
    fn shape(&self, n: usize, at: usize) -> (Shape, usize) {
        let meta = self.meta_of(n);
        let (flags, _, mut pos) = head(meta);
        let stored = self.stored_len(n, at);
        let wrapped = flags & flag::WRAPPED != 0;
        let tlen = stored - !wrapped as usize;
        let text = &self.text[at..at + tlen];
        let extra_ok = flags & flag::EXTRA == 0 || meta[1] & !extra::TLEN == 0;
        let narrow = flags & (flag::CLUSTERS | flag::WIDTHS | flag::EDGE | flag::FILL | flag::MARKS) == 0 && extra_ok && text.is_ascii();
        let mut shape = Shape {
            wrapped,
            cells: None,
            blank_tail: None,
        };
        if narrow {
            // Columns the style runs cover: past the text, they are blank
            // cells in their style.
            let mut runs = 0;
            if flags & flag::STYLED != 0 {
                for _ in 0..get_varint(meta, &mut pos) {
                    let v = get_varint(meta, &mut pos);
                    runs += (v >> 3) as usize;
                    if v & 7 == 7 {
                        get_varint(meta, &mut pos);
                    }
                }
            }
            shape.cells = Some(tlen.max(runs));
            // Spaces past the runs are blank cells in the default style;
            // a styled tail may or may not show.
            let spaces = text.iter().rev().take_while(|&&b| b == b' ').count();
            shape.blank_tail = (runs <= tlen - spaces).then_some(spaces);
        }
        (shape, at + stored)
    }

    /// Line `n`'s marks.
    fn marks(&self, n: usize, f: impl FnMut(Mark)) {
        let meta = self.meta_of(n);
        let flags = meta[0];
        if flags & flag::MARKS == 0 {
            return;
        }
        let (_, _, pos) = head(meta);
        read_marks(meta, marks_at(meta, flags, pos), f);
    }

    #[inline]
    fn meta_of(&self, n: usize) -> &'a [u8] {
        &self.meta[self.starts[n] as usize..]
    }

    #[inline]
    fn flags(&self, n: usize) -> u8 {
        self.meta_of(n)[0]
    }

    /// Where line `n`'s text starts in `text`, and its length (without the
    /// end byte).
    fn text_span(&self, n: usize) -> (usize, usize) {
        self.text_span_from(n, None)
    }

    /// [`View::text_span`], starting from `start` (line `n`'s text start,
    /// known from reading the line before) instead of a checkpoint.
    #[inline]
    fn text_span_from(&self, n: usize, start: Option<usize>) -> (usize, usize) {
        let at = start.unwrap_or_else(|| {
            let k = n / CHECKPOINT;
            let mut at = self.checkpoints[k] as usize;
            for j in k * CHECKPOINT..n {
                at += self.stored_len(j, at);
            }
            at
        });
        let len = self.stored_len(n, at);
        let ends = self.flags(n) & flag::WRAPPED == 0;
        (at, len - ends as usize)
    }

    /// Bytes of `text` line `n` takes, starting at `at`: `tlen` for a
    /// wrapped line, up to and including the end byte otherwise.
    #[inline]
    fn stored_len(&self, n: usize, at: usize) -> usize {
        match head(self.meta_of(n)) {
            (flags, Some(tlen), _) => tlen + (flags & flag::WRAPPED == 0) as usize,
            _ => find_end(&self.text[at..]).map_or(self.text.len() - at, |i| i + 1),
        }
    }

    /// Dictionary entry `slot` (1 onwards).
    fn style(&self, slot: u32) -> Style {
        let i = slot as usize - 1;
        let Some(&start) = self.style_starts.get(i) else {
            return Style::DEFAULT;
        };
        let end = self.style_starts.get(i + 1).map_or(self.styles.len(), |&e| e as usize);
        get_style(&self.styles[start as usize..end])
    }

    /// Expand line `n` into `row` (its cells' style ids indexing `styles`,
    /// the default first, and their links indexing `links`, `None` first).
    /// With FILL, the last run's style is carried on to `fill_to` cells.
    /// `start` is where the line's text starts, if the caller knows. Returns
    /// where the next line's text starts.
    fn decode(
        &self,
        n: usize,
        start: Option<usize>,
        fill_to: usize,
        row: &mut Row,
        styles: &mut Vec<Style>,
        links: &mut Vec<Option<Arc<LinkValue>>>,
    ) -> usize {
        let meta = self.meta_of(n);
        let (flags, _, mut pos) = head(meta);
        let (at, len) = self.text_span_from(n, start);
        let next = at + len + (flags & flag::WRAPPED == 0) as usize;
        // The text was written as UTF-8 by the encoder: it is read
        // without validating the whole line again (a cluster's text is
        // checked when it is stored in the row).
        let text = &self.text[at..at + len];
        row.reset(0, Cell::BLANK);
        // One allocation for a new row, whatever its runs and text add.
        row.cells.reserve(text.len().max(fill_to) + 1);
        row.wrapped = flags & flag::WRAPPED != 0;
        styles.clear();
        styles.push(Style::DEFAULT);
        links.clear();
        links.push(None);

        // The runs first: each lays down its cells as blanks in its style,
        // and the text then fills in content, so the runs are read once
        // and each cell is written once or twice.
        let runs_at = pos;
        if flags & flag::STYLED != 0 {
            // Slot → template in `styles`, direct-mapped: a style used by
            // several runs is decoded once (a miss just adds it again).
            let mut seen: [(u32, Cell); 32] = [(0, Cell::BLANK); 32];
            let runs = get_varint(meta, &mut pos);
            styles.reserve(runs as usize);
            for _ in 0..runs {
                let v = get_varint(meta, &mut pos);
                let mut slot = v & 7;
                if slot == 7 {
                    slot = get_varint(meta, &mut pos);
                }
                let e = &mut seen[(slot & 31) as usize];
                let t = if slot == 0 {
                    Cell::BLANK
                } else if e.0 == slot {
                    e.1
                } else {
                    let mut s = self.style(slot);
                    if s.link != 0 {
                        links.push(self.links.get(s.link as usize - 1).cloned());
                        s.link = links.len() as u32 - 1;
                    }
                    styles.push(s);
                    let t = Cell::template(styles.len() as u32 - 1, s.shows_empty());
                    *e = (slot, t);
                    t
                };
                let len = row.cells.len() + (v >> 3) as usize;
                row.cells.resize(len, t);
            }
        }

        if flags & (flag::CLUSTERS | flag::WIDTHS) == 0 && text.is_ascii() {
            let bytes = text;
            let styled = row.cells.len().min(bytes.len());
            let ascii = |b: u8| if b == b' ' { 0 } else { b as u64 };
            for (c, &b) in row.cells[..styled].iter_mut().zip(bytes) {
                *c = Cell::from_bits(c.bits() | ascii(b));
            }
            row.cells.extend(bytes[styled..].iter().map(|&b| Cell::from_bits(ascii(b))));
            if flags & flag::EDGE != 0 {
                place(row, bytes.len(), Cell::BLANK.with_width(Width::EdgeSpacer));
            }
        } else {
            let col = if flags & (flag::CLUSTERS | flag::WIDTHS) == 0 {
                self.decode_scalars(text, row)
            } else {
                self.decode_text(meta, &mut pos, flags, text, row)
            };
            if flags & flag::EDGE != 0 {
                place(row, col, Cell::BLANK.with_width(Width::EdgeSpacer));
            }
        }
        if flags & flag::FILL != 0
            && let Some(&last) = row.cells.last()
            && row.cells.len() < fill_to
        {
            row.cells.resize(fill_to, last.style_part());
        }
        if flags & flag::MARKS != 0 {
            read_marks(meta, marks_at(meta, flags, runs_at), |m| row.set_mark(m));
        }
        if has_placeholders(meta) {
            read_placeholders(meta, placeholders_at(meta, flags, marks_at(meta, flags, runs_at)), row);
        }
        next
    }

    /// The text of a line of single scalars with no width exceptions (CJK,
    /// accents, box drawing): each scalar is a cell at the table's width.
    /// Returns the column after it.
    #[inline(never)]
    fn decode_scalars(&self, text: &[u8], row: &mut Row) -> usize {
        let amb = self.ambiguous;
        let (mut i, mut col) = (0, 0);
        let mut last = ('\0', 1);
        while i < text.len() {
            let (c, n) = utf8_at(text, i);
            i += n;
            if c != last.0 {
                last = (c, single_width(c, amb));
            }
            let cell = if c == ' ' { Cell::BLANK } else { Cell::BLANK.with_char(c) };
            if last.1 == 2 {
                place(row, col, cell.with_width(Width::Wide));
                place(row, col + 1, Cell::BLANK.with_width(Width::Spacer));
                col += 2;
            } else {
                place(row, col, cell);
                col += 1;
            }
        }
        col
    }

    /// The text of a line with clusters or width exceptions, into cells (filling in those the runs laid down).
    /// Returns the column after it.
    #[inline(never)]
    fn decode_text(&self, meta: &[u8], pos: &mut usize, flags: u8, text: &[u8], row: &mut Row) -> usize {
        let mut clusters = 0;
        if flags & flag::CLUSTERS != 0 {
            clusters = get_varint(meta, pos);
        }
        let clusters_at = *pos;
        row.reserve_clusters(clusters as usize, text.len());
        if flags & flag::CLUSTERS != 0 {
            for _ in 0..clusters * 2 {
                get_varint(meta, pos);
            }
        }
        let mut widths = 0;
        if flags & flag::WIDTHS != 0 {
            widths = get_varint(meta, pos);
        }
        let mut wpos = *pos;
        let mut cpos = clusters_at;

        // Scalar index where the next cluster starts, its extra scalars
        // and whether it is wide.
        let mut next_cluster = u32::MAX;
        let mut extra = 0;
        if clusters > 0 {
            next_cluster = get_varint(meta, &mut cpos);
            extra = get_varint(meta, &mut cpos);
            clusters -= 1;
        }
        // Text entry of the next width exception, and its width.
        let mut next_width = u32::MAX;
        let mut width = 0;
        if widths > 0 {
            next_width = get_varint(meta, &mut wpos);
            width = meta.get(wpos).copied().unwrap_or(1);
            wpos += 1;
            widths -= 1;
        }

        let amb = self.ambiguous;
        let (mut scalar, mut entry, mut col) = (0u32, 0u32, 0usize);
        // The last scalar looked up and its width: runs of one character
        // (box drawing, a gradient's blocks) look it up once.
        let mut last = ('\0', 1);
        let mut i = 0;
        while i < text.len() {
            let (c, n) = utf8_at(text, i);
            let at = i;
            i += n;
            let mut end = i;
            let cluster = scalar == next_cluster;
            let mut cluster_width = 1;
            if cluster {
                cluster_width = 1 + (extra & 1) as u8;
                for _ in 0..extra >> 1 {
                    if i < text.len() {
                        i += utf8_at(text, i).1;
                        end = i;
                    }
                }
                scalar += 1 + (extra >> 1);
                if clusters > 0 {
                    next_cluster = scalar + get_varint(meta, &mut cpos);
                    extra = get_varint(meta, &mut cpos);
                    clusters -= 1;
                } else {
                    next_cluster = u32::MAX;
                }
            } else {
                scalar += 1;
            }
            let w = if entry == next_width {
                let w = width;
                if widths > 0 {
                    next_width = entry + 1 + get_varint(meta, &mut wpos);
                    width = meta.get(wpos).copied().unwrap_or(1);
                    wpos += 1;
                    widths -= 1;
                } else {
                    next_width = u32::MAX;
                }
                w
            } else if cluster {
                cluster_width
            } else if c == last.0 {
                last.1
            } else {
                last = (c, single_width(c, amb));
                last.1
            };
            entry += 1;
            let cell = if c == ' ' && !cluster {
                Cell::BLANK
            } else {
                Cell::BLANK.with_char(c)
            };
            if w == 2 {
                place(row, col, cell.with_width(Width::Wide));
                place(row, col + 1, Cell::BLANK.with_width(Width::Spacer));
            } else {
                place(row, col, cell);
            }
            if cluster {
                row.set_cluster(col, std::str::from_utf8(&text[at..end]).unwrap_or("\u{FFFD}\u{FFFD}"));
            }
            col += w as usize;
        }
        col
    }

    /// Check line `n`'s meta: every section reads to where the next line
    /// starts, slots name dictionary entries, and its text is UTF-8 with
    /// as many scalars as its clusters need.
    fn check_line(&self, n: usize, meta_end: usize) -> Result<(), String> {
        let meta = &self.meta[self.starts[n] as usize..meta_end];
        let flags = *meta.first().ok_or("empty meta")?;
        if flags & flag::EXTRA != 0 && !matches!(meta.get(1), Some(&x) if x != 0 && x & !(extra::TLEN | extra::PLACEHOLDERS) == 0) {
            return Err(format!("extra byte {:?}: only TLEN and PLACEHOLDERS are written yet", meta.get(1)));
        }
        let (at, len) = self.text_span(n);
        let text = std::str::from_utf8(self.text.get(at..at + len).ok_or("text past the end")?).map_err(|e| e.to_string())?;
        if text.bytes().any(|b| b < 0x20 && b != 0) {
            return Err(format!("text {text:?} holds a control"));
        }
        let (_, tlen, mut pos) = head(meta);
        if flags & flag::WRAPPED == 0 && self.text.get(at + len) != Some(&END) {
            return Err("an unwrapped line without its end byte".into());
        }
        if tlen.is_some_and(|t| t != len) || (flags & flag::WRAPPED == 0 && (tlen.is_some() != (len >= LONG_TEXT))) {
            return Err(format!("text length {tlen:?} recorded for {len} bytes"));
        }
        if flags & flag::STYLED != 0 {
            let runs = get_varint(meta, &mut pos);
            if runs == 0 {
                return Err("STYLED with no runs".into());
            }
            for _ in 0..runs {
                let v = get_varint(meta, &mut pos);
                let slot = if v & 7 == 7 { get_varint(meta, &mut pos) } else { v & 7 };
                if slot as usize > self.style_starts.len() {
                    return Err(format!("run names slot {slot}, the dictionary has {}", self.style_starts.len()));
                }
                if slot > 0 && self.style(slot).link as usize > self.links.len() {
                    return Err(format!(
                        "slot {slot} names link {}, the block has {}",
                        self.style(slot).link,
                        self.links.len()
                    ));
                }
            }
        }
        if flags & flag::CLUSTERS != 0 {
            let (mut need, count) = (0u64, get_varint(meta, &mut pos));
            for _ in 0..count {
                let gap = get_varint(meta, &mut pos);
                let extra = get_varint(meta, &mut pos) >> 1;
                if extra == 0 {
                    return Err("a cluster of one scalar".into());
                }
                need += gap as u64 + 1 + extra as u64;
            }
            if need > text.chars().count() as u64 {
                return Err(format!("clusters need {need} scalars, the text has {}", text.chars().count()));
            }
        }
        if flags & flag::WIDTHS != 0 {
            for _ in 0..get_varint(meta, &mut pos) {
                get_varint(meta, &mut pos);
                let w = meta.get(pos).copied();
                pos += 1;
                if !matches!(w, Some(1 | 2)) {
                    return Err(format!("width exception {w:?}"));
                }
            }
        }
        if flags & flag::MARKS != 0 {
            let count = meta.get(pos).copied().unwrap_or(0);
            if !(1..=4).contains(&count) {
                return Err(format!("{count} marks"));
            }
            pos += 1;
            for _ in 0..count {
                let k = meta.get(pos).copied().unwrap_or(0);
                pos += 1;
                if !(1..=4).contains(&(k & 0x7F)) || (k & 0x80 != 0 && k & 0x7F != 4) {
                    return Err(format!("mark kind {k:#x}"));
                }
                get_varint(meta, &mut pos);
                if k & 0x80 != 0 {
                    get_varint(meta, &mut pos);
                }
            }
        }
        if has_placeholders(meta) {
            let count = get_varint(meta, &mut pos);
            if count == 0 {
                return Err("PLACEHOLDERS with no stretches".into());
            }
            let zeros = text.bytes().filter(|&b| b == 0).count() as u64;
            let mut cells = 0u64;
            for _ in 0..count {
                let gap = get_varint(meta, &mut pos);
                let n = get_varint(meta, &mut pos);
                let r = get_varint(meta, &mut pos);
                let col = get_varint(meta, &mut pos);
                if n == 0 || r >= crate::image::placeholder::VALUES as u32 || col + n > 512 {
                    return Err(format!("placeholder stretch gap {gap} n {n} row {r} col {col}"));
                }
                cells += n as u64;
            }
            if cells != zeros {
                return Err(format!("{cells} placeholders for {zeros} 0x00 bytes of text"));
            }
        }
        if pos != meta.len() {
            return Err(format!("meta reads to {pos} of {}", meta.len()));
        }
        Ok(())
    }

    /// Check every line of the first `len` and the checkpoints.
    fn check(&self, len: usize) -> Result<(), String> {
        if self.checkpoints.len() < len.div_ceil(CHECKPOINT) {
            return Err(format!("{} checkpoints for {len} lines", self.checkpoints.len()));
        }
        let mut at = 0;
        for n in 0..len {
            if n % CHECKPOINT == 0 && self.checkpoints[n / CHECKPOINT] as usize != at {
                return Err(format!(
                    "checkpoint {} is {}, line {n} starts at {at}",
                    n / CHECKPOINT,
                    self.checkpoints[n / CHECKPOINT]
                ));
            }
            let end = self.starts.get(n + 1).map_or(self.meta.len(), |&s| s as usize);
            if (self.starts[n] as usize) >= end {
                return Err(format!("line {n}'s meta is empty"));
            }
            self.check_line(n, end).map_err(|e| format!("line {n}: {e}"))?;
            at += self.stored_len(n, at);
        }
        Ok(())
    }
}

/// The scalar starting at byte `i` of `text` (UTF-8 the encoder wrote)
/// and its length in bytes. Bytes that aren't UTF-8 read as U+FFFD.
#[inline(always)]
fn utf8_at(text: &[u8], i: usize) -> (char, usize) {
    let b = |k: usize| text.get(i + k).copied().unwrap_or(0) as u32 & 0x3F;
    let x = text[i] as u32;
    let (cp, n) = if x < 0x80 {
        (x, 1)
    } else if x < 0xE0 {
        ((x & 0x1F) << 6 | b(1), 2)
    } else if x < 0xF0 {
        ((x & 0x0F) << 12 | b(1) << 6 | b(2), 3)
    } else {
        ((x & 0x07) << 18 | b(1) << 12 | b(2) << 6 | b(3), 4)
    };
    (char::from_u32(cp).unwrap_or('\u{FFFD}'), n.min(text.len() - i))
}

/// Put content `cell` (in the default style) at `col`: into the cell a run
/// laid down there, keeping its style, or as a new cell at the end.
#[inline(always)]
fn place(row: &mut Row, col: usize, cell: Cell) {
    match row.cells.get_mut(col) {
        Some(c) => *c = Cell::from_bits(c.bits() | cell.bits()),
        None => row.cells.push(cell),
    }
}

/// The first 0x0A in `text`. Walking from a checkpoint skips up to 15
/// lines this way, so it tests 32 bytes at a time (four words, each with
/// the zero-byte trick on `word ^ 0x0A0A…`), then finds the byte.
#[inline]
fn find_end(text: &[u8]) -> Option<usize> {
    const LO: u64 = 0x0101_0101_0101_0101;
    const HI: u64 = 0x8080_8080_8080_8080;
    const PAT: u64 = LO * END as u64;
    #[inline(always)]
    fn zero(w: u64) -> u64 {
        let w = w ^ PAT;
        w.wrapping_sub(LO) & !w & HI
    }
    let word = |i: usize| u64::from_le_bytes(text[i..i + 8].try_into().expect("8 bytes"));
    let mut i = 0;
    while i + 32 <= text.len() && zero(word(i)) | zero(word(i + 8)) | zero(word(i + 16)) | zero(word(i + 24)) == 0 {
        i += 32;
    }
    while i + 8 <= text.len() {
        let z = zero(word(i));
        if z != 0 {
            // The lowest flagged byte is the first 0x0A (any false flags
            // come after a real one).
            return Some(i + (z.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    text[i..].iter().position(|&b| b == END).map(|p| i + p)
}

/// Where the next line's text starts, for reading lines one after another
/// ([`Scrollback::read`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct HistoryReader {
    /// The block (its first line) and the line it is for.
    next: Option<(u64, u64)>,
    at: usize,
}

/// What history holds, for measuring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryStats {
    /// Every byte held, the open block's spare capacity included.
    pub bytes: usize,
    /// Sealed blocks: how many, their bytes and their lines in use.
    pub sealed_blocks: usize,
    pub sealed_bytes: usize,
    pub sealed_lines: usize,
    /// Lines in the open block.
    pub open_lines: usize,
}

pub struct Scrollback {
    /// Sealed blocks, oldest first; the open block follows them.
    blocks: VecDeque<BlockEntry>,
    open: OpenBlock,
    limit: usize,
    byte_limit: usize,
    /// Bytes the sealed blocks hold.
    sealed_bytes: usize,
    /// Oldest line still visible (older lines may sit in the first block
    /// until the whole block can be freed).
    start: u64,
    /// One past the newest line: lines that left the screen, stored or
    /// not, so it equals the primary screen's first line number.
    end: u64,
    /// The newest line stored soft-wraps onto the next.
    last_wrapped: bool,
    /// The lowest line number popped since [`Scrollback::take_popped_from`].
    popped_from: Option<u64>,
    /// The same for search (§13.4), since [`Scrollback::take_search_popped`].
    search_popped: Option<u64>,
    /// The style table the slot cache was filled from: its epoch and sweep
    /// count. `[0, 0]` = unknown, so the cache is dropped each line.
    cache_key: [u64; 2],
    /// Image lists of blocks freed since [`Scrollback::take_freed_images`].
    freed_images: Vec<ImageKey>,
    scratch: Scratch,
}

impl Scrollback {
    pub fn new(limit: usize) -> Scrollback {
        Scrollback {
            blocks: VecDeque::new(),
            open: OpenBlock::new(0, 1),
            limit,
            byte_limit: BYTE_LIMIT,
            sealed_bytes: 0,
            start: 0,
            end: 0,
            last_wrapped: false,
            popped_from: None,
            search_popped: None,
            cache_key: [0, 0],
            freed_images: Vec::new(),
            scratch: Scratch::default(),
        }
    }

    /// [`Scrollback::new`] for a terminal's cells, written at the table's
    /// widths ([`Scrollback::set_widths_as_written`]).
    pub fn with_widths_as_written(limit: usize) -> Scrollback {
        let mut sb = Scrollback::new(limit);
        sb.set_widths_as_written(true);
        sb
    }

    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }

    /// Lines that ever left the screen (the line number of the next).
    pub fn total(&self) -> u64 {
        self.end
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Keep at most `limit` lines from now on. Lines over a smaller limit
    /// go at once (whole blocks freed, as when output pushes them out);
    /// with a larger one, lines still stored but hidden come back.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        if limit == 0 {
            self.clear();
            return;
        }
        self.start = self.oldest().max(self.end.saturating_sub(limit as u64));
        self.trim();
    }

    /// Test aid: the byte backstop (normally [`BYTE_LIMIT`]).
    #[doc(hidden)]
    pub fn set_byte_limit(&mut self, bytes: usize) {
        self.byte_limit = bytes;
        self.backstop();
    }

    /// Bytes held, for measuring.
    pub fn memory_bytes(&self) -> usize {
        self.sealed_bytes + self.open.bytes() + self.blocks.capacity() * std::mem::size_of::<BlockEntry>()
    }

    pub fn stats(&self) -> HistoryStats {
        HistoryStats {
            bytes: self.memory_bytes(),
            sealed_blocks: self.blocks.len(),
            sealed_bytes: self.sealed_bytes,
            sealed_lines: self.blocks.iter().map(|b| b.len as usize).sum(),
            open_lines: self.open.lines(),
        }
    }

    /// The sealed blocks and how many of each one's lines are in use, as a
    /// search captures them.
    pub fn sealed(&self) -> impl Iterator<Item = (&Arc<BlockData>, usize)> {
        self.blocks.iter().map(|b| (&b.data, b.len as usize))
    }

    pub fn clear(&mut self) {
        for b in &self.blocks {
            self.freed_images.extend_from_slice(&b.data.images);
        }
        self.freed_images.append(&mut self.open.images);
        self.blocks = VecDeque::new();
        self.open = OpenBlock::new(self.end, self.open.ambiguous);
        self.sealed_bytes = 0;
        self.start = self.end;
        self.last_wrapped = false;
    }

    /// Lines from now on are written with this ambiguous-width setting
    /// (§6.3): the open block is sealed first if it holds any lines.
    pub fn set_ambiguous(&mut self, ambiguous: u8) {
        if ambiguous != self.open.ambiguous {
            self.seal();
            self.open.ambiguous = ambiguous;
        }
    }

    /// Whether every single-scalar cell pushed from now on has the width
    /// the Unicode table gives at the current ambiguous-width setting. The
    /// terminal says so while the setting hasn't changed since its cells
    /// were written (it writes them at the table's width): storing then
    /// needs no table lookup to find width exceptions. Off by default,
    /// so any row stores exactly.
    pub fn set_widths_as_written(&mut self, on: bool) {
        self.scratch.widths_as_written = on;
    }

    /// While widths are trusted ([`Scrollback::set_widths_as_written`]),
    /// `row`'s single-scalar cells must have the table's width at the
    /// open block's setting: checked on the screen after every `advance`
    /// in debug builds, so a write path that breaks it is caught.
    pub fn check_widths(&self, row: &Row) -> Result<(), String> {
        if !self.scratch.widths_as_written {
            return Ok(());
        }
        for (col, &c) in row.cells.iter().enumerate() {
            if c.bits() & Cell::KIND != 0 || c.is_spacer() || c.codepoint() == 0 {
                continue;
            }
            let ch = char::from_u32(c.codepoint()).unwrap_or(' ');
            if ch != ' ' && single_width(ch, self.open.ambiguous) != actual(c) {
                return Err(format!("col {col}: {ch:?} is {} cells, the table says otherwise", actual(c)));
            }
        }
        Ok(())
    }

    /// Store a row (trailing blank cells dropped); its cells' style ids
    /// index `styles`.
    pub fn push(&mut self, row: &Row, styles: &[Style]) {
        self.push_keyed(row, styles, &[], [0, 0]);
    }

    /// [`Scrollback::push`] for rows of one screen's style table, whose
    /// styles' `link`s index `links` (the screen's link table), named by
    /// `key` (the table's epoch and sweep count): while it stays the same,
    /// the slot cache is kept from line to line. Returns the column from
    /// which the row's cells are all [`Cell::BLANK`] (as far as storing it
    /// found out), so the caller can clear only the cells before it.
    pub fn push_keyed(&mut self, row: &Row, styles: &[Style], links: &[Option<Arc<LinkValue>>], key: [u64; 2]) -> usize {
        self.end += 1;
        if self.limit == 0 {
            // Nothing is kept, but the count goes on: line numbers are the
            // screen's.
            self.start = self.end;
            self.open.first = self.end;
            return row.cells.len();
        }
        if key != self.cache_key || key == [0, 0] {
            self.open.reset_cache();
            self.cache_key = key;
        }
        if self.open.lines() == 0 {
            self.open.head_continues = self.last_wrapped;
        }
        // An empty row always fits.
        if !self.open.push(row, styles, links, &mut self.scratch) {
            if self.open.lines() > 0 {
                self.seal();
                self.open.head_continues = self.last_wrapped;
            }
            if !self.open.push(row, styles, links, &mut self.scratch) {
                // Too wide for a block (only past 4096 columns): cut it.
                let mut cut = row.clone();
                cut.resize_cells(MAX_CELLS);
                if cut.cells.last().is_some_and(|c| c.is_wide()) {
                    cut.free_clusters(MAX_CELLS - 1..MAX_CELLS);
                    cut.cells[MAX_CELLS - 1] = Cell::BLANK;
                }
                if !self.open.push(&cut, styles, links, &mut self.scratch) {
                    debug_assert!(false, "a 4096-cell row fits in an empty block");
                    self.open.push(&Row::default(), styles, links, &mut self.scratch);
                }
                // The cells past the cut weren't looked at.
                self.scratch.zero_from = row.cells.len();
            }
        }
        self.last_wrapped = row.wrapped;
        self.trim();
        self.scratch.zero_from.min(row.cells.len())
    }

    /// Seal the open block, if it holds any lines.
    fn seal(&mut self) {
        if self.open.lines() == 0 {
            return;
        }
        let len = self.open.lines() as u32;
        let data = self.open.seal();
        self.sealed_bytes += data.bytes();
        self.blocks.push_back(BlockEntry {
            data: Arc::new(data),
            len,
            filter: None,
        });
        self.backstop();
    }

    /// Free blocks from the oldest end while history holds more than the
    /// byte backstop. Checked when a block is sealed: the open block
    /// grows by at most 64 KB a part in between.
    fn backstop(&mut self) {
        while self.sealed_bytes + self.open.bytes() > self.byte_limit && !self.blocks.is_empty() {
            self.free_oldest();
        }
    }

    /// The first line still stored (hidden or not).
    fn oldest(&self) -> u64 {
        self.blocks.front().map_or(self.open.first, |b| b.data.first)
    }

    /// Apply the limits: hide lines past the line limit, free blocks with
    /// only hidden lines, and free blocks from the oldest end while past
    /// the byte backstop.
    fn trim(&mut self) {
        self.start = self.start.max(self.end.saturating_sub(self.limit as u64));
        while let Some(b) = self.blocks.front()
            && b.data.first + b.len as u64 <= self.start
        {
            self.free_oldest();
        }
    }

    /// Move the image lists of blocks freed since the last call to `out`:
    /// each names an image once per block (images.md §4.3).
    pub fn take_freed_images(&mut self, out: &mut Vec<ImageKey>) {
        out.append(&mut self.freed_images);
    }

    /// Add the own images every block still listed names to `out`.
    pub fn listed_images(&self, out: &mut std::collections::HashSet<ImageKey>) {
        for b in &self.blocks {
            out.extend(b.data.images.iter().copied());
        }
        out.extend(self.open.images.iter().copied());
    }

    fn free_oldest(&mut self) {
        if let Some(b) = self.blocks.pop_front() {
            self.freed_images.extend_from_slice(&b.data.images);
            self.sealed_bytes -= b.bytes();
            self.start = self.start.max(b.data.first + b.len as u64);
        }
    }

    /// Remove the newest line (pulling lines back onto the screen, §12.2)
    /// and expand it into `row`, as [`Scrollback::get_at_width`] does. In a
    /// sealed block only the entry's count goes down; the shared data
    /// stays as it is until the block is freed. Records the lowest line
    /// popped. False, leaving the buffers alone, if history is empty.
    pub fn pop_into(&mut self, fill_to: usize, row: &mut Row, styles: &mut Vec<Style>, links: &mut Vec<Option<Arc<LinkValue>>>) -> bool {
        if self.is_empty() {
            return false;
        }
        let line = self.end - 1;
        self.get_at_width(self.len() - 1, fill_to, row, styles, links);
        if self.open.lines() > 0 {
            self.open.pop();
        } else {
            let b = self.blocks.back_mut().expect("a stored line");
            b.len -= 1;
            if b.len == 0 {
                let b = self.blocks.pop_back().expect("a block");
                self.freed_images.extend_from_slice(&b.data.images);
                self.sealed_bytes -= b.bytes();
            }
            // The next push starts a new open block where this one ends.
            self.open = OpenBlock::new(line, self.open.ambiguous);
        }
        self.end = line;
        self.popped_from = Some(self.popped_from.map_or(line, |p| p.min(line)));
        self.search_popped = Some(self.search_popped.map_or(line, |p| p.min(line)));
        // Lines hidden by the limit may fit again.
        self.start = self.oldest().max(self.end.saturating_sub(self.limit as u64));
        self.last_wrapped = !self.is_empty() && self.wrapped(self.len() - 1);
        true
    }

    /// [`Scrollback::pop_into`] into new buffers.
    pub fn pop(&mut self) -> Option<(Row, Vec<Style>)> {
        let mut row = Row::default();
        let mut styles = Vec::new();
        self.pop_into(0, &mut row, &mut styles, &mut Vec::new()).then_some((row, styles))
    }

    /// The lowest line number popped since the last call, if any (§9.1):
    /// only lines at or after it can have changed.
    pub fn take_popped_from(&mut self) -> Option<u64> {
        self.popped_from.take()
    }

    /// Line `i` counting from the oldest visible line (0..len), and the
    /// styles its cells' ids index.
    pub fn get(&self, i: usize) -> Option<(Row, Vec<Style>)> {
        let mut row = Row::default();
        let mut styles = Vec::new();
        self.get_into(i, &mut row, &mut styles).then_some((row, styles))
    }

    /// [`Scrollback::get`] into buffers the caller reuses (the snapshot,
    /// §9.2). Returns false, leaving them alone, if there is no line `i`.
    pub fn get_into(&self, i: usize, row: &mut Row, styles: &mut Vec<Style>) -> bool {
        self.get_at_width(i, 0, row, styles, &mut Vec::new())
    }

    /// [`Scrollback::get_into`] for a row shown `cols` wide (a FILL line's
    /// last style carries on to the end of the row), with the links its
    /// styles' `link`s index (`None` first).
    pub fn get_at_width(
        &self,
        i: usize,
        cols: usize,
        row: &mut Row,
        styles: &mut Vec<Style>,
        links: &mut Vec<Option<Arc<LinkValue>>>,
    ) -> bool {
        self.read(i, cols, row, styles, links, &mut HistoryReader::default())
    }

    /// [`Scrollback::get_at_width`] for lines read one after another (the
    /// rows of a view): `reader` remembers where the next line's text
    /// starts, so reading line `i + 1` after line `i` needs no walk from a
    /// checkpoint. A reader is only good while history doesn't change:
    /// use a new one each time it may have.
    pub fn read(
        &self,
        i: usize,
        cols: usize,
        row: &mut Row,
        styles: &mut Vec<Style>,
        links: &mut Vec<Option<Arc<LinkValue>>>,
        reader: &mut HistoryReader,
    ) -> bool {
        let Some((view, n)) = self.locate(i) else { return false };
        let line = self.start + i as u64;
        let first = line - n as u64;
        let start = (reader.next == Some((first, line))).then_some(reader.at);
        let at = view.decode(n, start, cols, row, styles, links);
        reader.next = (n + 1 < view.starts.len()).then_some((first, line + 1));
        reader.at = at;
        true
    }

    /// Line `i`'s OSC 133 marks, without decoding its cells.
    pub fn marks(&self, i: usize) -> Vec<Mark> {
        let mut out = Vec::new();
        if let Some((v, n)) = self.locate(i) {
            v.marks(n, |m| out.push(m));
        }
        out
    }

    /// Whether line `i` has marks, from its flags.
    /// Whether stored line `i` has kitty placeholder cells.
    pub fn has_placeholders(&self, i: usize) -> bool {
        self.locate(i).is_some_and(|(v, n)| has_placeholders(v.meta_of(n)))
    }

    pub fn has_marks(&self, i: usize) -> bool {
        self.locate(i).is_some_and(|(v, n)| v.flags(n) & flag::MARKS != 0)
    }

    /// The nearest line with marks before line `i` (`up`) or at or after
    /// it, skipping blocks with none (§11.6): only flags bytes are read.
    pub fn marked(&self, i: usize, up: bool) -> Option<usize> {
        let len = self.len();
        let mut j = if up { i.min(len).checked_sub(1)? } else { i };
        while j < len {
            let line = self.start + j as u64;
            // The block holding `line`: its lines (visible ones) and marks.
            let (lo, hi, any) = if line >= self.open.first {
                (self.open.first, self.end, self.open.has_marks)
            } else {
                let b = &self.blocks[self.blocks.partition_point(|b| b.data.first <= line) - 1];
                (b.data.first, b.data.first + b.len as u64, b.data.has_marks)
            };
            let lo = (lo.max(self.start) - self.start) as usize;
            let hi = (hi - self.start) as usize;
            if any {
                if up {
                    if let Some(k) = (lo..=j).rev().find(|&k| self.has_marks(k)) {
                        return Some(k);
                    }
                } else if let Some(k) = (j..hi).find(|&k| self.has_marks(k)) {
                    return Some(k);
                }
            }
            if up {
                j = lo.checked_sub(1)?;
            } else {
                j = hi;
            }
        }
        None
    }

    /// Whether line `i` soft-wraps onto the next, without decoding it.
    pub fn wrapped(&self, i: usize) -> bool {
        self.locate(i).is_some_and(|(v, n)| v.flags(n) & flag::WRAPPED != 0)
    }

    /// Whether line `i`'s last style fills to the end of the row (FILL),
    /// without decoding it.
    pub fn fill(&self, i: usize) -> bool {
        self.locate(i).is_some_and(|(v, n)| v.flags(n) & flag::FILL != 0)
    }

    /// The text of line `i` as stored (no end byte), without decoding it.
    pub fn text(&self, i: usize) -> Option<&[u8]> {
        let (v, n) = self.locate(i)?;
        let (at, len) = v.text_span(n);
        Some(&v.text[at..at + len])
    }

    /// The block holding line `i` (0..len), and the line's index in it.
    #[inline]
    fn locate(&self, i: usize) -> Option<(View<'_>, usize)> {
        if i >= self.len() {
            return None;
        }
        let line = self.start + i as u64;
        if line >= self.open.first {
            return Some((self.open.view(), (line - self.open.first) as usize));
        }
        // Few blocks (about one per thousand lines): binary search.
        let idx = self.blocks.partition_point(|b| b.data.first <= line) - 1;
        let b = &self.blocks[idx];
        Some((b.data.view(b.len as usize), (line - b.data.first) as usize))
    }

    /// The structure, cheaply (after every `advance` in debug builds):
    /// blocks follow one another, the counts and byte totals add up, the
    /// limits hold, and no part of the open block holds more than 64 KB.
    pub fn check(&self) -> Result<(), String> {
        if self.start > self.end || self.len() > self.limit {
            return Err(format!("lines {}..{} with a limit of {}", self.start, self.end, self.limit));
        }
        let mut next = self.oldest();
        if self.start < next {
            return Err(format!("visible from {}, stored from {next}", self.start));
        }
        let mut bytes = 0;
        for b in &self.blocks {
            if b.data.first != next || b.len == 0 || b.len as usize > b.data.lines() {
                return Err(format!(
                    "block from {} with {} of {} lines, expected from {next}",
                    b.data.first,
                    b.len,
                    b.data.lines()
                ));
            }
            next += b.len as u64;
            bytes += b.bytes();
        }
        if self.open.first != next || next + self.open.lines() as u64 != self.end {
            return Err(format!(
                "open block from {} with {} lines, expected from {next} to {}",
                self.open.first,
                self.open.lines(),
                self.end
            ));
        }
        if bytes != self.sealed_bytes {
            return Err(format!("sealed blocks hold {bytes} bytes, counted {}", self.sealed_bytes));
        }
        if self.sealed_bytes > self.byte_limit {
            return Err(format!("sealed blocks hold {} bytes, over the backstop", self.sealed_bytes));
        }
        let o = &self.open;
        for (name, cap) in [
            ("meta", o.meta.capacity()),
            ("text", o.text.capacity()),
            ("styles", o.styles.capacity()),
        ] {
            if cap > PART + 1 {
                return Err(format!("the open block's {name} holds {cap} bytes"));
            }
        }
        if o.checkpoints.len() != o.lines().div_ceil(CHECKPOINT) || o.style_starts.len() != o.dict.len() {
            return Err(format!(
                "open block: {} checkpoints for {} lines, {} dictionary entries for {} styles",
                o.checkpoints.len(),
                o.lines(),
                o.style_starts.len(),
                o.dict.len()
            ));
        }
        if self.limit == 0 && (self.open.lines() > 0 || !self.blocks.is_empty()) {
            return Err("lines stored with a limit of 0".into());
        }
        Ok(())
    }

    /// [`Scrollback::check`], and every stored line read through: each
    /// line's sections, its text and the checkpoints. Costs a pass over
    /// all of history.
    #[doc(hidden)]
    pub fn check_all(&self) -> Result<(), String> {
        self.check()?;
        for (i, b) in self.blocks.iter().enumerate() {
            let d = &b.data;
            if d.starts.len() != d.lines() || d.checkpoints.len() != d.lines().div_ceil(CHECKPOINT) {
                return Err(format!("block {i}: {} checkpoints for {} lines", d.checkpoints.len(), d.lines()));
            }
            for (part, len) in [("meta", d.meta.len()), ("text", d.text.len()), ("styles", d.styles.len())] {
                if len > PART {
                    return Err(format!("block {i}: {part} is {len} bytes"));
                }
            }
            d.view(d.lines()).check(d.lines()).map_err(|e| format!("block {i}: {e}"))?;
        }
        self.open.view().check(self.open.lines()).map_err(|e| format!("open block: {e}"))
    }
}

fn put_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(data: &[u8], pos: &mut usize) -> u32 {
    let mut v = 0u32;
    let mut shift = 0;
    while let Some(&b) = data.get(*pos) {
        *pos += 1;
        if shift < 32 {
            v |= ((b & 0x7F) as u32) << shift;
        }
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    v
}

/// A colour's kind (0 default, 1 palette, 2 RGB), its bytes appended.
fn put_color(out: &mut Vec<u8>, c: Color) -> u8 {
    match c.kind() {
        ColorKind::Default => 0,
        ColorKind::Indexed(i) => {
            out.push(i);
            1
        }
        ColorKind::Rgb(r, g, b) => {
            out.extend_from_slice(&[r, g, b]);
            2
        }
    }
}

fn get_color(data: &[u8], pos: &mut usize, kind: u8) -> Color {
    let b = |i: usize| data.get(*pos + i).copied().unwrap_or(0);
    match kind {
        1 => {
            let c = Color::indexed(b(0));
            *pos += 1;
            c
        }
        2 => {
            let c = Color::rgb(b(0), b(1), b(2));
            *pos += 3;
            c
        }
        _ => Color::DEFAULT,
    }
}

mod head {
    pub const ATTRS: u8 = 1 << 6;
    pub const MORE: u8 = 1 << 7;
}
mod more {
    pub const UNDERLINE: u8 = 1 << 0;
    /// The link: an index in the block's link table.
    pub const LINK: u8 = 1 << 1;
    pub const IMAGE_HIGH: u8 = 1 << 2;
}

/// A dictionary entry (§11.5): `head [fg] [bg] [ul] [attrs] [more
/// [underline] [image_high]]`, the head's bits 0–1, 2–3 and 4–5 giving the
/// kind of each colour.
fn put_style(out: &mut Vec<u8>, s: &Style) {
    let at = out.len();
    out.push(0);
    let mut h = put_color(out, s.fg);
    h |= put_color(out, s.bg) << 2;
    h |= put_color(out, s.underline_color) << 4;
    if !s.attrs.is_empty() {
        h |= head::ATTRS;
        put_varint(out, s.attrs.bits() as u32);
    }
    let m = if s.underline != Underline::None { more::UNDERLINE } else { 0 }
        | if s.link != 0 { more::LINK } else { 0 }
        | if s.image_id_high != 0 { more::IMAGE_HIGH } else { 0 };
    if m != 0 {
        h |= head::MORE;
        out.push(m);
        if m & more::UNDERLINE != 0 {
            out.push(s.underline as u8);
        }
        if m & more::LINK != 0 {
            put_varint(out, s.link);
        }
        if m & more::IMAGE_HIGH != 0 {
            out.push(s.image_id_high);
        }
    }
    out[at] = h;
}

fn get_style(data: &[u8]) -> Style {
    let h = data.first().copied().unwrap_or(0);
    let mut pos = 1;
    let fg = get_color(data, &mut pos, h & 3);
    let bg = get_color(data, &mut pos, h >> 2 & 3);
    let underline_color = get_color(data, &mut pos, h >> 4 & 3);
    let mut s = Style {
        fg,
        bg,
        underline_color,
        ..Style::DEFAULT
    };
    if h & head::ATTRS != 0 {
        s.attrs = Attrs::from_bits(get_varint(data, &mut pos) as u16);
    }
    if h & head::MORE != 0 {
        let m = data.get(pos).copied().unwrap_or(0);
        pos += 1;
        if m & more::UNDERLINE != 0 {
            s.underline = match data.get(pos).copied().unwrap_or(0) {
                1 => Underline::Single,
                2 => Underline::Double,
                3 => Underline::Curly,
                4 => Underline::Dotted,
                5 => Underline::Dashed,
                _ => Underline::None,
            };
            pos += 1;
        }
        if m & more::LINK != 0 {
            s.link = get_varint(data, &mut pos);
        }
        if m & more::IMAGE_HIGH != 0 {
            s.image_id_high = data.get(pos).copied().unwrap_or(0);
        }
    }
    s
}

/// A block as a search takes it: its data, its lines in use and its
/// filter.
pub(crate) type SearchBlock = (Arc<BlockData>, usize, Option<Arc<Filter>>);

/// What search reads (§13.4, §13.5).
impl BlockData {
    /// Where each of the first `len` lines' text starts, and (last) where
    /// the text of those lines ends: `len + 1` offsets.
    pub(crate) fn line_starts(&self, len: usize) -> Vec<u32> {
        let view = self.view(len.min(self.starts.len()));
        let mut out = Vec::with_capacity(len + 1);
        let mut at = 0;
        for n in 0..view.starts.len() {
            out.push(at as u32);
            at += view.stored_len(n, at);
        }
        out.push(at as u32);
        out
    }

    /// Line `n`'s flags byte.
    pub(crate) fn line_flags(&self, n: usize) -> u8 {
        self.view(self.starts.len()).flags(n)
    }

    /// Expand line `n`, whose text starts at `start`, into `row`.
    pub(crate) fn decode_line(&self, n: usize, start: usize, row: &mut Row) {
        let (mut styles, mut links) = (Vec::new(), Vec::new());
        self.view(self.starts.len()).decode(n, Some(start), 0, row, &mut styles, &mut links);
    }
}

impl Scrollback {
    /// The blocks holding lines at or after `from` (a stable line
    /// number), oldest first, with how many lines of each are in use: the
    /// sealed ones shared, the open one copied (§13.4). Also the first
    /// visible line, so a search skips hidden ones.
    pub(crate) fn search_blocks(&self, from: u64) -> (Vec<SearchBlock>, u64) {
        let mut out: Vec<SearchBlock> = self
            .blocks
            .iter()
            .filter(|b| b.data.first + b.len as u64 > from)
            .map(|b| (b.data.clone(), b.len as usize, b.filter.clone()))
            .collect();
        if self.open.lines() > 0 {
            let o = &self.open;
            let data = BlockData {
                meta: o.meta.clone().into_boxed_slice(),
                starts: o.starts.clone().into_boxed_slice(),
                text: o.text.clone().into_boxed_slice(),
                checkpoints: o.checkpoints.clone().into_boxed_slice(),
                styles: o.styles.clone().into_boxed_slice(),
                style_starts: o.style_starts.clone().into_boxed_slice(),
                links: o.links.clone().into_boxed_slice(),
                first: o.first,
                width: if o.width == u16::MAX { 0 } else { o.width },
                ambiguous: o.ambiguous,
                head_continues: o.head_continues,
                has_marks: o.has_marks,
                images: o.images.clone().into_boxed_slice(),
            };
            out.push((Arc::new(data), o.lines(), None));
        }
        (out, self.start)
    }

    /// Up to `max` sealed blocks without a filter, newest first (§13.6).
    pub(crate) fn unfiltered(&self, max: usize) -> Vec<Arc<BlockData>> {
        self.blocks
            .iter()
            .rev()
            .filter(|b| b.filter.is_none())
            .take(max)
            .map(|b| b.data.clone())
            .collect()
    }

    /// Store `filter` for `data`'s block, if it is still stored. Filters
    /// count against the backstop, so this may free the oldest blocks.
    pub(crate) fn set_filter(&mut self, data: &Arc<BlockData>, filter: Arc<Filter>) {
        let i = self.blocks.partition_point(|b| b.data.first < data.first);
        if let Some(b) = self.blocks.get_mut(i)
            && Arc::ptr_eq(&b.data, data)
            && b.filter.is_none()
        {
            b.filter = Some(filter);
            self.sealed_bytes += Filter::BYTES;
            self.backstop();
        }
    }

    /// Sealed blocks with a filter.
    #[doc(hidden)]
    pub fn filtered_blocks(&self) -> usize {
        self.blocks.iter().filter(|b| b.filter.is_some()).count()
    }

    /// The first line of the block holding stored line `line` (the key
    /// search keeps that block's hits under).
    pub(crate) fn block_first(&self, line: u64) -> u64 {
        self.blocks
            .iter()
            .find(|b| b.data.first + b.len as u64 > line)
            .map_or(self.open.first, |b| b.data.first)
    }

    /// The first line still visible.
    pub(crate) fn first_visible(&self) -> u64 {
        self.start
    }

    /// The lowest line popped since the last call (§13.4: search's own
    /// mark, apart from the snapshot's).
    pub(crate) fn take_search_popped(&mut self) -> Option<u64> {
        self.search_popped.take()
    }

    /// The blocks holding history lines `lo..hi` (0 = oldest visible), in
    /// order: each one's part of that range and the width every line in it
    /// was written at (0 if mixed).
    pub(crate) fn widths(&self, lo: usize, hi: usize, mut f: impl FnMut(usize, usize, u16) -> bool) {
        let hi = hi.min(self.len());
        let mut i = lo;
        while i < hi {
            let line = self.start + i as u64;
            let (end, width) = if line >= self.open.first {
                (self.end, if self.open.width == u16::MAX { 0 } else { self.open.width })
            } else {
                let b = &self.blocks[self.blocks.partition_point(|b| b.data.first <= line) - 1];
                (b.data.first + b.len as u64, b.data.width)
            };
            let to = ((end - self.start) as usize).min(hi);
            if !f(i, to, width) {
                return;
            }
            i = to;
        }
    }

    /// The block holding line number `line` (stored, not hidden): its
    /// first line (which may be hidden), one past its last, whether it is
    /// the open block, and the width its lines were written at (0 if
    /// mixed).
    pub(crate) fn block_lines(&self, line: u64) -> (u64, u64, bool, u16) {
        if line >= self.open.first {
            let w = if self.open.width == u16::MAX { 0 } else { self.open.width };
            return (self.open.first, self.end, true, w);
        }
        let b = &self.blocks[self.blocks.partition_point(|b| b.data.first <= line) - 1];
        (b.data.first, b.data.first + b.len as u64, false, b.data.width)
    }

    /// The shapes of history lines `lo..hi` (0 = oldest visible), in
    /// order, read one after another without decoding: each one's index,
    /// the width its block was written at (0 if mixed) and its
    /// [`Shape`]. Stops when `f` returns false.
    pub(crate) fn shapes(&self, lo: usize, hi: usize, mut f: impl FnMut(usize, u16, Shape) -> bool) {
        let hi = hi.min(self.len());
        let mut i = lo;
        while i < hi {
            let Some((view, n0)) = self.locate(i) else { return };
            let line = self.start + i as u64;
            let (_, end, _, width) = self.block_lines(line);
            let to = ((end - self.start) as usize).min(hi);
            let mut at = view.text_span(n0).0;
            for (k, n) in (n0..n0 + (to - i)).enumerate() {
                let (shape, next) = view.shape(n, at);
                at = next;
                if !f(i + k, width, shape) {
                    return;
                }
            }
            i = to;
        }
    }

    /// Whether every one of history lines `lo..hi` was written `cols`
    /// wide, by the widths of the blocks holding them.
    pub(crate) fn all_at_width(&self, lo: usize, hi: usize, cols: usize) -> bool {
        let mut all = true;
        self.widths(lo, hi, |_, _, w| {
            all = w as usize == cols;
            all
        });
        all
    }

    /// Test aid: seal the open block now, so tests get block boundaries
    /// without 64 KB of output.
    #[doc(hidden)]
    pub fn seal_for_test(&mut self) {
        self.seal();
    }
}

#[cfg(test)]
mod tests;
