//! Frame building (`Frame::build`, no Metal): screens set up through
//! term-core's public API, turned into instances with a fake glyph source,
//! and checked for text, background runs, styles, decorations, cursor,
//! input-method text, padding, position clamping and theme colours.
//!
//! Every frame is built twice: from a snapshot (`Frame::build`, what the
//! app draws) and from the live terminal (`Frame::build_live`, the path it
//! replaced, kept as a reference for one PR). Both must give the same
//! picture, drawn here on the CPU, and the same instances when nothing is
//! drawn over the rows (cells design §17.3, invariant 7). A property test
//! keeps one frame across many changes, so its row cache is exercised.

use proptest::prelude::*;
use term_core::{Attrs, CursorShape, Point, Selection, Term, Terminal};
use term_font::{Metrics, Style};
use term_render::{Frame, FrameInput, GlyphInstance, GlyphSlot, GlyphSource, RectInstance, Rgba, Theme, rgb};

const M: Metrics = Metrics {
    cell_width: 10,
    cell_height: 20,
    baseline: 15,
    underline_offset: 2,
    underline_thickness: 1,
    strike_offset: 4,
};
const CW: i16 = M.cell_width as i16;
const CH: i16 = M.cell_height as i16;

/// Characters the fake source has no glyph for, like a real space.
const NO_GLYPH: [char; 2] = [' ', '·'];

/// Every glyph is "in the texture", with its codepoint in `u` (low 16 bits)
/// and `v` (the rest), placed exactly at the cell's top-left corner.
struct Fake;
impl GlyphSource for Fake {
    fn glyph(&mut self, cp: u32, _style: Style) -> Option<GlyphSlot> {
        if NO_GLYPH.contains(&char::from_u32(cp)?) {
            return None;
        }
        Some(GlyphSlot {
            u: cp as u16,
            v: (cp >> 16) as u16,
            w: M.cell_width as u16,
            h: M.cell_height as u16,
            left: 0,
            top: M.baseline as i16,
            color: false,
        })
    }
}

fn decode_cp(g: &GlyphInstance) -> char {
    char::from_u32(g.u as u32 | (g.v as u32) << 16).unwrap()
}

fn term(cols: usize, lines: usize, bytes: &str) -> Term {
    let mut t = Term::new(cols, lines, 100, term_core::ImagePool::process());
    t.advance(bytes.as_bytes());
    t
}

struct Opts<'a> {
    padding: (u32, u32),
    cursor_on: bool,
    focused: bool,
    preedit: Option<&'a str>,
    metrics: Metrics,
}

impl Default for Opts<'_> {
    fn default() -> Self {
        Opts {
            padding: (0, 0),
            cursor_on: true,
            focused: true,
            preedit: None,
            metrics: M,
        }
    }
}

fn input<'a>(theme: &'a Theme, o: &Opts<'a>) -> FrameInput<'a> {
    FrameInput {
        theme,
        metrics: o.metrics,
        padding: o.padding,
        cursor_on: o.cursor_on,
        focused: o.focused,
        preedit: o.preedit,
    }
}

/// Something is drawn over the rows: the frame's instances differ from the
/// reference path's, though the picture must not.
fn has_overlay(t: &Terminal, o: &Opts) -> bool {
    t.selection().is_some()
        || o.preedit.is_some_and(|p| !p.is_empty())
        || (o.focused && t.cursor_shape() == CursorShape::Block && t.modes().cursor_visible && t.cursor_in_view().is_some())
}

/// Build `t`'s frame with `frame` (whose row cache may hold earlier
/// frames' rows), and check it against the reference path.
fn build_into(frame: &mut Frame, t: &mut Terminal, theme: &Theme, o: &Opts) {
    let input = input(theme, o);
    frame.build(t, &input, &mut Fake);
    let mut live = Frame::default();
    live.build_live(t, &input, &mut Fake);
    let size = (t.cols(), t.lines());
    let pixels = size.0 * size.1 * (o.metrics.cell_width * o.metrics.cell_height) as usize;
    // Huge grids (the clamping test) are compared by instances only.
    let (a, b) = if pixels <= 1 << 22 {
        (picture(frame, size, o), picture(&live, size, o))
    } else {
        assert!(!has_overlay(t, o), "too big to draw");
        (vec![], vec![])
    };
    if let Some(i) = a.iter().zip(&b).position(|(a, b)| a != b) {
        let w = size.0 * o.metrics.cell_width as usize + 2 * o.padding.0 as usize;
        panic!(
            "snapshot and live frames differ at pixel ({}, {}): {:?} vs {:?}\n{}\nrects {:?}\nlive {:?}",
            i % w,
            i / w,
            a[i],
            b[i],
            t.dump(),
            frame.rects,
            live.rects,
        );
    }
    if !has_overlay(t, o) {
        assert_eq!(frame.rects, live.rects, "rects\n{}", t.dump());
        assert_eq!(frame.glyphs, live.glyphs, "glyphs\n{}", t.dump());
    }
    assert_eq!(frame.clear, live.clear);
    assert!(frame.cached_rows() <= t.lines(), "{} rows cached", frame.cached_rows());
}

