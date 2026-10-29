//! Turns terminal state into GPU instances. No Metal here, so it can be
//! tested and benchmarked on its own.

use term_core::{Attrs, CellRef, ColorKind, Content, CursorShape, StyleKey, Terminal, Underline, Width};
use term_font::{Metrics, Style as FontStyle};
use unicode_width::UnicodeWidthChar;

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
    pub is_color: u32,
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
}

#[derive(Clone, Debug)]
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
    /// Selected columns, half-open; empty when none are.
    sel: std::ops::Range<usize>,
    /// The block cursor's column, when it is drawn on this row. Other
    /// cursors are drawn on top of the frame, not in the row.
    block_cursor: Option<usize>,
    /// Columns covered by input-method text, which draw no glyphs.
    covered: std::ops::Range<usize>,
}

/// A cell as frame building reads it: `CellRef`, and in tests cells built
/// from any `Style`.
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
}

pub struct FrameInput<'a> {
    pub term: &'a Terminal,
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

#[derive(Default)]
pub struct Frame {
    pub clear: Rgba,
    pub rects: Vec<RectInstance>,
    pub glyphs: Vec<GlyphInstance>,
    /// Underlines, strikethrough and cursor; appended after the backgrounds.
    decorations: Vec<RectInstance>,
}

impl Frame {
    pub fn build(&mut self, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        self.rects.clear();
        self.glyphs.clear();
        self.decorations.clear();
        let FrameInput {
            term,
            theme,
            metrics: m,
            padding,
            cursor_on,
            focused,
            preedit,
        } = *input;
        let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
        let (px, py) = (padding.0 as i32, padding.1 as i32);
        let default_bg = theme.background;
        self.clear = with_alpha(default_bg, theme.background_opacity);

        // Where the cursor is in the view, if it is visible at all.
        let in_view = term.cursor_in_view();
        let show_cursor = term.modes().cursor_visible && (cursor_on || !focused || !term.cursor_blink());
        let cursor = in_view.filter(|_| show_cursor);
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
                block_cursor: cursor.and_then(|(l, c)| (block_cursor && l == line).then_some(c)),
                covered: preedit_cells
                    .as_ref()
                    .and_then(|(l, r)| (*l == line).then(|| r.clone()))
                    .unwrap_or(0..0),
            };
            self.build_row(&ctx, row.cells(), glyphs);
        }

        // Cursors other than a focused block are drawn on top as rectangles.
        if let Some((line, col)) = cursor
            && !block_cursor
        {
            let wide = term.visible_row(line).cell(col).width() == Width::Wide;
            let x = px + col as i32 * cw;
            let y = py + line as i32 * ch;
            let w = cw * if wide { 2 } else { 1 };
            let h = ch;
            let c = theme.cursor;
            let t = (m.cell_width as i32 / 8).max(2);
            let r = |x: i32, y: i32, w: i32, h: i32| RectInstance {
                x: s16(x),
                y: s16(y),
                w: w as u16,
                h: h as u16,
                color: c,
            };
            let rects: &[RectInstance] = match (focused, term.cursor_shape()) {
                (true, CursorShape::Bar) => &[r(x, y, t, h)],
                (true, CursorShape::Underline) => &[r(x, y + h - t, w, t)],
                // Unfocused: a hollow box.
                _ => &[r(x, y, w, 1), r(x, y + h - 1, w, 1), r(x, y, 1, h), r(x + w - 1, y, 1, h)],
            };
            self.decorations.extend_from_slice(rects);
        }
        if let Some((text, at)) = preedit {
            self.draw_preedit(text, at, input, glyphs);
        }
        self.rects.extend_from_slice(&self.decorations);
    }

    /// One row of cells, left to right from column 0.
    #[inline]
    fn build_row<C: CellView>(&mut self, ctx: &RowCtx, cells: impl Iterator<Item = C>, glyphs: &mut impl GlyphSource) {
        // Most rows have no selection, block cursor or input-method text:
        // they get a loop without those checks.
        if ctx.sel.is_empty() && ctx.block_cursor.is_none() && ctx.covered.is_empty() {
            self.row_cells::<false, C>(ctx, cells, glyphs)
        } else {
            self.row_cells::<true, C>(ctx, cells, glyphs)
        }
    }

    /// `build_row`, with `OVERLAY` false when the row has no selection,
    /// block cursor or input-method text.
    #[inline(always)]
    fn row_cells<const OVERLAY: bool, C: CellView>(&mut self, ctx: &RowCtx, cells: impl Iterator<Item = C>, glyphs: &mut impl GlyphSource) {
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
            if OVERLAY && ctx.block_cursor.is_some_and(|c| c == col || (spacer && c + 1 == col)) {
                bg = theme.cursor;
                fg = theme.cursor_text;
            }
            if pen.attrs.contains(Attrs::DIM) {
                fg = with_alpha(fg, 160);
            }

            if bg != run_color {
                self.push_run(run_color, default_bg, run_start, col, px, y, cw, ch);
                run_start = col;
                run_color = bg;
            }

            let x = px + col as i32 * cw;
            if let Content::Char(c) = cell.content()
                && c != ' '
                && !spacer
                && !pen.attrs.contains(Attrs::HIDDEN)
                && !(OVERLAY && ctx.covered.contains(&col))
                && let Some(g) = glyphs.glyph(c as u32, pen.face())
            {
                self.glyphs.push(GlyphInstance {
                    x: s16(x + g.left as i32),
                    y: s16(y + m.baseline as i32 - g.top as i32),
                    w: g.w,
                    h: g.h,
                    u: g.u,
                    v: g.v,
                    color: fg,
                    is_color: g.color as u32,
                });
            }
            if pen.decorated {
                self.decorate(pen, theme, fg, x, y, m);
            }
            end = col + 1;
        }
        self.pad_row(ctx, end, run_start, run_color);
    }

    /// Ends a row whose cells stopped at column `from`, with the background
    /// run so far. History rows can be shorter than the screen is wide: the
    /// rest is blank, which only shows a background where it is selected or
    /// under a block cursor.
    fn pad_row(&mut self, ctx: &RowCtx, from: usize, mut run_start: usize, mut run_color: Rgba) {
        let RowCtx { theme, m, px, y, .. } = *ctx;
        let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
        let default_bg = theme.background;
        for col in from..ctx.cols {
            let bg = if ctx.block_cursor == Some(col) {
                theme.cursor
            } else if ctx.sel.contains(&col) {
                theme.selection
            } else {
                default_bg
            };
            if bg != run_color {
                self.push_run(run_color, default_bg, run_start, col, px, y, cw, ch);
                run_start = col;
                run_color = bg;
            }
        }
        self.push_run(run_color, default_bg, run_start, ctx.cols, px, y, cw, ch);
    }

    /// Underline, overline and strike for one cell whose top-left corner is
    /// (`x`, `y`). `fg` is the text colour as drawn (after inverse, the
    /// block cursor and dim). Underlines take the style's underline colour
    /// when it has one (§10.5).
    fn decorate(&mut self, pen: Pen, theme: &Theme, fg: Rgba, x: i32, y: i32, m: Metrics) {
        let cw = m.cell_width as i32;
        let t = m.underline_thickness.max(1) as i32;
        let mut rect = |x: i32, y: i32, w: i32, color: Rgba| {
            self.decorations.push(RectInstance {
                x: s16(x),
                y: s16(y),
                w: w as u16,
                h: t as u16,
                color,
            })
        };
        if pen.underline != Underline::None {
            let uy = y + (m.baseline + m.underline_offset) as i32;
            let color = theme.resolve(pen.underline_color, fg);
            // Curly, dotted and dashed are drawn from rectangles until
            // term-font's pattern sprites land (F2.3, §10.5). Then each of
            // them becomes one sprite quad per cell, pushed here.
            match pen.underline {
                Underline::None => {}
                Underline::Single => rect(x, uy, cw, color),
                Underline::Double => {
                    rect(x, uy, cw, color);
                    rect(x, uy + 2 * t, cw, color);
                }
                Underline::Dotted => {
                    // Square dots a thickness wide, a thickness apart.
                    let mut dx = 0;
                    while dx < cw {
                        rect(x + dx, uy, t.min(cw - dx), color);
                        dx += 2 * t;
                    }
                }
                Underline::Dashed => {
                    // One dash per cell, centred so the gaps line up across
                    // cells: three quarters of the cell, then a gap.
                    let gap = (cw / 4).max(1);
                    rect(x + gap / 2, uy, cw - gap, color);
                }
                Underline::Curly => {
                    // One period of a wave per cell, in steps a thickness
                    // above and below the underline.
                    for (i, dy) in CURLY_WAVE.iter().enumerate() {
                        let x0 = i as i32 * cw / CURLY_SEGMENTS;
                        let x1 = (i as i32 + 1) * cw / CURLY_SEGMENTS;
                        if x1 > x0 {
                            rect(x + x0, uy + dy * t, x1 - x0, color);
                        }
                    }
                }
            }
        }
        if pen.attrs.contains(Attrs::OVERLINE) {
            rect(x, y, cw, fg);
        }
        if pen.attrs.contains(Attrs::STRIKE) {
            rect(x, y + m.baseline as i32 - m.strike_offset as i32, cw, fg);
        }
    }

    /// Input-method text at view row and column `at`.
    fn draw_preedit(&mut self, text: &str, at: (usize, usize), input: &FrameInput, glyphs: &mut impl GlyphSource) {
        let m = input.metrics;
        let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
        let (line, mut col) = at;
        let y = input.padding.1 as i32 + line as i32 * ch;
        for c in text.chars() {
            let width = char_cells(c);
            if width == 0 {
                continue;
            }
            if col + width > input.term.cols() {
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

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn push_run(&mut self, color: Rgba, default_bg: Rgba, start: usize, end: usize, px: i32, y: i32, cw: i32, ch: i32) {
        if color == default_bg || end <= start {
            return;
        }
        self.rects.push(RectInstance {
            x: s16(px + start as i32 * cw),
            y: s16(y),
            w: ((end - start) as i32 * cw) as u16,
            h: ch as u16,
            color,
        });
    }
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
            block_cursor: None,
            covered: 0..0,
        };
        let mut f = Frame::default();
        f.build_row(&ctx, cells.iter().copied(), &mut NoGlyphs);
        f.decorations
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
            block_cursor: None,
            covered: 0..0,
        };
        let mut f = Frame::default();
        f.build_row(&ctx, [TestCell('a', s)].into_iter(), &mut NoGlyphs);
        assert!(f.decorations.iter().all(|r| r.w > 0));
        assert_eq!(f.decorations.iter().map(|r| r.w).sum::<u16>(), 5);
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
