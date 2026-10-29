//! Scrollback stored compactly: each line is its UTF-8 text plus a list of
//! style runs, packed into blocks of about 64 KB. Lines are expanded back into
//! cells only when shown or copied. Whole blocks are freed from the oldest end.
//!
//! Line layout inside a block:
//! `[line flags u8][text len varint][text][run count varint][runs...]`
//! A run is `[tag u8][char count varint][fg u32]?[bg u32]?[style varint]?
//! [underline colour u32]?`, where the tag says which of the optional fields
//! follow (absent = default). A run count of 0 means the whole line has the
//! default style. Styles are stored by value, so history holds no style ids
//! and the screen's style table never has to keep an id alive for it.

use std::collections::VecDeque;

use crate::cell::{Cell, Color, Row, Width};
use crate::style::{Attrs, Style, Underline};

const BLOCK_BYTES: usize = 64 * 1024;
const WRAPPED: u8 = 1;
const HAS_FG: u8 = 1;
const HAS_BG: u8 = 2;
const HAS_STYLE: u8 = 4;
const HAS_UNDERLINE_COLOR: u8 = 8;

struct Block {
    data: Vec<u8>,
    /// Offset of each line in `data`.
    starts: Vec<u32>,
    /// Buffer line number of the first line in this block.
    first: u64,
}

pub struct Scrollback {
    blocks: VecDeque<Block>,
    limit: usize,
    /// Oldest line still visible (older lines may sit in the first block
    /// until the whole block can be freed).
    start: u64,
    /// One past the newest line; also the count of lines ever pushed.
    end: u64,
    /// Reused while encoding, so pushing a line allocates nothing.
    text: Vec<u8>,
    runs: Vec<(Look, u32)>,
}

