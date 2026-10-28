//! Turns terminal state into GPU instances. No Metal here, so it can be
//! tested and benchmarked on its own.

use term_core::{Cell, ColorKind, CursorShape, Terminal, flags};
use term_font::{Metrics, Style};

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
    fn glyph(&mut self, codepoint: u32, style: Style) -> Option<GlyphSlot>;
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
        let cursor_line = term.cursor.row + term.display_offset();
        let show_cursor = term.modes.cursor_visible && cursor_line < term.lines() && (cursor_on || !focused || !term.cursor_blink);
        let cursor = show_cursor.then_some((cursor_line, term.cursor.col));
        let block_cursor = focused && term.cursor_shape == CursorShape::Block;

        let selection = term.selection.map(|s| s.ordered());
        // Cells covered by input-method text don't draw their own glyphs.
        let preedit_cells = preedit.filter(|t| !t.is_empty()).map(|t| {
            let line = term.cursor.row + term.display_offset();
            let width: usize = t.chars().map(|c| if (c as u32) < 0x1100 { 1 } else { 2 }).sum();
            (line, term.cursor.col..term.cursor.col + width)
        });

        for line in 0..term.lines() {
            let row = term.visible_row(line);
            let y = py + line as i32 * ch;
            // Selected columns on this line, as a half-open range.
            let sel = selection.and_then(|(a, b)| {
                let p = term.visible_point(line, 0).line;
                if p < a.line || p > b.line {
                    return None;
                }
                let start = if p == a.line { a.col } else { 0 };
                let end = if p == b.line { b.col + 1 } else { usize::MAX };
                Some(start..end)
            });

            let mut run_start = 0usize;
            let mut run_color = default_bg;
            for col in 0..term.cols() {
                let cell = row.cells.get(col).copied().unwrap_or(Cell::BLANK);
                let mut fg = theme.resolve(cell.fg, theme.foreground);
                let mut bg = theme.resolve(cell.bg, default_bg);
                if cell.has(flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if sel.as_ref().is_some_and(|r| r.contains(&col)) {
                    bg = theme.selection;
                }
                let at_cursor = cursor.is_some_and(|(l, c)| l == line && (c == col || (cell.has(flags::WIDE_SPACER) && c + 1 == col)));
                if at_cursor && block_cursor {
                    bg = theme.cursor;
                    fg = theme.cursor_text;
                }
                if cell.has(flags::DIM) {
                    fg = with_alpha(fg, 160);
                }

                if bg != run_color {
                    self.push_run(run_color, default_bg, run_start, col, px, y, cw, ch);
                    run_start = col;
                    run_color = bg;
                }

                let x = px + col as i32 * cw;
                let cp = cell.codepoint();
                let covered = preedit_cells.as_ref().is_some_and(|(l, r)| *l == line && r.contains(&col));
                if cp != 0 && cp != ' ' as u32 && !cell.has(flags::WIDE_SPACER) && !cell.has(flags::HIDDEN) && !covered {
                    let style = Style::new(cell.has(flags::BOLD), cell.has(flags::ITALIC));
                    if let Some(g) = glyphs.glyph(cp, style) {
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
                }
                if cell.has(flags::UNDERLINE | flags::DOUBLE_UNDERLINE) {
                    let uy = y + (m.baseline + m.underline_offset) as i32;
                    let t = m.underline_thickness as u16;
                    self.decorations.push(RectInstance {
                        x: s16(x),
                        y: s16(uy),
                        w: cw as u16,
                        h: t,
                        color: fg,
                    });
                    if cell.has(flags::DOUBLE_UNDERLINE) {
                        self.decorations.push(RectInstance {
                            x: s16(x),
                            y: s16(uy + 2 * t as i32),
                            w: cw as u16,
                            h: t,
                            color: fg,
                        });
                    }
                }
                if cell.has(flags::STRIKE) {
                    let sy = y + m.baseline as i32 - m.strike_offset as i32;
                    self.decorations.push(RectInstance {
                        x: s16(x),
                        y: s16(sy),
                        w: cw as u16,
                        h: m.underline_thickness as u16,
                        color: fg,
                    });
                }
            }
            self.push_run(run_color, default_bg, run_start, term.cols(), px, y, cw, ch);
        }

        // Cursors other than a focused block are drawn on top as rectangles.
        if let Some((line, col)) = cursor
            && !block_cursor
        {
            let wide = term.visible_row(line).get(col).has(flags::WIDE);
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
            let rects: &[RectInstance] = match (focused, term.cursor_shape) {
                (true, CursorShape::Bar) => &[r(x, y, t, h)],
                (true, CursorShape::Underline) => &[r(x, y + h - t, w, t)],
                // Unfocused: a hollow box.
                _ => &[r(x, y, w, 1), r(x, y + h - 1, w, 1), r(x, y, 1, h), r(x + w - 1, y, 1, h)],
            };
            self.decorations.extend_from_slice(rects);
        }
        if let Some(text) = preedit.filter(|t| !t.is_empty()) {
            self.draw_preedit(text, input, glyphs);
        }
        self.rects.extend_from_slice(&self.decorations);
    }

    fn draw_preedit(&mut self, text: &str, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        let m = input.metrics;
        let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
        let line = input.term.cursor.row + input.term.display_offset();
        let y = input.padding.1 as i32 + line as i32 * ch;
        let mut col = input.term.cursor.col;
        for c in text.chars() {
            let width = if (c as u32) < 0x1100 { 1 } else { 2 };
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
            if let Some(g) = glyphs.glyph(c as u32, Style::Regular) {
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
