//! Shaping runs of cells into glyphs (cells design §10.2).
//!
//! The renderer collects a run's text cell by cell into a [`RunText`] and
//! [`Fonts::shape`] turns it into [`ShapedGlyph`]s placed on the cell grid.
//! With ligatures off (today, always) each cell's text is shaped alone:
//! single characters through the per-character lookup F2.1 caches, grapheme
//! clusters (emoji sequences, combining marks) with Core Text's CTLine
//! through a least-recently-used cache. With ligatures on (B.1) the whole
//! run goes through one CTLine, so the font's contextual alternates and
//! ligatures can span cells.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFArray, CFAttributedString, CFDictionary, CFIndex, CFRange, CFRetained, CFString, CGPoint};
use objc2_core_graphics::CGGlyph;
use objc2_core_text::{CTFont, CTLine, CTRun, kCTFontAttributeName};

use crate::{FontId, Fonts, GlyphKey, SpriteId, Style};

/// How runs are shaped. Today only ligatures; E.4's `font_features` list
/// joins it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Features {
    /// Shape whole runs with the font's ligatures and contextual
    /// alternates (B.1). Off: every cell's text is shaped alone.
    pub ligatures: bool,
}

/// One glyph of a shaped run, placed on the cell grid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShapedGlyph {
    /// The first cell the glyph belongs to, from the run's start.
    pub cell: u16,
    /// Cells it covers: 1, 2 for a wide cell, n for an n-cell ligature.
    pub cells: u8,
    pub key: GlyphKey,
    /// Pixel offset from that cell's origin, x right and y down: non-zero
    /// for the second and later glyphs of a cluster (combining marks).
    pub dx: i16,
    pub dy: i16,
}

/// A run's text, cell by cell, with the table that maps each UTF-16 unit
/// back to its cell.
#[derive(Default, Clone, Debug)]
pub struct RunText {
    text: Vec<u16>,
    /// UTF-16 start of each cell's text; one more at the end.
    starts: Vec<u32>,
    /// First cell of each cell's text; one more at the end (the run's width).
    cells: Vec<u16>,
    /// The cell-text index of every UTF-16 unit.
    owner: Vec<u16>,
}

impl RunText {
    pub fn new() -> RunText {
        let mut r = RunText::default();
        r.clear();
        r
    }

    /// Empty it, keeping its memory.
    pub fn clear(&mut self) {
        self.text.clear();
        self.owner.clear();
        self.starts.clear();
        self.starts.push(0);
        self.cells.clear();
        self.cells.push(0);
    }

    /// Append one cell's text, a character or a grapheme cluster, covering
    /// `width` cells (2 for a wide character; its spacer cell adds
    /// nothing). Blank cells are a space.
    pub fn push(&mut self, cluster: &str, width: u8) {
        let index = self.starts.len() as u16 - 1;
        let before = self.text.len();
        self.text.extend(cluster.encode_utf16());
        self.owner.resize(self.text.len(), index);
        if self.text.len() == before {
            // No text: still a cell, drawn as a space.
            self.text.push(b' ' as u16);
            self.owner.push(index);
        }
        self.starts.push(self.text.len() as u32);
        let end = self.cells.last().copied().unwrap_or(0) + width.max(1) as u16;
        self.cells.push(end);
    }

    /// [`push`](RunText::push) for one character.
    pub fn push_char(&mut self, ch: char, width: u8) {
        self.push(ch.encode_utf8(&mut [0; 4]), width)
    }

    /// The run's text.
    pub fn utf16(&self) -> &[u16] {
        &self.text
    }

    /// Cells the run covers.
    pub fn width(&self) -> u16 {
        *self.cells.last().unwrap_or(&0)
    }

