//! Turns terminal state into GPU instances. No Metal here, so it can be
//! tested and benchmarked on its own.

use term_core::{Attrs, Cell, CellRef, ColorKind, Content, CursorShape, HighlightKind, RowFlags, StyleKey, Terminal, Underline, Width};
use term_font::{Features, Fonts, GlyphKey, Metrics, RunText, SPRITE_FONT, ShapedGlyph, SpriteId, Style as FontStyle};
use unicode_width::UnicodeWidthChar;

use std::collections::HashMap;
use std::hash::BuildHasherDefault;

use crate::images::{Grid, Images, pieces_of_row};
use crate::row_cache::{Built, Entry, KeyHasher, MISSING, Mark, RowCache};
use crate::shape_cache::ShapeCache;
use crate::transition::slot;

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
pub(crate) fn char_cells(c: char) -> usize {
    c.width().unwrap_or(0)
}

fn with_alpha(c: Rgba, a: u8) -> Rgba {
    (c & 0x00FF_FFFF) | (a as u32) << 24
}

/// Relative luminance (WCAG 2) of a colour, alpha ignored.
fn luminance(c: Rgba) -> f32 {
    let lin = |v: u32| {
        let v = (v & 0xFF) as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c) + 0.7152 * lin(c >> 8) + 0.0722 * lin(c >> 16)
}

/// The WCAG 2 contrast ratio of two colours: 1 (the same) to 21 (black
/// and white).
pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The least contrast the selection has against the background: below
/// it a selection of a character or two is hard to see. Most themes'
/// own selection colours are well above it.
pub const SELECTION_CONTRAST: f32 = 1.5;
/// The least contrast the theme's text keeps on the selection while the
/// selection is made stronger.
pub const SELECTED_TEXT_CONTRAST: f32 = 3.0;
/// The contrast the text under a block cursor gets when the background
/// colour (its usual colour) can't give it.
pub const CURSOR_TEXT_CONTRAST: f32 = 4.5;
/// The least contrast the cursor has against the selection, so a cursor
/// inside a selection (drawn over it) is still seen. Built-in themes'
/// light orange cursors on light selections stand out by hue from about
/// 1.4.
pub const CURSOR_ON_SELECTION_CONTRAST: f32 = 1.4;

/// `cursor` made visible on `selection`: moved towards black or white
/// (whichever is further from the selection) until it has
/// [`CURSOR_ON_SELECTION_CONTRAST`]; unchanged if it has it already.
fn visible_cursor(cursor: Rgba, selection: Rgba) -> Rgba {
    if contrast(cursor, selection) >= CURSOR_ON_SELECTION_CONTRAST {
        return cursor;
    }
    let far = if luminance(selection) > 0.18 {
        rgb(0x000000)
    } else {
        rgb(0xffffff)
    };
    (1..=20)
        .map(|step| rgb_mix(cursor, far, step as f32 / 20.0))
        .find(|c| contrast(*c, selection) >= CURSOR_ON_SELECTION_CONTRAST)
        .unwrap_or(far)
}

/// `selection` made visible on `bg`: moved towards `fg` until it has
/// [`SELECTION_CONTRAST`] against `bg`, as far as `fg` keeps
/// [`SELECTED_TEXT_CONTRAST`] on it (or what it had, if less).
fn visible_selection(selection: Rgba, bg: Rgba, fg: Rgba) -> Rgba {
    if contrast(selection, bg) >= SELECTION_CONTRAST {
        return selection;
    }
    let text_floor = SELECTED_TEXT_CONTRAST.min(contrast(fg, selection));
    let mut best = selection;
    for step in 1..=50 {
        let c = rgb_mix(selection, fg, step as f32 / 100.0);
        if contrast(fg, c) < text_floor {
            break;
        }
        best = c;
        if contrast(c, bg) >= SELECTION_CONTRAST {
            break;
        }
    }
    best
}

/// The text colour on a block `cursor`: the background where that reads
/// ([`CURSOR_TEXT_CONTRAST`]), else the foreground, else whichever of
/// those, black and white reads best.
fn cursor_text_on(cursor: Rgba, bg: Rgba, fg: Rgba) -> Rgba {
    for c in [bg, fg] {
        if contrast(c, cursor) >= CURSOR_TEXT_CONTRAST {
            return c;
        }
    }
    [bg, fg, rgb(0x000000), rgb(0xffffff)]
        .into_iter()
        .max_by(|a, b| contrast(*a, cursor).total_cmp(&contrast(*b, cursor)))
        .unwrap_or(bg)
}

/// [`mix`] for theme colours (alpha opaque).
fn rgb_mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    mix(a & 0x00FF_FFFF, b & 0x00FF_FFFF, t) | 0xFF << 24
}

/// Where an instance's colour comes from, for theme transitions: 0 for a
/// colour of its own (truecolour, the 256-colour cube past the 16 ANSI
/// colours, a colour a program set with OSC 4, 10–12), else 1 + the
/// theme's slot in [`crate::transition::SLOTS`]. A transition blends a
/// tagged colour from the old theme's colour in that slot; an untagged
/// one stays as it is.
pub type Tag = u32;

/// The tag of transition slot `slot`.
#[inline]
pub const fn tag_of(slot: usize) -> Tag {
    slot as Tag + 1
}

/// A colour as frame building carries it: what it is and where it comes
/// from. Two paints are the same only when both are, so a truecolour
/// background equal to the theme's is still drawn, and stays put.
#[derive(Clone, Copy, Debug, Default, Eq)]
pub(crate) struct Paint {
    pub rgba: Rgba,
    pub tag: Tag,
}

impl PartialEq for Paint {
    /// One comparison, not two: it is in frame building's inner loop.
    #[inline]
    fn eq(&self, other: &Paint) -> bool {
        self.bits() == other.bits()
    }
}

impl Paint {
    #[inline]
    fn bits(self) -> u64 {
        self.rgba as u64 | (self.tag as u64) << 32
    }

