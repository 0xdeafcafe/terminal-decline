//! Shaping rows in faces with ligatures (ligatures design §4.2, §5), and
//! keeping what was shaped.
//!
//! A row is never shaped whole: B1.1 measured JetBrains Mono at ~300 µs
//! for one CTLine over 300 cells of code, nearly all of it the font's
//! `calt` lookups. Only the **stretches** of characters that can take
//! part in the face's substitutions ([`Fonts::ligates`]) are shaped, each
//! with up to two blank cells either side and, past those, one stand-in
//! letter for a neighbour that can't ([`Fonts::plain_char`]): such a
//! character is never changed and no rule tells it from any other, so a
//! stretch shapes the same wherever it is. Everything else is drawn
//! through the per-character path, as in Menlo.
//!
//! What a stretch shapes into is kept by its text (in a small map, emptied
//! when full), so `->` or `);` is shaped once per pane, and what a row
//! shapes into is kept by its [`RowKey`], for the rows on screen: only the
//! glyphs of cells that differ from the per-character path (the
//! **substitutions**), which most rows of code have none of. Colours,
//! the cursor and selection aren't in either, so they never invalidate
//! them.

use std::collections::HashMap;
use std::hash::BuildHasherDefault;

use term_core::{Content, RowKey, Snapshot};
use term_font::{Features, Fonts, RunText, ShapedGlyph, Style as FontStyle, is_code_drawn};

use crate::frame::CellView;
use crate::row_cache::KeyHasher;

const LINE: Features = Features { ligatures: true };

/// Stretches kept by text before the map is emptied: a 300×100 screen of
/// code has under a hundred different ones, and a full map is about
/// 120 KB per pane (2048 buckets of two boxes, the texts, few glyphs).
const STRETCHES: usize = 1024;
/// Spare buffers kept for rows' substitutions.
const SPARE: usize = 16;
/// Blank cells kept either side of a stretch as its context; stretches
/// with more blanks than twice this between them are shaped apart.
const BLANKS: usize = 2;

/// What a cell is to stretch-finding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// A character that can take part in its face's substitutions.
    Lig,
    /// A blank cell, in a face whose space can.
    Blank,
    /// A one-cell character (or blank) that can't: its neighbour stands
    /// in for it.
    Plain,
    /// Breaks a run: another face, no ligatures, wide characters and
    /// their spacers, code-drawn characters, clusters.
    Edge,
}

#[derive(Clone, Copy, Debug)]
struct Seen {
    kind: Kind,
    face: FontStyle,
    ch: char,
}

#[derive(Default)]
struct Entry {
    subs: Vec<ShapedGlyph>,
    used: u64,
}

/// What rows and stretches were shaped against.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
struct Against {
    terminal_id: u64,
    history_epoch: u64,
    cols: u16,
    fonts: u64,
}

/// The shaped substitutions of one pane's visible rows, by row key, and of
/// the stretches it has shaped, by text. Kept in each pane's `Frame` (so
/// in its `PaneRender`), so panes never share or drop each other's.
#[derive(Default)]
pub(crate) struct ShapeCache {
    rows: HashMap<RowKey, Entry, BuildHasherDefault<KeyHasher>>,
    spare: Vec<Vec<ShapedGlyph>>,
    stretches: HashMap<Box<[u16]>, Box<[ShapedGlyph]>, BuildHasherDefault<KeyHasher>>,
    against: Against,
    frame: u64,
    // Scratch, kept to save allocating.
    seen: Vec<Seen>,
    key: Vec<u16>,
    run: RunText,
    shaped: Vec<ShapedGlyph>,
    /// The last row shaped without a key (the reference path).
    unkeyed: Vec<ShapedGlyph>,
    /// Rows and stretches shaped (not found), for tests and benches.
    shaped_rows: u64,
    shaped_stretches: u64,
}

impl ShapeCache {
    /// Before building a frame's rows from `snap` with fonts `fonts`
    /// (`GlyphSource::fonts_key`): drop everything if the terminal, its
    /// history numbering, its width or the fonts changed, and history rows
    /// at or after a pulled-back line (cells design §9.1).
    pub fn check(&mut self, snap: &Snapshot, fonts: u64) {
        let now = Against {
            terminal_id: snap.terminal_id,
            history_epoch: snap.history_epoch,
            cols: snap.cols,
            fonts,
        };
        if now != self.against {
            if now.fonts != self.against.fonts {
                self.stretches.clear();
            }
            self.against = now;
            self.clear_rows();
        }
        if let Some(from) = snap.popped_from {
            let spare = &mut self.spare;
            self.rows.retain(|k, e| {
                let keep = !matches!(k, RowKey::History { line, .. } if *line >= from);
                if !keep && spare.len() < SPARE {
                    spare.push(std::mem::take(&mut e.subs));
                }
                keep
            });
        }
        self.frame += 1;
    }

