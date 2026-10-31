//! History saved across a restart (the app's `restore_scrollback`): the
//! newest blocks, compressed as history keeps them (R.7), written to a
//! byte string and read back into a new terminal's empty history.
//!
//! Sealed blocks already compressed are written as they are; the others,
//! the open block and the screen's rows (stored first through a history
//! of their own, so they come back as lines) are compressed with
//! [`Packed::pack`] as idle compression would. No line is decoded or
//! stored again. Images don't outlive the process, so blocks are written
//! without their image lists: a placeholder cell of an image that is
//! gone draws nothing.
//!
//! ```text
//! magic "TDHIST\0\0", version:u32, bytes:u64 (all that follows the
//! checksum), checksum:u64 (FNV-1a 64 of it), then:
//! saved_at:u64 (seconds since 1970), keep:u64 (lines shown), screen:u32
//! (of those, rows that were on the screen), blocks:u32, and per block
//! its lines in use:u32 and the block (`Packed::write`).
//! ```
//!
//! Integers are little-endian. Reading never panics: a file cut short or
//! changed fails the checksum, and every block is decompressed and its
//! lines read through before history takes any of them.

use std::sync::Arc;

use super::packed::Packed;
use super::{BlockEntry, Body, OpenBlock, Scrollback, flag};
use crate::cell::Row;
use crate::links::LinkValue;
use crate::style::Style;

const MAGIC: &[u8; 8] = b"TDHIST\0\0";
const VERSION: u32 = 1;
/// Magic, version, length and checksum.
const HEADER: usize = 8 + 4 + 8 + 8;

/// What [`Scrollback::restore`] put back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Restored {
    /// Lines of history now shown.
    pub lines: usize,
    /// The newest of them that were on the screen when it was saved.
    pub screen: usize,
    /// When it was saved, in seconds since 1970.
    pub saved_at: u64,
}

/// Why saved history couldn't be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreError {
    /// Not saved history, or from a later format.
    NotHistory,
    /// Cut short or changed since it was written.
    Damaged,
    /// History already has lines.
    NotEmpty,
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RestoreError::NotHistory => "not saved history",
            RestoreError::Damaged => "damaged",
            RestoreError::NotEmpty => "history isn't empty",
        })
    }
}

pub(super) mod put {
    pub fn u16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(out: &mut Vec<u8>, v: u64) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    /// A length (u32) and the bytes.
    pub fn bytes(out: &mut Vec<u8>, b: &[u8]) {
        u32(out, b.len() as u32);
        out.extend_from_slice(b);
    }
}

/// Reads what [`put`] wrote; None once the input is cut short.
pub(super) struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.0.len() {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
}

