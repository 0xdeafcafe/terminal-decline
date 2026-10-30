//! Sealed blocks compressed with LZ4 while history is idle (R.7, cells
//! design §11.8), and the small cache of blocks decompressed again.
//!
//! A sealed block is compressed by building a [`Packed`] from its
//! [`BlockData`] off the lock and swapping it into the block's entry
//! under the lock. Readers that hold the old `Arc<BlockData>` (a search,
//! a cached decode) carry on with it. What every line needs without
//! decompressing stays outside: the line count, each line's flags byte,
//! the first line, width, ambiguous setting, `head_continues`,
//! `has_marks`, the image list and the links (shared `Arc`s). Everything
//! else (meta, text, the dictionary and the offsets into them) is one
//! LZ4 block.

use std::sync::{Arc, Mutex};

use super::BlockData;
use crate::image::ImageKey;
use crate::links::LinkValue;

/// A sealed block, compressed.
#[derive(Debug)]
pub struct Packed {
    first: u64,
    width: u16,
    ambiguous: u8,
    head_continues: bool,
    has_marks: bool,
    images: Box<[ImageKey]>,
    links: Box<[Arc<LinkValue>]>,
    /// Each line's flags byte.
    flags: Flags,
    /// Lengths of the parts as written, in order: meta, starts, text,
    /// checkpoints, styles, style_starts (u16 parts in entries).
    parts: [u32; 6],
    /// Compressed sizes of meta, text and styles in `lz`.
    sizes: [u32; 3],
    /// Meta, text, styles and the offsets (as deltas, low bytes then
    /// high), each compressed on its own, back to back.
    lz: Box<[u8]>,
}

impl Packed {
    /// Compress `d` (off the lock: about 100–300 µs a block).
    pub fn pack(d: &BlockData) -> Packed {
        let parts = [
            d.meta.len() as u32,
            d.starts.len() as u32,
            d.text.len() as u32,
            d.checkpoints.len() as u32,
            d.styles.len() as u32,
            d.style_starts.len() as u32,
        ];
        // The offsets as deltas, low bytes then high bytes: a line's meta
        // and a style's entry are mostly the same size, so they repeat.
        let mut offsets = Vec::with_capacity(2 * (d.starts.len() + d.checkpoints.len() + d.style_starts.len()));
        for v in [&d.starts[..], &d.checkpoints, &d.style_starts] {
            let start = offsets.len();
            let mut prev = 0u16;
            for &x in v {
                offsets.push(x.wrapping_sub(prev) as u8);
                prev = x;
            }
            prev = 0;
            for &x in v {
                offsets.push((x.wrapping_sub(prev) >> 8) as u8);
                prev = x;
            }
            debug_assert_eq!(offsets.len() - start, 2 * v.len());
        }
        let mut lz = Vec::new();
        let mut sizes = [0u32; 3];
        // Each part on its own: they have little in common.
        for (k, part) in [&d.meta[..], &d.text, &d.styles, &offsets].into_iter().enumerate() {
            let c = lz4_flex::block::compress(part);
            if k < 3 {
                sizes[k] = c.len() as u32;
            }
            lz.extend_from_slice(&c);
        }
        Packed {
            first: d.first,
            width: d.width,
            ambiguous: d.ambiguous,
            head_continues: d.head_continues,
            has_marks: d.has_marks,
            images: d.images.clone(),
            links: d.links.clone(),
            flags: Flags::new(d.starts.iter().map(|&s| d.meta[s as usize])),
            parts,
            sizes,
            lz: lz.into_boxed_slice(),
        }
    }

    /// The block as written (well under 100 µs a block).
    pub fn unpack(&self) -> BlockData {
        let p = self.parts.map(|n| n as usize);
        let [m, t, s] = self.sizes.map(|n| n as usize);
        let lz = &self.lz;
        let part = |from: usize, to: usize, size: usize| -> Box<[u8]> {
            lz4_flex::block::decompress(&lz[from..to], size)
                .expect("history block decompresses")
                .into_boxed_slice()
        };
        let meta = part(0, m, p[0]);
        let text = part(m, m + t, p[2]);
        let styles = part(m + t, m + t + s, p[4]);
        let offsets = part(m + t + s, lz.len(), 2 * (p[1] + p[3] + p[5]));
        let mut at = 0;
        let mut u16s = |n: usize| -> Box<[u16]> {
            let (lo, hi) = offsets[at..at + 2 * n].split_at(n);
            at += 2 * n;
            let mut prev = 0u16;
            lo.iter()
                .zip(hi)
                .map(|(&l, &h)| {
                    prev = prev.wrapping_add(u16::from_le_bytes([l, h]));
                    prev
                })
                .collect()
        };
        let starts = u16s(p[1]);
        let checkpoints = u16s(p[3]);
        let style_starts = u16s(p[5]);
        BlockData {
            meta,
            starts,
            text,
            checkpoints,
            styles,
            style_starts,
            links: self.links.clone(),
            first: self.first,
            width: self.width,
            ambiguous: self.ambiguous,
            head_continues: self.head_continues,
            has_marks: self.has_marks,
            images: self.images.clone(),
        }
    }