    /// Forget every row, and with `stretches` every stretch.
    pub fn forget(&mut self, stretches: bool) {
        self.clear_rows();
        if stretches {
            self.stretches.clear();
        }
    }

    fn clear_rows(&mut self) {
        for (_, e) in self.rows.drain() {
            if self.spare.len() < SPARE {
                self.spare.push(e.subs);
            }
        }
    }

    /// After a frame that built rows: drop the rows `snap` doesn't show.
    pub fn keep_visible(&mut self, snap: &Snapshot) {
        let frame = self.frame;
        for row in &snap.rows {
            if let Some(e) = self.rows.get_mut(&row.key) {
                e.used = frame;
            }
        }
        let spare = &mut self.spare;
        self.rows.retain(|_, e| {
            let keep = e.used == frame;
            if !keep && spare.len() < SPARE {
                spare.push(std::mem::take(&mut e.subs));
            }
            keep
        });
    }

    /// The substitutions of row `key` (`None`: don't keep them, the
    /// reference path), whose cells are `cells`: glyphs placed by column,
    /// in column order, for the cells that don't draw as their own
    /// character alone. Shaped unless kept.
    pub fn row<C: CellView>(&mut self, key: Option<RowKey>, cells: impl Iterator<Item = C>, fonts: &Fonts) -> &[ShapedGlyph] {
        let frame = self.frame;
        let Some(key) = key else {
            let mut subs = std::mem::take(&mut self.unkeyed);
            subs.clear();
            self.shape(cells, fonts, &mut subs);
            self.unkeyed = subs;
            return &self.unkeyed;
        };
        if self.rows.contains_key(&key) {
            let e = self.rows.get_mut(&key).expect("just found");
            e.used = frame;
            return &e.subs;
        }
        let mut subs = self.spare.pop().unwrap_or_default();
        subs.clear();
        self.shape(cells, fonts, &mut subs);
        self.shaped_rows += 1;
        let e = self.rows.entry(key).or_default();
        e.subs = subs;
        e.used = frame;
        &e.subs
    }

    /// Rows and stretches shaped so far (not found kept). Test aid.
    pub fn shaped(&self) -> (u64, u64) {
        (self.shaped_rows, self.shaped_stretches)
    }

    /// Rows kept now.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Find the row's stretches and append their substitutions to `out`.
    fn shape<C: CellView>(&mut self, cells: impl Iterator<Item = C>, fonts: &Fonts, out: &mut Vec<ShapedGlyph>) {
        self.seen.clear();
        let lig = FontStyle::ALL.map(|s| fonts.features(s).ligatures);
        let space = FontStyle::ALL.map(|s| fonts.ligates(' ', s));
        let mut last_key = None;
        let mut face = FontStyle::Regular;
        let mut any = false;
        for cell in cells {
            let key = cell.key();
            if last_key != Some(key) {
                last_key = Some(key);
                let a = cell.style().attrs;
                face = FontStyle::new(a.contains(term_core::Attrs::BOLD), a.contains(term_core::Attrs::ITALIC));
            }
            let (kind, ch) = if !lig[face as usize] || cell.is_spacer() {
                (Kind::Edge, ' ')
            } else {
                match cell.content() {
                    Content::Empty | Content::Char(' ') if space[face as usize] => (Kind::Blank, ' '),
                    Content::Empty | Content::Char(' ') => (Kind::Plain, ' '),
                    Content::Char(c) if is_code_drawn(c) => (Kind::Edge, c),
                    Content::Char(c) if fonts.ligates(c, face) => (Kind::Lig, c),
                    Content::Char(c) if crate::frame::char_cells(c) == 1 => (Kind::Plain, c),
                    _ => (Kind::Edge, ' '),
                }
            };
            any |= kind == Kind::Lig;
            self.seen.push(Seen { kind, face, ch });
        }
        if !any {
            return;
        }
        let n = self.seen.len();
        let mut i = 0;
        while i < n {
            if self.seen[i].kind != Kind::Lig {
                i += 1;
                continue;
            }
            // From this character to the last one joined to it: same
            // face, no more than 2 × BLANKS blanks between.
            let face = self.seen[i].face;
            let (first, mut last) = (i, i);
            let mut j = i + 1;
            while j < n && self.seen[j].face == face {
                match self.seen[j].kind {
                    Kind::Lig => last = j,
                    Kind::Blank if j - last <= 2 * BLANKS => {}
                    _ => break,
                }
                j += 1;
            }
            let blank = |k: usize| self.seen[k].kind == Kind::Blank && self.seen[k].face == face;
            let mut start = first;
            while start > 0 && first - start < BLANKS && blank(start - 1) {
                start -= 1;
            }
            let mut end = last + 1;
            while end < n && end - (last + 1) < BLANKS && blank(end) {
                end += 1;
            }
            let plain = |k: usize| self.seen[k].kind == Kind::Plain && self.seen[k].face == face;
            let left = (start > 0 && plain(start - 1)).then(|| self.seen[start - 1].ch);
            let right = (end < n && plain(end)).then(|| self.seen[end].ch);
            self.stretch(start..end, left, right, face, fonts, out);
            i = last + 1;
        }
    }

