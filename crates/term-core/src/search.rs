//! The search engine (§13.3–§13.8 of the cells design): the query and how
//! it matches, the job that searches history off the terminal lock, and
//! the hits.
//!
//! What is searched is the text history stores (§11.3), and the same text
//! built from the screen's cells: a line's characters and clusters, a
//! space for an empty cell, nothing for spacers, and 0x0A after each
//! logical line. Wrapped lines run on with no separator, so a match can
//! cross a soft wrap, and the blocks' texts back to back are all of
//! history.
//!
//! Every place a match starts is a hit, overlapping ones included
//! ("aa" is found twice in "aaa"): hits are then the same whichever order
//! blocks are searched in, and a hit is known by where it starts.
//!
//! The terminal's side (start the search, add the worker's hits, keep up
//! with output, find next and previous, highlight) is in `term/search.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use memchr::memmem;

use crate::cell::Row;
use crate::scrollback::{BlockData, SEARCH_SLOW_FLAGS, text_end};
use crate::term::Point;
use crate::unicode::{self, Gcb};

/// Most hits a search keeps (§19).
pub const HIT_CAP: usize = 100_000;

/// Which matches count, by what is either side of them (§13.8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WordMatch {
    /// Anywhere.
    #[default]
    Contains,
    /// At the start of a word: no word character just before it.
    StartsWith,
    /// A whole word: no word character either side.
    FullWord,
    /// At the end of a word (from the find pasteboard only).
    EndsWith,
}

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub text: String,
    pub ignore_case: bool,
    pub word: WordMatch,
}

/// Search's word rule (§13.8): letters, digits, combining marks and `_`.
/// Not double-click's rule, which keeps paths and URLs whole.
pub fn is_word_char(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    c.is_alphanumeric() || matches!(unicode::props(c).gcb(), Gcb::Extend | Gcb::SpacingMark)
}

/// Simple case folding (§13.5): `a` and `b` match ignoring case when
/// `fold(a) == fold(b)`. Built from the standard library's case tables:
/// a character's fold is the lower case of its upper case where both are
/// single characters, so σ, ς and Σ fold together, and so do θ, ϑ and ϴ,
/// and ǆ, ǅ and Ǆ. Left out, so ASCII text stays on the byte path: the
/// KELVIN SIGN, LONG S and Turkish dotted and dotless i (no non-ASCII
/// character folds to ASCII). Not covered: full folding (`ß` is not
/// `ss`) and canonical equivalence.
pub fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    if matches!(c, '\u{212A}' | '\u{017F}' | '\u{0130}' | '\u{0131}') {
        return c;
    }
    let up = single(c.to_uppercase()).unwrap_or(c);
    let f = single(up.to_lowercase()).or_else(|| single(c.to_lowercase())).unwrap_or(c);
    if f.is_ascii() { c } else { f }
}

fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let c = it.next()?;
    it.next().is_none().then_some(c)
}

/// Whether `c` has another case (so a query of none needs no folding).
fn cased(c: char) -> bool {
    fold(c) != c || c.is_uppercase() || c.is_lowercase()
}

/// The scalar at `i` and its length (U+FFFD for a bad byte).
#[inline]
pub(crate) fn char_at(text: &[u8], i: usize) -> (char, usize) {
    let b = text[i];
    if b < 0x80 {
        return (b as char, 1);
    }
    let n = match b {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => return ('\u{FFFD}', 1),
    };
    match text.get(i..i + n).and_then(|s| std::str::from_utf8(s).ok()) {
        Some(s) => (s.chars().next().unwrap_or('\u{FFFD}'), n),
        None => ('\u{FFFD}', 1),
    }
}

/// The scalar ending at `i` (the one just before it).
fn char_before(text: &[u8], i: usize) -> Option<char> {
    if i == 0 {
        return None;
    }
    let mut j = i - 1;
    while j > 0 && i - j < 4 && text[j] & 0xC0 == 0x80 {
        j -= 1;
    }
    Some(char_at(text, j).0)
}

