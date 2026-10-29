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
//! - `tlen`: the text length, for WRAPPED lines only (their text has no end
//!   byte). Wrapped lines keep every cell, trailing blanks as spaces;
//!   others stop at the last cell with content.
//! - text: one entry per cell: a scalar, a cluster's scalars, a space for
//!   an empty cell; nothing for spacers.
//! - `runs`: `count`, then per run `varint(cells << 3 | slot)` and, when
//!   slot is 7, `varint(index)`: the dictionary entry (0 = default). Runs
//!   count columns; columns after the last run are in the default style.
//! - `clusters`: `count`, then per cluster `gap` (scalars since the last
//!   one ended) and `extra` (its scalars after the first).
//! - `widths`: `count`, then per exception `gap` (text entries since the
//!   last one) and the width (1 or 2): only cells whose width isn't what
//!   the Unicode table, the mode 2027 rule and the block's ambiguous
//!   setting give.
//! - `FILL`: the row's last column was an empty cell whose style shows,
//!   so the last run's style continues to the end of the row at any width.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::cell::{Cell, Color, ColorKind, Row, Width};
use crate::style::{Attrs, Style, Underline};
use crate::unicode;

/// The most bytes in one part of a block (meta, text, the dictionary), so
/// that every offset fits in 16 bits.
const PART: usize = u16::MAX as usize;
/// A checkpoint (a text offset) every this many lines.
const CHECKPOINT: usize = 16;
/// The hidden backstop: history never holds more than this many bytes (it
/// frees whole blocks from the oldest end until it doesn't).
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
    /// Prompt marks follow (OSC 133, with PR 9).
    #[allow(dead_code)]
    pub const MARKS: u8 = 1 << 6;
    /// An `extra` byte follows the flags (placeholders, PR 10).
    pub const EXTRA: u8 = 1 << 7;
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
    /// Stable line number of the first line.
    first: u64,
    /// The width every line here was written at, 0 if mixed.
    width: u16,
    /// The ambiguous-width setting its lines were written with.
    ambiguous: u8,
    /// The first line continues a logical line from the block before.
    head_continues: bool,
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
}

/// History's own record of a sealed block, changed only under the lock.
#[derive(Debug)]
struct BlockEntry {
    data: Arc<BlockData>,
    /// Lines still in use: `pop()` lowers it, the shared data is untouched.
    len: u32,
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
    /// Style (link left out) → dictionary slot, for this block only.
    dict: HashMap<Style, u32>,
    /// Screen style id → slot, indexed by `id & 63`, in front of `dict`:
    /// the common case is one compare per run. Reset with the block, and
    /// whenever the screen's table may have reused ids (a sweep, another
    /// table).
    cache: [(u32, u32); 64],
}

