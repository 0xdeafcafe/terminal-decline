//! Scrollback stored compactly: each line is its UTF-8 text plus a list of
//! style runs, packed into blocks of about 64 KB. Lines are expanded back into
//! cells only when shown or copied. Whole blocks are freed from the oldest end.
//!
//! Line layout inside a block:
//! `[line flags u8][text len varint][text][run count varint][runs...]`
//! A run is `[tag u8][char count varint][fg u32]?[bg u32]?[style varint]?`,
//! where the tag says which of the optional fields follow (absent = default).
//! A run count of 0 means the whole line has the default style.

use std::collections::VecDeque;

use crate::cell::{Cell, Color, Row, flags};

const BLOCK_BYTES: usize = 64 * 1024;
const WRAPPED: u8 = 1;
const HAS_FG: u8 = 1;
const HAS_BG: u8 = 2;
const HAS_STYLE: u8 = 4;

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

    /// Store a row (trailing blank cells dropped).
    pub fn push(&mut self, row: &Row) {
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
        encode(row, &mut block.data, &mut self.text, &mut self.runs);
        self.end += 1;

        if self.len() > self.limit {
            self.start = self.end - self.limit as u64;
            // Free blocks whose lines are all gone.
            while self.blocks.len() > 1 && self.blocks[1].first <= self.start {
                self.blocks.pop_front();
            }
        }
    }

    /// Line `i` counting from the oldest visible line (0..len).
    pub fn get(&self, i: usize) -> Option<Row> {
        if i >= self.len() {
            return None;
        }
        let line = self.start + i as u64;
        // Few blocks (≈ one per thousand lines): binary search on first line.
        let idx = self.blocks.partition_point(|b| b.first <= line) - 1;
        let block = &self.blocks[idx];
        let n = (line - block.first) as usize;
        let from = block.starts[n] as usize;
        let mut row = Row::default();
        decode(&block.data[from..], &mut row);
        Some(row)
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

/// The parts of a cell's look that runs record (style flags include WIDE so
/// spacers can be rebuilt).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Look {
    fg: Color,
    bg: Color,
    style: u32,
}

const DEFAULT_LOOK: Look = Look {
    fg: Color::DEFAULT,
    bg: Color::DEFAULT,
    style: 0,
};

/// How many cells of `cells` to store: up to and including the last one
/// that isn't blank ([`Cell::is_blank`]).
fn stored_len(cells: &[Cell]) -> usize {
    // Most trailing cells are all zero (never written, or erased with the
    // default background): skip those with a cheap check first, 8 cells at
    // a time (an OR over 96 bytes, which vectorises), then one at a time.
    let zero = |c: &[Cell]| c.iter().fold(0, |a, c| a | c.packed | c.fg.0 | c.bg.0) == 0;
    let mut len = cells.len();
    while len >= 8 && zero(&cells[len - 8..len]) {
        len -= 8;
    }
    while len > 0 && zero(&cells[len - 1..len]) {
        len -= 1;
    }
    cells[..len].iter().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1)
}

fn encode(row: &Row, out: &mut Vec<u8>, body: &mut Vec<u8>, runs: &mut Vec<(Look, u32)>) {
    let cells = &row.cells[..stored_len(&row.cells)];
    out.push(if row.wrapped { WRAPPED } else { 0 });

    // Common case: every cell is ASCII in the default style (no flags, no
    // colours), so the text is a byte copy and there are no runs.
    if cells.iter().fold(0, |a, c| a | (c.packed & !0x7F) | c.fg.0 | c.bg.0) == 0 {
        put_varint(out, cells.len() as u32);
        out.reserve(cells.len() + 1);
        out.extend(cells.iter().map(|c| if c.packed == 0 { b' ' } else { c.packed as u8 }));
        put_varint(out, 0);
        return;
    }

    body.clear();
    runs.clear();
    let mut utf8 = [0u8; 4];
    for c in cells {
        if c.has(flags::WIDE_SPACER) {
            continue;
        }
        let cp = c.codepoint();
        if cp < 0x80 {
            body.push(if cp == 0 { b' ' } else { cp as u8 });
        } else {
            body.extend_from_slice(c.ch().encode_utf8(&mut utf8).as_bytes());
        }
        let look = Look {
            fg: c.fg,
            bg: c.bg,
            style: c.flags() >> 21,
        };
        match runs.last_mut() {
            Some((l, n)) if *l == look => *n += 1,
            _ => runs.push((look, 1)),
        }
    }
    put_varint(out, body.len() as u32);
    out.extend_from_slice(body);

    if runs.iter().all(|(l, _)| *l == DEFAULT_LOOK) {
        put_varint(out, 0);
        return;
    }
    put_varint(out, runs.len() as u32);
    for &(look, n) in runs.iter() {
        let tag = ((look.fg != Color::DEFAULT) as u8 * HAS_FG)
            | ((look.bg != Color::DEFAULT) as u8 * HAS_BG)
            | ((look.style != 0) as u8 * HAS_STYLE);
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
    }
}