    #[inline]
    fn alpha(self, a: u8) -> Paint {
        Paint {
            rgba: with_alpha(self.rgba, a),
            ..self
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectInstance {
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
    pub color: Rgba,
    /// Where `color` comes from ([`Tag`]).
    pub tag: Tag,
}

impl RectInstance {
    /// A rectangle in `paint`.
    #[inline]
    pub(crate) fn painted(x: i16, y: i16, w: u16, h: u16, paint: Paint) -> RectInstance {
        RectInstance {
            x,
            y,
            w,
            h,
            color: paint.rgba,
            tag: paint.tag,
        }
    }
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
    /// Bits 8–12: where `color` comes from ([`Tag`]). The shader reads
    /// only those; frame building keeps the glyph's cell and flags in the
    /// others, for the overlays drawn over cached rows.
    pub is_color: u32,
}

impl GlyphInstance {
    pub const COLOR: u32 = 1;
    /// The cell is dim (its text colour has a lower alpha).
    const DIM: u32 = 1 << 1;
    /// Keeps its colour under the block cursor: an underline pattern in
    /// the underline colour the style sets, as its rectangles would.
    const FIXED: u32 = 1 << 2;
    pub const TAG_SHIFT: u32 = 8;
    pub const TAG_MASK: u32 = 0x1F << Self::TAG_SHIFT;
    const COL_SHIFT: u32 = 16;

    /// `is_color` for a glyph from `slot` in column `col`, in a colour
    /// with tag `tag`.
    #[inline]
    fn flags(slot: &GlyphSlot, col: usize, dim: bool, tag: Tag) -> u32 {
        slot.color as u32 | (dim as u32) << 1 | tag << Self::TAG_SHIFT | (col.min(u16::MAX as usize) as u32) << Self::COL_SHIFT
    }

    /// Where its colour comes from.
    #[inline]
    pub fn tag(&self) -> Tag {
        (self.is_color & Self::TAG_MASK) >> Self::TAG_SHIFT
    }

    /// The glyph in `paint`.
    #[inline]
    fn painted(self, paint: Paint) -> GlyphInstance {
        GlyphInstance {
            color: paint.rgba,
            is_color: self.is_color & !Self::TAG_MASK | paint.tag << Self::TAG_SHIFT,
            ..self
        }
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
    assert!(size_of::<RectInstance>() == 16);
    assert!(offset_of!(RectInstance, x) == 0);
    assert!(offset_of!(RectInstance, w) == 4);
    assert!(offset_of!(RectInstance, color) == 8);
    assert!(offset_of!(RectInstance, tag) == 12);
    // A tag fits its bits in `is_color`.
    assert!(tag_of(crate::transition::SLOTS - 1) <= GlyphInstance::TAG_MASK >> GlyphInstance::TAG_SHIFT);
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

    /// The slot for a glyph's key: the glyphs of cluster cells, ligatures,
    /// contextual alternates and the other glyphs of cells shaped with
    /// them, sprites across two cells (ambiguous code-drawn characters)
    /// and underline patterns. `None` as for
    /// [`glyph`](GlyphSource::glyph), and when there is no room for it in
    /// the texture yet.
    fn glyph_key(&mut self, key: GlyphKey) -> Option<GlyphSlot> {
        let _ = key;
        None
    }

    /// The fonts the glyphs come from, to shape rows in faces with
    /// ligatures ([`Fonts::features`]). `None`: every cell is drawn alone.
    fn fonts(&self) -> Option<&Fonts> {
        None
    }

    /// Names [`fonts`](GlyphSource::fonts): what rows were shaped with is
    /// kept until this changes.
    fn fonts_key(&self) -> u64 {
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
    /// Search hits (E.7), and the current one: the background mixed
    /// with yellow and orange, so they suit light and dark themes.
    pub search_hit: Rgba,
    pub search_current: Rgba,
    /// 0..16 are the configurable ANSI colours, the rest is the xterm cube.
    pub palette: [Rgba; 256],
    /// 0–255; below 255 the window background shows through.
    pub background_opacity: u8,
    /// One bit per transition slot ([`crate::transition::SLOTS`]) whose
    /// colour a program set (OSC 4, 10–12): the program's, not the
    /// theme's, so it stays put in a theme transition.
    pub program_set: u32,
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
        let mut theme = Theme {
            foreground: rgb(fg),
            background: rgb(bg),
            cursor: rgb(cursor),
            cursor_text: rgb(bg),
            selection: rgb(selection),
            search_hit: rgb(mix(bg, 0xe5c07b, 0.45)),
            search_current: rgb(mix(bg, 0xff9f43, 0.85)),
            palette,
            background_opacity: 255,
            program_set: 0,
        };
        theme.keep_legible();
        theme
    }

    /// A new background (0xRRGGBB), with the colours made from it: the
    /// text under a block cursor and the search hits'.
    pub fn set_background(&mut self, bg: u32) {
        self.background = rgb(bg);
        self.search_hit = rgb(mix(bg, 0xe5c07b, 0.45));
        self.search_current = rgb(mix(bg, 0xff9f43, 0.85));
        self.keep_legible();
    }

    /// Make the selection and the text under a block cursor readable
    /// whatever the theme (or a program, OSC 10-12) chose: the selection
    /// gets at least [`SELECTION_CONTRAST`] against the background, moved
    /// towards the foreground, so even one selected character shows; the
    /// text under the cursor is the background colour where that reads,
    /// else the colour that reads best; a cursor of (nearly) the
    /// selection's colour is moved away from it, so it shows inside a
    /// selection. Run again after changing the foreground, background or
    /// cursor colour.
    pub fn keep_legible(&mut self) {
        self.selection = visible_selection(self.selection, self.background, self.foreground);
        self.cursor = visible_cursor(self.cursor, self.selection);
        self.cursor_text = cursor_text_on(self.cursor, self.background, self.foreground);
    }

    /// The tag of transition slot `slot` in this theme: none if a
    /// program set its colour.
    #[inline]
    fn tag(&self, slot: usize) -> Tag {
        if self.program_set >> slot & 1 != 0 { 0 } else { tag_of(slot) }
    }

    /// The colour in transition slot `slot` (not the divider, which isn't
    /// the theme's), tagged.
    #[inline]
    pub(crate) fn paint(&self, slot: usize) -> Paint {
        let rgba = match slot {
            0..16 => self.palette[slot],
            slot::FOREGROUND => self.foreground,
            slot::BACKGROUND => self.background,
            slot::CURSOR => self.cursor,
            slot::CURSOR_TEXT => self.cursor_text,
            slot::SELECTION => self.selection,
            slot::SEARCH_HIT => self.search_hit,
            slot::SEARCH_CURRENT => self.search_current,
            // The divider is the window's, not the theme's.
            _ => {
                debug_assert!(false, "not a theme colour: {slot}");
                0
            }
        };
        Paint { rgba, tag: self.tag(slot) }
    }

    #[inline]
    fn resolve(&self, c: term_core::Color, default: Paint) -> Paint {
        match c.kind() {
            ColorKind::Default => default,
            ColorKind::Indexed(i) => {
                // Without branches: the 16 ANSI colours a program didn't set.
                let own = (i < 16) & (self.program_set >> (i & 15) & 1 == 0);
                Paint {
                    rgba: self.palette[i as usize],
                    tag: (i as Tag + 1) * own as Tag,
                }
            }
            ColorKind::Rgb(r, g, b) => Paint {
                rgba: r as u32 | (g as u32) << 8 | (b as u32) << 16 | 0xFF << 24,
                tag: 0,
            },
        }
    }
}

/// `a` (0xRRGGBB) moved `t` of the way to `b`.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

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
    /// Glyphs of cells that don't draw as their character alone
    /// (ligatures and their neighbours, by column; ligatures design §4.1).
    subs: &'a [ShapedGlyph],
}

/// A cell's text colour as drawn, for the pieces of ligatures in it.
#[derive(Clone, Copy, Default)]
struct Ink {
    fg: Paint,
    dim: bool,
    /// False for hidden cells and cells under input-method text.
    drawn: bool,
}

/// Buffers kept between rows: for rows with substitutions, and for
/// shaping cluster cells.
#[derive(Default)]
struct Pieces {
    inks: Vec<Ink>,
    /// Columns whose own glyph a substitution replaces.
    replaced: Vec<bool>,
    /// A cluster cell's text, and the glyphs it shaped into.
    run: RunText,
    shaped: Vec<ShapedGlyph>,
    clusters: Clusters,
}

/// A cell as frame building reads it: `CellRef`, a snapshot's cells, and
/// in tests cells built from any `Style`.
pub(crate) trait CellView {
    /// Equal keys mean equal styles.
    type Key: Copy + PartialEq;
    fn key(&self) -> Self::Key;
    fn style(&self) -> term_core::Style;
    fn content(&self) -> Content<'_>;
    fn is_spacer(&self) -> bool;
    /// The left half of a two-cell character or cluster.
    fn is_wide(&self) -> bool;
    /// Neither a cluster nor either half of a two-cell cell: drawn as its
    /// character alone, if any.
    fn is_simple(&self) -> bool;
    /// A kitty placeholder cell's image row and column.
    #[inline]
    fn placeholder(&self) -> Option<(u16, u16)> {
        match self.content() {
            Content::Placeholder { row, col } => Some((row, col)),
            _ => None,
        }
    }
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
    #[inline]
    fn is_wide(&self) -> bool {
        self.width() == Width::Wide
    }
    #[inline]
    fn is_simple(&self) -> bool {
        self.width() == Width::Narrow && !matches!(CellRef::content(self), Content::Cluster(_))
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
        // A placeholder's content bits are its piece, not a character.
        if let Some((row, col)) = self.cell.placeholder() {
            return Content::Placeholder { row, col };
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
    #[inline]
    fn is_wide(&self) -> bool {
        self.cell.is_wide()
    }
    /// Two bit tests of one word, which compile to one.
    #[inline]
    fn is_simple(&self) -> bool {
        !self.cell.is_wide_part() && !self.cell.is_cluster()
    }
    #[inline]
    fn placeholder(&self) -> Option<(u16, u16)> {
        self.cell.placeholder()
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
    fg: Paint,
    bg: Paint,
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
        let default = |rgba, slot| Paint {
            rgba,
            tag: theme.tag(slot),
        };
        let mut fg = theme.resolve(style.fg, default(theme.foreground, slot::FOREGROUND));
        let mut bg = theme.resolve(style.bg, default(theme.background, slot::BACKGROUND));
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
    /// Where `clear` comes from: the theme's background unless a program
    /// set it.
    pub clear_tag: Tag,
    pub rects: Vec<RectInstance>,
    pub glyphs: Vec<GlyphInstance>,
    /// Underlines, strikethrough and cursor; appended after the backgrounds.
    decorations: Vec<RectInstance>,
    rows: RowCache,
    /// Indexes into the snapshot's rows of the rows to build this frame.
    to_build: Vec<usize>,
    /// Rows shaped in faces with ligatures (ligatures design §4.2).
    shapes: ShapeCache,
    pieces: Pieces,
    /// The glyph generation the snapshot was taken against.
    snapped_with: u64,
    /// The images this frame draws (images.md §6.3).
    pub images: Images,
    /// Where `rects` splits for the image layers.
    pub split: RectSplit,
    /// Where each visible row's instances are in `rects` and `glyphs`,
    /// as the last frame laid them out.
    laid: Vec<Laid>,
    /// This frame's, while building.
    laid_spare: Vec<Laid>,
    /// `laid` says where the rows are (false after `build_live`, which
    /// writes them whole).
    laid_valid: bool,
    /// The overlays between and after the rows, as last laid out.
    kept: Kept,
    spare_glyphs: Vec<GlyphInstance>,
    /// What each of the last frames wrote, for the GPU's copies.
    changes: Changes,
}

/// Where one visible row's instances were laid out: (start, len) in
/// each region.
#[derive(Clone, Copy)]
struct Laid {
    slot: u32,
    /// The row cache entry's version laid out.
    version: u64,
    bg: (u32, u32),
    deco: (u32, u32),
    glyphs: (u32, u32),
    /// What was laid out is the entry's instances as they are, not
    /// recoloured under the block cursor or hidden under input-method
    /// text.
    clean: bool,
}

impl Laid {
    fn missing(at: usize) -> Laid {
        Laid {
            slot: MISSING,
            version: 0,
            bg: (at as u32, 0),
            deco: (0, 0),
            glyphs: (0, 0),
            clean: false,
        }
    }
}

/// The overlays as last laid out, and where.
#[derive(Default)]
struct Kept {
    /// Highlights, the block cursor and hovered links' underlines,
    /// between the rows' backgrounds and decorations.
    over_rects: Vec<RectInstance>,
    over: (u32, u32),
    /// Other cursors and input-method text, after the rows.
    tail_rects: Vec<RectInstance>,
    tail: (u32, u32),
    tail_glyphs: Vec<GlyphInstance>,
    tail_g: (u32, u32),
    /// This frame's, while building.
    tail_rects_new: Vec<RectInstance>,
    tail_glyphs_new: Vec<GlyphInstance>,
}

/// Lays instances out in a vector kept from the last frame: each piece
/// is left alone when it is already there, written over its old place
/// when it is as long, and otherwise the vector is cut there and the
/// rest appended.
struct Lay<'a, T> {
    out: &'a mut Vec<T>,
    at: usize,
    append: bool,
    /// The first instance written.
    dirty: usize,
}

impl<'a, T: Copy> Lay<'a, T> {
    /// `valid`: `out` holds the last frame's layout.
    fn new(out: &'a mut Vec<T>, valid: bool) -> Self {
        if !valid {
            out.clear();
        }
        Lay {
            out,
            at: 0,
            append: !valid,
            dirty: usize::MAX,
        }
    }

    /// Put `data` next. `same`: it is what was laid out at `prev`
    /// (start, len) last frame. Returns where it went.
    #[inline]
    fn put(&mut self, same: bool, prev: Option<(u32, u32)>, data: &[T]) -> (u32, u32) {
        let (start, n) = (self.at, data.len());
        let here = !self.append && prev == Some((start as u32, n as u32));
        if !(here && same) {
            if here {
                self.out[start..start + n].copy_from_slice(data);
            } else {
                if !self.append {
                    self.out.truncate(start);
                    self.append = true;
                }
                self.out.extend_from_slice(data);
            }
            if n > 0 {
                self.dirty = self.dirty.min(start);
            }
        }
        self.at += n;
        (start as u32, n as u32)
    }

    /// Drop what is left of the last frame's; returns the instances
    /// written.
    fn finish(self) -> std::ops::Range<usize> {
        self.out.truncate(self.at);
        let len = self.out.len();
        self.dirty.min(len)..len
    }
}

type Ranges = (std::ops::Range<usize>, std::ops::Range<usize>);

/// The instances each of the last few frames wrote, by version.
#[derive(Default)]
struct Changes {
    log: Vec<(u64, Ranges)>,
}

/// Versions of frames' instances: unique in the process, so a buffer
/// can't take another frame's for its own.
static VERSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Changes {
    const KEPT: usize = 4;

    fn push(&mut self, rects: std::ops::Range<usize>, glyphs: std::ops::Range<usize>) {
        if self.log.len() == Self::KEPT {
            self.log.remove(0);
        }
        let v = VERSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.log.push((v, (rects, glyphs)));
    }

    fn version(&self) -> u64 {
        self.log.last().map_or(0, |e| e.0)
    }

    /// The hull of what was written after `version`, within the lengths
    /// now.
    fn since(&self, version: u64, rects: usize, glyphs: usize) -> Option<Ranges> {
        let i = self.log.iter().position(|e| e.0 == version)?;
        let (mut r, mut g) = ((rects, 0), (glyphs, 0));
        for (_, (a, b)) in &self.log[i + 1..] {
            if !a.is_empty() {
                r = (r.0.min(a.start), r.1.max(a.end));
            }
            if !b.is_empty() {
                g = (g.0.min(b.start), g.1.max(b.end));
            }
        }
        let clip = |(s, e): (usize, usize), len: usize| {
            let e = e.min(len);
            if s < e { s..e } else { 0..0 }
        };
        Some((clip(r, rects), clip(g, glyphs)))
    }
}

/// How a frame's `rects` interleave with its image layers (images.md
/// §6.3): `rects[..backgrounds]` are the cell backgrounds, drawn over the
/// images under the backgrounds and under the images under the text; the
/// rest (highlights, the block cursor, decorations) over those. `cursor`
/// is drawn again over the images above the text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RectSplit {
    pub backgrounds: usize,
    pub cursor: std::ops::Range<usize>,
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
        self.snapshot(term, input, glyphs.generation());
        if !self.build_snapshot(input, glyphs) {
            // The glyph textures were emptied since: every row again.
            self.forget_rows();
            self.snapshot(term, input, glyphs.generation());
            self.build_snapshot(input, glyphs);
        }
    }

    /// The part of [`build`](Frame::build) that needs the terminal (its
    /// lock): drop cached rows if what they were built with changed (the
    /// glyph textures' `generation` among it), and take the snapshot,
    /// copying the rows not cached. Shaping and building wait for
    /// [`build_snapshot`](Frame::build_snapshot), which doesn't need it.
    pub fn snapshot(&mut self, term: &mut Terminal, input: &FrameInput, generation: u64) {
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
            glyphs: generation,
        });
        self.rows.snapshot(term);
        self.images.set_pool(term.image_pool(), term.graphics().owner());
        self.snapped_with = generation;
    }

    /// The last [`snapshot`](Frame::snapshot), for adding to its
    /// highlights before it is built (the app's search hits, until
    /// term-core's search puts them in itself).
    pub fn snapshot_mut(&mut self) -> &mut term_core::Snapshot {
        &mut self.rows.snap
    }

    /// Build the frame from the last [`snapshot`](Frame::snapshot), taken
    /// with the same `input`, without the terminal: shape the rows it
    /// copied in faces with ligatures, build them and draw the overlays.
    /// False, having built nothing, when the glyph textures' generation
    /// changed since the snapshot (another pane emptied them): the cached
    /// rows are then wrong, and the frame must be built again from a new
    /// snapshot, with the terminal.
    pub fn build_snapshot(&mut self, input: &FrameInput, glyphs: &mut impl GlyphSource) -> bool {
        if glyphs.generation() != self.snapped_with {
            return false;
        }
        self.rows.take(&mut self.to_build);
        self.shapes.check(&self.rows.snap, glyphs.fonts_key());
        self.redraw(input, glyphs);
        if !self.to_build.is_empty() {
            self.shapes.keep_visible(&self.rows.snap);
        }
        self.images.tell_pool();
        true
    }

    /// Build again the rows this frame built, and the frame: after a glyph
    /// texture that ran out of room grew (keeping every slot), so the
    /// glyphs that didn't fit are drawn. Rows kept from earlier frames had
    /// all theirs.
    pub fn redraw(&mut self, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        self.build_rows(input, glyphs);
        self.rows.place();
        let (py, ch) = (input.padding.1 as i32, input.metrics.cell_height as i32);
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

    /// Forget the rows shaped, and with `stretches` the stretches too, so
    /// the next build shapes again: for measuring shaping. Test aid.
    #[doc(hidden)]
    pub fn forget_shaping(&mut self, stretches: bool) {
        self.shapes.forget(stretches);
    }

    /// Rows kept in the shaping cache, and how many rows and stretches
    /// were shaped so far (not found kept). Test aid.
    #[doc(hidden)]
    pub fn shaping(&self) -> (usize, u64, u64) {
        let (rows, stretches) = self.shapes.shaped();
        (self.shapes.len(), rows, stretches)
    }

    /// Build the rows the snapshot copied into the row cache, where they
    /// are drawn this frame.
    fn build_rows(&mut self, input: &FrameInput, glyphs: &mut impl GlyphSource) {
        let cols = self.rows.snap.cols as usize;
        let (py, ch) = (input.padding.1 as i32, input.metrics.cell_height as i32);
        let shaping = glyphs.fonts().is_some_and(shapes_runs);
        for &i in &self.to_build {
            let row = &self.rows.snap.rows[i];
            let key = row.key;
            let subs: &[ShapedGlyph] = match glyphs.fonts() {
                Some(fonts) if shaping => {
                    let styles = self.rows.styles_for(row.flags.contains(RowFlags::HISTORY));
                    let snap = &self.rows.snap;
                    let cells = snap.row_cells(row).unwrap_or(&[]);
                    let clusters = SnapClusters { snap, row };
                    let cells = cells.iter().map(|&cell| SnapCell {
                        cell,
                        styles,
                        clusters: &clusters,
                    });
                    self.shapes.row(Some(key), cells, fonts)
                }
                _ => &[],
            };
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
                subs,
            };
            // The entry is taken out while it's built, as its cells and
            // styles are borrowed from the cache.
            let mut entry = std::mem::take(self.rows.entry_mut(slot));
            entry.y = ctx.y;
            let row = &self.rows.snap.rows[i];
            let styles = self.rows.styles_for(row.flags.contains(RowFlags::HISTORY));
            let snap = &self.rows.snap;
            let all = snap.row_cells(row).unwrap_or(&[]);
            // A row's blank end draws nothing (roadmap R.2): its cells
            // are skipped. Not with substitutions, whose pieces may reach
            // into blank cells (shaped as spaces).
            let cells = if subs.is_empty() { trim_blank_end(all) } else { all };
            let clusters = SnapClusters { snap, row };
            let cells = cells.iter().map(|&cell| SnapCell {
                cell,
                styles,
                clusters: &clusters,
            });
            build_row(&ctx, cells, glyphs, &mut entry, &mut self.pieces);
            if row.flags.contains(RowFlags::PLACEHOLDERS) {
                let cells = snap.row_cells(row).unwrap_or(&[]).iter().map(|&cell| SnapCell {
                    cell,
                    styles,
                    clusters: &clusters,
                });
                pieces_of_row(cells, cols, &mut entry.pieces);
            }
            let used = std::mem::replace(self.rows.entry_mut(slot), entry);
            debug_assert!(used.glyphs.is_empty());
        }
    }

    /// The frame from the cached rows, with the overlays on top: rows'
    /// backgrounds, highlights, the block cursor, rows' decorations, rows'
    /// glyphs, other cursors and input-method text.
    ///
    /// The instances stay where they were (roadmap R.2): a row whose
    /// instances are already at their place in `rects` and `glyphs`
    /// (the same row, not moved, at the same offset) isn't copied again,
    /// a changed row of the same length is written over its old one, and
    /// only from a row whose length changed on is the rest written out.
    /// What was written is logged for the GPU buffers ([`Frame::changes_since`]).
    fn assemble(&mut self, input: &FrameInput, source: &mut impl GlyphSource) {
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
        self.clear_tag = theme.paint(slot::BACKGROUND).tag;
        let Frame {
            rects,
            glyphs,
            decorations: over,
            rows,
            images,
            split,
            laid: laid_prev,
            laid_spare: laid,
            laid_valid,
            kept,
            spare_glyphs,
            ..
        } = self;
        let snap = &rows.snap;
        let cols = snap.cols as usize;
        let row_y = |line: usize| py + line as i32 * ch;
        let full = |line: usize, cells: std::ops::Range<usize>, paint: Paint| {
            RectInstance::painted(
                s16(px + cells.start as i32 * cw),
                s16(row_y(line)),
                ((cells.end - cells.start) as i32 * cw) as u16,
                ch as u16,
                paint,
            )
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
        laid.clear();
        let mut out = Lay::new(rects, *laid_valid);
        for (line, &slot) in rows.order.iter().enumerate() {
            if slot == MISSING {
                laid.push(Laid::missing(out.at));
                continue;
            }
            let e = rows.entry(slot);
            let p = laid_prev.get(line).filter(|p| p.slot == slot);
            let same = p.is_some_and(|p| p.version == e.version && p.clean);
            let bg = out.put(same, p.map(|p| p.bg), &e.backgrounds);
            laid.push(Laid {
                slot,
                version: e.version,
                bg,
                deco: (0, 0),
                glyphs: (0, 0),
                clean: true,
            });
        }
        split.backgrounds = out.at;

        // Images: direct placements, and the pieces placeholder cells show.
        let pieces = rows
            .order
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s != MISSING)
            .filter_map(|(line, &slot)| {
                let p = &rows.entry(slot).pieces;
                (!p.is_empty()).then_some((line, p.as_slice()))
            });
        let grid = Grid {
            cw: cw as f32,
            ch: ch as f32,
            px: px as f32,
            py: py as f32,
            cols: snap.cols,
            lines: snap.lines,
        };
        if snap.placements.is_empty() && snap.virtual_placements.is_empty() {
            images.clear();
        } else {
            images.build(snap, pieces, grid);
        }
        let images = &*images;
        // A highlight over image cells is a tint, so the image shows.
        let highlight = |rects: &mut Vec<RectInstance>, line: usize, a: usize, b: usize, color: Paint| {
            let mut at = a;
            for c in images.covered_on(line) {
                let (c0, c1) = ((c.start as usize).max(at), (c.end as usize).min(b));
                if c0 >= c1 {
                    continue;
                }
                if at < c0 {
                    rects.push(full(line, at..c0, color));
                }
                rects.push(full(line, c0..c1, color.alpha(((color.rgba >> 24) / 2) as u8)));
                at = c1;
            }
            if at < b {
                rects.push(full(line, at..b, color));
            }
        };
        // Highlights (search hits under the selection), then the block
        // cursor over them, then hovered links' underlines.
        over.clear();
        for (line, row) in snap.rows.iter().enumerate() {
            for h in snap.row_highlights(row) {
                let (a, b) = (h.cols.start as usize, (h.cols.end as usize).min(cols));
                match h.kind {
                    HighlightKind::SearchHit if a < b => highlight(over, line, a, b, theme.paint(slot::SEARCH_HIT)),
                    HighlightKind::CurrentSearchHit if a < b => highlight(over, line, a, b, theme.paint(slot::SEARCH_CURRENT)),
                    _ => {}
                }
            }
        }
        for (line, row) in snap.rows.iter().enumerate() {
            for h in snap.row_highlights(row) {
                let (a, b) = (h.cols.start as usize, (h.cols.end as usize).min(cols));
                if a < b && h.kind == HighlightKind::Selection {
                    highlight(over, line, a, b, theme.paint(slot::SELECTION));
                }
            }
        }
        if let Some((line, cells)) = &block
            && cells.start < cols
        {
            over.push(full(*line, cells.start..cells.end.min(cols), theme.paint(slot::CURSOR)));
        }
        for (line, row) in snap.rows.iter().enumerate() {
            for h in snap.row_highlights(row) {
                let (a, b) = (h.cols.start as usize, (h.cols.end as usize).min(cols));
                // Underlined in the text colour, over the backgrounds.
                if a < b && h.kind == HighlightKind::HoveredLink {
                    over.push(RectInstance {
                        y: s16(row_y(line) + (m.baseline + m.underline_offset) as i32),
                        h: m.underline_thickness.max(1) as u16,
                        ..full(line, a..b, theme.paint(slot::FOREGROUND))
                    });
                }
            }
        }
        for &line in &snap.failed {
            over.push(failed_mark(line as usize, input));
        }
        kept.over = out.put(*over == kept.over_rects, Some(kept.over), over);
        std::mem::swap(over, &mut kept.over_rects);

        // Decorations and glyphs, recoloured under the block cursor and
        // hidden under input-method text.
        let under_cursor = |dim: bool| {
            let text = theme.paint(slot::CURSOR_TEXT);
            if dim { text.alpha(160) } else { text }
        };
        for (line, l) in laid.iter_mut().enumerate() {
            if l.slot == MISSING {
                l.deco = (out.at as u32, 0);
                continue;
            }
            let e = rows.entry(l.slot);
            let p = laid_prev.get(line).filter(|p| p.slot == l.slot);
            let same = p.is_some_and(|p| p.version == l.version && p.clean);
            let on = cells_on(&block, line);
            if on.is_empty() {
                l.deco = out.put(same, p.map(|p| p.deco), &e.decorations);
            } else {
                l.deco = out.put(false, p.map(|p| p.deco), &e.decorations);
                l.clean = false;
                let at = l.deco.0 as usize;
                let placed = &mut out.out[at..at + e.decorations.len()];
                for (r, mark) in placed.iter_mut().zip(&e.decoration_marks) {
                    if mark.flags & Mark::TEXT_COLOUR != 0 && on.contains(&(mark.col as usize)) {
                        let p = under_cursor(mark.flags & Mark::DIM != 0);
                        (r.color, r.tag) = (p.rgba, p.tag);
                    }
                }
            }
        }

        // Other cursors and input-method text, over everything.
        let tail = &mut kept.tail_rects_new;
        let gtail = &mut kept.tail_glyphs_new;
        tail.clear();
        gtail.clear();
        if let Some(c) = &cursor
            && block.is_none()
        {
            cursor_rects(tail, c, input);
        }
        split.cursor = out.at..out.at + tail.len();
        if let Some((text, at)) = preedit {
            draw_preedit(tail, gtail, text, at, cols, input, source);
        }
        kept.tail = out.put(*tail == kept.tail_rects, Some(kept.tail), tail);
        std::mem::swap(tail, &mut kept.tail_rects);
        let rects_dirty = out.finish();

        let mut out = Lay::new(glyphs, *laid_valid);
        for (line, l) in laid.iter_mut().enumerate() {
            if l.slot == MISSING {
                l.glyphs = (out.at as u32, 0);
                continue;
            }
            let e = rows.entry(l.slot);
            let p = laid_prev.get(line).filter(|p| p.slot == l.slot);
            let same = p.is_some_and(|p| p.version == l.version && p.clean);
            let on = cells_on(&block, line);
            let hidden = cells_on(&covered, line);
            if on.is_empty() && hidden.is_empty() {
                l.glyphs = out.put(same, p.map(|p| p.glyphs), &e.glyphs);
                continue;
            }
            l.clean = false;
            spare_glyphs.clear();
            for g in &e.glyphs {
                let col = g.col();
                if hidden.contains(&col) {
                    continue;
                }
                spare_glyphs.push(match () {
                    _ if !on.contains(&col) || g.is_color & GlyphInstance::FIXED != 0 => *g,
                    _ => g.painted(under_cursor(g.is_color & GlyphInstance::DIM != 0)),
                });
            }
            l.glyphs = out.put(false, p.map(|p| p.glyphs), spare_glyphs);
        }
        kept.tail_g = out.put(*gtail == kept.tail_glyphs, Some(kept.tail_g), gtail);
        std::mem::swap(gtail, &mut kept.tail_glyphs);
        let glyphs_dirty = out.finish();

        std::mem::swap(laid, laid_prev);
        *laid_valid = true;
        self.changes.push(rects_dirty, glyphs_dirty);
    }

    /// The version of `rects` and `glyphs` now: changes with every
    /// frame built, unique in the process.
    pub(crate) fn version(&self) -> u64 {
        self.changes.version()
    }

    /// The parts of `rects` and `glyphs` (instance ranges) that may
    /// differ from what they were at `version`; `None` when that is too
    /// long ago, or wasn't this frame's: copy them whole.
    pub(crate) fn changes_since(&self, version: u64) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
        self.changes.since(version, self.rects.len(), self.glyphs.len())
    }

