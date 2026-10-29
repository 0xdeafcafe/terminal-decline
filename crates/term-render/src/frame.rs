//! Turns terminal state into GPU instances. No Metal here, so it can be
//! tested and benchmarked on its own.

use term_core::{Attrs, Cell, CellRef, ColorKind, Content, CursorShape, HighlightKind, RowFlags, StyleKey, Terminal, Underline, Width};
use term_font::{Metrics, Style as FontStyle};
use unicode_width::UnicodeWidthChar;

use crate::row_cache::{Built, Entry, MISSING, Mark, RowCache};

/// RGBA8 with red in the low byte (matches `unpack_unorm4x8_to_float`).
pub type Rgba = u32;

pub const fn rgb(hex: u32) -> Rgba {
    let r = (hex >> 16) & 0xFF;
    let g = (hex >> 8) & 0xFF;
    let b = hex & 0xFF;
    r | g << 8 | b << 16 | 0xFF << 24
}

/// Pixel positions are i16 on the GPU (32k pixels is beyond any display);
/// clamp rather than wrap.
#[inline]
fn s16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Cells a character takes, by the same rule term-core uses when it prints:
/// 0 for characters it drops (controls, combining marks).
#[inline]
fn char_cells(c: char) -> usize {
    c.width().unwrap_or(0)
}

fn with_alpha(c: Rgba, a: u8) -> Rgba {
    (c & 0x00FF_FFFF) | (a as u32) << 24
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectInstance {
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
    pub color: Rgba,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphInstance {
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
    pub u: u16,
    pub v: u16,
    pub color: Rgba,
    /// Bit 0 ([`GlyphInstance::COLOR`]): sample the colour (emoji) texture.
    /// The shader reads only that bit; frame building keeps the glyph's
    /// cell in the others, for the overlays drawn over cached rows.
    pub is_color: u32,
}

impl GlyphInstance {
    pub const COLOR: u32 = 1;
    /// The cell is dim (its text colour has a lower alpha).
    const DIM: u32 = 1 << 1;
    const COL_SHIFT: u32 = 16;

    /// `is_color` for a glyph from `slot` in column `col`.
    #[inline]
    fn flags(slot: &GlyphSlot, col: usize, dim: bool) -> u32 {
        slot.color as u32 | (dim as u32) << 1 | (col.min(u16::MAX as usize) as u32) << Self::COL_SHIFT
    }

    /// The column it was built for.
    #[inline]
    fn col(&self) -> usize {
        (self.is_color >> Self::COL_SHIFT) as usize
    }
}

// These layouts must match `struct Rect` and `struct Glyph` in shaders.metal,
// which read the instance buffers directly.
const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<RectInstance>() == 12);
    assert!(offset_of!(RectInstance, x) == 0);
    assert!(offset_of!(RectInstance, w) == 4);
    assert!(offset_of!(RectInstance, color) == 8);
    assert!(size_of::<GlyphInstance>() == 20);
    assert!(offset_of!(GlyphInstance, x) == 0);
    assert!(offset_of!(GlyphInstance, w) == 4);
    assert!(offset_of!(GlyphInstance, u) == 8);
    assert!(offset_of!(GlyphInstance, color) == 12);
    assert!(offset_of!(GlyphInstance, is_color) == 16);
};

/// Where a rasterised glyph sits in the glyph texture.
#[derive(Clone, Copy, Debug)]
pub struct GlyphSlot {
    pub u: u16,
    pub v: u16,
    pub w: u16,
    pub h: u16,
    pub left: i16,
    pub top: i16,
    pub color: bool,
}

pub trait GlyphSource {
    /// `None` for glyphs that draw nothing (spaces) or are missing.
    fn glyph(&mut self, codepoint: u32, style: FontStyle) -> Option<GlyphSlot>;

    /// Changes whenever slots handed out before may have moved (a glyph
    /// texture emptied or replaced, the fonts changed): frame building
    /// then drops the rows it cached.
    fn generation(&self) -> u64 {
        0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub foreground: Rgba,
    pub background: Rgba,
    pub cursor: Rgba,
    pub cursor_text: Rgba,
    pub selection: Rgba,
    /// 0..16 are the configurable ANSI colours, the rest is the xterm cube.
    pub palette: [Rgba; 256],
    /// 0–255; below 255 the window background shows through.
    pub background_opacity: u8,
}

impl Default for Theme {
    fn default() -> Theme {
        // A neutral dark theme.
        let ansi = [
            0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, //
            0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
        ];
        Theme::new(ansi, 0xd8dee9, 0x16181c, 0xd8dee9, 0x2e3a4f)
    }
}

impl Theme {
    pub fn new(ansi: [u32; 16], fg: u32, bg: u32, cursor: u32, selection: u32) -> Theme {
        let mut palette = [0; 256];
        for (i, c) in ansi.iter().enumerate() {
            palette[i] = rgb(*c);
        }
        let level = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
        for i in 0..216u32 {
            let (r, g, b) = (level(i / 36), level(i / 6 % 6), level(i % 6));
            palette[16 + i as usize] = rgb(r << 16 | g << 8 | b);
        }
        for i in 0..24u32 {
            let v = 8 + i * 10;
            palette[232 + i as usize] = rgb(v << 16 | v << 8 | v);
        }
        Theme {
            foreground: rgb(fg),
            background: rgb(bg),
            cursor: rgb(cursor),
            cursor_text: rgb(bg),
            selection: rgb(selection),
            palette,
            background_opacity: 255,
        }
    }