    /// Every byte it holds, for the backstop.
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Packed>()
            + 2 * std::mem::size_of::<usize>()
            + self.lz.len()
            + self.flags.bytes()
            + self.links.iter().map(|l| super::link_bytes(l)).sum::<usize>()
            + self.images.len() * std::mem::size_of::<ImageKey>()
    }

    /// Line `n`'s flags byte.
    pub fn flags(&self, n: usize) -> u8 {
        self.flags.get(n)
    }
}

/// Each line's flags byte, as runs: most lines of a block share theirs.
#[derive(Debug)]
struct Flags {
    /// One past the last line of each run, and the run's flags.
    runs: Box<[(u16, u8)]>,
}

impl Flags {
    fn new(flags: impl Iterator<Item = u8>) -> Flags {
        let mut runs: Vec<(u16, u8)> = Vec::new();
        for (n, f) in flags.enumerate() {
            match runs.last_mut() {
                Some(r) if r.1 == f => r.0 = n as u16 + 1,
                _ => runs.push((n as u16 + 1, f)),
            }
        }
        Flags {
            runs: runs.into_boxed_slice(),
        }
    }

    fn get(&self, n: usize) -> u8 {
        let i = self.runs.partition_point(|r| r.0 as usize <= n);
        self.runs[i].1
    }

    fn len(&self) -> usize {
        self.runs.last().map_or(0, |r| r.0 as usize)
    }

    fn bytes(&self) -> usize {
        self.runs.len() * std::mem::size_of::<(u16, u8)>()
    }
}

/// A sealed block as history stores it: as written, or compressed. A
/// search or a filter job takes one of these under the lock and reads it
/// off the lock ([`Body::unpack`]).
#[derive(Clone, Debug)]
pub enum Body {
    Plain(Arc<BlockData>),
    Packed(Arc<Packed>),
}

impl Body {
    pub fn first(&self) -> u64 {
        match self {
            Body::Plain(d) => d.first,
            Body::Packed(p) => p.first,
        }
    }

    pub fn lines(&self) -> usize {
        match self {
            Body::Plain(d) => d.starts.len(),
            Body::Packed(p) => p.flags.len(),
        }
    }

    pub fn width(&self) -> u16 {
        match self {
            Body::Plain(d) => d.width,
            Body::Packed(p) => p.width,
        }
    }

    pub fn has_marks(&self) -> bool {
        match self {
            Body::Plain(d) => d.has_marks,
            Body::Packed(p) => p.has_marks,
        }
    }

    pub fn images(&self) -> &[ImageKey] {
        match self {
            Body::Plain(d) => &d.images,
            Body::Packed(p) => &p.images,
        }
    }

    pub fn bytes(&self) -> usize {
        match self {
            Body::Plain(d) => d.bytes(),
            Body::Packed(p) => p.bytes(),
        }
    }

    pub fn is_packed(&self) -> bool {
        matches!(self, Body::Packed(_))
    }

    /// The same block (the same shared data).
    pub fn same(&self, other: &Body) -> bool {
        match (self, other) {
            (Body::Plain(a), Body::Plain(b)) => Arc::ptr_eq(a, b),
            (Body::Packed(a), Body::Packed(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// The block as written: shared if it is, decompressed (not cached)
    /// if not.
    pub fn unpack(&self) -> Arc<BlockData> {
        match self {
            Body::Plain(d) => d.clone(),
            Body::Packed(p) => Arc::new(p.unpack()),
        }
    }
}

/// Blocks decompressed again, most recent first: reading a view's rows,
/// a pull-back or a reflow walks one or two blocks at a time.
#[derive(Debug, Default)]
pub struct Cache(Mutex<[Option<Cached>; CACHED]>);

/// A compressed block and its data decompressed.
type Cached = (Arc<Packed>, Arc<BlockData>);

/// Blocks the cache keeps (R.7: two).
pub const CACHED: usize = 2;

impl Cache {
    /// `p` decompressed, from the cache if it is there.
    pub fn get(&self, p: &Arc<Packed>) -> Arc<BlockData> {
        let mut slots = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = slots.iter().position(|s| s.as_ref().is_some_and(|(k, _)| Arc::ptr_eq(k, p))) {
            slots[..=i].rotate_right(1);
            return slots[0].as_ref().expect("a cached block").1.clone();
        }
        let d = Arc::new(p.unpack());
        slots.rotate_right(1);
        slots[0] = Some((p.clone(), d.clone()));
        d
    }

    /// Bytes the decompressed blocks hold.
    pub fn bytes(&self) -> usize {
        let slots = self.0.lock().unwrap_or_else(|e| e.into_inner());
        slots.iter().flatten().map(|(_, d)| d.bytes()).sum()
    }

    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
    }
}