    /// `rects` and `glyphs` were written whole, not laid out row by row.
    fn written_whole(&mut self) {
        self.laid_valid = false;
        let (r, g) = (self.rects.len(), self.glyphs.len());
        self.changes.push(0..r, 0..g);
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
        self.clear_tag = theme.paint(slot::BACKGROUND).tag;

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
        let shaping = glyphs.fonts().is_some_and(shapes_runs);
        for line in 0..term.lines() {
            let row = term.visible_row(line);
            let subs: &[ShapedGlyph] = match glyphs.fonts() {
                Some(fonts) if shaping => self.shapes.row(None, row.cells(), fonts),
                _ => &[],
            };
            // Selected columns on this line (history rows at another
            // width map the selection through their layout).
            let sel = if selection.is_some() { term.selected_cols(line) } else { 0..0 };
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
                subs,
            };
            let mut sink = Direct {
                rects: &mut self.rects,
                glyphs: &mut self.glyphs,
                decorations: &mut self.decorations,
            };
            build_row(&ctx, row.cells(), glyphs, &mut sink, &mut self.pieces);
        }

        // Cursors other than a focused block are drawn on top as rectangles.
        if let Some(c) = &cursor
            && !block_cursor
        {
            cursor_rects(&mut self.decorations, c, input);
        }
        if let Some((text, at)) = preedit {
            draw_preedit(&mut self.decorations, &mut self.glyphs, text, at, cols, input, glyphs);
        }
        self.rects.extend_from_slice(&self.decorations);
        // The reference path draws no images.
        self.images.clear();
        self.split = RectSplit {
            backgrounds: self.rects.len(),
            cursor: 0..0,
        };
        self.written_whole();
    }
}