fn build_with(t: &mut Terminal, theme: &Theme, o: Opts) -> Frame {
    let mut f = Frame::default();
    build_into(&mut f, t, theme, &o);
    f
}

fn build(t: &mut Terminal) -> Frame {
    build_with(t, &Theme::default(), Opts::default())
}

/// What a pixel shows: its background after every rect, and the glyph on
/// top (texture position and colour), if any.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Px {
    bg: [u8; 4],
    glyph: Option<(u16, u16, Rgba)>,
}

/// The frame drawn on the CPU as Metal draws it: the clear colour, rects
/// in order blended "over", then glyphs in order on top (the fake glyphs
/// are solid, so the last one wins).
fn picture(f: &Frame, (cols, lines): (usize, usize), o: &Opts) -> Vec<Px> {
    let (cw, ch) = (o.metrics.cell_width as usize, o.metrics.cell_height as usize);
    let w = cols * cw + 2 * o.padding.0 as usize;
    let h = lines * ch + 2 * o.padding.1 as usize;
    let clear = Px {
        bg: f.clear.to_le_bytes(),
        glyph: None,
    };
    let mut px = vec![clear; w * h];
    let span = |at: i16, len: u16, max: usize| {
        let end = (at as i32 + len as i32).clamp(0, max as i32) as usize;
        (at.max(0) as usize).min(end)..end
    };
    for r in &f.rects {
        let src = r.color.to_le_bytes();
        let a = src[3] as u32;
        for y in span(r.y, r.h, h) {
            for x in span(r.x, r.w, w) {
                let d = &mut px[y * w + x].bg;
                for i in 0..4 {
                    d[i] = ((src[i] as u32 * a + d[i] as u32 * (255 - a) + 127) / 255) as u8;
                }
            }
        }
    }
    for g in &f.glyphs {
        for y in span(g.y, g.h, h) {
            for x in span(g.x, g.w, w) {
                px[y * w + x].glyph = Some((g.u, g.v, g.color));
            }
        }
    }
    px
}

/// Hide the cursor so it doesn't add rects of its own.
const NO_CURSOR: &str = "\x1b[?25l";

fn rect(col: i16, line: i16, cells: u16, color: Rgba) -> RectInstance {
    RectInstance {
        x: col * CW,
        y: line * CH,
        w: cells * CW as u16,
        h: CH as u16,
        color,
    }
}

/// The glyphs as a grid of characters, by position → cell.
fn decoded(f: &Frame, cols: usize, lines: usize, padding: (u32, u32)) -> Vec<String> {
    let mut grid = vec![vec![' '; cols]; lines];
    for g in &f.glyphs {
        let (x, y) = (g.x as i32 - padding.0 as i32, g.y as i32 - padding.1 as i32);
        assert!(x >= 0 && y >= 0 && x % CW as i32 == 0 && y % CH as i32 == 0, "{g:?} not on a cell");
        let (col, line) = ((x / CW as i32) as usize, (y / CH as i32) as usize);
        assert!(col < cols && line < lines, "{g:?} outside the grid");
        assert_eq!(grid[line][col], ' ', "two glyphs in cell {col},{line}");
        grid[line][col] = decode_cp(g);
    }
    grid.into_iter().map(|r| r.into_iter().collect()).collect()
}

/// What the grid should show: one character per column, spaces for empty
/// cells, wide-character spacers, hidden text and characters without a glyph.
fn grid_text(t: &Terminal) -> Vec<String> {
    (0..t.lines())
        .map(|line| {
            let row = t.visible_row(line);
            (0..t.cols())
                .map(|col| {
                    let c = row.cell(col);
                    if c.is_spacer() || c.style().attrs.contains(Attrs::HIDDEN) || NO_GLYPH.contains(&c.ch()) {
                        ' '
                    } else {
                        c.ch()
                    }
                })
                .collect()
        })
        .collect()
}

// ---- text ----------------------------------------------------------------

fn token() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "a",
        "Z",
        "~",
        " ",
        "·",
        "é",
        "漢",
        "字",
        "😀",
        "→",
        "\r\n",
        "\n",
        "\r",
        "\t",
        "\x08",
        "\x1b[1m",
        "\x1b[3m",
        "\x1b[7m",
        "\x1b[8m",
        "\x1b[2m",
        "\x1b[4m",
        "\x1b[9m",
        "\x1b[0m",
        "\x1b[31m",
        "\x1b[44m",
        "\x1b[38;2;1;2;3m",
        "\x1b[H",
        "\x1b[2;3H",
        "\x1b[K",
        "\x1b[2J",
        "\x1b[3D",
        "\x1b[@",
        "\x1b[P",
        "\x1b[L",
        "\x1b[M",
    ])
}