fn decode(data: &[u8], row: &mut Row) {
    let mut pos = 0;
    let line_flags = data.first().copied().unwrap_or(0);
    pos += 1;
    row.wrapped = line_flags & WRAPPED != 0;
    let text_len = get_varint(data, &mut pos) as usize;
    let text = std::str::from_utf8(data.get(pos..pos + text_len).unwrap_or(&[])).unwrap_or("");
    pos += text_len;
    let mut run_count = get_varint(data, &mut pos);

    let mut look = DEFAULT_LOOK;
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
            look = DEFAULT_LOOK;
            if tag & HAS_FG != 0 {
                look.fg = Color(get_u32(data, &mut pos));
            }
            if tag & HAS_BG != 0 {
                look.bg = Color(get_u32(data, &mut pos));
            }
            if tag & HAS_STYLE != 0 {
                look.style = get_varint(data, &mut pos);
            }
            run_count -= 1;
        }
        left = left.saturating_sub(1);
        let style = look.style << 21;
        let cp = if ch == ' ' && style & flags::WIDE == 0 { 0 } else { ch as u32 };
        row.cells.push(Cell {
            packed: cp | style,
            fg: look.fg,
            bg: look.bg,
        });
        if style & flags::WIDE != 0 {
            row.cells.push(Cell {
                packed: (style & !flags::WIDE) | flags::WIDE_SPACER,
                fg: look.fg,
                bg: look.bg,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The encoder as it was before its fast paths, kept to check that they
    /// write exactly the same bytes.
    fn encode_reference(row: &Row, out: &mut Vec<u8>) {
        let len = row.cells.iter().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
        let cells = &row.cells[..len];
        out.push(if row.wrapped { WRAPPED } else { 0 });

        let mut body = Vec::new();
        let mut runs: Vec<(Look, u32)> = Vec::new();
        let mut utf8 = [0u8; 4];
        for c in cells {
            if c.has(flags::WIDE_SPACER) {
                continue;
            }
            let cp = c.codepoint();
            if cp < 0x80 {
                body.push(if cp == 0 { b' ' } else { cp as u8 });
            } else {
                body.extend_from_slice(c.ch().encode_utf8(&mut utf8).as_bytes());
            }
            let look = Look {
                fg: c.fg,
                bg: c.bg,
                style: c.flags() >> 21,
            };
            match runs.last_mut() {
                Some((l, n)) if *l == look => *n += 1,
                _ => runs.push((look, 1)),
            }
        }
        put_varint(out, body.len() as u32);
        out.extend_from_slice(&body);

        if runs.iter().all(|(l, _)| *l == DEFAULT_LOOK) {
            put_varint(out, 0);
            return;
        }
        put_varint(out, runs.len() as u32);
        for &(look, n) in runs.iter() {
            let tag = ((look.fg != Color::DEFAULT) as u8 * HAS_FG)
                | ((look.bg != Color::DEFAULT) as u8 * HAS_BG)
                | ((look.style != 0) as u8 * HAS_STYLE);
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
        }
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

    /// A random row, mostly the kinds of cells the encoder treats specially:
    /// empty, default-style ASCII, typed spaces, styled or coloured cells,
    /// blanks that still show (background, inverse, underline), non-ASCII
    /// and wide characters.
    fn random_row(r: &mut Rng) -> Row {
        let len = r.below(90) as usize;
        let ascii_only = r.below(3) == 0;
        let mut cells = Vec::with_capacity(len + 1);
        while cells.len() < len {
            let ch = 0x20 + r.below(95) as u32;
            let colour = |r: &mut Rng| match r.below(3) {
                0 => Color::DEFAULT,
                1 => Color::indexed(r.below(256) as u8),
                _ => Color::rgb(r.below(256) as u8, 0, r.below(256) as u8),
            };
            let kind = if ascii_only { r.below(3) } else { r.below(12) };
            let cell = match kind {
                0 => Cell::BLANK,
                1 => Cell { packed: ch, ..Cell::BLANK },
                2 => Cell {
                    packed: ' ' as u32,
                    ..Cell::BLANK
                },
                3 => Cell {
                    packed: ch | flags::BOLD,
                    ..Cell::BLANK
                },
                4 => Cell {
                    packed: ch,
                    fg: colour(r),
                    bg: Color::DEFAULT,
                },
                5 => Cell::erased(colour(r)),
                6 => Cell {
                    packed: [flags::INVERSE, flags::UNDERLINE, flags::STRIKE, flags::ITALIC][r.below(4) as usize],
                    ..Cell::BLANK
                },
                7 => Cell {
                    packed: 0,
                    fg: colour(r),
                    bg: Color::DEFAULT,
                },
                8 => Cell {
                    packed: ['é', 'Ω', '─', '\u{10FFFD}'][r.below(4) as usize] as u32,
                    ..Cell::BLANK
                },
                9 => Cell {
                    packed: 0x7F,
                    ..Cell::BLANK
                },
                _ => {
                    let (fg, bg) = (colour(r), colour(r));
                    let style = if r.below(2) == 0 { 0 } else { flags::UNDERLINE };
                    cells.push(Cell {
                        packed: '漢' as u32 | style | flags::WIDE,
                        fg,
                        bg,
                    });
                    Cell {
                        packed: style | flags::WIDE_SPACER,
                        fg,
                        bg,
                    }
                }
            };
            cells.push(cell);
        }
        // Trailing blanks of every kind.
        for _ in 0..r.below(40) {
            cells.push(match r.below(4) {
                0 | 1 => Cell::BLANK,
                2 => Cell {
                    packed: flags::BOLD,
                    ..Cell::BLANK
                },
                _ => Cell {
                    packed: 0,
                    fg: Color::indexed(3),
                    bg: Color::DEFAULT,
                },
            });
        }
        Row {
            cells,
            wrapped: r.below(2) == 0,
        }
    }

    #[test]
    fn encoder_writes_the_same_bytes_as_the_reference() {
        let mut r = Rng(0x5EED_1234_ABCD_0001);
        let (mut body, mut runs) = (Vec::new(), Vec::new());
        for i in 0..200_000 {
            let row = random_row(&mut r);
            let (mut fast, mut slow) = (vec![0xEE], vec![0xEE]);
            encode(&row, &mut fast, &mut body, &mut runs);
            encode_reference(&row, &mut slow);
            assert_eq!(fast, slow, "row {i}: {row:?}");
        }
    }

    fn row_of(cells: Vec<Cell>, wrapped: bool) -> Row {
        Row { cells, wrapped }
    }

    #[test]
    fn round_trip_styles_and_wide() {
        let red = Color::indexed(1);
        let rgb = Color::rgb(1, 2, 3);
        let cells = vec![
            Cell {
                packed: 'a' as u32,
                ..Cell::BLANK
            },
            Cell {
                packed: 'b' as u32 | flags::BOLD,
                fg: red,
                bg: Color::DEFAULT,
            },
            Cell {
                packed: '漢' as u32 | flags::WIDE,
                fg: red,
                bg: rgb,
            },
            Cell {
                packed: flags::WIDE_SPACER,
                fg: red,
                bg: rgb,
            },
            Cell::BLANK,
            Cell {
                packed: 'z' as u32 | flags::UNDERLINE,
                ..Cell::BLANK
            },
            Cell::BLANK,
            Cell::BLANK,
        ];
        let mut sb = Scrollback::new(10);
        sb.push(&row_of(cells.clone(), true));
        let back = sb.get(0).unwrap();
        assert!(back.wrapped);
        assert_eq!(back.cells, cells[..6].to_vec());
    }

    #[test]
    fn plain_lines_are_small() {
        let mut sb = Scrollback::new(100_000);
        let mut row = Row::new(200, Cell::BLANK);
        for (i, c) in "the quick brown fox jumps over the lazy dog".chars().enumerate() {
            row.cells[i].packed = c as u32;
        }
        for _ in 0..10_000 {
            sb.push(&row);
        }
        let per_line = sb.memory_bytes() / 10_000;
        assert!(per_line < 60, "{per_line} bytes per line");
        assert_eq!(sb.get(9_999).unwrap().cells.len(), 43);
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
        let mut row = Row::new(200, Cell::BLANK);
        for c in &mut row.cells {
            c.packed = 'x' as u32;
        }
        for _ in 0..20_000 {
            sb.push(&row);
            // 500 lines of ~205 bytes span two blocks; a third may be filling.
            assert!(sb.blocks.len() <= 3, "{} blocks", sb.blocks.len());
        }
        assert_eq!(sb.len(), 500);
        sb.clear();
        assert!(sb.is_empty());
        assert_eq!(sb.memory_bytes(), 0);
        assert!(sb.get(0).is_none());
        sb.push(&row);
        assert_eq!(sb.len(), 1);
        assert_eq!(sb.get(0).unwrap().cells.len(), 200);
    }

    #[test]
    fn limit_and_block_freeing() {
        let mut sb = Scrollback::new(1000);
        let mut row = Row::new(80, Cell::BLANK);
        for i in 0..100_000u32 {
            let s = i.to_string();
            for (j, c) in s.chars().enumerate() {
                row.cells[j].packed = c as u32;
            }
            sb.push(&row);
        }
        assert_eq!(sb.len(), 1000);
        let first: String = sb.get(0).unwrap().cells.iter().map(|c| c.ch()).collect();
        assert_eq!(first, "99000");
        assert!(sb.blocks.len() <= 2, "old blocks freed");
    }
}