/// The mark beside a prompt whose command exited non-zero (view row
/// `line`): a thin bar the row's height in the theme's red (ANSI 1),
/// in the middle of the left padding, or at the grid's left edge when
/// there is too little padding for it.
pub(crate) fn failed_mark(line: usize, input: &FrameInput) -> RectInstance {
    let m = input.metrics;
    let pad = input.padding.0;
    let w = (m.cell_width / 5).max(1);
    let x = if pad > w { (pad - w) / 2 } else { 0 };
    let y = input.padding.1 as i32 + line as i32 * m.cell_height as i32;
    RectInstance::painted(s16(x as i32), s16(y), w as u16, m.cell_height as u16, input.theme.paint(1))
}

/// A cursor other than a focused block: a bar, an underline or (when
/// unfocused) a hollow box.
fn cursor_rects(out: &mut Vec<RectInstance>, c: &CursorDraw, input: &FrameInput) {
    let m = input.metrics;
    let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
    let (line, col) = c.at;
    let x = input.padding.0 as i32 + col as i32 * cw;
    let y = input.padding.1 as i32 + line as i32 * ch;
    let w = cw * if c.wide { 2 } else { 1 };
    let h = ch;
    let color = input.theme.paint(slot::CURSOR);
    let t = (m.cell_width as i32 / 8).max(2);
    let r = |x: i32, y: i32, w: i32, h: i32| RectInstance::painted(s16(x), s16(y), w as u16, h as u16, color);
    let rects: &[RectInstance] = match (input.focused, c.shape) {
        (true, CursorShape::Bar) => &[r(x, y, t, h)],
        (true, CursorShape::Underline) => &[r(x, y + h - t, w, t)],
        // Unfocused: a hollow box.
        _ => &[r(x, y, w, 1), r(x, y + h - 1, w, 1), r(x, y, 1, h), r(x + w - 1, y, 1, h)],
    };
    out.extend_from_slice(rects);
}