/// Move `i` forward to a scalar's start.
fn align(text: &[u8], mut i: usize) -> usize {
    while i < text.len() && text[i] & 0xC0 == 0x80 {
        i += 1;
    }
    i
}

enum Kind {
    /// Byte for byte (case-sensitive, or a query with no cased letters).
    Exact(memmem::Finder<'static>),
    /// An ASCII query ignoring case: found in the text lower-cased (ASCII
    /// only) into a scratch buffer.
    Ascii(memmem::Finder<'static>),
    /// A query with non-ASCII cased letters ignoring case: scalar by
    /// scalar, comparing folds. Its first scalar's fold, if ASCII, is
    /// jumped to with memchr.
    Fold(Vec<char>),
}

/// A compiled query (§13.5).
pub struct Matcher {
    kind: Kind,
    word: WordMatch,
    /// Scalars in the query.
    chars: usize,
}

impl Matcher {
    /// None for an empty query.
    pub fn new(q: &Query) -> Option<Matcher> {
        if q.text.is_empty() {
            return None;
        }
        let chars = q.text.chars().count();
        let kind = if !q.ignore_case || !q.text.chars().any(cased) {
            Kind::Exact(memmem::Finder::new(q.text.as_bytes()).into_owned())
        } else if q.text.is_ascii() {
            Kind::Ascii(memmem::Finder::new(q.text.to_ascii_lowercase().as_bytes()).into_owned())
        } else {
            Kind::Fold(q.text.chars().map(fold).collect())
        };
        Some(Matcher { kind, word: q.word, chars })
    }

    /// Bytes either side of a boundary that a match crossing it, and the
    /// scalar next to it, can take.
    pub(crate) fn reach(&self) -> usize {
        4 * self.chars + 4
    }

    pub(crate) fn word(&self) -> WordMatch {
        self.word
    }

    fn word_ok(&self, before: Option<char>, after: Option<char>) -> bool {
        let edge = |c: Option<char>| !c.is_some_and(is_word_char);
        match self.word {
            WordMatch::Contains => true,
            WordMatch::StartsWith => edge(before),
            WordMatch::EndsWith => edge(after),
            WordMatch::FullWord => edge(before) && edge(after),
        }
    }

    /// Every match in `hay` as (start, end) byte offsets, overlapping ones
    /// included. `before` and `after` are the scalars just outside it (for
    /// the word checks); `scratch` is reused between calls.
    pub fn find(&self, hay: &[u8], before: Option<char>, after: Option<char>, scratch: &mut Vec<u8>, out: &mut Vec<(usize, usize)>) {
        let mut push = |s: usize, e: usize| {
            if self.word == WordMatch::Contains
                || self.word_ok(
                    if s == 0 { before } else { char_before(hay, s) },
                    if e == hay.len() { after } else { Some(char_at(hay, e).0) },
                )
            {
                out.push((s, e));
            }
        };
        match &self.kind {
            Kind::Exact(f) => find_all(f, hay, &mut push),
            Kind::Ascii(f) => {
                scratch.clear();
                scratch.extend(hay.iter().map(u8::to_ascii_lowercase));
                find_all(f, scratch, &mut push);
            }
            Kind::Fold(q) => {
                let first = q[0];
                let mut i = 0;
                while i < hay.len() {
                    if first.is_ascii() {
                        let (lo, up) = (first as u8, (first as u8).to_ascii_uppercase());
                        match memchr::memchr2(lo, up, &hay[i..]) {
                            Some(j) => i += j,
                            None => break,
                        }
                    }
                    if let Some(e) = fold_match(q, hay, i) {
                        push(i, e);
                    }
                    i = align(hay, i + 1);
                }
            }
        }
    }
}

fn find_all(f: &memmem::Finder<'_>, hay: &[u8], push: &mut impl FnMut(usize, usize)) {
    let n = f.needle().len();
    let mut at = 0;
    while let Some(i) = f.find(&hay[at..]) {
        push(at + i, at + i + n);
        at += i + 1;
    }
}

/// Where a match of the folded query `q` starting at `i` ends, if one does.
fn fold_match(q: &[char], hay: &[u8], mut i: usize) -> Option<usize> {
    for &c in q {
        if i >= hay.len() {
            return None;
        }
        let (t, n) = char_at(hay, i);
        if fold(t) != c {
            return None;
        }
        i += n;
    }
    Some(i)
}

/// One hit (16 bytes): where it starts, and where it ends (exclusive), on
/// the start line or `end_lines` stored lines after it (§13.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hit {
    pub line: u64,
    pub col: u16,
    pub end_lines: u16,
    pub end_col: u16,
}

impl Hit {
    pub(crate) fn from_parts(start: (u64, u16), end: (u64, u16)) -> Hit {
        Hit {
            line: start.0,
            col: start.1,
            end_lines: (end.0 - start.0).min(u16::MAX as u64) as u16,
            end_col: end.1,
        }
    }