proptest! {
    #[test]
    fn glyphs_spell_out_the_visible_grid(
        cols in 1usize..30,
        lines in 1usize..8,
        input in prop::collection::vec(token(), 0..200),
        scroll in 0isize..20,
        focused: bool,
    ) {
        let mut t = term(cols, lines, &input.concat());
        t.state.scroll_view(scroll);
        let theme = Theme::default();
        let f = build_with(&mut t.state, &theme, Opts { focused, ..Opts::default() });
        // term-core may round tiny sizes up.
        let (cols, lines) = (t.state.cols(), t.state.lines());
        prop_assert_eq!(decoded(&f, cols, lines, (0, 0)), grid_text(&t.state));
    }
}

/// Everything that changes rows, moves them or changes what is drawn over
/// them, for the row cache.
fn change() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "x",
        "漢",
        "\x1b[?1049h",
        "\x1b[?1049l",
        "\x1b[2;3r",
        "\x1b[r",
        "\x1b[S",
        "\x1b[T",
        "\x1bc",
        "\x1b[3J",
        "\x1b[2 q",
        "\x1b[4 q",
        "\x1b[6 q",
        "\x1b[1 q",
        "\x1b[?25l",
        "\x1b[?25h",
        "\x1b[4:3;58;5;3m",
        "\x1b[53;21m",
        "\x1b[4;2m",
        "\x1b[7;44m",
        "\x1b[3;1H",
        "\x1b[1;1H\x1b[2K",
        "\x1b#8",
    ])
}

#[derive(Debug, Clone)]
struct Step {
    bytes: Vec<&'static str>,
    scroll: isize,
    /// View rows and columns of the selection's ends.
    selection: Option<((usize, usize), (usize, usize))>,
    resize: Option<(usize, usize)>,
    focused: bool,
    cursor_on: bool,
    preedit: Option<&'static str>,
    padding: (u32, u32),
}

fn step() -> impl Strategy<Value = Step> {
    (
        prop::collection::vec(prop_oneof![3 => token(), 1 => change()], 0..40),
        -3isize..4,
        prop::option::weighted(0.3, ((0usize..8, 0usize..30), (0usize..8, 0usize..30))),
        prop::option::weighted(0.1, (1usize..30, 1usize..8)),
        prop::bool::weighted(0.8),
        prop::bool::weighted(0.8),
        prop::option::weighted(0.2, prop::sample::select(vec!["q", "漢字", "→x"])),
        prop::sample::select(vec![(0, 0), (0, 0), (0, 7), (3, 0)]),
    )
        .prop_map(|(bytes, scroll, selection, resize, focused, cursor_on, preedit, padding)| Step {
            bytes,
            scroll,
            selection,
            resize,
            focused,
            cursor_on,
            preedit,
            padding,
        })
}

proptest! {
    /// One frame kept across many frames, as the app keeps it: rows reused
    /// from its cache, rows copied, scrolled history, the alternate screen,
    /// resets, resizes, style table sweeps, selection, cursor and
    /// input-method text. Each frame must draw what the live terminal
    /// shows (`build_into` compares them).
    #[test]
    fn frames_kept_across_changes_match_the_live_terminal(
        cols in 1usize..30,
        lines in 1usize..8,
        steps in prop::collection::vec(step(), 1..12),
    ) {
        let mut t = Term::new(cols, lines, 20, term_core::ImagePool::process());
        t.state.set_sweep_at(Some(8));
        let theme = Theme::default();
        let mut frame = Frame::default();
        for s in steps {
            t.advance(s.bytes.concat().as_bytes());
            if let Some((cols, lines)) = s.resize {
                t.state.resize(cols, lines);
            }
            t.state.scroll_view(s.scroll);
            let (cols, lines) = (t.state.cols(), t.state.lines());
            let at = |(l, c): (usize, usize)| {
                let mut p = t.state.point_at(l.min(lines - 1), 0);
                p.col = c.min(cols - 1);
                p
            };
            let selection = s.selection.map(|(a, b)| Selection::new(at(a), at(b)));
            t.state.set_selection(selection);
            let o = Opts {
                focused: s.focused,
                cursor_on: s.cursor_on,
                preedit: s.preedit,
                padding: s.padding,
                ..Opts::default()
            };
            build_into(&mut frame, &mut t.state, &theme, &o);
            if s.preedit.is_none() {
                prop_assert_eq!(decoded(&frame, cols, lines, s.padding), grid_text(&t.state));
            }
        }
    }
}

#[test]
fn glyph_text_matches_simple_screen() {
    let mut t = term(6, 2, "ab cd\r\n漢x·y");
    let f = build(&mut t.state);
    assert_eq!(decoded(&f, 6, 2, (0, 0)), ["ab cd ", "漢 x y "]);
}

// ---- background runs -----------------------------------------------------