/// Input-method text at view row and column `at`.
fn draw_preedit(
    rects: &mut Vec<RectInstance>,
    out: &mut Vec<GlyphInstance>,
    text: &str,
    at: (usize, usize),
    cols: usize,
    input: &FrameInput,
    glyphs: &mut impl GlyphSource,
) {
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
        let (fg, bg) = (input.theme.paint(slot::FOREGROUND), input.theme.paint(slot::BACKGROUND));
        rects.push(RectInstance::painted(s16(x), s16(y), w, ch as u16, bg));
        let uy = y + (m.baseline + m.underline_offset) as i32;
        rects.push(RectInstance::painted(s16(x), s16(uy), w, m.underline_thickness as u16, fg));
        if let Some(g) = glyphs.glyph(c as u32, FontStyle::Regular) {
            out.push(GlyphInstance {
                x: s16(x + g.left as i32),
                y: s16(y + m.baseline as i32 - g.top as i32),
                w: g.w,
                h: g.h,
                u: g.u,
                v: g.v,
                color: fg.rgba,
                is_color: g.color as u32 | fg.tag << GlyphInstance::TAG_SHIFT,
            });
        }
        col += width;
    }
}

/// `cells` up to and with its last cell that draws something if the row
/// ended there ([`Cell::is_blank`]): past it, only the default background
/// shows, which `pad_row` leaves out.
#[inline]
fn trim_blank_end(cells: &[Cell]) -> &[Cell] {
    // Eight cells at a time: a blank end is usually most of a row.
    let mut n = cells.len();
    while n >= 8 && cells[n - 8..n].iter().fold(0, |a, c| a | c.bits()) & Cell::NOT_BLANK == 0 {
        n -= 8;
    }
    while n > 0 && cells[n - 1].is_blank() {
        n -= 1;
    }
    &cells[..n]
}