    #[inline]
    fn resolve(&self, c: term_core::Color, default: Rgba) -> Rgba {
        match c.kind() {
            ColorKind::Default => default,
            ColorKind::Indexed(i) => self.palette[i as usize],
            ColorKind::Rgb(r, g, b) => r as u32 | (g as u32) << 8 | (b as u32) << 16 | 0xFF << 24,
        }
    }
}

/// Steps in a curly underline's wave per cell, and each step's offset from
/// the underline in thicknesses (a coarse sine: up, back, down, back).
const CURLY_SEGMENTS: i32 = 8;
const CURLY_WAVE: [i32; CURLY_SEGMENTS as usize] = [0, -1, -1, 0, 0, 1, 1, 0];

/// What a row needs from the frame around it.
struct RowCtx<'a> {
    theme: &'a Theme,
    m: Metrics,
    px: i32,
    /// The row's top, pixels.
    y: i32,
    cols: usize,
    /// Selected columns, half-open; empty when none are. Only the
    /// reference path (`build_live`) draws highlights into rows.
    sel: std::ops::Range<usize>,
    /// The columns under a block cursor, on this row; empty when it isn't
    /// on it. Only the reference path draws the cursor into rows.
    block_cursor: std::ops::Range<usize>,
    /// Columns covered by input-method text, which draw no glyphs.
    covered: std::ops::Range<usize>,
}

/// A cell as frame building reads it: `CellRef`, a snapshot's cells, and
/// in tests cells built from any `Style`.
trait CellView {
    /// Equal keys mean equal styles.
    type Key: Copy + PartialEq;
    fn key(&self) -> Self::Key;
    fn style(&self) -> term_core::Style;
    fn content(&self) -> Content<'_>;
    fn is_spacer(&self) -> bool;
}

impl CellView for CellRef<'_> {
    type Key = StyleKey;
    #[inline]
    fn key(&self) -> StyleKey {
        self.style_key()
    }
    #[inline]
    fn style(&self) -> term_core::Style {
        CellRef::style(self)
    }
    #[inline]
    fn content(&self) -> Content<'_> {
        CellRef::content(self)
    }
    #[inline]
    fn is_spacer(&self) -> bool {
        CellRef::is_spacer(self)
    }
}

/// A cell copied into a snapshot, with the styles its id indexes (the
/// screen's mirror, or the snapshot's history styles).
#[derive(Clone, Copy)]
struct SnapCell<'a> {
    cell: Cell,
    styles: &'a [term_core::Style],
    /// Where a cluster cell's text is. A reference, read only for cluster
    /// cells: working the text out for every cell made full frames 17%
    /// slower.
    clusters: &'a SnapClusters<'a>,
}

/// A snapshot row's cluster text.
struct SnapClusters<'a> {
    snap: &'a term_core::Snapshot,
    row: &'a term_core::SnapRow,
}

impl CellView for SnapCell<'_> {
    type Key = u32;
    #[inline]
    fn key(&self) -> u32 {
        self.cell.style_id()
    }
    #[inline]
    fn style(&self) -> term_core::Style {
        self.styles.get(self.cell.style_id() as usize).copied().unwrap_or_default()
    }
    /// As `CellRef::content` reads it, with a cluster cell's text from the
    /// snapshot.
    #[inline]
    fn content(&self) -> Content<'_> {
        if self.cell.is_cluster() {
            return Content::Cluster(self.clusters.snap.cluster(self.clusters.row, self.cell.codepoint()));
        }
        match self.cell.codepoint() {
            0 => Content::Empty,
            cp => char::from_u32(cp).map_or(Content::Empty, Content::Char),
        }
    }
    #[inline]
    fn is_spacer(&self) -> bool {
        self.cell.is_spacer()
    }
}

/// Where a row's instances go: the frame itself (the reference path) or a
/// row cache entry, which also records each one's cell.
trait RowSink {
    fn background(&mut self, r: RectInstance);
    fn glyph(&mut self, g: GlyphInstance);
    fn decoration(&mut self, r: RectInstance, mark: Mark);
}

/// The frame's own vectors, for `build_live`.
struct Direct<'a> {
    rects: &'a mut Vec<RectInstance>,
    glyphs: &'a mut Vec<GlyphInstance>,
    decorations: &'a mut Vec<RectInstance>,
}

impl RowSink for Direct<'_> {
    #[inline]
    fn background(&mut self, r: RectInstance) {
        self.rects.push(r);
    }
    #[inline]
    fn glyph(&mut self, g: GlyphInstance) {
        self.glyphs.push(g);
    }
    #[inline]
    fn decoration(&mut self, r: RectInstance, _: Mark) {
        self.decorations.push(r);
    }
}

impl RowSink for Entry {
    #[inline]
    fn background(&mut self, r: RectInstance) {
        self.backgrounds.push(r);
    }
    #[inline]
    fn glyph(&mut self, g: GlyphInstance) {
        self.glyphs.push(g);
    }
    #[inline]
    fn decoration(&mut self, r: RectInstance, mark: Mark) {
        self.decorations.push(r);
        self.decoration_marks.push(mark);
    }
}