#[test]
fn default_background_emits_nothing() {
    let mut t = term(10, 3, &format!("{NO_CURSOR}hello\r\n\x1b[1mworld"));
    let f = build(&mut t.state);
    assert!(f.rects.is_empty(), "{:?}", f.rects);
    assert_eq!(f.clear, Theme::default().background);
}

#[test]
fn same_colour_cells_merge_into_one_run() {
    let mut t = term(10, 2, &format!("{NO_CURSOR}\x1b[41mabc\x1b[0m  \x1b[42mde\x1b[0m\r\n\x1b[41mx"));
    let theme = Theme::default();
    let f = build(&mut t.state);
    assert_eq!(
        f.rects,
        [
            rect(0, 0, 3, theme.palette[1]),
            rect(5, 0, 2, theme.palette[2]),
            rect(0, 1, 1, theme.palette[1])
        ]
    );
}

/// The selection is drawn over the rows' backgrounds, which don't change
/// with it (cells design §9.3).
#[test]
fn selection_is_drawn_over_the_run() {
    let mut t = term(10, 1, &format!("{NO_CURSOR}\x1b[41mabcdef"));
    let line = t.state.point_at(0, 0).line;
    t.state
        .set_selection(Some(Selection::new(Point { line, col: 3 }, Point { line, col: 2 })));
    let theme = Theme::default();
    let f = build(&mut t.state);
    assert_eq!(f.rects, [rect(0, 0, 6, theme.palette[1]), rect(2, 0, 2, theme.selection)]);
}

#[test]
fn selection_covers_short_history_rows_to_the_edge() {
    // "ab" scrolls into history, which may store it shorter than the
    // screen: the selection still reaches the last column.
    let mut t = term(10, 2, &format!("{NO_CURSOR}ab\r\ncd\r\nef"));
    t.state.scroll_view(1);
    assert!(t.state.visible_row(0).len() < 10, "history row stored short");
    let line = t.state.point_at(0, 0).line;
    t.state
        .set_selection(Some(Selection::new(Point { line, col: 1 }, Point { line, col: 9 })));
    let theme = Theme::default();
    let f = build(&mut t.state);
    assert_eq!(f.rects, [rect(1, 0, 9, theme.selection)]);
    assert_eq!(decoded(&f, 10, 2, (0, 0)), ["ab        ", "cd        "]);
}

#[test]
fn run_breaks_at_inverse() {
    let mut t = term(10, 1, &format!("{NO_CURSOR}\x1b[41mab\x1b[7mcd\x1b[27mef"));
    let theme = Theme::default();
    let f = build(&mut t.state);
    assert_eq!(
        f.rects,
        [
            rect(0, 0, 2, theme.palette[1]),
            rect(2, 0, 2, theme.foreground),
            rect(4, 0, 2, theme.palette[1])
        ]
    );
}

#[test]
fn block_cursor_is_drawn_over_the_run() {
    let mut t = term(10, 1, "\x1b[41mabcdef\x1b[4G");
    let theme = Theme::default();
    let f = build(&mut t.state);
    assert_eq!(f.rects, [rect(0, 0, 6, theme.palette[1]), rect(3, 0, 1, theme.cursor)]);
}

// ---- styles --------------------------------------------------------------

fn glyph_at(f: &Frame, col: i16, line: i16) -> Option<&GlyphInstance> {
    f.glyphs.iter().find(|g| g.x == col * CW && g.y == line * CH)
}

#[test]
fn inverse_swaps_colours() {
    let mut t = term(10, 1, &format!("{NO_CURSOR}\x1b[7ma\x1b[0m\x1b[31;42;7mb"));
    let theme = Theme::default();
    let f = build(&mut t.state);
    assert_eq!(glyph_at(&f, 0, 0).unwrap().color, theme.background);
    assert_eq!(glyph_at(&f, 1, 0).unwrap().color, theme.palette[2]);
    assert_eq!(f.rects, [rect(0, 0, 1, theme.foreground), rect(1, 0, 1, theme.palette[1])]);
}

#[test]
fn dim_lowers_alpha() {
    let mut t = term(10, 1, "a\x1b[2mb");
    let theme = Theme::default();
    let f = build(&mut t.state);
    let (a, b) = (glyph_at(&f, 0, 0).unwrap().color, glyph_at(&f, 1, 0).unwrap().color);
    assert_eq!(a, theme.foreground);
    assert_eq!(b & 0x00FF_FFFF, theme.foreground & 0x00FF_FFFF, "same colour");
    assert!(b >> 24 < 255 && b >> 24 > 0, "lower alpha: {b:#x}");
}

#[test]
fn hidden_emits_no_glyph() {
    let mut t = term(10, 1, "a\x1b[8mbc\x1b[28md");
    let f = build(&mut t.state);
    assert_eq!(decoded(&f, 10, 1, (0, 0)), ["a  d      "]);
}

#[test]
fn wide_glyph_emitted_once() {
    let mut t = term(10, 1, "漢字😀");
    let f = build(&mut t.state);
    assert_eq!(f.glyphs.len(), 3);
    assert_eq!(decoded(&f, 10, 1, (0, 0)), ["漢 字 😀     "]);
}