/// Whether any face of `fonts` shapes runs (ligatures design §1).
fn shapes_runs(fonts: &Fonts) -> bool {
    FontStyle::ALL.iter().any(|&s| fonts.features(s).ligatures)
}

/// One row of cells, left to right from column 0.
#[inline]
fn build_row<C: CellView, S: RowSink>(
    ctx: &RowCtx,
    cells: impl Iterator<Item = C>,
    glyphs: &mut impl GlyphSource,
    out: &mut S,
    pieces: &mut Pieces,
) {
    // Most rows have no selection, block cursor or input-method text (and
    // rows built for the cache never do), and no ligatures: they get a
    // loop without those checks.
    let overlay = !(ctx.sel.is_empty() && ctx.block_cursor.is_empty() && ctx.covered.is_empty());
    if ctx.subs.is_empty() {
        if overlay {
            row_cells::<true, false, C, S>(ctx, cells, glyphs, out, pieces)
        } else {
            row_cells::<false, false, C, S>(ctx, cells, glyphs, out, pieces)
        }
        return;
    }
    pieces.replaced.clear();
    pieces.replaced.resize(ctx.cols, false);
    for g in ctx.subs {
        let at = g.cell as usize;
        for r in pieces.replaced.iter_mut().take(at + g.cells as usize).skip(at) {
            *r = true;
        }
    }
    pieces.inks.clear();
    if overlay {
        row_cells::<true, true, C, S>(ctx, cells, glyphs, out, pieces)
    } else {
        row_cells::<false, true, C, S>(ctx, cells, glyphs, out, pieces)
    }
    draw_subs(ctx, &pieces.inks, glyphs, out);
}

/// `build_row`, with `OVERLAY` false when the row has no selection, block
/// cursor or input-method text, and `LIG` false when it has no
/// substitutions.
#[inline(always)]
fn row_cells<const OVERLAY: bool, const LIG: bool, C: CellView, S: RowSink>(
    ctx: &RowCtx,
    cells: impl Iterator<Item = C>,
    glyphs: &mut impl GlyphSource,
    out: &mut S,
    pieces: &mut Pieces,
) {
    let RowCtx { theme, m, px, y, .. } = *ctx;
    let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
    let default_bg = theme.paint(slot::BACKGROUND);
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
            bg = theme.paint(slot::SELECTION);
        }
        if OVERLAY && ctx.block_cursor.contains(&col) {
            bg = theme.paint(slot::CURSOR);
            fg = theme.paint(slot::CURSOR_TEXT);
        }
        if pen.attrs.contains(Attrs::DIM) {
            fg = fg.alpha(160);
        }

        if bg != run_color {
            push_run(out, run_color, default_bg, run_start, col, px, y, cw, ch);
            run_start = col;
            run_color = bg;
        }

        let x = px + col as i32 * cw;
        if LIG {
            pieces.inks.push(Ink {
                fg,
                dim: pen.attrs.contains(Attrs::DIM),
                drawn: !pen.attrs.contains(Attrs::HIDDEN) && !(OVERLAY && ctx.covered.contains(&col)),
            });
        }
        let covered = OVERLAY && ctx.covered.contains(&col);
        if cell.is_simple() {
            // The common case, kept to one test of the cell: a character
            // (or nothing) in one cell.
            if let Content::Char(c) = cell.content()
                && c != ' '
                && !pen.attrs.contains(Attrs::HIDDEN)
                && !covered
                && !(LIG && pieces.replaced[col])
                && let Some(g) = glyphs.glyph(c as u32, pen.face())
            {
                out.glyph(GlyphInstance {
                    x: s16(x + g.left as i32),
                    y: s16(y + m.baseline as i32 - g.top as i32),
                    w: g.w,
                    h: g.h,
                    u: g.u,
                    v: g.v,
                    color: fg.rgba,
                    is_color: GlyphInstance::flags(&g, col, pen.attrs.contains(Attrs::DIM), fg.tag),
                });
            }
        } else if !spacer && !pen.attrs.contains(Attrs::HIDDEN) && !covered && !(LIG && pieces.replaced[col]) {
            let wide = cell.is_wide();
            let dim = pen.attrs.contains(Attrs::DIM);
            other_cell(cell.content(), wide, pen.face(), (col, x, y), fg, dim, m, glyphs, out, pieces);
        }
        if pen.decorated {
            decorate(out, glyphs, pen, theme, fg, x, y, m, pen.mark(col), covered);
        }
        end = col + 1;
    }
    pad_row(out, ctx, end, run_start, run_color);
}

/// A cell that isn't a character in one cell: a wide character, a
/// cluster, or a one-cell character in two cells.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn other_cell(
    content: Content,
    wide: bool,
    face: FontStyle,
    (col, x, y): (usize, i32, i32),
    fg: Paint,
    dim: bool,
    m: Metrics,
    glyphs: &mut impl GlyphSource,
    out: &mut impl RowSink,
    pieces: &mut Pieces,
) {
    match content {
        Content::Char(c) if c != ' ' => {
            let Some(mut g) = glyphs.glyph(c as u32, face) else { return };
            // A character wider than itself: an ambiguous-width one drawn
            // at two cells.
            if wide && char_cells(c) == 1 {
                g = wide_narrow(c, g, m.cell_width as i32, glyphs);
            }
            out.glyph(GlyphInstance {
                x: s16(x + g.left as i32),
                y: s16(y + m.baseline as i32 - g.top as i32),
                w: g.w,
                h: g.h,
                u: g.u,
                v: g.v,
                color: fg.rgba,
                is_color: GlyphInstance::flags(&g, col, dim, fg.tag),
            });
        }
        Content::Cluster(text) => draw_cluster(text, wide, face, (col, x, y), fg, dim, m, glyphs, out, pieces),
        _ => {}
    }
}

/// The glyph of a one-cell character in a two-cell cell (an ambiguous-width
/// character with `ambiguous_width = 2`, cells design §6.3), from `slot`,
/// its glyph alone: a code-drawn one is the sprite across both cells, a
/// font glyph is centred in them (at its font's size), a colour glyph is
/// already fitted to two cells.
fn wide_narrow(c: char, mut slot: GlyphSlot, cw: i32, glyphs: &mut impl GlyphSource) -> GlyphSlot {
    if let Some(id) = SpriteId::for_char(c) {
        return glyphs.glyph_key(GlyphKey::sprite(id, 2)).unwrap_or(slot);
    }
    if !slot.color {
        slot.left += (cw / 2) as i16;
    }
    slot
}