impl Scrollback {
    pub fn new(limit: usize) -> Scrollback {
        Scrollback {
            blocks: VecDeque::new(),
            limit,
            start: 0,
            end: 0,
            text: Vec::new(),
            runs: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }

    /// Lines ever pushed (buffer line number of the next line).
    pub fn total(&self) -> u64 {
        self.end
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Bytes held, for measuring.
    pub fn memory_bytes(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| b.data.capacity() + b.starts.capacity() * 4)
            .sum::<usize>()
            + self.blocks.capacity() * std::mem::size_of::<Block>()
    }

    pub fn clear(&mut self) {
        self.blocks = VecDeque::new();
        self.start = self.end;
    }

    /// Store a row (trailing blank cells dropped); its cells' style ids
    /// index `styles`.
    pub fn push(&mut self, row: &Row, styles: &[Style]) {
        if self.limit == 0 {
            return;
        }
        let new_block = match self.blocks.back() {
            None => true,
            Some(b) => b.data.len() >= BLOCK_BYTES,
        };
        if new_block {
            if let Some(b) = self.blocks.back_mut() {
                b.data.shrink_to_fit();
                b.starts.shrink_to_fit();
            }
            self.blocks.push_back(Block {
                data: Vec::new(),
                starts: Vec::new(),
                first: self.end,
            });
        }
        let block = self.blocks.back_mut().unwrap();
        block.starts.push(block.data.len() as u32);
        encode(row, styles, &mut block.data, &mut self.text, &mut self.runs);
        self.end += 1;

        if self.len() > self.limit {
            self.start = self.end - self.limit as u64;
            // Free blocks whose lines are all gone.
            while self.blocks.len() > 1 && self.blocks[1].first <= self.start {
                self.blocks.pop_front();
            }
        }
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
        let Some(data) = self.line_data(i) else { return false };
        row.cells.clear();
        decode(data, row, styles);
        true
    }

    /// Whether line `i` soft-wraps onto the next, without decoding it.
    pub fn wrapped(&self, i: usize) -> bool {
        self.line_data(i).is_some_and(|d| d.first().is_some_and(|f| f & WRAPPED != 0))
    }

    fn line_data(&self, i: usize) -> Option<&[u8]> {
        if i >= self.len() {
            return None;
        }
        let line = self.start + i as u64;
        // Few blocks (≈ one per thousand lines): binary search on first line.
        let idx = self.blocks.partition_point(|b| b.first <= line) - 1;
        let block = &self.blocks[idx];
        let n = (line - block.first) as usize;
        Some(&block.data[block.starts[n] as usize..])
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
        v |= ((b & 0x7F) as u32) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    v
}

fn get_u32(data: &[u8], pos: &mut usize) -> u32 {
    let b = data.get(*pos..*pos + 4).map_or([0; 4], |s| [s[0], s[1], s[2], s[3]]);
    *pos += 4;
    u32::from_le_bytes(b)
}

/// The bits of a run's style field. The first eleven are the 12-byte
/// cell's old flags, so lines in the styles those cells could hold encode
/// exactly as before; the rest hold what they couldn't.
mod look {
    pub const BOLD: u32 = 1 << 0;
    pub const DIM: u32 = 1 << 1;
    pub const ITALIC: u32 = 1 << 2;
    pub const UNDERLINE: u32 = 1 << 3;
    pub const STRIKE: u32 = 1 << 4;
    pub const INVERSE: u32 = 1 << 5;
    pub const HIDDEN: u32 = 1 << 6;
    /// The characters are wide: a spacer follows each.
    pub const WIDE: u32 = 1 << 7;
    pub const BLINK: u32 = 1 << 9;
    pub const DOUBLE_UNDERLINE: u32 = 1 << 10;
    pub const OVERLINE: u32 = 1 << 11;
    /// With UNDERLINE: 1 curly, 2 dotted, 3 dashed.
    pub const UNDERLINE_KIND_SHIFT: u32 = 12;
    pub const OWN_IMAGE: u32 = 1 << 14;
}

/// The attributes kept at the same bit in [`Attrs`] and in a run's style
/// field.
const SAME_BITS: u16 = Attrs::BOLD.bits()
    | Attrs::DIM.bits()
    | Attrs::ITALIC.bits()
    | Attrs::STRIKE.bits()
    | Attrs::INVERSE.bits()
    | Attrs::HIDDEN.bits()
    | Attrs::BLINK.bits();

const _: () = {
    assert!(Attrs::BOLD.bits() as u32 == look::BOLD);
    assert!(Attrs::DIM.bits() as u32 == look::DIM);
    assert!(Attrs::ITALIC.bits() as u32 == look::ITALIC);
    assert!(Attrs::STRIKE.bits() as u32 == look::STRIKE);
    assert!(Attrs::INVERSE.bits() as u32 == look::INVERSE);
    assert!(Attrs::HIDDEN.bits() as u32 == look::HIDDEN);
    assert!(Attrs::BLINK.bits() as u32 == look::BLINK);
};

/// What a run records about its cells: the style by value (all of it but
/// the link, which has no table yet, and the image id's high byte, which
/// only placeholders use) and whether the characters are wide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Look {
    fg: Color,
    bg: Color,
    underline_color: Color,
    style: u32,
}

const DEFAULT_LOOK: Look = Look {
    fg: Color::DEFAULT,
    bg: Color::DEFAULT,
    underline_color: Color::DEFAULT,
    style: 0,
};

impl Look {
    fn of(s: &Style) -> Look {
        let a = s.attrs;
        let mut style = (a.bits() & SAME_BITS) as u32;
        if a.contains(Attrs::OVERLINE) {
            style |= look::OVERLINE;
        }
        if a.contains(Attrs::OWN_IMAGE) {
            style |= look::OWN_IMAGE;
        }
        style |= match s.underline {
            Underline::None => 0,
            Underline::Single => look::UNDERLINE,
            Underline::Double => look::DOUBLE_UNDERLINE,
            Underline::Curly => look::UNDERLINE | 1 << look::UNDERLINE_KIND_SHIFT,
            Underline::Dotted => look::UNDERLINE | 2 << look::UNDERLINE_KIND_SHIFT,
            Underline::Dashed => look::UNDERLINE | 3 << look::UNDERLINE_KIND_SHIFT,
        };
        Look {
            fg: s.fg,
            bg: s.bg,
            underline_color: s.underline_color,
            style,
        }
    }