/// A style resolved against the theme, worked out once per run of
/// same-styled cells. Kept small, as a coloured screen changes style every
/// few cells: the underline colour is only resolved for underlined cells.
#[derive(Clone, Copy)]
struct Pen {
    /// Text colour and background, after inverse.
    fg: Rgba,
    bg: Rgba,
    attrs: Attrs,
    underline: Underline,
    /// Unresolved: the default draws underlines in the text colour.
    underline_color: term_core::Color,
    /// Any of underline, overline or strike.
    decorated: bool,
}

impl Pen {
    #[inline]
    fn new(style: &term_core::Style, theme: &Theme) -> Pen {
        let a = style.attrs;
        let mut fg = theme.resolve(style.fg, theme.foreground);
        let mut bg = theme.resolve(style.bg, theme.background);
        if a.contains(Attrs::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        Pen {
            fg,
            bg,
            attrs: a,
            underline: style.underline,
            underline_color: style.underline_color,
            decorated: style.underline != Underline::None || a.intersects(Attrs::OVERLINE | Attrs::STRIKE),
        }
    }

    #[inline]
    fn face(&self) -> FontStyle {
        FontStyle::new(self.attrs.contains(Attrs::BOLD), self.attrs.contains(Attrs::ITALIC))
    }

    /// The flags of a [`Mark`] for this pen's cells.
    #[inline]
    fn mark(&self, col: usize) -> Mark {
        Mark {
            col: col.min(u16::MAX as usize) as u16,
            flags: if self.attrs.contains(Attrs::DIM) { Mark::DIM } else { 0 },
        }
    }
}

/// How a frame is drawn, apart from the terminal.
pub struct FrameInput<'a> {
    pub theme: &'a Theme,
    pub metrics: Metrics,
    /// Padding around the grid, pixels.
    pub padding: (u32, u32),
    /// Blink phase: false hides a blinking cursor.
    pub cursor_on: bool,
    pub focused: bool,
    /// Text being composed by an input method, drawn at the cursor.
    pub preedit: Option<&'a str>,
}

/// One frame's instances, and what building them keeps between frames
/// (the row cache, cells design §9.3).
#[derive(Default)]
pub struct Frame {
    pub clear: Rgba,
    pub rects: Vec<RectInstance>,
    pub glyphs: Vec<GlyphInstance>,
    /// Underlines, strikethrough and cursor; appended after the backgrounds.
    decorations: Vec<RectInstance>,
    rows: RowCache,
    /// Indexes into the snapshot's rows of the rows to build this frame.
    to_build: Vec<usize>,
}

/// Rows whose top is this far down (pixels) are built every frame rather
/// than kept: positions are clamped to the i16 range, so their instances
/// may not move exactly. Only grids over ~30k pixels tall reach it.
const FAR: i32 = i16::MAX as i32 / 2;

/// The cursor as this frame draws it.
struct CursorDraw {
    /// View row and column.
    at: (usize, usize),
    shape: CursorShape,
    wide: bool,
}

impl Frame {
    /// Build this frame from a snapshot of `term` (cells design §9): rows
    /// whose change counter the row cache has are reused, the others are
    /// copied and built, and the cursor, selection and input-method text
    /// are drawn over them. Holds `term` (the lock) for all of it until R.1
    /// moves building out.
    pub fn build(&mut self, term: &mut Terminal, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        let m = input.metrics;
        self.rows.check(&Built {
            theme: input.theme.clone(),
            metrics: [
                m.cell_width,
                m.cell_height,
                m.baseline,
                m.underline_offset,
                m.underline_thickness,
                m.strike_offset,
            ],
            padding_x: input.padding.0,
            glyphs: glyphs.generation(),
        });
        self.rows.snapshot(term);
        self.rows.take(&mut self.to_build);
        self.build_rows(input, glyphs);
        self.rows.place();
        let (py, ch) = (input.padding.1 as i32, m.cell_height as i32);
        self.rows.move_rows(|line| py + line as i32 * ch);
        self.assemble(input, glyphs);
        self.rows.drop_far(FAR);
    }

    /// Drop every cached row: the next frame copies and builds them all.
    #[doc(hidden)]
    pub fn forget_rows(&mut self) {
        self.rows.forget_rows();
    }

    /// Rows in the row cache. Test aid.
    #[doc(hidden)]
    pub fn cached_rows(&self) -> usize {
        self.rows.len()
    }

    /// Build the rows the snapshot copied into the row cache, where they
    /// are drawn this frame.
    fn build_rows(&mut self, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        let cols = self.rows.snap.cols as usize;
        let (py, ch) = (input.padding.1 as i32, input.metrics.cell_height as i32);
        for &i in &self.to_build {
            let row = &self.rows.snap.rows[i];
            let key = row.key;
            let slot = self.rows.entry_for(key);
            let ctx = RowCtx {
                theme: input.theme,
                m: input.metrics,
                px: input.padding.0 as i32,
                y: py + i as i32 * ch,
                cols,
                sel: 0..0,
                block_cursor: 0..0,
                covered: 0..0,
            };
            // The entry is taken out while it's built, as its cells and
            // styles are borrowed from the cache.
            let mut entry = std::mem::take(self.rows.entry_mut(slot));
            entry.y = ctx.y;
            let row = &self.rows.snap.rows[i];
            let styles = self.rows.styles_for(row.flags.contains(RowFlags::HISTORY));
            let snap = &self.rows.snap;
            let cells = snap.row_cells(row).unwrap_or(&[]);
            let clusters = SnapClusters { snap, row };
            let cells = cells.iter().map(|&cell| SnapCell {
                cell,
                styles,
                clusters: &clusters,
            });
            build_row(&ctx, cells, glyphs, &mut entry);
            let used = std::mem::replace(self.rows.entry_mut(slot), entry);
            debug_assert!(used.glyphs.is_empty());
        }
    }