    pub fn start(&self) -> Point {
        Point {
            line: self.line,
            col: self.col as usize,
        }
    }

    /// The last line it is on.
    pub fn end_line(&self) -> u64 {
        self.line + self.end_lines as u64
    }

    /// Its last cell (for a selection, which includes both ends).
    pub fn last(&self) -> Point {
        Point {
            line: self.end_line(),
            col: (self.end_col as usize).saturating_sub(1),
        }
    }

    /// Its columns on `line` (`cols` wide), if it is on it.
    pub fn cols_on(&self, line: u64, cols: usize) -> Option<std::ops::Range<usize>> {
        if line < self.line || line > self.end_line() {
            return None;
        }
        let a = if line == self.line { self.col as usize } else { 0 };
        let b = if line == self.end_line() { self.end_col as usize } else { cols };
        let b = b.min(cols);
        (a < b).then_some(a..b)
    }
}

/// Text built from rows the way history stores it, with where each
/// cell's text starts: the screen's text, a line from a block, or (in
/// tests) everything.
#[derive(Default)]
pub(crate) struct Cells {
    pub text: Vec<u8>,
    /// Per cell with text, and per line end: its offset in `text`, line,
    /// column and width. In text order, tiling `text`.
    map: Vec<(u32, u64, u16, u8)>,
}

impl Cells {
    pub fn clear(&mut self) {
        self.text.clear();
        self.map.clear();
    }

    /// Add `row`, stable line `line`.
    pub fn push_row(&mut self, row: &Row, line: u64) {
        let cells = &row.cells[..];
        let n = if row.wrapped { cells.len() } else { text_end(cells) };
        let mut buf = [0; 4];
        for (col, &c) in cells[..n].iter().enumerate() {
            if c.is_spacer() {
                continue;
            }
            let off = self.text.len() as u32;
            if c.is_cluster() {
                self.text
                    .extend_from_slice(row.clusters().map_or("", |x| x.get(c.codepoint())).as_bytes());
            } else if c.codepoint() == 0 {
                self.text.push(b' ');
            } else {
                self.text.extend_from_slice(c.ch().encode_utf8(&mut buf).as_bytes());
            }
            self.map.push((off, line, col as u16, if c.is_wide() { 2 } else { 1 }));
        }
        if !row.wrapped {
            self.map.push((self.text.len() as u32, line, n as u16, 1));
            self.text.push(b'\n');
        }
    }

    /// The (line, column) where a match starting at `off` starts, if `off`
    /// is where a cell's text starts (so not inside a cluster).
    pub fn start_at(&self, off: usize) -> Option<(u64, u16)> {
        let i = self.map.binary_search_by_key(&(off as u32), |m| m.0).ok()?;
        Some((self.map[i].1, self.map[i].2))
    }