    /// How many cell texts were pushed.
    pub fn len(&self) -> usize {
        self.starts.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn cluster(&self, i: usize) -> &[u16] {
        &self.text[self.starts[i] as usize..self.starts[i + 1] as usize]
    }

    fn cell(&self, i: usize) -> (u16, u8) {
        (self.cells[i], (self.cells[i + 1] - self.cells[i]) as u8)
    }
}

/// The one character `units` encode, if they encode exactly one.
fn single_char(units: &[u16]) -> Option<char> {
    let mut it = char::decode_utf16(units.iter().copied());
    match (it.next(), it.next()) {
        (Some(Ok(ch)), None) => Some(ch),
        _ => None,
    }
}

/// A cached glyph: its key and offset inside its cluster.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CachedGlyph {
    key: GlyphKey,
    dx: i16,
    dy: i16,
}

const NIL: u32 = u32::MAX;

struct Entry {
    /// The face, then the cluster's UTF-16.
    key: Box<[u16]>,
    glyphs: Box<[CachedGlyph]>,
    prev: u32,
    next: u32,
}

/// Shaped clusters by (face, cluster text), least recently used first out.
/// A slab of entries in a doubly linked list, most recent at the head.
pub(crate) struct ClusterCache {
    map: HashMap<Box<[u16]>, u32>,
    entries: Vec<Entry>,
    head: u32,
    tail: u32,
    cap: usize,
    /// Lookup key built here, so a hit allocates nothing.
    scratch: Vec<u16>,
}

impl ClusterCache {
    /// The cap from the design: 4096 clusters per [`Fonts`].
    pub const CAP: usize = 4096;