/// A cluster cell (cells design §6.5): its text shaped alone through
/// term-font's shaped-cluster cache (once: [`Clusters`] keeps what it
/// drew), and each glyph drawn at the cell's
/// origin plus its offset (combining marks). A cluster in two cells whose
/// first character is one cell wide (an ambiguous character and its
/// marks at `ambiguous_width = 2`) has its font glyphs centred, as
/// [`wide_narrow`] does; emoji are colour glyphs already fitted to both.
/// Without fonts (test sources) it draws its first character, where the
/// cell starts.
#[allow(clippy::too_many_arguments)]
fn draw_cluster(
    text: &str,
    wide: bool,
    face: FontStyle,
    (col, x, y): (usize, i32, i32),
    fg: Paint,
    dim: bool,
    m: Metrics,
    glyphs: &mut impl GlyphSource,
    out: &mut impl RowSink,
    pieces: &mut Pieces,
) {
    let Some(first) = text.chars().next() else { return };
    let centre = wide && char_cells(first) == 1;
    let place = |out: &mut _, slot: GlyphSlot, dx: i16, dy: i16, centred: bool| {
        let shift = if centred { m.cell_width as i32 / 2 } else { 0 };
        RowSink::glyph(
            out,
            GlyphInstance {
                x: s16(x + shift + dx as i32 + slot.left as i32),
                y: s16(y + m.baseline as i32 + dy as i32 - slot.top as i32),
                w: slot.w,
                h: slot.h,
                u: slot.u,
                v: slot.v,
                color: fg.rgba,
                is_color: GlyphInstance::flags(&slot, col, dim, fg.tag),
            },
        )
    };
    if glyphs.fonts().is_none() {
        if let Some(slot) = glyphs.glyph(first as u32, face) {
            place(out, slot, 0, 0, false);
        }
        return;
    }
    let Pieces { run, shaped, clusters, .. } = pieces;
    let now = (glyphs.generation(), glyphs.fonts_key());
    if clusters.against != now || clusters.map.len() >= CLUSTERS {
        clusters.map.clear();
        clusters.placed.clear();
        clusters.against = now;
    }
    clusters.key.clear();
    clusters.key.push(face as u8 | (wide as u8) << 2);
    clusters.key.extend_from_slice(text.as_bytes());
    if let Some(&(start, len)) = clusters.map.get(&clusters.key[..]) {
        for p in &clusters.placed[start as usize..start as usize + len as usize] {
            place(out, p.slot, p.dx, p.dy, p.centred);
        }
        return;
    }
    shaped.clear();
    run.clear();
    run.push(text, if wide { 2 } else { 1 });
    if let Some(fonts) = glyphs.fonts() {
        fonts.shape(run, face, Features::default(), shaped);
    }
    let start = clusters.placed.len();
    let mut complete = true;
    for g in shaped.iter() {
        let Some(slot) = glyphs.glyph_key(g.key) else {
            // No ink, or no room in the texture yet: not kept, as the
            // glyph may be there when the row is built again.
            complete = false;
            continue;
        };
        let centred = centre && !slot.color && g.key.font != SPRITE_FONT;
        place(out, slot, g.dx, g.dy, centred);
        clusters.placed.push(Placed {
            slot,
            dx: g.dx,
            dy: g.dy,
            centred,
        });
    }
    let len = clusters.placed.len() - start;
    if complete && len <= u16::MAX as usize {
        clusters.map.insert(clusters.key[..].into(), (start as u32, len as u16));
    } else {
        clusters.placed.truncate(start);
    }
}

/// Clusters kept by [`Clusters`] before it is emptied: more different
/// clusters than a 300×100 screen of them holds.
const CLUSTERS: usize = 4096;