    /// The (line, column after it) where a match ending at `off`
    /// (exclusive) ends, if `off` is where a cell's text ends.
    pub fn end_at(&self, off: usize) -> Option<(u64, u16)> {
        if off == 0 || off > self.text.len() {
            return None;
        }
        if off < self.text.len() && self.map.binary_search_by_key(&(off as u32), |m| m.0).is_err() {
            return None;
        }
        let i = self.map.partition_point(|m| m.0 < off as u32) - 1;
        let m = self.map[i];
        Some((m.1, m.2 + m.3 as u16))
    }
}

/// A block as a search sees it: its shared data and the lines in use when
/// the search started (§11.2), and the text window to search (lines
/// hidden by the limit skipped), worked out off the lock.
pub(crate) struct Seg {
    pub data: Arc<BlockData>,
    pub len: usize,
    /// Lines at the start hidden by the limit.
    pub hidden: usize,
    starts: Option<Vec<u32>>,
    window: Option<(usize, usize)>,
}

impl Seg {
    pub fn new(data: Arc<BlockData>, len: usize, first_visible: u64) -> Seg {
        let hidden = first_visible.saturating_sub(data.first()).min(len as u64) as usize;
        Seg {
            data,
            len,
            hidden,
            starts: None,
            window: None,
        }
    }

    fn starts(&mut self) -> &[u32] {
        let (data, len) = (&self.data, self.len);
        self.starts.get_or_insert_with(|| data.line_starts(len))
    }

    /// The text searched: from the first visible line to the end of the
    /// lines in use.
    pub fn window(&mut self) -> (usize, usize) {
        if let Some(w) = self.window {
            return w;
        }
        let w = if self.hidden == 0 && self.len == self.data.lines() {
            (0, self.data.text().len())
        } else {
            let (h, len) = (self.hidden, self.len);
            let s = self.starts();
            (s[h] as usize, s[len] as usize)
        };
        self.window = Some(w);
        w
    }

    /// The stored line holding byte `off` (by index), and where its text
    /// starts.
    fn line_of(&mut self, off: usize) -> (usize, usize) {
        let s = self.starts();
        let n = s.partition_point(|&x| x as usize <= off).saturating_sub(1).min(s.len() - 2);
        (n, s[n] as usize)
    }

    /// The offset of line `n`'s text (`n` up to `len`).
    pub fn line_start(&mut self, n: usize) -> usize {
        self.starts()[n] as usize
    }
}

/// A place in history's text: a segment and an offset in its text.
type Pos = (usize, usize);

/// Turns byte offsets in blocks into stable (line, column) places,
/// decoding only the lines hits are on (and not even those for ASCII
/// lines without clusters).
#[derive(Default)]
pub(crate) struct Mapper {
    /// The line last decoded: (segment, line index), its text start, its cells.
    cached: Option<(usize, usize, usize)>,
    cells: Cells,
    row: Row,
}

impl Mapper {
    fn line(&mut self, segs: &mut [Seg], seg: usize, off: usize) -> (usize, usize, bool) {
        let (n, start) = segs[seg].line_of(off);
        let s = &segs[seg];
        let end = segs[seg].starts.as_ref().map_or(0, |v| v[n + 1] as usize);
        let s_text = &s.data.text()[start..end];
        let fast = s.data.line_flags(n) & SEARCH_SLOW_FLAGS == 0 && s_text.is_ascii();
        if !fast && self.cached != Some((seg, n, start)) {
            self.cells.clear();
            s.data.decode_line(n, start, &mut self.row);
            self.cells.push_row(&self.row, s.data.first() + n as u64);
            debug_assert_eq!(self.cells.text, s_text, "search text of line {n}");
            self.cached = Some((seg, n, start));
        }
        (n, start, fast)
    }

    fn start(&mut self, segs: &mut [Seg], (seg, off): Pos) -> Option<(u64, u16)> {
        let (n, start, fast) = self.line(segs, seg, off);
        if fast {
            return Some((segs[seg].data.first() + n as u64, (off - start) as u16));
        }
        self.cells.start_at(off - start)
    }