const NO_ENTRY: (u32, u32) = (u32::MAX, 0);

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
            dict: HashMap::new(),
            cache: [NO_ENTRY; 64],
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
            + self.dict.capacity() * (std::mem::size_of::<(Style, u32)>() + 1)
    }

    fn reset_cache(&mut self) {
        self.cache = [NO_ENTRY; 64];
    }

    /// Turn it into a sealed block, leaving an empty one starting where it
    /// ended.
    fn seal(&mut self) -> BlockData {
        let next = OpenBlock::new(self.first + self.lines() as u64, self.ambiguous);
        let b = std::mem::replace(self, next);
        BlockData {
            meta: b.meta.into_boxed_slice(),
            starts: b.starts.into_boxed_slice(),
            text: b.text.into_boxed_slice(),
            checkpoints: b.checkpoints.into_boxed_slice(),
            styles: b.styles.into_boxed_slice(),
            style_starts: b.style_starts.into_boxed_slice(),
            first: b.first,
            width: if b.width == u16::MAX { 0 } else { b.width },
            ambiguous: b.ambiguous,
            head_continues: b.head_continues,
        }
    }

    /// The dictionary slot of screen style `id` (0 = default), adding the
    /// style to the dictionary when it is new.
    #[inline]
    fn slot(&mut self, id: u32, styles: &[Style]) -> u32 {
        if id == 0 {
            return 0;
        }
        let e = self.cache[(id & 63) as usize];
        if e.0 == id {
            return e.1;
        }
        self.slot_slow(id, styles)
    }

    #[cold]
    #[inline(never)]
    fn slot_slow(&mut self, id: u32, styles: &[Style]) -> u32 {
        let mut s = styles.get(id as usize).copied().unwrap_or_default();
        // Links get their own per-block table with OSC 8 (PR 9); until
        // then there are none to keep.
        s.link = 0;
        let slot = if s == Style::DEFAULT {
            0
        } else if let Some(&slot) = self.dict.get(&s) {
            slot
        } else {
            room(&mut self.style_starts, 1);
            room(&mut self.styles, 15);
            self.style_starts.push(self.styles.len() as u16);
            put_style(&mut self.styles, &s);
            let slot = self.style_starts.len() as u32;
            self.dict.insert(s, slot);
            slot
        };
        self.cache[(id & 63) as usize] = (id, slot);
        slot
    }

    /// Store `row`. False, with nothing changed, if a part would pass
    /// 64 KB.
    fn push(&mut self, row: &Row, styles: &[Style], scratch: &mut Scratch) -> bool {
        let before = (self.meta.len(), self.text.len(), self.styles.len(), self.style_starts.len());
        let n = self.lines();
        room(&mut self.starts, 1);
        self.starts.push(before.0 as u16);
        let checkpoint = n.is_multiple_of(CHECKPOINT);
        if checkpoint {
            room(&mut self.checkpoints, 1);
            self.checkpoints.push(before.1 as u16);
        }
        self.encode(row, styles, scratch);
        if self.meta.len() <= PART && self.text.len() <= PART && self.styles.len() <= PART {
            let w = row.cells.len() as u16;
            self.width = if n == 0 || self.width == w { w } else { 0 };
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
        self.reset_cache();
        false
    }

    fn encode(&mut self, row: &Row, styles: &[Style], scratch: &mut Scratch) {
        let cells = &row.cells[..];
        let cols = cells.len();
        let wrapped = row.wrapped;

        // Common case: every cell is narrow ASCII (or empty) in the
        // default style, so the text is a byte copy and there are no runs.
        if cells.iter().fold(0, |a, c| a | c.bits()) & !0x7F == 0 {
            let end = if wrapped { cols } else { ascii_text_end(cells) };
            room(&mut self.meta, 6);
            if wrapped {
                self.meta.push(flag::WRAPPED);
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
        let text_end = if wrapped { text_cols } else { text_end(cells) };
        let last = cells[cols - 1];
        let fill = !wrapped && last.shows_empty() && last.bits() & (Cell::CONTENT | Cell::KIND) == 0 && !last.is_spacer();
        let run_end = if wrapped {
            cols
        } else {
            let shows = cells.iter().rposition(|c| c.shows_empty()).map_or(0, |i| i + 1);
            text_end.max(shows)
        };

        // The text, and the clusters and width exceptions it needs.
        scratch.clusters.clear();
        scratch.widths.clear();
        let cluster_bytes = row.clusters().map_or(0, |x| x.raw().0.len());
        room(&mut self.text, text_end * 4 + cluster_bytes + 1);
        let text_start = self.text.len();
        let amb = self.ambiguous;
        let (mut scalars, mut cluster_end, mut entry, mut width_next) = (0u32, 0u32, 0u32, 0u32);
        for &c in &cells[..text_end] {
            if c.is_spacer() {
                continue;
            }
            let bits = c.bits();
            let predicted;
            if bits & (Cell::CONTENT & !0x7F | Cell::KIND | Cell::WIDTH) == 0 {
                // Narrow ASCII (or empty), not a cluster: the usual case.
                self.text.push(if bits as u8 == 0 { b' ' } else { bits as u8 });
                scalars += 1;
                entry += 1;
                continue;
            } else if c.is_cluster() {
                let s = row.clusters().map_or("", |x| x.get(c.codepoint()));
                let n = s.chars().count() as u32;
                if n == 0 {
                    self.text.push(b' ');
                    predicted = 1;
                    scalars += 1;
                } else {
                    self.text.extend_from_slice(s.as_bytes());
                    if n > 1 {
                        scratch.clusters.push((scalars - cluster_end, n - 1));
                        cluster_end = scalars + n;
                        predicted = unicode::cluster_width(s, amb);
                    } else {
                        predicted = single_width(s.chars().next().unwrap_or(' '), amb);
                    }
                    scalars += n;
                }
            } else {
                let ch = match c.codepoint() {
                    0 => ' ',
                    cp => char::from_u32(cp).unwrap_or(' '),
                };
                let mut buf = [0u8; 4];
                self.text.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                predicted = single_width(ch, amb);
                scalars += 1;
            }
            let actual = if c.is_wide() { 2 } else { 1 };
            if actual != predicted {
                scratch.widths.push((entry - width_next, actual));
                width_next = entry + 1;
            }
            entry += 1;
        }
        let tlen = self.text.len() - text_start;
        if !wrapped {
            self.text.push(END);
        }

        // Runs, by column, merged when neighbours share a slot; the
        // default tail needs none.
        scratch.runs.clear();
        let mut last_id = u32::MAX;
        let mut slot = 0;
        for &c in &cells[..run_end] {
            let id = c.style_id();
            if id != last_id {
                last_id = id;
                slot = self.slot(id, styles);
            }
            match scratch.runs.last_mut() {
                Some((n, s)) if *s == slot => *n += 1,
                _ => scratch.runs.push((1, slot)),
            }
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
        room(
            &mut self.meta,
            6 + 6 + scratch.runs.len() * 8 + scratch.clusters.len() * 10 + scratch.widths.len() * 6,
        );
        self.meta.push(flags);
        if wrapped {
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

/// Buffers the encoder reuses, so storing a line allocates nothing.
#[derive(Debug, Default)]
struct Scratch {
    /// (gap, extra scalars) per cluster.
    clusters: Vec<(u32, u32)>,
    /// (gap, width) per width exception.
    widths: Vec<(u32, u8)>,
    /// (cells, slot) per run.
    runs: Vec<(u32, u32)>,
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

/// The width a single scalar's cell is given when written (§6.3): 2 for
/// wide characters (and ambiguous ones with the setting at 2), else 1.
#[inline]
fn single_width(c: char, ambiguous: u8) -> u8 {
    if c.is_ascii() {
        return 1;
    }
    if unicode::props(c).width_with(ambiguous) == 2 { 2 } else { 1 }
}

/// Columns of text to store for an unwrapped row of narrow ASCII: up to the
/// last cell that is neither empty nor a space.
fn ascii_text_end(cells: &[Cell]) -> usize {
    // Most trailing cells were never written: skip them 8 at a time (an
    // OR over 64 bytes, which vectorises), then one at a time.
    let mut len = cells.len();
    while len >= 8 && cells[len - 8..len].iter().fold(0, |a, c| a | c.bits()) == 0 {
        len -= 8;
    }
    while len > 0 && matches!(cells[len - 1].bits(), 0 | 0x20) {
        len -= 1;
    }
    len
}

/// Columns of text to store for an unwrapped row: up to the last cell with
/// content other than a space, or a wide one (and its spacer).
fn text_end(cells: &[Cell]) -> usize {
    // An empty wide cell keeps its width only through the text.
    let has_text =
        |c: &Cell| c.is_wide() || c.bits() & (Cell::CONTENT | Cell::KIND) != 0 && (c.is_cluster() || c.codepoint() != ' ' as u32);
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
    ambiguous: u8,
}

impl<'a> View<'a> {
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
        let k = n / CHECKPOINT;
        let mut at = self.checkpoints[k] as usize;
        for j in k * CHECKPOINT..n {
            at += self.stored_len(j, at);
        }
        let len = self.stored_len(n, at);
        let ends = self.flags(n) & flag::WRAPPED == 0;
        (at, len - ends as usize)
    }

    /// Bytes of `text` line `n` takes, starting at `at`: `tlen` for a
    /// wrapped line, up to and including the end byte otherwise.
    #[inline]
    fn stored_len(&self, n: usize, at: usize) -> usize {
        let meta = self.meta_of(n);
        if meta[0] & flag::WRAPPED != 0 {
            let mut pos = 1 + (meta[0] & flag::EXTRA != 0) as usize;
            get_varint(meta, &mut pos) as usize
        } else {
            find_end(&self.text[at..]).map_or(self.text.len() - at, |i| i + 1)
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
    /// the default first). With FILL, the last run's style is carried on
    /// to `fill_to` cells.
    fn decode(&self, n: usize, fill_to: usize, row: &mut Row, styles: &mut Vec<Style>) {
        let meta = self.meta_of(n);
        let flags = meta[0];
        let mut pos = 1 + (flags & flag::EXTRA != 0) as usize;
        if flags & flag::WRAPPED != 0 {
            get_varint(meta, &mut pos);
        }
        let (at, len) = self.text_span(n);
        let text = std::str::from_utf8(&self.text[at..at + len]).unwrap_or("");
        row.reset(0, Cell::BLANK);
        row.wrapped = flags & flag::WRAPPED != 0;
        styles.clear();
        styles.push(Style::DEFAULT);

        let runs_at = pos;
        if flags & flag::STYLED != 0 {
            for _ in 0..get_varint(meta, &mut pos) {
                if get_varint(meta, &mut pos) & 7 == 7 {
                    get_varint(meta, &mut pos);
                }
            }
        }

        if flags & (flag::CLUSTERS | flag::WIDTHS) == 0 && text.is_ascii() {
            row.cells.reserve(text.len() + 1);
            row.cells.extend(
                text.bytes()
                    .map(|b| if b == b' ' { Cell::BLANK } else { Cell::BLANK.with_char(b as char) }),
            );
        } else {
            self.decode_text(meta, &mut pos, flags, text, row);
        }
        if flags & flag::EDGE != 0 {
            row.cells.push(Cell::BLANK.with_width(Width::EdgeSpacer));
        }

        if flags & flag::STYLED != 0 {
            let mut pos = runs_at;
            let mut col = 0;
            // The slot of the run before and its id in `styles`: runs next
            // to each other often go back and forth between two styles.
            let mut seen: [(u32, Cell); 2] = [(0, Cell::BLANK); 2];
            for _ in 0..get_varint(meta, &mut pos) {
                let v = get_varint(meta, &mut pos);
                let mut slot = v & 7;
                if slot == 7 {
                    slot = get_varint(meta, &mut pos);
                }
                let cells = (v >> 3) as usize;
                let t = if slot == 0 {
                    Cell::BLANK
                } else if seen[0].0 == slot {
                    seen[0].1
                } else if seen[1].0 == slot {
                    seen.swap(0, 1);
                    seen[0].1
                } else {
                    let s = self.style(slot);
                    let id = styles.iter().position(|x| *x == s).unwrap_or_else(|| {
                        styles.push(s);
                        styles.len() - 1
                    });
                    let t = Cell::template(id as u32, s.shows_empty());
                    seen[1] = seen[0];
                    seen[0] = (slot, t);
                    t
                };
                let have = row.cells.len().min(col + cells);
                if t != Cell::BLANK {
                    for c in &mut row.cells[col.min(have)..have] {
                        *c = c.with_style(t);
                    }
                }
                if col + cells > have {
                    row.cells.resize(col + cells, t);
                }
                col += cells;
            }
        }
        if flags & flag::FILL != 0
            && let Some(&last) = row.cells.last()
            && row.cells.len() < fill_to
        {
            row.cells.resize(fill_to, last.style_part());
        }
    }

    /// The text of a line with clusters, width exceptions or non-ASCII
    /// characters, into cells.
    #[inline(never)]
    fn decode_text(&self, meta: &[u8], pos: &mut usize, flags: u8, text: &str, row: &mut Row) {
        let mut clusters = 0;
        if flags & flag::CLUSTERS != 0 {
            clusters = get_varint(meta, pos);
        }
        let clusters_at = *pos;
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

        // Scalar index where the next cluster starts, and its extra scalars.
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
        let (mut scalar, mut entry) = (0u32, 0u32);
        let mut chars = text.char_indices();
        while let Some((at, c)) = chars.next() {
            let mut end = at + c.len_utf8();
            let cluster = scalar == next_cluster;
            if cluster {
                for _ in 0..extra {
                    if let Some((i, d)) = chars.next() {
                        end = i + d.len_utf8();
                    }
                }
                scalar += 1 + extra;
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
                unicode::cluster_width(&text[at..end], amb)
            } else {
                single_width(c, amb)
            };
            entry += 1;
            let col = row.cells.len();
            let cell = if c == ' ' && !cluster {
                Cell::BLANK
            } else {
                Cell::BLANK.with_char(c)
            };
            if w == 2 {
                row.cells.push(cell.with_width(Width::Wide));
                row.cells.push(Cell::BLANK.with_width(Width::Spacer));
            } else {
                row.cells.push(cell);
            }
            if cluster {
                row.set_cluster(col, &text[at..end]);
            }
        }
    }

    /// Check line `n`'s meta: every section reads to where the next line
    /// starts, slots name dictionary entries, and its text is UTF-8 with
    /// as many scalars as its clusters need.
    fn check_line(&self, n: usize, meta_end: usize) -> Result<(), String> {
        let meta = &self.meta[self.starts[n] as usize..meta_end];
        let flags = *meta.first().ok_or("empty meta")?;
        if flags & (flag::MARKS | flag::EXTRA) != 0 {
            return Err(format!("flags {flags:#x}: marks or extra, which nothing writes yet"));
        }
        let (at, len) = self.text_span(n);
        let text = std::str::from_utf8(self.text.get(at..at + len).ok_or("text past the end")?).map_err(|e| e.to_string())?;
        if text.bytes().any(|b| b < 0x20 && b != 0) {
            return Err(format!("text {text:?} holds a control"));
        }
        let mut pos = 1;
        if flags & flag::WRAPPED != 0 {
            get_varint(meta, &mut pos);
        } else if self.text.get(at + len) != Some(&END) {
            return Err("an unwrapped line without its end byte".into());
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
            }
        }
        if flags & flag::CLUSTERS != 0 {
            let (mut need, count) = (0u64, get_varint(meta, &mut pos));
            for _ in 0..count {
                let gap = get_varint(meta, &mut pos);
                let extra = get_varint(meta, &mut pos);
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

/// The first 0x0A in `text`, eight bytes at a time.
#[inline]
fn find_end(text: &[u8]) -> Option<usize> {
    const LO: u64 = 0x0101_0101_0101_0101;
    const HI: u64 = 0x8080_8080_8080_8080;
    let mut i = 0;
    while i + 8 <= text.len() {
        let w = u64::from_le_bytes(text[i..i + 8].try_into().expect("8 bytes")) ^ (LO * END as u64);
        let zero = w.wrapping_sub(LO) & !w & HI;
        if zero != 0 {
            return Some(i + (zero.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    text[i..].iter().position(|&b| b == END).map(|p| i + p)
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
    /// The style table the slot cache was filled from: its epoch and sweep
    /// count. `[0, 0]` = unknown, so the cache is dropped each line.
    cache_key: [u64; 2],
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
            cache_key: [0, 0],
            scratch: Scratch::default(),
        }
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
        self.trim();
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

    /// Store a row (trailing blank cells dropped); its cells' style ids
    /// index `styles`.
    pub fn push(&mut self, row: &Row, styles: &[Style]) {
        self.push_keyed(row, styles, [0, 0]);
    }

    /// [`Scrollback::push`] for rows of one style table, named by `key`
    /// (its epoch and sweep count): while it stays the same, the slot
    /// cache is kept from line to line.
    pub fn push_keyed(&mut self, row: &Row, styles: &[Style], key: [u64; 2]) {
        self.end += 1;
        if self.limit == 0 {
            // Nothing is kept, but the count goes on: line numbers are the
            // screen's.
            self.start = self.end;
            self.open.first = self.end;
            return;
        }
        if key != self.cache_key || key == [0, 0] {
            self.open.reset_cache();
            self.cache_key = key;
        }
        if self.open.lines() == 0 {
            self.open.head_continues = self.last_wrapped;
        }
        // An empty row always fits.
        if !self.open.push(row, styles, &mut self.scratch) {
            if self.open.lines() > 0 {
                self.seal();
                self.open.head_continues = self.last_wrapped;
            }
            if !self.open.push(row, styles, &mut self.scratch) {
                // Too wide for a block (only past 4096 columns): cut it.
                let mut cut = row.clone();
                cut.resize_cells(MAX_CELLS);
                if cut.cells.last().is_some_and(|c| c.is_wide()) {
                    cut.free_clusters(MAX_CELLS - 1..MAX_CELLS);
                    cut.cells[MAX_CELLS - 1] = Cell::BLANK;
                }
                if !self.open.push(&cut, styles, &mut self.scratch) {
                    debug_assert!(false, "a 4096-cell row fits in an empty block");
                    self.open.push(&Row::default(), styles, &mut self.scratch);
                }
            }
        }
        self.last_wrapped = row.wrapped;
        self.trim();
    }

    /// Seal the open block, if it holds any lines.
    fn seal(&mut self) {
        if self.open.lines() == 0 {
            return;
        }
        let len = self.open.lines() as u32;
        let data = self.open.seal();
        self.sealed_bytes += data.bytes();
        self.blocks.push_back(BlockEntry { data: Arc::new(data), len });
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
        while self.sealed_bytes + self.open.bytes() > self.byte_limit && !self.blocks.is_empty() {
            self.free_oldest();
        }
    }

    fn free_oldest(&mut self) {
        if let Some(b) = self.blocks.pop_front() {
            self.sealed_bytes -= b.data.bytes();
            self.start = self.start.max(b.data.first + b.len as u64);
        }
    }

    /// Remove the newest line (pulling lines back onto the screen, §12.2)
    /// and expand it into `row`, as [`Scrollback::get_at_width`] does. In a
    /// sealed block only the entry's count goes down; the shared data
    /// stays as it is until the block is freed. Records the lowest line
    /// popped. False, leaving the buffers alone, if history is empty.
    pub fn pop_into(&mut self, fill_to: usize, row: &mut Row, styles: &mut Vec<Style>) -> bool {
        if self.is_empty() {
            return false;
        }
        let line = self.end - 1;
        self.get_at_width(self.len() - 1, fill_to, row, styles);
        if self.open.lines() > 0 {
            self.open.pop();
        } else {
            let b = self.blocks.back_mut().expect("a stored line");
            b.len -= 1;
            if b.len == 0 {
                let b = self.blocks.pop_back().expect("a block");
                self.sealed_bytes -= b.data.bytes();
            }
            // The next push starts a new open block where this one ends.
            self.open = OpenBlock::new(line, self.open.ambiguous);
        }
        self.end = line;
        self.popped_from = Some(self.popped_from.map_or(line, |p| p.min(line)));
        // Lines hidden by the limit may fit again.
        self.start = self.oldest().max(self.end.saturating_sub(self.limit as u64));
        self.last_wrapped = !self.is_empty() && self.wrapped(self.len() - 1);
        true
    }

    /// [`Scrollback::pop_into`] into new buffers.
    pub fn pop(&mut self) -> Option<(Row, Vec<Style>)> {
        let mut row = Row::default();
        let mut styles = Vec::new();
        self.pop_into(0, &mut row, &mut styles).then_some((row, styles))
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
        self.get_at_width(i, 0, row, styles)
    }

    /// [`Scrollback::get_into`] for a row shown `cols` wide: a FILL line's
    /// last style carries on to the end of the row.
    pub fn get_at_width(&self, i: usize, cols: usize, row: &mut Row, styles: &mut Vec<Style>) -> bool {
        let Some((view, n)) = self.locate(i) else { return false };
        view.decode(n, cols, row, styles);
        true
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
            bytes += b.data.bytes();
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
        if !self.blocks.is_empty() && self.sealed_bytes + self.open.bytes() > self.byte_limit {
            return Err(format!("{} bytes held, over the backstop", self.sealed_bytes + self.open.bytes()));
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
    /// The link (PR 9).
    #[allow(dead_code)]
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
    let m = if s.underline != Underline::None { more::UNDERLINE } else { 0 } | if s.image_id_high != 0 { more::IMAGE_HIGH } else { 0 };
    if m != 0 {
        h |= head::MORE;
        out.push(m);
        if m & more::UNDERLINE != 0 {
            out.push(s.underline as u8);
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
        if m & more::IMAGE_HIGH != 0 {
            s.image_id_high = data.get(pos).copied().unwrap_or(0);
        }
    }
    s
}

#[cfg(test)]
mod tests;