    pub fn new(cap: usize) -> ClusterCache {
        ClusterCache {
            map: HashMap::new(),
            entries: Vec::new(),
            head: NIL,
            tail: NIL,
            cap: cap.max(1),
            scratch: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.entries.clear();
        (self.head, self.tail) = (NIL, NIL);
    }

    fn key(&mut self, face: Style, text: &[u16]) {
        self.scratch.clear();
        self.scratch.push(face as u16);
        self.scratch.extend_from_slice(text);
    }

    fn unlink(&mut self, i: u32) {
        let (prev, next) = (self.entries[i as usize].prev, self.entries[i as usize].next);
        match prev {
            NIL => self.head = next,
            p => self.entries[p as usize].next = next,
        }
        match next {
            NIL => self.tail = prev,
            n => self.entries[n as usize].prev = prev,
        }
    }

    fn push_front(&mut self, i: u32) {
        let e = &mut self.entries[i as usize];
        (e.prev, e.next) = (NIL, self.head);
        match self.head {
            NIL => self.tail = i,
            h => self.entries[h as usize].prev = i,
        }
        self.head = i;
    }

    /// The glyphs for `text` in `face`, marking them most recently used.
    pub fn get(&mut self, face: Style, text: &[u16]) -> Option<&[CachedGlyph]> {
        self.key(face, text);
        let i = *self.map.get(&self.scratch[..])?;
        if self.head != i {
            self.unlink(i);
            self.push_front(i);
        }
        Some(&self.entries[i as usize].glyphs)
    }

    /// Store `glyphs` for `text` in `face`, evicting the least recently
    /// used entry when full.
    pub fn insert(&mut self, face: Style, text: &[u16], glyphs: &[CachedGlyph]) {
        self.key(face, text);
        if let Some(&i) = self.map.get(&self.scratch[..]) {
            self.entries[i as usize].glyphs = glyphs.into();
            self.unlink(i);
            self.push_front(i);
            return;
        }
        let key: Box<[u16]> = self.scratch[..].into();
        let i = if self.map.len() >= self.cap {
            // Reuse the oldest entry's slot.
            let i = self.tail;
            self.unlink(i);
            let old = std::mem::replace(&mut self.entries[i as usize].key, key.clone());
            self.map.remove(&old);
            self.entries[i as usize].glyphs = glyphs.into();
            i
        } else {
            self.entries.push(Entry {
                key: key.clone(),
                glyphs: glyphs.into(),
                prev: NIL,
                next: NIL,
            });
            self.entries.len() as u32 - 1
        };
        self.map.insert(key, i);
        self.push_front(i);
    }
}

/// One glyph out of a CTLine, before it is put on the grid.
struct LineGlyph {
    /// The cell text it came from.
    cluster: usize,
    font: FontId,
    glyph: CGGlyph,
    x: f64,
    y: f64,
}

impl Fonts {
    /// Shape `run` in `face` into glyphs on the cell grid, appended to
    /// `out`. Spaces give no glyphs; code-drawn characters give sprite keys
    /// across their cells. Keys follow [`Fonts::glyph_key`]'s rule: colour
    /// glyphs cover 2 cells, every other font glyph 1, whatever the cell's
    /// width, so the same bitmap has one key. Every glyph sits at its
    /// cell's origin plus its offset inside its cluster, never at Core
    /// Text's accumulated advance, so a fallback font with another advance
    /// can't drift off the grid.
    ///
    /// With `features.ligatures` off every cell's text is shaped alone:
    /// single characters through the per-character lookup, clusters through
    /// CTLine and the shaped-cluster cache. With it on the whole run is one
    /// CTLine, and a glyph that swallows the following cells' text (a true
    /// ligature) covers their cells too.
    pub fn shape(&self, run: &RunText, face: Style, features: Features, out: &mut Vec<ShapedGlyph>) {
        if features.ligatures {
            self.shape_line(run, face, out);
            return;
        }
        for i in 0..run.len() {
            let (cell, cells) = run.cell(i);
            let text = run.cluster(i);
            if text == [b' ' as u16] {
                continue;
            }
            if let Some(ch) = single_char(text) {
                if let Some(mut key) = self.glyph_key(ch, face) {
                    let mut cells = cells;
                    if key.font == crate::SPRITE_FONT {
                        key.cells = cells;
                    } else if key.variant & GlyphKey::FITTED != 0 {
                        cells = self.icon_cells(&mut key, cells, run, i);
                    }
                    out.push(ShapedGlyph {
                        cell,
                        cells,
                        key,
                        dx: 0,
                        dy: 0,
                    });
                }
                continue;
            }
            let place = |g: &CachedGlyph| ShapedGlyph {
                cell,
                cells,
                key: g.key,
                dx: g.dx,
                dy: g.dy,
            };
            if let Some(hit) = self.clusters.borrow_mut().get(face, text) {
                out.extend(hit.iter().map(place));
                continue;
            }
            let glyphs = self.shape_cluster(text, face);
            out.extend(glyphs.iter().map(place));
            self.clusters.borrow_mut().insert(face, text, &glyphs);
        }
    }

    /// Clusters in the shaped-cluster cache.
    pub fn shaped_clusters(&self) -> usize {
        self.clusters.borrow().len()
    }

    /// Empty the shaped-cluster cache; for measuring shaping cold.
    #[doc(hidden)]
    pub fn clear_shaped_clusters(&self) {
        self.clusters.borrow_mut().clear();
    }

    /// One cluster through CTLine, offsets relative to its first glyph.
    fn shape_cluster(&self, text: &[u16], face: Style) -> Vec<CachedGlyph> {
        let mut glyphs = Vec::new();
        let Some(line) = self.line(text, face) else { return glyphs };
        let mut origin = None;
        self.walk(&line, |_, font, glyph, x, y| {
            let (ox, oy) = *origin.get_or_insert((x, y));
            glyphs.push(CachedGlyph {
                key: self.font_key(font, glyph),
                dx: (x - ox).round() as i16,
                dy: -(y - oy).round() as i16,
            });
        });
        glyphs
    }