    fn end(&mut self, segs: &mut [Seg], (seg, off): Pos) -> Option<(u64, u16)> {
        let (n, start, fast) = self.line(segs, seg, off - 1);
        if fast {
            return Some((segs[seg].data.first() + n as u64, (off - start) as u16));
        }
        self.cells.end_at(off - start)
    }

    /// The hit for a match from `a` to `b`, if it starts and ends on cluster
    /// boundaries.
    pub fn hit(&mut self, segs: &mut [Seg], a: Pos, b: Pos) -> Option<Hit> {
        let s = self.start(segs, a)?;
        let e = self.end(segs, b)?;
        Some(Hit::from_parts(s, e))
    }
}

/// The last scalar of the windows before segment `i`.
fn last_char_before(segs: &mut [Seg], i: usize) -> Option<char> {
    for j in (0..i).rev() {
        let (a, b) = segs[j].window();
        if b > a {
            return char_before(&segs[j].data.text()[a..b], b - a);
        }
    }
    None
}

/// The first scalar of the first non-empty window in `segs`, else `after`.
fn first_char_from(segs: &mut [Seg], after: Option<char>) -> Option<char> {
    for g in segs {
        let (a, b) = g.window();
        if b > a {
            return Some(char_at(g.data.text(), a).0);
        }
    }
    after
}

/// Find matches starting in segment `i` at or after `from` (an offset in
/// its text), ending in it or in the segments after it: `out` gets their
/// (start, end) places. `after` is the scalar after the last segment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_seg(
    m: &Matcher,
    segs: &mut [Seg],
    i: usize,
    from: Option<usize>,
    boundary_only: bool,
    after: Option<char>,
    scratch: &mut Vec<u8>,
    raw: &mut Vec<(usize, usize)>,
    out: &mut Vec<(Pos, Pos)>,
) {
    let (wa, wb) = segs[i].window();
    let from = from.map_or(wa, |f| align(segs[i].data.text(), f.clamp(wa, wb)));
    if from >= wb && !boundary_only {
        return;
    }
    let before = if from > wa {
        char_before(segs[i].data.text(), from)
    } else {
        last_char_before(segs, i)
    };
    // The scalar after this window: the next window's first.
    let next_first = first_char_from(&mut segs[i + 1..], after);
    if !boundary_only {
        raw.clear();
        m.find(&segs[i].data.text()[from..wb], before, next_first, scratch, raw);
        out.extend(raw.iter().map(|&(s, e)| ((i, from + s), (i, from + e))));
    }
    let from = if boundary_only { wa } else { from };

    // The boundary with what follows (§13.5): the end of this window and
    // the start of the next ones, searched together; only matches that
    // cross it are kept (the others are found in their own windows).
    let reach = m.reach();
    let tail_at = align(segs[i].data.text(), wb.saturating_sub(reach).max(from));
    let mut buf: Vec<u8> = segs[i].data.text()[tail_at..wb].to_vec();
    let tail = buf.len();
    // (segment, its window start, where its piece starts in buf)
    let mut pieces = Vec::new();
    let mut tail_after = None;
    let mut j = i + 1;
    while j < segs.len() && buf.len() - tail < reach {
        let (a, b) = segs[j].window();
        let want = reach - (buf.len() - tail);
        let text = segs[j].data.text();
        let end = align(text, (a + want).min(b));
        pieces.push((j, a, buf.len()));
        buf.extend_from_slice(&text[a..end]);
        if end < b {
            tail_after = Some(Some(char_at(text, end).0));
            break;
        }
        j += 1;
    }
    if pieces.is_empty() {
        return;
    }
    let tail_after = match tail_after {
        Some(c) => c,
        None => {
            let j = j.min(segs.len());
            first_char_from(&mut segs[j..], after)
        }
    };
    let before = if tail_at > wa {
        char_before(segs[i].data.text(), tail_at)
    } else {
        last_char_before(segs, i)
    };
    raw.clear();
    m.find(&buf, before, tail_after, scratch, raw);
    for &(s, e) in raw.iter() {
        if s < tail && e > tail && tail_at + s >= from {
            let p = pieces.partition_point(|p| p.2 < e) - 1;
            let (seg, a, at) = pieces[p];
            out.push(((i, tail_at + s), (seg, a + (e - at))));
        }
    }
}