    /// The frame from the cached rows, with the overlays on top: rows'
    /// backgrounds, highlights, the block cursor, rows' decorations, rows'
    /// glyphs, other cursors and input-method text.
    fn assemble(&mut self, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        self.rects.clear();
        self.glyphs.clear();
        self.decorations.clear();
        let FrameInput {
            theme,
            metrics: m,
            padding,
            cursor_on,
            focused,
            preedit,
        } = *input;
        let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
        let (px, py) = (padding.0 as i32, padding.1 as i32);
        self.clear = with_alpha(theme.background, theme.background_opacity);
        let rows = &self.rows;
        let snap = &rows.snap;
        let cols = snap.cols as usize;
        let row_y = |line: usize| py + line as i32 * ch;
        let full = |line: usize, cells: std::ops::Range<usize>, color: Rgba| RectInstance {
            x: s16(px + cells.start as i32 * cw),
            y: s16(row_y(line)),
            w: ((cells.end - cells.start) as i32 * cw) as u16,
            h: ch as u16,
            color,
        };

        let c = snap.cursor;
        let in_view = c.at.map(|(l, c)| (l as usize, c as usize));
        let show = c.visible && (cursor_on || !focused || !c.blinking);
        let cursor = in_view.filter(|_| show).map(|at| CursorDraw {
            at,
            shape: c.shape,
            wide: c.wide,
        });
        // The focused block cursor recolours the text in its cells.
        let block = cursor
            .as_ref()
            .filter(|c| focused && c.shape == CursorShape::Block)
            .map(|c| (c.at.0, c.at.1..c.at.1 + if c.wide { 2 } else { 1 }));
        let preedit = preedit.filter(|t| !t.is_empty()).zip(in_view);
        let covered = preedit.map(|(t, (line, col))| {
            let width: usize = t.chars().map(char_cells).sum();
            (line, col..col + width)
        });
        let cells_on = |o: &Option<(usize, std::ops::Range<usize>)>, line: usize| {
            o.as_ref().filter(|(l, _)| *l == line).map(|(_, r)| r.clone()).unwrap_or(0..0)
        };

        // Backgrounds.
        for &slot in rows.order.iter().filter(|&&s| s != MISSING) {
            self.rects.extend_from_slice(&rows.entry(slot).backgrounds);
        }
        // Highlights, then the block cursor over them.
        for (line, row) in snap.rows.iter().enumerate() {
            for h in snap.row_highlights(row) {
                let (a, b) = (h.cols.start as usize, (h.cols.end as usize).min(cols));
                match h.kind {
                    _ if a >= b => {}
                    HighlightKind::Selection => self.rects.push(full(line, a..b, theme.selection)),
                    // Underlined in the text colour, over the backgrounds.
                    HighlightKind::HoveredLink => self.decorations.push(RectInstance {
                        y: s16(row_y(line) + (m.baseline + m.underline_offset) as i32),
                        h: m.underline_thickness.max(1) as u16,
                        ..full(line, a..b, theme.foreground)
                    }),
                    // Drawn once search (E.7) sets them.
                    HighlightKind::SearchHit | HighlightKind::CurrentSearchHit => {}
                }
            }
        }
        if let Some((line, cells)) = &block
            && cells.start < cols
        {
            self.rects.push(full(*line, cells.start..cells.end.min(cols), theme.cursor));
        }

        // Decorations and glyphs, recoloured under the block cursor and
        // hidden under input-method text.
        let under_cursor = |mark: Mark| {
            if mark.flags & Mark::DIM != 0 {
                with_alpha(theme.cursor_text, 160)
            } else {
                theme.cursor_text
            }
        };
        for (line, &slot) in rows.order.iter().enumerate() {
            if slot == MISSING {
                continue;
            }
            let e = rows.entry(slot);
            let on = cells_on(&block, line);
            if on.is_empty() {
                self.decorations.extend_from_slice(&e.decorations);
            } else {
                self.decorations
                    .extend(e.decorations.iter().zip(&e.decoration_marks).map(|(r, mark)| {
                        let recolour = mark.flags & Mark::TEXT_COLOUR != 0 && on.contains(&(mark.col as usize));
                        RectInstance {
                            color: if recolour { under_cursor(*mark) } else { r.color },
                            ..*r
                        }
                    }));
            }
        }
        for (line, &slot) in rows.order.iter().enumerate() {
            if slot == MISSING {
                continue;
            }
            let e = rows.entry(slot);
            let on = cells_on(&block, line);
            let hidden = cells_on(&covered, line);
            if on.is_empty() && hidden.is_empty() {
                self.glyphs.extend_from_slice(&e.glyphs);
            } else {
                for g in &e.glyphs {
                    let col = g.col();
                    if hidden.contains(&col) {
                        continue;
                    }
                    let color = match () {
                        _ if !on.contains(&col) => g.color,
                        _ if g.is_color & GlyphInstance::DIM != 0 => with_alpha(theme.cursor_text, 160),
                        _ => theme.cursor_text,
                    };
                    self.glyphs.push(GlyphInstance { color, ..*g });
                }
            }
        }

        if let Some(c) = &cursor
            && block.is_none()
        {
            self.cursor_rects(c, input);
        }
        if let Some((text, at)) = preedit {
            self.draw_preedit(text, at, cols, input, glyphs);
        }
        self.rects.extend_from_slice(&self.decorations);
    }