#[test]
fn cluster_cells_without_fonts_draw_their_first_character() {
    // The fake source has no fonts to shape with: a cluster draws its
    // base (clusters shaped with real fonts are tested in clusters.rs).
    let mut t = term(10, 1, "e\u{301}❤\u{FE0F}🇬🇧x");
    let f = build(&mut t.state);
    assert_eq!(decoded(&f, 10, 1, (0, 0)), ["e❤ 🇬 x    "]);
}

// ---- decorations ---------------------------------------------------------

#[test]
fn underline_positions_from_metrics() {
    let mut t = term(10, 2, &format!("{NO_CURSOR}\r\n\x1b[4ma\x1b[24;21mb\x1b[24;9mc"));
    let fg = Theme::default().foreground;
    let f = build(&mut t.state);
    let uy = CH + (M.baseline + M.underline_offset) as i16;
    let line = |col: i16, y: i16| RectInstance {
        x: col * CW,
        y,
        w: CW as u16,
        h: M.underline_thickness as u16,
        color: fg,
    };
    let double_gap = 2 * M.underline_thickness as i16;
    let strike_y = CH + M.baseline as i16 - M.strike_offset as i16;
    assert_eq!(f.rects, [line(0, uy), line(1, uy), line(1, uy + double_gap), line(2, strike_y)]);
}

#[test]
fn decorations_take_the_text_colour_as_drawn() {
    // Red underline; inverse strike (the text colour is the background);
    // a dim underline (the text colour's alpha). Underline colours set
    // with SGR 58 are tested in frame.rs until term-core keeps them.
    let mut t = term(10, 1, &format!("{NO_CURSOR}\x1b[31;4ma\x1b[0;7;9mb\x1b[0;2;4mc"));
    let theme = Theme::default();
    let f = build(&mut t.state);
    let thin: Vec<_> = f
        .rects
        .iter()
        .filter(|r| r.h == M.underline_thickness as u16)
        .map(|r| (r.x / CW, r.color))
        .collect();
    let dim = (theme.foreground & 0x00FF_FFFF) | 160 << 24;
    assert_eq!(thin, [(0, theme.palette[1]), (1, theme.background), (2, dim)]);
}

#[test]
fn decorations_follow_thicker_metrics() {
    let m = Metrics {
        underline_thickness: 3,
        underline_offset: 4,
        ..M
    };
    let mut t = term(4, 1, &format!("{NO_CURSOR}\x1b[21ma"));
    let theme = Theme::default();
    let f = build_with(
        &mut t.state,
        &theme,
        Opts {
            metrics: m,
            ..Opts::default()
        },
    );
    let ys: Vec<_> = f.rects.iter().map(|r| (r.y, r.h)).collect();
    assert_eq!(ys, [(19, 3), (25, 3)]);
}

#[test]
fn hovered_link_is_underlined_in_the_text_colour() {
    let mut t = term(10, 2, &format!("{NO_CURSOR}a \x1b]8;;https://x\x1b\\link\x1b]8;;\x1b\\ b"));
    let theme = Theme::default();
    let plain = build(&mut t.state);
    // Only the snapshot path draws the hovered link: not compared with the
    // reference path.
    let hovered = |t: &mut Term| {
        let mut f = Frame::default();
        f.build(&mut t.state, &input(&theme, &Opts::default()), &mut Fake);
        f
    };
    t.state.set_hover(Some(t.state.point_at(0, 3)));
    let f = hovered(&mut t);
    let underline = RectInstance {
        x: 2 * CW,
        y: (M.baseline + M.underline_offset) as i16,
        w: 4 * CW as u16,
        h: M.underline_thickness as u16,
        color: theme.foreground,
    };
    assert_eq!(f.rects, [plain.rects.clone(), vec![underline]].concat());
    t.state.set_hover(Some(t.state.point_at(0, 0)));
    assert_eq!(hovered(&mut t).rects, plain.rects, "off the link");
}

// ---- cursor --------------------------------------------------------------

fn cursor_rects(f: &Frame, theme: &Theme) -> Vec<RectInstance> {
    f.rects.iter().copied().filter(|r| r.color == theme.cursor).collect()
}

#[test]
fn block_cursor_swaps_cell_colours() {
    let mut t = term(10, 2, "ab\x1b[1G");
    let theme = Theme::default();
    let f = build(&mut t.state);
    assert_eq!(f.rects, [rect(0, 0, 1, theme.cursor)]);
    assert_eq!(glyph_at(&f, 0, 0).unwrap().color, theme.cursor_text);
    assert_eq!(glyph_at(&f, 1, 0).unwrap().color, theme.foreground);
}