/// Cluster cells' glyphs as drawn, by face, width and text, so a cluster
/// drawn before costs one lookup rather than term-font's cache and a
/// texture lookup per glyph (which made a screen of clusters 3× slower to
/// build). Emptied when the glyph textures' generation or the fonts
/// change, and when full; clusters with a glyph that wasn't found aren't
/// kept.
#[derive(Default)]
struct Clusters {
    /// (face | width, text) to a range of `placed`.
    map: HashMap<Box<[u8]>, (u32, u16), BuildHasherDefault<KeyHasher>>,
    placed: Vec<Placed>,
    /// The glyph source's generation and fonts the slots came from.
    against: (u64, u64),
    key: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Placed {
    slot: GlyphSlot,
    dx: i16,
    dy: i16,
    centred: bool,
}

/// The substitutions of a row, after its cells: each glyph at its cell's
/// origin plus its offset. A split glyph (a ligature, or an alternate
/// whose ink reaches past its cell) is drawn in pieces, one per cell its
/// ink touches, each clipped to that cell's columns and in that cell's
/// text colour; the leftmost piece stays open to the left and the
/// rightmost to the right, and pieces in hidden cells or under
/// input-method text are left out (ligatures design §4.1). Each piece
/// records its cell, so the block cursor recolours only the pieces in its
/// cells.
fn draw_subs(ctx: &RowCtx, inks: &[Ink], glyphs: &mut impl GlyphSource, out: &mut impl RowSink) {
    let RowCtx { m, px, y, subs, .. } = *ctx;
    let Some(last) = inks.len().checked_sub(1) else { return };
    let cw = m.cell_width as i32;
    let cell_of = |x: i32| ((x - px).div_euclid(cw.max(1))).clamp(0, last as i32) as usize;
    for g in subs {
        let Some(slot) = glyphs.glyph_key(g.key) else { continue };
        let x0 = px + g.cell as i32 * cw + g.dx as i32 + slot.left as i32;
        let x1 = x0 + slot.w as i32;
        let gy = s16(y + m.baseline as i32 + g.dy as i32 - slot.top as i32);
        let (first, end) = if g.split {
            (cell_of(x0), cell_of(x1 - 1))
        } else {
            let c = (g.cell as usize).min(last);
            (c, c)
        };
        for (c, &ink) in inks.iter().enumerate().take(end + 1).skip(first) {
            let l = if c == first { x0 } else { px + c as i32 * cw };
            let r = if c == end { x1 } else { px + (c as i32 + 1) * cw };
            if !ink.drawn || r <= l {
                continue;
            }
            out.glyph(GlyphInstance {
                x: s16(l),
                y: gy,
                w: (r - l) as u16,
                h: slot.h,
                u: slot.u + (l - x0) as u16,
                v: slot.v,
                color: ink.fg.rgba,
                is_color: GlyphInstance::flags(&slot, c, ink.dim, ink.fg.tag),
            });
        }
    }
}

/// Ends a row whose cells stopped at column `from`, with the background
/// run so far. History rows can be shorter than the screen is wide: the
/// rest is blank, which only shows a background where it is selected or
/// under a block cursor.
fn pad_row(out: &mut impl RowSink, ctx: &RowCtx, from: usize, mut run_start: usize, mut run_color: Paint) {
    let RowCtx { theme, m, px, y, .. } = *ctx;
    let (cw, ch) = (m.cell_width as i32, m.cell_height as i32);
    let default_bg = theme.paint(slot::BACKGROUND);
    if ctx.sel.is_empty() && ctx.block_cursor.is_empty() {
        // Nothing but the default background past `from`.
        if run_color != default_bg {
            push_run(out, run_color, default_bg, run_start, from, px, y, cw, ch);
        }
        return;
    }
    for col in from..ctx.cols {
        let bg = if ctx.block_cursor.contains(&col) {
            theme.paint(slot::CURSOR)
        } else if ctx.sel.contains(&col) {
            theme.paint(slot::SELECTION)
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
/// has one (§10.5). Single and double underlines, overline and strike are
/// rectangles; curly, dotted and dashed underlines are term-font's pattern
/// sprites, one glyph per cell, left out under input-method text (which
/// hides glyphs, where rectangles are drawn over).
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn decorate(
    out: &mut impl RowSink,
    glyphs: &mut impl GlyphSource,
    pen: Pen,
    theme: &Theme,
    fg: Paint,
    x: i32,
    y: i32,
    m: Metrics,
    mark: Mark,
    covered: bool,
) {
    let cw = m.cell_width as i32;
    let t = m.underline_thickness.max(1) as i32;
    let text = Mark {
        flags: mark.flags | Mark::TEXT_COLOUR,
        ..mark
    };
    let rect = |out: &mut _, x: i32, y: i32, w: i32, color: Paint, mark: Mark| {
        RowSink::decoration(out, RectInstance::painted(s16(x), s16(y), w as u16, t as u16, color), mark)
    };
    if pen.underline != Underline::None {
        let uy = y + (m.baseline + m.underline_offset) as i32;
        let (color, mark) = match pen.underline_color.kind() {
            ColorKind::Default => (fg, text),
            _ => (theme.resolve(pen.underline_color, fg), mark),
        };
        let pattern = match pen.underline {
            Underline::None => None,
            Underline::Single => {
                rect(out, x, uy, cw, color, mark);
                None
            }
            Underline::Double => {
                rect(out, x, uy, cw, color, mark);
                rect(out, x, uy + 2 * t, cw, color, mark);
                None
            }
            Underline::Dotted => Some(SpriteId::UNDERLINE_DOTTED),
            Underline::Dashed => Some(SpriteId::UNDERLINE_DASHED),
            Underline::Curly => Some(SpriteId::UNDERLINE_CURLY),
        };
        if let Some(id) = pattern
            && !covered
            && let Some(g) = glyphs.glyph_key(GlyphKey::sprite(id, 1))
        {
            // In its own colour: kept under the block cursor.
            let fixed = if mark.flags & Mark::TEXT_COLOUR == 0 {
                GlyphInstance::FIXED
            } else {
                0
            };
            out.glyph(GlyphInstance {
                x: s16(x + g.left as i32),
                y: s16(y + m.baseline as i32 - g.top as i32),
                w: g.w,
                h: g.h,
                u: g.u,
                v: g.v,
                color: color.rgba,
                is_color: GlyphInstance::flags(&g, mark.col as usize, mark.flags & Mark::DIM != 0, color.tag) | fixed,
            });
        }
    }
    if pen.attrs.contains(Attrs::OVERLINE) {
        rect(out, x, y, cw, fg, text);
    }
    if pen.attrs.contains(Attrs::STRIKE) {
        rect(out, x, y + m.baseline as i32 - m.strike_offset as i32, cw, fg, text);
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn push_run(out: &mut impl RowSink, color: Paint, default_bg: Paint, start: usize, end: usize, px: i32, y: i32, cw: i32, ch: i32) {
    if color == default_bg || end <= start {
        return;
    }
    out.background(RectInstance::painted(
        s16(px + start as i32 * cw),
        s16(y),
        ((end - start) as i32 * cw) as u16,
        ch as u16,
        color,
    ));
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
        fn is_wide(&self) -> bool {
            false
        }
        fn is_simple(&self) -> bool {
            true
        }
    }

    /// No character glyphs; sprites by key, as term-font draws underline
    /// patterns: the cells' width, a thickness tall, at the underline
    /// (`top` negative), with the sprite id in `u` and its cells in `v`.
    struct NoGlyphs;
    impl GlyphSource for NoGlyphs {
        fn glyph(&mut self, _: u32, _: FontStyle) -> Option<GlyphSlot> {
            None
        }
        fn glyph_key(&mut self, key: GlyphKey) -> Option<GlyphSlot> {
            let id = key.sprite_id()?;
            Some(GlyphSlot {
                u: id.0,
                v: key.cells as u16,
                w: M.cell_width as u16 * key.cells as u16,
                h: T,
                left: 0,
                top: -(M.underline_offset as i16),
                color: false,
            })
        }
    }

    /// The decorations of one row of `cells` at the top-left of the grid,
    /// tags aside (`colours_carry_where_they_come_from` checks those).
    fn decorations(theme: &Theme, cells: &[TestCell]) -> Vec<RectInstance> {
        let mut d = row(theme, cells).decorations;
        d.iter_mut().for_each(|r| r.tag = 0);
        d
    }

    /// One row of `cells` at the top-left of the grid.
    fn row(theme: &Theme, cells: &[TestCell]) -> Entry {
        let ctx = RowCtx {
            theme,
            m: M,
            px: 0,
            y: 0,
            cols: cells.len(),
            sel: 0..0,
            block_cursor: 0..0,
            covered: 0..0,
            subs: &[],
        };
        let mut e = Entry::default();
        build_row(&ctx, cells.iter().copied(), &mut NoGlyphs, &mut e, &mut Pieces::default());
        e
    }

    fn styled(f: impl FnOnce(&mut Style)) -> Style {
        let mut s = Style::DEFAULT;
        f(&mut s);
        s
    }

    fn line(x: i16, y: i16, w: u16, color: Rgba) -> RectInstance {
        RectInstance {
            x,
            y,
            w,
            h: T,
            color,
            tag: 0,
        }
    }

    /// Each instance knows the theme slot its colour came from, by what
    /// the cell asked for, not by its RGB: a truecolour equal to a theme
    /// colour, a cube colour and a colour a program set have none.
    #[test]
    fn colours_carry_where_they_come_from() {
        use crate::transition::slot;
        let mut theme = Theme::default();
        let bg_of = |fg: Color, bg: Color, attrs: Attrs| {
            styled(|s| {
                s.fg = fg;
                s.bg = bg;
                s.attrs = attrs;
                s.underline = Underline::Single;
            })
        };
        let fg = theme.foreground;
        let (r, g, b) = ((fg & 0xff) as u8, (fg >> 8 & 0xff) as u8, (fg >> 16 & 0xff) as u8);
        let cells = [
            TestCell('a', bg_of(Color::DEFAULT, Color::indexed(2), Attrs::empty())),
            TestCell('b', bg_of(Color::indexed(5), Color::rgb(r, g, b), Attrs::empty())),
            TestCell('c', bg_of(Color::rgb(r, g, b), Color::indexed(100), Attrs::empty())),
            TestCell('d', bg_of(Color::indexed(1), Color::DEFAULT, Attrs::INVERSE)),
        ];
        let tags = |theme: &Theme| {
            let e = row(theme, &cells);
            let bg: Vec<_> = e.backgrounds.iter().map(|r| (r.x / 16, r.tag)).collect();
            // The underlines are in the text colour.
            let fg: Vec<_> = e.decorations.iter().map(|r| ((r.x / 16) as usize, r.tag)).collect();
            (bg, fg)
        };
        let (bg, fg) = tags(&theme);
        assert_eq!(bg, [(0, tag_of(2)), (1, 0), (2, 0), (3, tag_of(1))]);
        assert_eq!(
            fg,
            [(0, tag_of(slot::FOREGROUND)), (1, tag_of(5)), (2, 0), (3, tag_of(slot::BACKGROUND))]
        );
        // The truecolour background equal to the theme's text colour is
        // drawn even so.
        assert_eq!(row(&theme, &cells).backgrounds[1].color, theme.foreground);
        // A program set ANSI 2 and the background: theirs, not the theme's.
        theme.program_set = 1 << 2 | 1 << slot::BACKGROUND;
        let (bg, fg) = tags(&theme);
        assert_eq!(bg, [(0, 0), (1, 0), (2, 0), (3, tag_of(1))]);
        assert_eq!(fg[3], (3, 0));
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
    fn patterned_underlines_are_one_sprite_per_cell() {
        let theme = Theme::default();
        let fg = theme.foreground;
        for (underline, id) in [
            (Underline::Curly, SpriteId::UNDERLINE_CURLY),
            (Underline::Dotted, SpriteId::UNDERLINE_DOTTED),
            (Underline::Dashed, SpriteId::UNDERLINE_DASHED),
        ] {
            let s = styled(|s| s.underline = underline);
            let e = row(&theme, &[TestCell('a', s), TestCell(' ', s)]);
            assert_eq!(e.decorations, [], "{underline:?}");
            let got: Vec<_> = e.glyphs.iter().map(|g| (g.x, g.y, g.w, g.h, g.u, g.v, g.color, g.col())).collect();
            // At the underline, the sprite across one cell, in each cell.
            let at = |col: i16| (col * 16, UY, 16, T, id.0, 1, fg, col as usize);
            assert_eq!(got, [at(0), at(1)], "{underline:?}");
            assert!(e.glyphs.iter().all(|g| g.is_color & GlyphInstance::FIXED == 0));
        }
    }

    #[test]
    fn patterned_underlines_in_their_own_colour_keep_it_under_the_cursor() {
        let theme = Theme::default();
        let blue = styled(|s| {
            s.underline = Underline::Curly;
            s.underline_color = Color::rgb(0, 0, 255);
        });
        let dim = styled(|s| {
            s.underline = Underline::Dotted;
            s.attrs = Attrs::DIM;
        });
        let e = row(&theme, &[TestCell('a', blue), TestCell('b', dim)]);
        let got: Vec<_> = e
            .glyphs
            .iter()
            .map(|g| (g.color, g.is_color & (GlyphInstance::FIXED | GlyphInstance::DIM)))
            .collect();
        assert_eq!(
            got,
            [
                (rgb(0x0000ff), GlyphInstance::FIXED),
                (with_alpha(theme.foreground, 160), GlyphInstance::DIM)
            ]
        );
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