    /// The reference path: the frame read straight from the live terminal,
    /// with the cursor, selection and input-method text drawn into the
    /// rows, as before frames were built from snapshots. Kept for one PR
    /// so the tests can check that both give the same picture (§17.4).
    #[doc(hidden)]
    pub fn build_live(&mut self, term: &Terminal, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        self.rects.clear();
        self.glyphs.clear();
        self.decorations.clear();
        let FrameInput {
            theme,
            metrics: m,
            padding,
            cursor_on,
            focused,
            preedit,
        } = *input;
        let ch = m.cell_height as i32;
        let (px, py) = (padding.0 as i32, padding.1 as i32);
        self.clear = with_alpha(theme.background, theme.background_opacity);

        // Where the cursor is in the view, if it is visible at all.
        let in_view = term.cursor_in_view();
        let show_cursor = term.modes().cursor_visible && (cursor_on || !focused || !term.cursor_blink());
        let cursor = in_view.filter(|_| show_cursor).map(|at| CursorDraw {
            at,
            shape: term.cursor_shape(),
            wide: term.visible_row(at.0).cell(at.1).width() == Width::Wide,
        });
        let block_cursor = focused && term.cursor_shape() == CursorShape::Block;

        let selection = term.selection().map(|s| s.ordered());
        // Input-method text is drawn at the cursor, when it is in view.
        let preedit = preedit.filter(|t| !t.is_empty()).zip(in_view);
        // Cells covered by input-method text don't draw their own glyphs.
        let preedit_cells = preedit.map(|(t, (line, col))| {
            let width: usize = t.chars().map(char_cells).sum();
            (line, col..col + width)
        });

        let cols = term.cols();
        for line in 0..term.lines() {
            let row = term.visible_row(line);
            // Selected columns on this line, as a half-open range.
            let sel = selection
                .and_then(|(a, b)| {
                    let p = term.point_at(line, 0).line;
                    if p < a.line || p > b.line {
                        return None;
                    }
                    let start = if p == a.line { a.col } else { 0 };
                    let end = if p == b.line { b.col + 1 } else { usize::MAX };
                    Some(start..end)
                })
                .unwrap_or(0..0);
            let ctx = RowCtx {
                theme,
                m,
                px,
                y: py + line as i32 * ch,
                cols,
                sel,
                block_cursor: cursor
                    .as_ref()
                    .filter(|c| block_cursor && c.at.0 == line)
                    .map(|c| c.at.1..c.at.1 + if c.wide { 2 } else { 1 })
                    .unwrap_or(0..0),
                covered: preedit_cells
                    .as_ref()
                    .and_then(|(l, r)| (*l == line).then(|| r.clone()))
                    .unwrap_or(0..0),
            };
            let mut sink = Direct {
                rects: &mut self.rects,
                glyphs: &mut self.glyphs,
                decorations: &mut self.decorations,
            };
            build_row(&ctx, row.cells(), glyphs, &mut sink);
        }

        // Cursors other than a focused block are drawn on top as rectangles.
        if let Some(c) = &cursor
            && !block_cursor
        {
            self.cursor_rects(c, input);
        }
        if let Some((text, at)) = preedit {
            self.draw_preedit(text, at, cols, input, glyphs);
        }
        self.rects.extend_from_slice(&self.decorations);
    }

    /// A cursor other than a focused block: a bar, an underline or (when
    /// unfocused) a hollow box.
    fn cursor_rects(&mut self, c: &CursorDraw, input: &FrameInput) {
        let m = input.metrics;
        let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
        let (line, col) = c.at;
        let x = input.padding.0 as i32 + col as i32 * cw;
        let y = input.padding.1 as i32 + line as i32 * ch;
        let w = cw * if c.wide { 2 } else { 1 };
        let h = ch;
        let color = input.theme.cursor;
        let t = (m.cell_width as i32 / 8).max(2);
        let r = |x: i32, y: i32, w: i32, h: i32| RectInstance {
            x: s16(x),
            y: s16(y),
            w: w as u16,
            h: h as u16,
            color,
        };
        let rects: &[RectInstance] = match (input.focused, c.shape) {
            (true, CursorShape::Bar) => &[r(x, y, t, h)],
            (true, CursorShape::Underline) => &[r(x, y + h - t, w, t)],
            // Unfocused: a hollow box.
            _ => &[r(x, y, w, 1), r(x, y + h - 1, w, 1), r(x, y, 1, h), r(x + w - 1, y, 1, h)],
        };
        self.decorations.extend_from_slice(rects);
    }

    /// Input-method text at view row and column `at`.
    fn draw_preedit(&mut self, text: &str, at: (usize, usize), cols: usize, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        let m = input.metrics;
        let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
        let (line, mut col) = at;
        let y = input.padding.1 as i32 + line as i32 * ch;
        for c in text.chars() {
            let width = char_cells(c);
            if width == 0 {
                continue;
            }
            if col + width > cols {
                break;
            }
            let x = input.padding.0 as i32 + col as i32 * cw;
            let w = (cw * width as i32) as u16;
            self.decorations.push(RectInstance {
                x: s16(x),
                y: s16(y),
                w,
                h: ch as u16,
                color: input.theme.background,
            });
            let uy = y + (m.baseline + m.underline_offset) as i32;
            self.decorations.push(RectInstance {
                x: s16(x),
                y: s16(uy),
                w,
                h: m.underline_thickness as u16,
                color: input.theme.foreground,
            });
            if let Some(g) = glyphs.glyph(c as u32, FontStyle::Regular) {
                self.glyphs.push(GlyphInstance {
                    x: s16(x + g.left as i32),
                    y: s16(y + m.baseline as i32 - g.top as i32),
                    w: g.w,
                    h: g.h,
                    u: g.u,
                    v: g.v,
                    color: input.theme.foreground,
                    is_color: g.color as u32,
                });
            }
            col += width;
        }
    }
}