#[test]
fn bar_cursor() {
    let mut t = term(10, 2, "ab\x1b[6 q");
    let theme = Theme::default();
    let f = build(&mut t.state);
    let t_px = (M.cell_width as i16 / 8).max(2);
    assert_eq!(
        cursor_rects(&f, &theme),
        [RectInstance {
            x: 2 * CW,
            y: 0,
            w: t_px as u16,
            h: CH as u16,
            color: theme.cursor
        }]
    );
    assert_eq!(glyph_at(&f, 1, 0).unwrap().color, theme.foreground, "bar doesn't recolour text");
}

#[test]
fn underline_cursor() {
    let mut t = term(10, 2, "\r\nab\x1b[4 q");
    let theme = Theme::default();
    let f = build(&mut t.state);
    let t_px = (M.cell_width as i16 / 8).max(2);
    assert_eq!(
        cursor_rects(&f, &theme),
        [RectInstance {
            x: 2 * CW,
            y: 2 * CH - t_px,
            w: CW as u16,
            h: t_px as u16,
            color: theme.cursor
        }]
    );
}

fn hollow_box(col: i16, line: i16, cells: i16, color: Rgba) -> Vec<RectInstance> {
    let (x, y, w, h) = (col * CW, line * CH, cells * CW, CH);
    let r = |x: i16, y: i16, w: i16, h: i16| RectInstance {
        x,
        y,
        w: w as u16,
        h: h as u16,
        color,
    };
    vec![r(x, y, w, 1), r(x, y + h - 1, w, 1), r(x, y, 1, h), r(x + w - 1, y, 1, h)]
}

#[test]
fn unfocused_cursor_is_a_hollow_box() {
    let theme = Theme::default();
    for shape in ["\x1b[2 q", "\x1b[4 q", "\x1b[6 q"] {
        let mut t = term(10, 2, &format!("ab{shape}"));
        let f = build_with(
            &mut t.state,
            &theme,
            Opts {
                focused: false,
                ..Opts::default()
            },
        );
        assert_eq!(cursor_rects(&f, &theme), hollow_box(2, 0, 1, theme.cursor), "{shape:?}");
        assert!(f.glyphs.iter().all(|g| g.color == theme.foreground));
    }
}

#[test]
fn cursor_hidden_when_cursor_visible_off() {
    let theme = Theme::default();
    let mut t = term(10, 2, "ab\x1b[?25l");
    for focused in [true, false] {
        let f = build_with(
            &mut t.state,
            &theme,
            Opts {
                focused,
                ..Opts::default()
            },
        );
        assert!(f.rects.is_empty(), "focused={focused}: {:?}", f.rects);
    }
}

#[test]
fn cursor_hidden_in_blink_off_phase_only_when_blinking_and_focused() {
    let theme = Theme::default();
    let mut blinking = term(10, 2, "ab\x1b[1 q");
    assert!(blinking.state.cursor_blink());
    let off = |t: &mut Term, focused| {
        build_with(
            &mut t.state,
            &theme,
            Opts {
                cursor_on: false,
                focused,
                ..Opts::default()
            },
        )
    };
    assert!(off(&mut blinking, true).rects.is_empty(), "blink phase off hides it");
    assert_eq!(cursor_rects(&off(&mut blinking, false), &theme).len(), 4, "unfocused doesn't blink");
    let mut steady = term(10, 2, "ab\x1b[2 q");
    assert_eq!(
        off(&mut steady, true).rects,
        [rect(2, 0, 1, theme.cursor)],
        "steady cursor ignores the phase"
    );
}

#[test]
fn cursor_hidden_when_scrolled_out_of_view() {
    let theme = Theme::default();
    let mut t = term(10, 3, "1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7");
    let f = build(&mut t.state);
    assert_eq!(cursor_rects(&f, &theme), [rect(1, 2, 1, theme.cursor)]);
    t.state.scroll_view(1);
    let f = build(&mut t.state);
    assert!(cursor_rects(&f, &theme).is_empty(), "cursor line pushed below the view");
    t.state.scroll_view(-1);
    assert_eq!(cursor_rects(&build(&mut t.state), &theme).len(), 1);
}

#[test]
fn cursor_on_wide_char_is_two_cells() {
    let theme = Theme::default();
    let mut t = term(10, 2, "a漢b\x1b[2G");
    assert_eq!(t.state.cursor().col(), 1);
    let f = build(&mut t.state);
    assert_eq!(f.rects, [rect(1, 0, 2, theme.cursor)], "block covers both halves");
    assert_eq!(glyph_at(&f, 1, 0).unwrap().color, theme.cursor_text);

    let f = build_with(
        &mut t.state,
        &theme,
        Opts {
            focused: false,
            ..Opts::default()
        },
    );
    assert_eq!(cursor_rects(&f, &theme), hollow_box(1, 0, 2, theme.cursor));

    let mut t = term(10, 2, "a漢b\x1b[2G\x1b[4 q");
    let f = build(&mut t.state);
    assert_eq!(cursor_rects(&f, &theme)[0].w, 2 * CW as u16);
}

// ---- input-method text ---------------------------------------------------