/// FNV-1a, 64 bits, 8 bytes a step (about 2 GB/s): enough to tell a file
/// cut short or changed by accident, not a defence against one forged.
fn checksum(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let (chunks, rest) = b.as_chunks::<8>();
    for c in chunks {
        h = (h ^ u64::from_le_bytes(*c)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    for &x in rest {
        h = (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The blocks to save, oldest first, with their lines in use.
type Saving = Vec<(Arc<Packed>, u32)>;

impl Scrollback {
    /// This history's blocks, compressed (the open block too), with their
    /// lines in use, after `out`'s.
    fn packed_blocks_into(&self, out: &mut Saving) {
        for b in &self.blocks {
            let p = match &b.data {
                Body::Packed(p) => p.clone(),
                Body::Plain(d) => Arc::new(Packed::pack(d)),
            };
            out.push((p, b.len));
        }
        if self.open.lines() > 0 {
            let d = self.open.snapshot();
            out.push((Arc::new(Packed::pack(&d)), self.open.lines() as u32));
        }
    }

    /// The newest `max` lines of history and then `screen` (rows of the
    /// screen whose styles index `styles` and links `links`), saved as
    /// the module's header says, at `saved_at`. Whole blocks: the
    /// oldest one may hold lines over `max`, which are hidden again when
    /// restored.
    pub fn save<'r>(
        &self,
        screen: impl IntoIterator<Item = &'r Row>,
        styles: &[Style],
        links: &[Option<Arc<LinkValue>>],
        max: usize,
        saved_at: u64,
    ) -> Vec<u8> {
        let mut rows = Scrollback::new(usize::MAX);
        rows.byte_limit = usize::MAX;
        rows.set_ambiguous(self.open.ambiguous);
        rows.last_wrapped = self.last_wrapped;
        let mut on_screen = 0;
        for row in screen {
            rows.push_keyed(row, styles, links, [0, 0]);
            on_screen += 1;
        }
        let mut blocks = Vec::new();
        self.packed_blocks_into(&mut blocks);
        rows.packed_blocks_into(&mut blocks);
        // Lines shown: those not hidden, at most `max`.
        let shown = (self.len() + on_screen).min(max);
        let mut held = 0usize;
        let mut from = blocks.len();
        while from > 0 && held < shown {
            from -= 1;
            held += blocks[from].1 as usize;
        }
        let blocks = &blocks[from..];
        let mut body = Vec::with_capacity(64 + blocks.iter().map(|(p, _)| p.bytes()).sum::<usize>());
        put::u64(&mut body, saved_at);
        put::u64(&mut body, shown as u64);
        put::u32(&mut body, on_screen.min(shown) as u32);
        put::u32(&mut body, blocks.len() as u32);
        for (p, len) in blocks {
            put::u32(&mut body, *len);
            p.write(&mut body);
        }
        let mut out = Vec::with_capacity(HEADER + body.len());
        out.extend_from_slice(MAGIC);
        put::u32(&mut out, VERSION);
        put::u64(&mut out, body.len() as u64);
        put::u64(&mut out, checksum(&body));
        out.extend_from_slice(&body);
        out
    }

    /// Put back history [`Scrollback::save`] wrote, into this empty one.
    /// Its own limits apply: lines over the line limit are hidden, blocks
    /// past the byte backstop freed.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<Restored, RestoreError> {
        if self.end != 0 || !self.blocks.is_empty() || self.open.lines() > 0 {
            return Err(RestoreError::NotEmpty);
        }
        if bytes.len() < HEADER || &bytes[..8] != MAGIC {
            return Err(RestoreError::NotHistory);
        }
        let mut r = Reader(&bytes[8..HEADER]);
        let (version, len, sum) = (r.u32(), r.u64(), r.u64());
        if version != Some(VERSION) {
            return Err(RestoreError::NotHistory);
        }
        let body = &bytes[HEADER..];
        if len != Some(body.len() as u64) || sum != Some(checksum(body)) {
            return Err(RestoreError::Damaged);
        }
        let (blocks, keep, screen, saved_at) = read_body(body).ok_or(RestoreError::Damaged)?;
        let total: u64 = blocks.iter().map(|b| b.len as u64).sum();
        if keep > total || screen as u64 > keep {
            return Err(RestoreError::Damaged);
        }
        for b in &blocks {
            self.sealed_bytes += b.bytes();
        }
        self.blocks = blocks.into();
        self.end = total;
        self.start = total - keep;
        self.open = OpenBlock::new(total, self.open.ambiguous);
        self.last_wrapped = !self.is_empty() && self.flags_of(self.len() - 1).is_some_and(|f| f & flag::WRAPPED != 0);
        self.trim();
        self.backstop();
        Ok(Restored {
            lines: self.len(),
            screen: (screen as usize).min(self.len()),
            saved_at,
        })
    }
}

/// The blocks of a saved history's body, numbered from 0, each one
/// decompressed and its lines read through; and the lines to show, the
/// screen's rows among them and when it was saved.
fn read_body(body: &[u8]) -> Option<(Vec<BlockEntry>, u64, u32, u64)> {
    let mut r = Reader(body);
    let saved_at = r.u64()?;
    let keep = r.u64()?;
    let screen = r.u32()?;
    let n = r.u32()? as usize;
    let mut blocks = Vec::with_capacity(n.min(4096));
    let mut first = 0u64;
    for i in 0..n {
        let len = r.u32()?;
        let mut p = Packed::read(&mut r, first)?;
        if i == 0 {
            // Nothing before it to continue.
            p.set_head_continues(false);
        }
        let d = p.try_unpack()?;
        if len == 0 || len as usize > d.lines() || !sound(&d, &p) {
            return None;
        }
        blocks.push(BlockEntry {
            first,
            data: Body::Packed(Arc::new(p)),
            len,
            filter: None,
        });
        first += len as u64;
    }
    r.0.is_empty().then_some((blocks, keep, screen, saved_at))
}

/// Whether a block read back is one history can use: its offsets in
/// order and in bounds, its flags its lines', and every line readable.
fn sound(d: &super::BlockData, p: &Packed) -> bool {
    let ascending_within = |v: &[u16], len: usize| v.windows(2).all(|w| w[0] <= w[1]) && v.iter().all(|&x| (x as usize) <= len);
    if d.starts.is_empty()
        || !ascending_within(&d.starts, d.meta.len().saturating_sub(1))
        || !ascending_within(&d.checkpoints, d.text.len())
        || !ascending_within(&d.style_starts, d.styles.len())
        || d.starts.iter().enumerate().any(|(n, &s)| d.meta[s as usize] != p.flags(n))
    {
        return false;
    }
    // The line reader trusts what history wrote; a block that got this
    // far is one it wrote, but a panic here only loses the restore.
    let check = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| d.view(d.lines()).check(d.lines()).is_ok()));
    check.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Cell;

    fn line(text: &str) -> Row {
        let mut r = Row::new(text.chars().count().max(1), Cell::BLANK);
        for (i, c) in text.chars().enumerate() {
            r.cells[i] = Cell::BLANK.with_char(c);
        }
        r
    }

    fn texts(sb: &Scrollback) -> Vec<String> {
        (0..sb.len())
            .map(|i| String::from_utf8_lossy(&sb.text(i).unwrap()).trim_end_matches('\n').to_string())
            .collect()
    }

    #[test]
    fn round_trip_keeps_the_newest_lines_in_whole_blocks() {
        let mut sb = Scrollback::new(1000);
        for i in 0..300 {
            sb.push(&line(&format!("line {i}")), &[Style::DEFAULT]);
            if i % 100 == 99 {
                sb.seal_for_test();
            }
        }
        // Some compressed, some not.
        let mut job = sb.compress_job(1);
        job.run();
        sb.add_compressed(job);
        let screen = [line("on screen")];
        let bytes = sb.save(&screen, &[Style::DEFAULT], &[], 150, 42);
        let mut back = Scrollback::new(1000);
        let r = back.restore(&bytes).unwrap();
        assert_eq!(
            r,
            Restored {
                lines: 150,
                screen: 1,
                saved_at: 42
            }
        );
        let t = texts(&back);
        assert_eq!(t.first().map(String::as_str), Some("line 151"));
        assert_eq!(t[148], "line 299");
        assert_eq!(t[149], "on screen");
        back.check_all().unwrap();
        // A smaller limit hides more.
        let mut small = Scrollback::new(10);
        assert_eq!(small.restore(&bytes).unwrap().lines, 10);
        small.check().unwrap();
        // Only into an empty history.
        assert_eq!(back.restore(&bytes), Err(RestoreError::NotEmpty));
    }

    #[test]
    fn damaged_or_foreign_bytes_restore_nothing() {
        let mut sb = Scrollback::new(100);
        for i in 0..40 {
            sb.push(&line(&format!("l{i}")), &[Style::DEFAULT]);
        }
        let bytes = sb.save(&[], &[], &[], 100, 0);
        for cut in [0, 5, HEADER, bytes.len() / 2, bytes.len() - 1] {
            let mut back = Scrollback::new(100);
            assert!(back.restore(&bytes[..cut]).is_err(), "cut at {cut}");
            assert!(back.is_empty());
        }
        let mut flipped = bytes.clone();
        let at = flipped.len() - 3;
        flipped[at] ^= 0x40;
        assert_eq!(Scrollback::new(100).restore(&flipped), Err(RestoreError::Damaged));
        let mut other = bytes.clone();
        other[9] = 9;
        assert_eq!(Scrollback::new(100).restore(&other), Err(RestoreError::NotHistory));
        assert_eq!(Scrollback::new(100).restore(b"hello"), Err(RestoreError::NotHistory));
        // A body that passes the checksum but whose blocks don't add up.
        let mut forged = bytes[..HEADER].to_vec();
        let mut body = bytes[HEADER..].to_vec();
        let n = body.len();
        body[n - 2] ^= 0xff;
        forged[12..20].copy_from_slice(&(body.len() as u64).to_le_bytes());
        forged[20..28].copy_from_slice(&checksum(&body).to_le_bytes());
        forged.extend_from_slice(&body);
        let mut back = Scrollback::new(100);
        assert_eq!(back.restore(&forged), Err(RestoreError::Damaged));
        assert!(back.is_empty());
    }

    #[test]
    fn empty_history_saves_and_restores_nothing() {
        let bytes = Scrollback::new(100).save(&[], &[], &[], 100, 7);
        let mut back = Scrollback::new(100);
        assert_eq!(
            back.restore(&bytes).unwrap(),
            Restored {
                lines: 0,
                screen: 0,
                saved_at: 7
            }
        );
        back.check().unwrap();
    }
}