/// One row of cells, left to right from column 0.
#[inline]
fn build_row<C: CellView, S: RowSink>(ctx: &RowCtx, cells: impl Iterator<Item = C>, glyphs: &mut impl GlyphSource, out: &mut S) {
    // Most rows have no selection, block cursor or input-method text (and
    // rows built for the cache never do): they get a loop without those
    // checks.
    if ctx.sel.is_empty() && ctx.block_cursor.is_empty() && ctx.covered.is_empty() {
        row_cells::<false, C, S>(ctx, cells, glyphs, out)
    } else {
        row_cells::<true, C, S>(ctx, cells, glyphs, out)
    }
}

/// `build_row`, with `OVERLAY` false when the row has no selection, block
/// cursor or input-method text.
#[inline(always)]
fn row_cells<const OVERLAY: bool, C: CellView, S: RowSink>(
    ctx: &RowCtx,
    cells: impl Iterator<Item = C>,
    glyphs: &mut impl GlyphSource,
    out: &mut S,
) {
    let RowCtx { theme, m, px, y, .. } = *ctx;
    let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
    let default_bg = theme.background;
    let mut run_start = 0usize;
    let mut run_color = default_bg;
    // Runs of same-styled cells reuse the previous cell's pen: the style
    // is only read and resolved when it changes.
    let mut last_key = None;
    let mut pen = Pen::new(&term_core::Style::DEFAULT, theme);
    let mut end = 0;
    // Zipped with the columns rather than `take`n: a slice iterator
    // zipped with a range compiles to an indexed loop.
    for (cell, col) in cells.zip(0..ctx.cols) {
        let key = cell.key();
        if last_key != Some(key) {
            last_key = Some(key);
            pen = Pen::new(&cell.style(), theme);
        }
        let spacer = cell.is_spacer();
        let (mut fg, mut bg) = (pen.fg, pen.bg);
        if OVERLAY && ctx.sel.contains(&col) {
            bg = theme.selection;
        }
        if OVERLAY && ctx.block_cursor.contains(&col) {
            bg = theme.cursor;
            fg = theme.cursor_text;
        }
        if pen.attrs.contains(Attrs::DIM) {
            fg = with_alpha(fg, 160);
        }

        if bg != run_color {
            push_run(out, run_color, default_bg, run_start, col, px, y, cw, ch);
            run_start = col;
            run_color = bg;
        }

        let x = px + col as i32 * cw;
        // A cluster draws its first character until clusters are shaped
        // (F3.2).
        let c = match cell.content() {
            Content::Char(c) => Some(c),
            Content::Cluster(s) => s.chars().next(),
            Content::Empty | Content::Placeholder { .. } => None,
        };
        if let Some(c) = c
            && c != ' '
            && !spacer
            && !pen.attrs.contains(Attrs::HIDDEN)
            && !(OVERLAY && ctx.covered.contains(&col))
            && let Some(g) = glyphs.glyph(c as u32, pen.face())
        {
            out.glyph(GlyphInstance {
                x: s16(x + g.left as i32),
                y: s16(y + m.baseline as i32 - g.top as i32),
                w: g.w,
                h: g.h,
                u: g.u,
                v: g.v,
                color: fg,
                is_color: GlyphInstance::flags(&g, col, pen.attrs.contains(Attrs::DIM)),
            });
        }
        if pen.decorated {
            decorate(out, pen, theme, fg, x, y, m, pen.mark(col));
        }
        end = col + 1;
    }
    pad_row(out, ctx, end, run_start, run_color);
}

/// Ends a row whose cells stopped at column `from`, with the background
/// run so far. History rows can be shorter than the screen is wide: the
/// rest is blank, which only shows a background where it is selected or
/// under a block cursor.
fn pad_row(out: &mut impl RowSink, ctx: &RowCtx, from: usize, mut run_start: usize, mut run_color: Rgba) {
    let RowCtx { theme, m, px, y, .. } = *ctx;
    let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
    let default_bg = theme.background;
    if ctx.sel.is_empty() && ctx.block_cursor.is_empty() {
        // Nothing but the default background past `from`.
        if run_color != default_bg {
            push_run(out, run_color, default_bg, run_start, from, px, y, cw, ch);
        }
        return;
    }
    for col in from..ctx.cols {
        let bg = if ctx.block_cursor.contains(&col) {
            theme.cursor
        } else if ctx.sel.contains(&col) {
            theme.selection
        } else {
            default_bg
        };
        if bg != run_color {
            push_run(out, run_color, default_bg, run_start, col, px, y, cw, ch);
            run_start = col;
            run_color = bg;
        }
    }
    push_run(out, run_color, default_bg, run_start, ctx.cols, px, y, cw, ch);
}