#[test]
fn preedit_suppresses_glyphs_under_it() {
    let theme = Theme::default();
    let mut t = term(10, 2, "\r\nabcdef\x1b[2G");
    let f = build_with(
        &mut t.state,
        &theme,
        Opts {
            preedit: Some("xy"),
            ..Opts::default()
        },
    );
    assert_eq!(decoded(&f, 10, 2, (0, 0)), ["          ", "axydef    "]);
    // Each preedit cell gets a background and an underline.
    let bg: Vec<_> = f.rects.iter().filter(|r| r.color == theme.background).collect();
    assert_eq!(bg.len(), 2);
    assert_eq!((bg[0].x, bg[0].y), (CW, CH));
}

#[test]
fn preedit_follows_the_cursor_in_view() {
    let theme = Theme::default();
    let mut t = term(10, 2, "1\r\n2\r\n3\r\nab");
    let opts = || Opts {
        preedit: Some("xy"),
        ..Opts::default()
    };
    assert_eq!(
        decoded(&build_with(&mut t.state, &theme, opts()), 10, 2, (0, 0)),
        ["3         ", "abxy      "]
    );
    // Scrolled back by one row, the cursor's row is out of view: no preedit.
    t.state.scroll_view(1);
    let f = build_with(&mut t.state, &theme, opts());
    assert_eq!(decoded(&f, 10, 2, (0, 0)), ["2         ", "3         "]);
    assert!(f.rects.iter().all(|r| r.color != theme.background), "no preedit background");
}

#[test]
fn preedit_clipped_at_right_edge() {
    let mut t = term(5, 1, "abc");
    let theme = Theme::default();
    let f = build_with(
        &mut t.state,
        &theme,
        Opts {
            preedit: Some("xyz"),
            ..Opts::default()
        },
    );
    assert_eq!(decoded(&f, 5, 1, (0, 0)), ["abcxy"]);
    // A wide character that doesn't fit is dropped whole.
    let mut t = term(5, 1, "abcd");
    let f = build_with(
        &mut t.state,
        &theme,
        Opts {
            preedit: Some("漢"),
            ..Opts::default()
        },
    );
    assert_eq!(decoded(&f, 5, 1, (0, 0)), ["abcd "]);
}

/// Regression: preedit used "below U+1100 is 1 cell, else 2", so '→'
/// (U+2192) took two cells while term-core prints it in one.
#[test]
fn preedit_width_matches_term_core() {
    let theme = Theme::default();
    let mut t = term(8, 1, "abcdefg\x1b[1G");
    let f = build_with(
        &mut t.state,
        &theme,
        Opts {
            preedit: Some("→漢x"),
            ..Opts::default()
        },
    );
    assert_eq!(decoded(&f, 8, 1, (0, 0)), ["→漢 xefg "]);

    // And the same text printed by term-core lands in the same cells.
    let printed = term(8, 1, "→漢xefg");
    assert_eq!(grid_text(&printed.state), ["→漢 xefg "]);
}

// ---- layout --------------------------------------------------------------

#[test]
fn padding_offsets_everything() {
    let theme = Theme::default();
    let screen = "\x1b[41mab\x1b[0m \x1b[4mcd\x1b[0m\r\n漢\x1b[9mz\x1b[0m";
    let mut t = term(8, 3, screen);
    let pad = (7u32, 9u32);
    let plain = build_with(
        &mut t.state,
        &theme,
        Opts {
            preedit: Some("q"),
            ..Opts::default()
        },
    );
    let padded = build_with(
        &mut t.state,
        &theme,
        Opts {
            preedit: Some("q"),
            padding: pad,
            ..Opts::default()
        },
    );
    let (dx, dy) = (pad.0 as i16, pad.1 as i16);
    let moved: Vec<_> = plain
        .rects
        .iter()
        .map(|r| RectInstance {
            x: r.x + dx,
            y: r.y + dy,
            ..*r
        })
        .collect();
    assert_eq!(padded.rects, moved);
    let moved: Vec<_> = plain
        .glyphs
        .iter()
        .map(|g| GlyphInstance {
            x: g.x + dx,
            y: g.y + dy,
            ..*g
        })
        .collect();
    assert_eq!(padded.glyphs, moved);
    assert!(!plain.rects.is_empty() && !plain.glyphs.is_empty());
}