    fn style(&self) -> Style {
        let st = self.style;
        let mut attrs = Attrs::from_bits(st as u16 & SAME_BITS);
        attrs.set(Attrs::OVERLINE, st & look::OVERLINE != 0);
        attrs.set(Attrs::OWN_IMAGE, st & look::OWN_IMAGE != 0);
        let underline = if st & look::DOUBLE_UNDERLINE != 0 {
            Underline::Double
        } else if st & look::UNDERLINE != 0 {
            match (st >> look::UNDERLINE_KIND_SHIFT) & 3 {
                1 => Underline::Curly,
                2 => Underline::Dotted,
                3 => Underline::Dashed,
                _ => Underline::Single,
            }
        } else {
            Underline::None
        };
        Style {
            fg: self.fg,
            bg: self.bg,
            underline_color: self.underline_color,
            attrs,
            underline,
            ..Style::DEFAULT
        }
    }
}

/// How many cells of `cells` to store: up to and including the last one
/// that isn't blank ([`Cell::is_blank`]).
fn stored_len(cells: &[Cell]) -> usize {
    // Most trailing cells are blank (never written, or erased with the
    // default background): skip them 8 at a time (an OR over 64 bytes,
    // which vectorises), then one at a time.
    let blank = |c: &[Cell]| c.iter().fold(0, |a, c| a | c.bits()) & Cell::NOT_BLANK == 0;
    let mut len = cells.len();
    while len >= 8 && blank(&cells[len - 8..len]) {
        len -= 8;
    }
    while len > 0 && cells[len - 1].is_blank() {
        len -= 1;
    }
    len
}

fn encode(row: &Row, styles: &[Style], out: &mut Vec<u8>, body: &mut Vec<u8>, runs: &mut Vec<(Look, u32)>) {
    let cells = &row.cells[..stored_len(&row.cells)];
    out.push(if row.wrapped { WRAPPED } else { 0 });

    // Common case: every cell is narrow ASCII (or empty) in the default
    // style, so the text is a byte copy and there are no runs.
    if cells.iter().fold(0, |a, c| a | c.bits()) & !0x7F == 0 {
        put_varint(out, cells.len() as u32);
        out.reserve(cells.len() + 1);
        out.extend(cells.iter().map(|c| if c.bits() == 0 { b' ' } else { c.bits() as u8 }));
        put_varint(out, 0);
        return;
    }

    // Room for 4 bytes a cell, written in place and cut to length after.
    body.clear();
    body.resize(cells.len() * 4, 0);
    let mut at = 0;
    runs.clear();
    // Neighbouring cells mostly share a style id and width: work out a
    // run's look only when they change. Two ids can still have the same
    // look (it leaves out the link), so a new look may extend the last run.
    const KEY: u64 = 0xFFFF_FFFF << 32 | Cell::WIDTH;
    let mut last_key = u64::MAX;
    for (i, &c) in cells.iter().enumerate() {
        if c.is_spacer() {
            continue;
        }
        let cp = c.codepoint();
        // One test for the usual case: a narrow ASCII character (or none),
        // not a cluster.
        if c.bits() & (Cell::CONTENT & !0x7F | Cell::KIND) == 0 {
            body[at] = if cp == 0 { b' ' } else { cp as u8 };
            at += 1;
        } else if c.is_cluster() {
            // History keeps a cluster's first scalar until v2 (§17.5).
            at += row.first_char(i).encode_utf8(&mut body[at..at + 4]).len();
        } else {
            at += c.ch().encode_utf8(&mut body[at..at + 4]).len();
        }
        let key = c.bits() & KEY;
        if key == last_key {
            // Same style id and width as the cell before: same run.
            if let Some((_, n)) = runs.last_mut() {
                *n += 1;
            }
            continue;
        }
        last_key = key;
        let mut look = styles.get(c.style_id() as usize).map_or(DEFAULT_LOOK, Look::of);
        if c.is_wide() {
            look.style |= look::WIDE;
        }
        match runs.last_mut() {
            Some((l, n)) if *l == look => *n += 1,
            _ => runs.push((look, 1)),
        }
    }
    body.truncate(at);
    put_varint(out, body.len() as u32);
    out.extend_from_slice(body);
    put_runs(out, runs);
}

fn put_runs(out: &mut Vec<u8>, runs: &[(Look, u32)]) {
    if runs.iter().all(|(l, _)| *l == DEFAULT_LOOK) {
        put_varint(out, 0);
        return;
    }
    put_varint(out, runs.len() as u32);
    for &(look, n) in runs.iter() {
        let tag = ((look.fg != Color::DEFAULT) as u8 * HAS_FG)
            | ((look.bg != Color::DEFAULT) as u8 * HAS_BG)
            | ((look.style != 0) as u8 * HAS_STYLE)
            | ((look.underline_color != Color::DEFAULT) as u8 * HAS_UNDERLINE_COLOR);
        out.push(tag);
        put_varint(out, n);
        if tag & HAS_FG != 0 {
            out.extend_from_slice(&look.fg.0.to_le_bytes());
        }
        if tag & HAS_BG != 0 {
            out.extend_from_slice(&look.bg.0.to_le_bytes());
        }
        if tag & HAS_STYLE != 0 {
            put_varint(out, look.style);
        }
        if tag & HAS_UNDERLINE_COLOR != 0 {
            out.extend_from_slice(&look.underline_color.0.to_le_bytes());
        }
    }
}

/// Expand a stored line into `row`, its cells' style ids indexing
/// `styles` (which starts with the default style, id 0).
fn decode(data: &[u8], row: &mut Row, styles: &mut Vec<Style>) {
    let mut pos = 0;
    let line_flags = data.first().copied().unwrap_or(0);
    pos += 1;
    row.wrapped = line_flags & WRAPPED != 0;
    let text_len = get_varint(data, &mut pos) as usize;
    let text = std::str::from_utf8(data.get(pos..pos + text_len).unwrap_or(&[])).unwrap_or("");
    pos += text_len;
    let mut run_count = get_varint(data, &mut pos);

    styles.clear();
    styles.push(Style::DEFAULT);
    let mut template = Cell::BLANK;
    let mut wide = false;
    let mut left = u32::MAX; // chars left in the current run
    if run_count > 0 {
        left = 0;
    }
    row.cells.clear();
    row.cells.reserve(text.len());
    for ch in text.chars() {
        if left == 0 && run_count > 0 {
            let tag = data.get(pos).copied().unwrap_or(0);
            pos += 1;
            left = get_varint(data, &mut pos);
            let mut look = DEFAULT_LOOK;
            if tag & HAS_FG != 0 {
                look.fg = Color(get_u32(data, &mut pos));
            }
            if tag & HAS_BG != 0 {
                look.bg = Color(get_u32(data, &mut pos));
            }
            if tag & HAS_STYLE != 0 {
                look.style = get_varint(data, &mut pos);
            }
            if tag & HAS_UNDERLINE_COLOR != 0 {
                look.underline_color = Color(get_u32(data, &mut pos));
            }
            run_count -= 1;
            wide = look.style & look::WIDE != 0;
            let style = look.style();
            // Runs next to each other differ, but often only in width.
            let id = if style == Style::DEFAULT {
                0
            } else if styles.last() == Some(&style) {
                styles.len() - 1
            } else {
                styles.push(style);
                styles.len() - 1
            };
            template = Cell::template(id as u32, style.shows_empty());
        }
        left = left.saturating_sub(1);
        if wide {
            row.cells.push(template.with_char(ch).with_width(Width::Wide));
            row.cells.push(template.with_width(Width::Spacer));
        } else if ch == ' ' {
            row.cells.push(template);
        } else {
            row.cells.push(template.with_char(ch));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The encoder as it was before its fast paths, kept to check that they
    /// write exactly the same bytes.
    fn encode_reference(row: &Row, styles: &[Style], out: &mut Vec<u8>) {
        let len = row.cells.iter().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
        let cells = &row.cells[..len];
        out.push(if row.wrapped { WRAPPED } else { 0 });

        let mut body = Vec::new();
        let mut runs: Vec<(Look, u32)> = Vec::new();
        let mut utf8 = [0u8; 4];
        for c in cells {
            if c.is_spacer() {
                continue;
            }
            let cp = c.codepoint();
            if cp < 0x80 {
                body.push(if cp == 0 { b' ' } else { cp as u8 });
            } else {
                body.extend_from_slice(c.ch().encode_utf8(&mut utf8).as_bytes());
            }
            let mut look = Look::of(&styles[c.style_id() as usize]);
            if c.is_wide() {
                look.style |= look::WIDE;
            }
            match runs.last_mut() {
                Some((l, n)) if *l == look => *n += 1,
                _ => runs.push((look, 1)),
            }
        }
        put_varint(out, body.len() as u32);
        out.extend_from_slice(&body);
        put_runs(out, &runs);
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn colour(r: &mut Rng) -> Color {
        match r.below(3) {
            0 => Color::DEFAULT,
            1 => Color::indexed(r.below(256) as u8),
            _ => Color::rgb(r.below(256) as u8, 0, r.below(256) as u8),
        }
    }

    /// A table of styles, the default first: foreground only, background,
    /// attributes, every underline, underline colours.
    fn random_styles(r: &mut Rng) -> Vec<Style> {
        let mut styles = vec![Style::DEFAULT];
        for _ in 0..20 {
            let underline = [
                Underline::None,
                Underline::None,
                Underline::Single,
                Underline::Double,
                Underline::Curly,
                Underline::Dotted,
                Underline::Dashed,
            ][r.below(7) as usize];
            styles.push(Style {
                fg: colour(r),
                bg: if r.below(2) == 0 { Color::DEFAULT } else { colour(r) },
                underline_color: if r.below(3) == 0 { colour(r) } else { Color::DEFAULT },
                attrs: Attrs::from_bits(r.next() as u16 & (SAME_BITS | Attrs::OVERLINE.bits())),
                underline,
                ..Style::DEFAULT
            });
        }
        styles
    }

    fn template(styles: &[Style], id: usize) -> Cell {
        Cell::template(id as u32, styles[id].shows_empty())
    }

    /// A random row, mostly the kinds of cells the encoder treats specially:
    /// empty, default-style ASCII, typed spaces, styled cells, blanks that
    /// still show (background, inverse, underline), non-ASCII and wide
    /// characters.
    fn random_row(r: &mut Rng, styles: &[Style]) -> Row {
        let len = r.below(90) as usize;
        let ascii_only = r.below(3) == 0;
        let mut cells = Vec::with_capacity(len + 1);
        while cells.len() < len {
            let ch = (0x20 + r.below(95) as u8) as char;
            let any = template(styles, r.below(styles.len() as u64) as usize);
            let kind = if ascii_only { r.below(3) } else { r.below(10) };
            let cell = match kind {
                0 => Cell::BLANK,
                1 => Cell::BLANK.with_char(ch),
                2 => Cell::BLANK.with_char(' '),
                3 => any.with_char(ch),
                4 => any,
                5 => any.with_char(['é', 'Ω', '─', '\u{10FFFD}'][r.below(4) as usize]),
                6 => Cell::BLANK.with_char('\u{7F}'),
                _ => {
                    cells.push(any.with_char('漢').with_width(Width::Wide));
                    any.with_width(Width::Spacer)
                }
            };
            cells.push(cell);
        }
        // Trailing blanks of every kind.
        for _ in 0..r.below(40) {
            let id = r.below(styles.len() as u64) as usize;
            cells.push(match r.below(3) {
                0 | 1 => Cell::BLANK,
                _ if !styles[id].shows_empty() => template(styles, id),
                _ => Cell::BLANK,
            });
        }
        let mut row = Row::new(0, Cell::BLANK);
        row.cells = cells;
        row.wrapped = r.below(2) == 0;
        row
    }

    #[test]
    fn encoder_writes_the_same_bytes_as_the_reference() {
        let mut r = Rng(0x5EED_1234_ABCD_0001);
        let (mut body, mut runs) = (Vec::new(), Vec::new());
        for i in 0..200_000 {
            if i % 1000 == 0 {
                r.next();
            }
            let styles = random_styles(&mut Rng(1 + i as u64 / 1000));
            let row = random_row(&mut r, &styles);
            let (mut fast, mut slow) = (vec![0xEE], vec![0xEE]);
            encode(&row, &styles, &mut fast, &mut body, &mut runs);
            encode_reference(&row, &styles, &mut slow);
            assert_eq!(fast, slow, "row {i}: {row:?}");
        }
    }

    #[test]
    fn styles_round_trip_by_value() {
        let mut r = Rng(7);
        for _ in 0..2000 {
            let styles = random_styles(&mut r);
            let row = random_row(&mut r, &styles);
            let mut sb = Scrollback::new(10);
            sb.push(&row, &styles);
            let (back, back_styles) = sb.get(0).unwrap();
            assert_eq!(back.wrapped, row.wrapped);
            let mut len = stored_len(&row.cells);
            // A wide character's spacer may be blank: it comes back too.
            if len > 0 && row.cells[len - 1].is_wide() {
                len += 1;
            }
            assert_eq!(back.cells.len(), len);
            for (a, b) in row.cells[..len].iter().zip(&back.cells) {
                assert_eq!(styles[a.style_id() as usize], back_styles[b.style_id() as usize]);
                assert_eq!(a.shows_empty(), b.shows_empty());
                assert_eq!(a.width(), b.width());
                let cp = if a.codepoint() == ' ' as u32 && !a.is_wide() {
                    0
                } else {
                    a.codepoint()
                };
                assert_eq!(cp, b.codepoint());
            }
        }
    }

    /// Lines in the styles the 12-byte cells could hold encode as they did
    /// then: these are the bytes the old encoder wrote.
    #[test]
    fn old_styles_keep_their_bytes() {
        let styles = vec![
            Style::DEFAULT,
            Style {
                fg: Color::indexed(1),
                attrs: Attrs::BOLD,
                ..Style::DEFAULT
            },
            Style {
                bg: Color::indexed(2),
                underline: Underline::Single,
                ..Style::DEFAULT
            },
        ];
        let mut row = Row::new(6, Cell::BLANK);
        row.cells[0] = template(&styles, 1).with_char('a');
        row.cells[1] = template(&styles, 2).with_char('漢').with_width(Width::Wide);
        row.cells[2] = template(&styles, 2).with_width(Width::Spacer);
        let mut out = Vec::new();
        encode(&row, &styles, &mut out, &mut Vec::new(), &mut Vec::new());
        #[rustfmt::skip]
        let old = [
            0, 4, b'a', 0xE6, 0xBC, 0xA2, 2,
            HAS_FG | HAS_STYLE, 1, 1, 0, 0, 1, 1,
            HAS_BG | HAS_STYLE, 1, 2, 0, 0, 1, 0x88, 1,
        ];
        assert_eq!(out, old);
    }

    fn row_of(cells: Vec<Cell>, wrapped: bool) -> Row {
        let mut row = Row::new(0, Cell::BLANK);
        row.cells = cells;
        row.wrapped = wrapped;
        row
    }

    #[test]
    fn plain_lines_are_small() {
        let mut sb = Scrollback::new(100_000);
        let mut row = Row::new(200, Cell::BLANK);
        for (i, c) in "the quick brown fox jumps over the lazy dog".chars().enumerate() {
            row.cells[i] = Cell::BLANK.with_char(c);
        }
        for _ in 0..10_000 {
            sb.push(&row, &[Style::DEFAULT]);
        }
        let per_line = sb.memory_bytes() / 10_000;
        assert!(per_line < 60, "{per_line} bytes per line");
        assert_eq!(sb.get(9_999).unwrap().0.cells.len(), 43);
    }

    #[test]
    fn varint_edge_values() {
        let cases: &[(u32, usize)] = &[
            (0, 1),
            (1, 1),
            (127, 1),
            (128, 2),
            (16_383, 2),
            (16_384, 3),
            ((1 << 21) - 1, 3),
            (1 << 21, 4),
            ((1 << 28) - 1, 4),
            (1 << 28, 5),
            (u32::MAX, 5),
        ];
        for &(v, len) in cases {
            let mut out = vec![0xAA];
            put_varint(&mut out, v);
            out.push(0x55);
            assert_eq!(out.len(), len + 2, "{v} takes {len} bytes");
            let mut pos = 1;
            assert_eq!(get_varint(&out, &mut pos), v);
            assert_eq!(pos, len + 1, "{v} read to its end");
        }
        // All values back to back.
        let mut out = Vec::new();
        for &(v, _) in cases {
            put_varint(&mut out, v);
        }
        let mut pos = 0;
        for &(v, _) in cases {
            assert_eq!(get_varint(&out, &mut pos), v);
        }
        assert_eq!(pos, out.len());
    }

    #[test]
    fn varint_truncated_input_stops_at_the_end() {
        let mut pos = 0;
        assert_eq!(get_varint(&[0x80, 0x81], &mut pos), 0x80);
        assert_eq!(pos, 2);
        let mut pos = 5;
        assert_eq!(get_varint(&[1], &mut pos), 0);
    }

    #[test]
    fn freeing_keeps_at_most_one_spare_block_and_clear_frees_all() {
        let mut sb = Scrollback::new(500);
        let row = row_of(vec![Cell::BLANK.with_char('x'); 200], false);
        for _ in 0..20_000 {
            sb.push(&row, &[Style::DEFAULT]);
            // 500 lines of ~205 bytes span two blocks; a third may be filling.
            assert!(sb.blocks.len() <= 3, "{} blocks", sb.blocks.len());
        }
        assert_eq!(sb.len(), 500);
        sb.clear();
        assert!(sb.is_empty());
        assert_eq!(sb.memory_bytes(), 0);
        assert!(sb.get(0).is_none());
        sb.push(&row, &[Style::DEFAULT]);
        assert_eq!(sb.len(), 1);
        assert_eq!(sb.get(0).unwrap().0.cells.len(), 200);
    }

    #[test]
    fn limit_and_block_freeing() {
        let mut sb = Scrollback::new(1000);
        let mut row = Row::new(80, Cell::BLANK);
        for i in 0..100_000u32 {
            let s = i.to_string();
            for (j, c) in s.chars().enumerate() {
                row.cells[j] = Cell::BLANK.with_char(c);
            }
            sb.push(&row, &[Style::DEFAULT]);
        }
        assert_eq!(sb.len(), 1000);
        let first: String = sb.get(0).unwrap().0.cells.iter().map(|c| c.ch()).collect();
        assert_eq!(first, "99000");
        assert!(sb.blocks.len() <= 2, "old blocks freed");
    }
}