    /// The whole run through one CTLine (ligatures on).
    fn shape_line(&self, run: &RunText, face: Style, out: &mut Vec<ShapedGlyph>) {
        let Some(line) = self.line(run.utf16(), face) else { return };
        let mut glyphs = Vec::new();
        self.walk(&line, |index, font, glyph, x, y| {
            let cluster = run.owner.get(index).copied().unwrap_or(0) as usize;
            glyphs.push(LineGlyph {
                cluster,
                font,
                glyph,
                x,
                y,
            });
        });
        // Each cluster's origin is its first glyph in string order; a cell
        // whose text left no glyph of its own was swallowed by the glyph
        // before it.
        let mut origin: Vec<Option<(f64, f64)>> = vec![None; run.len()];
        let mut order: Vec<usize> = (0..glyphs.len()).collect();
        order.sort_by_key(|&g| glyphs[g].cluster);
        for &g in &order {
            let o = &mut origin[glyphs[g].cluster];
            if o.is_none() {
                *o = Some((glyphs[g].x, glyphs[g].y));
            }
        }
        for g in &glyphs {
            let text = run.cluster(g.cluster);
            if text == [b' ' as u16] {
                continue;
            }
            let (cell, _) = run.cell(g.cluster);
            let next = (g.cluster + 1..run.len()).find(|&c| origin[c].is_some()).unwrap_or(run.len());
            let mut cells = (run.cells[next] - cell).min(u8::MAX as u16) as u8;
            let ch = single_char(text);
            let key = match ch.and_then(SpriteId::for_char) {
                Some(id) => GlyphKey::sprite(id, cells),
                None => {
                    let mut key = self.font_key(g.font, g.glyph);
                    if key.cells == 1 && ch.is_some_and(|ch| self.is_icon(ch, g.font)) {
                        key.variant |= GlyphKey::FITTED;
                        cells = self.icon_cells(&mut key, cells, run, g.cluster);
                    }
                    key
                }
            };
            let (ox, oy) = origin[g.cluster].unwrap_or((g.x, g.y));
            out.push(ShapedGlyph {
                cell,
                cells,
                key,
                dx: (g.x - ox).round() as i16,
                dy: -(g.y - oy).round() as i16,
            });
        }
    }

    /// The cells an icon in cell text `i` of `run` is fitted to, set in
    /// `key` and returned: 2 when its cell is wide, or when its ink is wider
    /// than a cell and the next cell is blank (Nerd Font's convention);
    /// otherwise 1.
    fn icon_cells(&self, key: &mut GlyphKey, cells: u8, run: &RunText, i: usize) -> u8 {
        let next_blank = i + 1 < run.len() && run.cluster(i + 1) == [b' ' as u16];
        let wide = cells >= 2 || (next_blank && self.wider_than_cell(key.font, key.glyph));
        key.cells = if wide { 2 } else { 1 };
        cells.max(key.cells)
    }

    /// The key for a glyph CTLine picked: 2 cells for colour glyphs.
    fn font_key(&self, font: FontId, glyph: CGGlyph) -> GlyphKey {
        let color = self.registry.borrow().faces[font as usize].color;
        GlyphKey {
            font,
            glyph,
            cells: if color { 2 } else { 1 },
            variant: 0,
        }
    }

    /// A CTLine of `text` in `face`, with the font's default features.
    pub(crate) fn line(&self, text: &[u16], face: Style) -> Option<CFRetained<CTLine>> {
        let font = self.registry.borrow().faces[face as usize].font.clone();
        unsafe {
            let s = CFString::with_characters(None, text.as_ptr(), text.len() as CFIndex)?;
            let attrs = CFDictionary::<CFString, CTFont>::from_slices(&[kCTFontAttributeName], &[&font]);
            let attributed = CFAttributedString::new(None, Some(&s), Some(attrs.as_opaque()))?;
            Some(CTLine::with_attributed_string(&attributed))
        }
    }