/// Hits the worker found in one block, for [`crate::Terminal::add_search_hits`].
#[derive(Debug, Default)]
pub struct HitBatch {
    pub(crate) generation: u64,
    /// The block's first line: hits are kept per block.
    pub(crate) block: u64,
    pub(crate) hits: Vec<Hit>,
    /// The last batch: history has been searched through.
    pub(crate) done: bool,
}

impl HitBatch {
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// History's part of a search (§13.4): taken under the lock by
/// [`crate::Terminal::start_search`], run on the search worker without
/// it. Each [`SearchJob::next_batch`] searches one block, nearest the view
/// first, then alternately older and newer; the last batch says it is done.
pub struct SearchJob {
    matcher: Option<Arc<Matcher>>,
    segs: Vec<Seg>,
    order: Vec<usize>,
    next: usize,
    after: Option<char>,
    generation: u64,
    cancel: Arc<AtomicU64>,
    finished: bool,
    mapper: Mapper,
    scratch: Vec<u8>,
    raw: Vec<(usize, usize)>,
    found: Vec<(Pos, Pos)>,
}

impl SearchJob {
    pub(crate) fn new(
        matcher: Option<Arc<Matcher>>,
        segs: Vec<Seg>,
        start: usize,
        after: Option<char>,
        generation: u64,
        cancel: Arc<AtomicU64>,
    ) -> SearchJob {
        let n = segs.len();
        let mut order = Vec::with_capacity(n);
        if n > 0 {
            let start = start.min(n - 1);
            order.push(start);
            for d in 1..n {
                if d <= start {
                    order.push(start - d);
                }
                if start + d < n {
                    order.push(start + d);
                }
            }
        }
        SearchJob {
            matcher,
            segs,
            order,
            next: 0,
            after,
            generation,
            cancel,
            finished: false,
            mapper: Mapper::default(),
            scratch: Vec::new(),
            raw: Vec::new(),
            found: Vec::new(),
        }
    }

    /// Whether a newer search has replaced this one (it should stop).
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) != self.generation
    }

    /// Blocks still to search.
    pub fn remaining(&self) -> usize {
        self.order.len() - self.next
    }

    /// Search the next block: its hits, or, once all are searched, a last
    /// empty batch marked done; then None. None at once when cancelled.
    pub fn next_batch(&mut self) -> Option<HitBatch> {
        if self.finished || self.cancelled() {
            return None;
        }
        let Some(m) = self.matcher.clone() else {
            self.finished = true;
            return Some(self.batch(0, Vec::new(), true));
        };
        let Some(&i) = self.order.get(self.next) else {
            self.finished = true;
            return Some(self.batch(0, Vec::new(), true));
        };
        self.next += 1;
        self.found.clear();
        scan_seg(
            &m,
            &mut self.segs,
            i,
            None,
            false,
            self.after,
            &mut self.scratch,
            &mut self.raw,
            &mut self.found,
        );
        let mut hits = Vec::with_capacity(self.found.len());
        for &(a, b) in &self.found {
            if let Some(h) = self.mapper.hit(&mut self.segs, a, b) {
                hits.push(h);
            }
        }
        hits.sort_unstable();
        let block = self.segs[i].data.first();
        Some(self.batch(block, hits, false))
    }

    fn batch(&self, block: u64, hits: Vec<Hit>, done: bool) -> HitBatch {
        HitBatch {
            generation: self.generation,
            block,
            hits,
            done,
        }
    }

    /// Run to the end (or until cancelled), handing each batch to `f`.
    pub fn run(&mut self, mut f: impl FnMut(HitBatch)) {
        while let Some(b) = self.next_batch() {
            f(b);
        }
    }
}

#[cfg(test)]
mod tests;