#[test]
fn positions_clamp_on_huge_grids() {
    let m = Metrics {
        cell_width: 200,
        cell_height: 400,
        ..M
    };
    let (cols, lines) = (300, 100);
    let mut text = String::from(NO_CURSOR);
    for l in 0..lines {
        text.push_str(&"a".repeat(cols));
        if l + 1 < lines {
            text.push_str("\r\n");
        }
    }
    let mut t = term(cols, lines, &text);
    let theme = Theme::default();
    let f = build_with(
        &mut t.state,
        &theme,
        Opts {
            metrics: m,
            ..Opts::default()
        },
    );
    assert_eq!(f.glyphs.len(), cols * lines);
    // Row-major order: x never decreases along a line, y never decreases
    // down the screen, and nothing wraps around to negative.
    for pair in f.glyphs.chunks(cols) {
        assert!(pair.windows(2).all(|w| w[0].x <= w[1].x && w[0].y == w[1].y));
    }
    assert!(f.glyphs.windows(2).all(|w| w[0].y <= w[1].y));
    assert!(f.glyphs.iter().all(|g| g.x >= 0 && g.y >= 0));
    let last = f.glyphs.last().unwrap();
    assert_eq!((last.x, last.y), (i16::MAX, i16::MAX));

    // Scrolled by a line and drawn again from the row cache: rows moved
    // up from past the clamp come out where the live terminal puts them.
    let mut f = f;
    for line in ["\r\nb", "\r\nc\x1b[3;1Hd"] {
        t.advance(line.as_bytes());
        let o = Opts {
            metrics: m,
            ..Opts::default()
        };
        build_into(&mut f, &mut t.state, &theme, &o);
    }
}

// ---- theme ---------------------------------------------------------------

#[test]
fn rgb_byte_order() {
    assert_eq!(rgb(0x112233), 0xFF33_2211, "red in the low byte, alpha on top");
    assert_eq!(rgb(0xFF0000).to_le_bytes(), [0xFF, 0, 0, 0xFF]);
}

#[test]
fn palette_cube_and_grey_ramp_match_xterm() {
    let p = Theme::default().palette;
    // xterm's 6×6×6 cube levels and 24-step grey ramp.
    let levels = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    for r in 0..6 {
        for g in 0..6 {
            for b in 0..6 {
                let i = 16 + 36 * r + 6 * g + b;
                assert_eq!(p[i], rgb(levels[r] << 16 | levels[g] << 8 | levels[b]), "index {i}");
            }
        }
    }
    for i in 0..24u32 {
        let v = 0x08 + 0x0a * i;
        assert_eq!(p[232 + i as usize], rgb(v << 16 | v << 8 | v), "index {}", 232 + i);
    }
    assert_eq!(p[67], rgb(0x5f87af));
    assert_eq!(p[208], rgb(0xff8700));
    assert_eq!(p[244], rgb(0x808080));
    assert_eq!(p[255], rgb(0xeeeeee));
}

#[test]
fn indexed_and_truecolour_cells_resolve_exactly() {
    let mut t = term(
        10,
        1,
        &format!("{NO_CURSOR}\x1b[38;2;1;2;3;48;2;250;128;7ma\x1b[38;5;67;48;5;244mb"),
    );
    let theme = Theme::default();
    let f = build(&mut t.state);
    assert_eq!(glyph_at(&f, 0, 0).unwrap().color, 0xFF03_0201);
    assert_eq!(glyph_at(&f, 1, 0).unwrap().color, rgb(0x5f87af));
    assert_eq!(f.rects, [rect(0, 0, 1, rgb(0xfa8007)), rect(1, 0, 1, theme.palette[244])]);
}

#[test]
fn background_opacity_sets_clear_alpha() {
    let theme = Theme {
        background_opacity: 128,
        ..Theme::default()
    };
    let mut t = term(4, 1, "");
    let f = build_with(&mut t.state, &theme, Opts::default());
    assert_eq!(f.clear, (theme.background & 0x00FF_FFFF) | 128 << 24);
}

/// Search hits added to the snapshot are drawn under the selection, in
/// the theme's search colours.
#[test]
fn search_hits_are_drawn_under_the_selection() {
    use term_core::{Highlight, HighlightKind};
    let mut t = term(8, 2, "abcdef");
    t.state
        .set_selection(Some(Selection::new(Point { line: 0, col: 2 }, Point { line: 0, col: 2 })));
    let theme = Theme::default();
    let o = Opts::default();
    let input = input(&theme, &o);
    let mut f = Frame::default();
    f.snapshot(&mut t.state, &input, 0);
    let snap = f.snapshot_mut();
    // Row 0 already holds the selection's highlight; put the hits around it.
    let sel = snap.highlights.clone();
    snap.highlights = vec![
        Highlight {
            cols: 0..2,
            kind: HighlightKind::CurrentSearchHit,
        },
        Highlight {
            cols: 2..5,
            kind: HighlightKind::SearchHit,
        },
    ];
    snap.highlights.extend(sel);
    let n = snap.highlights.len() as u32;
    snap.rows[0].highlights = 0..n;
    snap.rows[1].highlights = n..n;
    assert!(f.build_snapshot(&input, &mut Fake));
    // Then the block cursor, after the text.
    let rects: Vec<RectInstance> = f.rects.iter().copied().filter(|r| r.color != theme.cursor).collect();
    assert_eq!(
        rects,
        [
            rect(0, 0, 2, theme.search_current),
            rect(2, 0, 3, theme.search_hit),
            rect(2, 0, 1, theme.selection)
        ]
    );
}