    /// Every glyph of `line`: its UTF-16 index, font id (registering fonts
    /// Core Text picked), glyph and position. Glyphs from a font the
    /// registry can't take any more, and deleted glyphs, are skipped.
    pub(crate) fn walk(&self, line: &CTLine, mut f: impl FnMut(usize, FontId, CGGlyph, f64, f64)) {
        let runs: CFRetained<CFArray> = unsafe { line.glyph_runs() };
        let mut glyphs: Vec<CGGlyph> = Vec::new();
        let mut positions: Vec<CGPoint> = Vec::new();
        let mut indices: Vec<CFIndex> = Vec::new();
        for r in 0..runs.count() {
            let run: &CTRun = unsafe { &*(runs.value_at_index(r) as *const CTRun) };
            let font = unsafe {
                let attrs = run.attributes();
                let font = attrs.value(kCTFontAttributeName as *const CFString as *const c_void) as *const CTFont;
                if font.is_null() {
                    continue;
                }
                &*font
            };
            let Some(id) = self.font_id(font) else { continue };
            let n = unsafe { run.glyph_count() } as usize;
            if n == 0 {
                continue;
            }
            glyphs.resize(n, 0);
            positions.resize(n, CGPoint::default());
            indices.resize(n, 0);
            let all = CFRange { location: 0, length: 0 };
            unsafe {
                run.glyphs(all, NonNull::new_unchecked(glyphs.as_mut_ptr()));
                run.positions(all, NonNull::new_unchecked(positions.as_mut_ptr()));
                run.string_indices(all, NonNull::new_unchecked(indices.as_mut_ptr()));
            }
            for i in 0..n {
                if glyphs[i] == 0xFFFF {
                    continue;
                }
                f(indices[i] as usize, id, glyphs[i], positions[i].x, positions[i].y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(n: u16) -> [CachedGlyph; 1] {
        [CachedGlyph {
            key: GlyphKey {
                font: 0,
                glyph: n,
                cells: 1,
                variant: 0,
            },
            dx: 0,
            dy: 0,
        }]
    }

    #[test]
    fn cache_evicts_the_least_recently_used() {
        let mut c = ClusterCache::new(3);
        for n in 0..3u16 {
            c.insert(Style::Regular, &[n], &g(n));
        }
        // Touch 0, so 1 is now the oldest.
        assert_eq!(c.get(Style::Regular, &[0]).unwrap(), &g(0));
        c.insert(Style::Regular, &[9], &g(9));
        assert_eq!(c.len(), 3);
        assert!(c.get(Style::Regular, &[1]).is_none(), "the oldest went");
        for n in [0, 2, 9] {
            assert_eq!(c.get(Style::Regular, &[n]).unwrap(), &g(n));
        }
        // Faces are part of the key.
        assert!(c.get(Style::Bold, &[0]).is_none());
        // Many more than the cap: never over it, the newest always there.
        for n in 100..1000u16 {
            c.insert(Style::Italic, &[n, n], &g(n));
            assert!(c.len() <= 3);
            assert_eq!(c.get(Style::Italic, &[n, n]).unwrap(), &g(n));
        }
        assert_eq!(c.entries.len(), 3, "slots are reused");
        c.clear();
        assert_eq!(c.len(), 0);
        c.insert(Style::Regular, &[1], &g(1));
        assert_eq!(c.get(Style::Regular, &[1]).unwrap(), &g(1));
    }

    #[test]
    fn run_text_maps_units_to_cells() {
        let mut r = RunText::new();
        r.push("a", 1);
        r.push("漢", 2);
        r.push("", 1);
        r.push("👍🏽", 2);
        assert_eq!(r.len(), 4);
        assert_eq!(r.width(), 6);
        assert_eq!(r.utf16().len(), 1 + 1 + 1 + 4);
        assert_eq!(r.owner, [0, 1, 2, 3, 3, 3, 3]);
        assert_eq!((r.cell(1), r.cell(2), r.cell(3)), ((1, 2), (3, 1), (4, 2)));
        assert_eq!(r.cluster(2), [b' ' as u16]);
        r.clear();
        assert!(r.is_empty() && r.width() == 0);
    }
}