/// Underline, overline and strike for one cell whose top-left corner is
/// (`x`, `y`). `fg` is the text colour as drawn (after inverse, the block
/// cursor and dim). Underlines take the style's underline colour when it
/// has one (§10.5).
#[allow(clippy::too_many_arguments)]
fn decorate(out: &mut impl RowSink, pen: Pen, theme: &Theme, fg: Rgba, x: i32, y: i32, m: Metrics, mark: Mark) {
    let cw = m.cell_width as i32;
    let t = m.underline_thickness.max(1) as i32;
    let text = Mark {
        flags: mark.flags | Mark::TEXT_COLOUR,
        ..mark
    };
    let mut rect = |x: i32, y: i32, w: i32, color: Rgba, mark: Mark| {
        out.decoration(
            RectInstance {
                x: s16(x),
                y: s16(y),
                w: w as u16,
                h: t as u16,
                color,
            },
            mark,
        )
    };
    if pen.underline != Underline::None {
        let uy = y + (m.baseline + m.underline_offset) as i32;
        let (color, mark) = match pen.underline_color.kind() {
            ColorKind::Default => (fg, text),
            _ => (theme.resolve(pen.underline_color, fg), mark),
        };
        let mut rect = |x, y, w| rect(x, y, w, color, mark);
        // Curly, dotted and dashed are drawn from rectangles until
        // term-font's pattern sprites land (F2.3, §10.5). Then each of
        // them becomes one sprite quad per cell, pushed here.
        match pen.underline {
            Underline::None => {}
            Underline::Single => rect(x, uy, cw),
            Underline::Double => {
                rect(x, uy, cw);
                rect(x, uy + 2 * t, cw);
            }
            Underline::Dotted => {
                // Square dots a thickness wide, a thickness apart.
                let mut dx = 0;
                while dx < cw {
                    rect(x + dx, uy, t.min(cw - dx));
                    dx += 2 * t;
                }
            }
            Underline::Dashed => {
                // One dash per cell, centred so the gaps line up across
                // cells: three quarters of the cell, then a gap.
                let gap = (cw / 4).max(1);
                rect(x + gap / 2, uy, cw - gap);
            }
            Underline::Curly => {
                // One period of a wave per cell, in steps a thickness
                // above and below the underline.
                for (i, dy) in CURLY_WAVE.iter().enumerate() {
                    let x0 = i as i32 * cw / CURLY_SEGMENTS;
                    let x1 = (i as i32 + 1) * cw / CURLY_SEGMENTS;
                    if x1 > x0 {
                        rect(x + x0, uy + dy * t, x1 - x0);
                    }
                }
            }
        }
    }
    if pen.attrs.contains(Attrs::OVERLINE) {
        rect(x, y, cw, fg, text);
    }
    if pen.attrs.contains(Attrs::STRIKE) {
        rect(x, y + m.baseline as i32 - m.strike_offset as i32, cw, fg, text);
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn push_run(out: &mut impl RowSink, color: Rgba, default_bg: Rgba, start: usize, end: usize, px: i32, y: i32, cw: i32, ch: i32) {
    if color == default_bg || end <= start {
        return;
    }
    out.background(RectInstance {
        x: s16(px + start as i32 * cw),
        y: s16(y),
        w: ((end - start) as i32 * cw) as u16,
        h: ch as u16,
        color,
    });
}

/// Rows built from `Style`s directly, for what term-core can't set yet
/// (underline colour, curly, dotted and dashed underlines, overline). The
/// rest of frame building is tested through term-core in `tests/frame.rs`.
#[cfg(test)]
mod tests {
    use super::*;
    use term_core::{Color, Style};

    const M: Metrics = Metrics {
        cell_width: 16,
        cell_height: 20,
        baseline: 14,
        underline_offset: 2,
        underline_thickness: 2,
        strike_offset: 4,
    };
    /// Where underlines start: the top of the underline in row 0.
    const UY: i16 = (M.baseline + M.underline_offset) as i16;
    const T: u16 = M.underline_thickness as u16;

    #[derive(Clone, Copy)]
    struct TestCell(char, Style);

    impl CellView for TestCell {
        type Key = Style;
        fn key(&self) -> Style {
            self.1
        }
        fn style(&self) -> Style {
            self.1
        }
        fn content(&self) -> Content<'_> {
            Content::Char(self.0)
        }
        fn is_spacer(&self) -> bool {
            false
        }
    }

    struct NoGlyphs;
    impl GlyphSource for NoGlyphs {
        fn glyph(&mut self, _: u32, _: FontStyle) -> Option<GlyphSlot> {
            None
        }
    }

    /// The decorations of one row of `cells` at the top-left of the grid.
    fn decorations(theme: &Theme, cells: &[TestCell]) -> Vec<RectInstance> {
        let ctx = RowCtx {
            theme,
            m: M,
            px: 0,
            y: 0,
            cols: cells.len(),
            sel: 0..0,
            block_cursor: 0..0,
            covered: 0..0,
        };
        let mut e = Entry::default();
        build_row(&ctx, cells.iter().copied(), &mut NoGlyphs, &mut e);
        e.decorations
    }

    fn styled(f: impl FnOnce(&mut Style)) -> Style {
        let mut s = Style::DEFAULT;
        f(&mut s);
        s
    }

    fn line(x: i16, y: i16, w: u16, color: Rgba) -> RectInstance {
        RectInstance { x, y, w, h: T, color }
    }

    #[test]
    fn plain_text_draws_no_decorations() {
        assert_eq!(decorations(&Theme::default(), &[TestCell('a', Style::DEFAULT)]), []);
    }

    #[test]
    fn underline_takes_its_own_colour_when_set() {
        let theme = Theme::default();
        let red = styled(|s| {
            s.underline = Underline::Single;
            s.underline_color = Color::rgb(255, 0, 0);
        });
        let palette = styled(|s| {
            s.underline = Underline::Double;
            s.underline_color = Color::indexed(4);
        });
        assert_eq!(
            decorations(&theme, &[TestCell('a', red), TestCell('b', palette)]),
            [
                line(0, UY, 16, rgb(0xff0000)),
                line(16, UY, 16, theme.palette[4]),
                line(16, UY + 2 * T as i16, 16, theme.palette[4]),
            ]
        );
    }

    #[test]
    fn underline_without_a_colour_takes_the_text_colour() {
        let theme = Theme::default();
        let green = styled(|s| {
            s.underline = Underline::Single;
            s.fg = Color::indexed(2);
        });
        // Inverse: the text colour is the default background.
        let inverse = styled(|s| {
            s.underline = Underline::Single;
            s.attrs = Attrs::INVERSE;
        });
        let dim = styled(|s| {
            s.underline = Underline::Single;
            s.attrs = Attrs::DIM;
        });
        assert_eq!(
            decorations(&theme, &[TestCell('a', green), TestCell('b', inverse), TestCell('c', dim)]),
            [
                line(0, UY, 16, theme.palette[2]),
                line(16, UY, 16, theme.background),
                line(32, UY, 16, with_alpha(theme.foreground, 160)),
            ]
        );
    }

    #[test]
    fn dotted_underline_is_square_dots_a_thickness_apart() {
        let theme = Theme::default();
        let s = styled(|s| s.underline = Underline::Dotted);
        let fg = theme.foreground;
        // 16 pixels, dots 2 wide every 4: at 0, 4, 8 and 12.
        let want: Vec<_> = (0..4).map(|i| line(i * 4, UY, T, fg)).collect();
        assert_eq!(decorations(&theme, &[TestCell('a', s)]), want);
    }

    #[test]
    fn dashed_underline_is_one_centred_dash_per_cell() {
        let theme = Theme::default();
        let s = styled(|s| s.underline = Underline::Dashed);
        let fg = theme.foreground;
        assert_eq!(
            decorations(&theme, &[TestCell('a', s), TestCell('b', s)]),
            [line(2, UY, 12, fg), line(18, UY, 12, fg)]
        );
    }

    #[test]
    fn curly_underline_is_a_wave_across_the_cell() {
        let theme = Theme::default();
        let blue = Color::rgb(0, 0, 255);
        let s = styled(|s| {
            s.underline = Underline::Curly;
            s.underline_color = blue;
        });
        let d = decorations(&theme, &[TestCell('a', s)]);
        // Eight steps of 2 pixels, edge to edge, in the underline colour.
        assert_eq!(d.len(), 8);
        assert!(d.iter().all(|r| r.w == 2 && r.h == T && r.color == rgb(0x0000ff)));
        let xs: Vec<_> = d.iter().map(|r| r.x).collect();
        assert_eq!(xs, [0, 2, 4, 6, 8, 10, 12, 14]);
        // Up a thickness, back, down a thickness, back; inside the cell.
        let ys: Vec<_> = d.iter().map(|r| r.y - UY).collect();
        let t = T as i16;
        assert_eq!(ys, [0, -t, -t, 0, 0, t, t, 0]);
        assert!(d.iter().all(|r| r.y >= 0 && r.y + r.h as i16 <= M.cell_height as i16));
    }

    #[test]
    fn curly_underline_on_narrow_cells_skips_empty_steps() {
        let theme = Theme::default();
        let m = Metrics { cell_width: 5, ..M };
        let s = styled(|s| s.underline = Underline::Curly);
        let ctx = RowCtx {
            theme: &theme,
            m,
            px: 0,
            y: 0,
            cols: 1,
            sel: 0..0,
            block_cursor: 0..0,
            covered: 0..0,
        };
        let mut e = Entry::default();
        build_row(&ctx, [TestCell('a', s)].into_iter(), &mut NoGlyphs, &mut e);
        assert!(e.decorations.iter().all(|r| r.w > 0));
        assert_eq!(e.decorations.iter().map(|r| r.w).sum::<u16>(), 5);
    }

    #[test]
    fn overline_and_strike_take_the_text_colour_not_the_underline_colour() {
        let theme = Theme::default();
        let s = styled(|s| {
            s.attrs = Attrs::OVERLINE | Attrs::STRIKE;
            s.underline = Underline::Single;
            s.underline_color = Color::rgb(255, 0, 0);
            s.fg = Color::indexed(3);
        });
        let fg = theme.palette[3];
        let strike_y = (M.baseline - M.strike_offset) as i16;
        assert_eq!(
            decorations(&theme, &[TestCell('a', s)]),
            [line(0, UY, 16, rgb(0xff0000)), line(0, 0, 16, fg), line(0, strike_y, 16, fg)]
        );
    }

    #[test]
    fn overline_alone_on_an_empty_cell() {
        let theme = Theme::default();
        let s = styled(|s| s.attrs = Attrs::OVERLINE);
        assert_eq!(decorations(&theme, &[TestCell(' ', s)]), [line(0, 0, 16, theme.foreground)]);
    }
}