    /// The substitutions of the stretch of cells `at`, with neighbours
    /// `left` and `right` that can't take part (stood in for by the face's
    /// plain letter), appended to `out` by column.
    fn stretch(
        &mut self,
        at: std::ops::Range<usize>,
        left: Option<char>,
        right: Option<char>,
        face: FontStyle,
        fonts: &Fonts,
        out: &mut Vec<ShapedGlyph>,
    ) {
        let stand_in = |c: char| fonts.plain_char(face).unwrap_or(c);
        let (left, right) = (left.map(stand_in), right.map(stand_in));
        self.key.clear();
        self.key
            .push(face as u16 | (left.is_some() as u16) << 8 | (right.is_some() as u16) << 9);
        let mut buf = [0u16; 2];
        for c in left.into_iter().chain(at.clone().map(|k| self.seen[k].ch)).chain(right) {
            self.key.extend_from_slice(c.encode_utf16(&mut buf));
        }
        let base = at.start as u16;
        if let Some(subs) = self.stretches.get(&self.key[..]) {
            out.extend(subs.iter().map(|g| ShapedGlyph { cell: g.cell + base, ..*g }));
            return;
        }
        self.shaped_stretches += 1;
        let run = &mut self.run;
        run.clear();
        if let Some(c) = left {
            run.push_char(c, 1);
        }
        for k in at.clone() {
            match self.seen[k].kind {
                Kind::Blank => run.push("", 1),
                _ => run.push_char(self.seen[k].ch, 1),
            }
        }
        if let Some(c) = right {
            run.push_char(c, 1);
        }
        self.shaped.clear();
        fonts.shape(run, face, LINE, &mut self.shaped);
        // Cells of the stretch, from the run's start, and which of them
        // don't draw as their character alone: a glyph of theirs is split,
        // or a glyph before covers them.
        let off = left.is_some() as u16;
        let len = at.len() as u16;
        let mut replaced = [false; 64];
        let mut replaced_big = Vec::new();
        let replaced: &mut [bool] = if (len as usize) <= replaced.len() {
            &mut replaced[..len as usize]
        } else {
            replaced_big.resize(len as usize, false);
            &mut replaced_big
        };
        for g in &self.shaped {
            let Some(c) = g.cell.checked_sub(off).filter(|&c| c < len) else {
                continue;
            };
            let c = c as usize;
            if g.split {
                replaced[c] = true;
            }
            for r in replaced.iter_mut().take((c + g.cells as usize).min(len as usize)).skip(c + 1) {
                *r = true;
            }
        }
        let subs: Box<[ShapedGlyph]> = self
            .shaped
            .iter()
            .filter_map(|g| {
                let c = g.cell.checked_sub(off).filter(|&c| c < len)?;
                replaced[c as usize].then_some(ShapedGlyph {
                    cell: c,
                    cells: g.cells.min((len - c) as u8),
                    ..*g
                })
            })
            .collect();
        out.extend(subs.iter().map(|g| ShapedGlyph { cell: g.cell + base, ..*g }));
        if self.stretches.len() >= STRETCHES {
            self.stretches.clear();
        }
        self.stretches.insert(self.key[..].into(), subs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::{Attrs, Style};
    use term_font::{FontConfig, register_font_file};

    /// JetBrains Mono (checked in to term-font) at 26 px, runs allowed.
    fn jetbrains() -> Fonts {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../term-font/tests/fonts/JetBrainsMono-Regular.ttf");
        assert!(register_font_file(std::path::Path::new(path)));
        let mut f = Fonts::with_config(&FontConfig::new("JetBrains Mono", 13.0), 2.0);
        assert_eq!(f.family(), "JetBrains Mono");
        f.allow_runs(true);
        f
    }

    #[derive(Clone, Copy)]
    struct C(char, bool);
    impl CellView for C {
        type Key = bool;
        fn key(&self) -> bool {
            self.1
        }
        fn style(&self) -> Style {
            let mut s = Style::DEFAULT;
            if self.1 {
                s.attrs = Attrs::BOLD;
            }
            s
        }
        fn content(&self) -> Content<'_> {
            Content::Char(self.0)
        }
        fn is_spacer(&self) -> bool {
            false
        }
    }

    /// The substitutions of `cells` shaped the slow way: each run of one
    /// face through one CTLine.
    fn whole(fonts: &Fonts, cells: &[C]) -> Vec<ShapedGlyph> {
        let mut out = Vec::new();
        let mut start = 0;
        while start < cells.len() {
            let bold = cells[start].1;
            let end = (start..cells.len()).find(|&i| cells[i].1 != bold).unwrap_or(cells.len());
            let face = FontStyle::new(bold, false);
            let mut run = RunText::new();
            for c in &cells[start..end] {
                if c.0 == ' ' { run.push("", 1) } else { run.push_char(c.0, 1) }
            }
            let mut g = Vec::new();
            fonts.shape(&run, face, LINE, &mut g);
            let n = end - start;
            let mut replaced = vec![false; n];
            for g in &g {
                let c = g.cell as usize;
                replaced[c] |= g.split;
                for r in replaced.iter_mut().take(c + g.cells as usize).skip(c + 1) {
                    *r = true;
                }
            }
            out.extend(g.iter().filter(|g| replaced[g.cell as usize]).map(|g| ShapedGlyph {
                cell: g.cell + start as u16,
                ..*g
            }));
            start = end;
        }
        out
    }

    const CODE: &[&str] = &[
        "    pub fn shape(&self, run: &RunText, face: Style, out: &mut Vec<ShapedGlyph>) -> bool {",
        "        if features.ligatures && self.shape_line(run, face, out) { return; }",
        "const total = items.filter((x) => x.price >= 10 && x.id !== undefined).map((x) => x.price * 2);",
        "// the cell's own glyph /* comment */ let next = (i + 1..n).find(|&c| own[c]).unwrap_or(n);",
        "match kind[i] { Kind::Blank => {} Kind::Other => self.shape_cell(run, face, i, out), _ => {} }",
        "<!-- html --> <a href=\"x\">www.example.com</a> 0xFF 0x1f 12:34 A:B 1920x1080 ::= <=> <|> |||",
        "x == y != z === w !== v <= u >= t -> s => r <- q <-- p --> o ... .. ..= ?? ?. ;; ~> <~ ## #{ #[",
        "fi fl ffi ffl TODO: FIXME HACK NOTE  ---  ===  ***  ///  ||  &&  ++  --  **  /*  */  </  />",
        "    -    --    ---    ->    -->    <!--    =    ==    ===    !=    !==    =/=    /=    ",
        "ABC::DEF->GHI<JKL>  MNO_PQR  [STU]  {VWX}  1_000_000  3.14e-10  0b1010  #include <stdio.h>",
    ];

    /// Rows of code and of operators in every pair and triple, some with
    /// bold words, some shifted, so stretches meet faces, edges and every
    /// kind of neighbour.
    fn rows() -> Vec<Vec<C>> {
        let mut lines: Vec<Vec<C>> = Vec::new();
        for (k, l) in CODE.iter().enumerate() {
            for shift in 0..4 {
                lines.push(l.chars().skip(shift).map(|c| C(c, false)).collect());
            }
            lines.push(l.chars().enumerate().map(|(i, c)| C(c, (i / (5 + k)) % 3 == 1)).collect());
        }
        let ops = "-<>=!|&:*/.+?#~%^;_[](){}";
        let mut pairs = String::new();
        for a in ops.chars() {
            for b in ops.chars() {
                pairs.extend([a, b, 'a']);
                for c in "=->".chars() {
                    pairs.extend([a, b, c, ' ']);
                }
            }
        }
        for chunk in pairs.chars().collect::<Vec<_>>().chunks(120) {
            lines.push(chunk.iter().map(|&c| C(c, false)).collect());
        }
        lines
    }

    #[test]
    fn stretches_shape_like_whole_runs() {
        let fonts = jetbrains();
        let mut cache = ShapeCache::default();
        let mut subs = 0;
        for cells in rows() {
            let want = whole(&fonts, &cells);
            let mut got = Vec::new();
            cache.shape(cells.iter().copied(), &fonts, &mut got);
            let text: String = cells.iter().map(|c| c.0).collect();
            assert_eq!(got, want, "{text:?}");
            subs += got.len();
        }
        assert!(subs > 100, "only {subs} substitutions: is the font's calt on?");
    }
}
