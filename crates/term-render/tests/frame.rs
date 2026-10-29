//! Frame building (`Frame::build`, no Metal): screens set up through
//! term-core's public API, turned into instances with a fake glyph source,
//! and checked for text, background runs, styles, decorations, cursor,
//! input-method text, padding, position clamping and theme colours.

use proptest::prelude::*;
use term_core::{Attrs, Point, Selection, Term, Terminal};
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
    let mut t = Term::new(cols, lines, 100);
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

fn build_with(t: &Terminal, theme: &Theme, o: Opts) -> Frame {
    let input = FrameInput {
        term: t,
        theme,
        metrics: o.metrics,
        padding: o.padding,
        cursor_on: o.cursor_on,
        focused: o.focused,
        preedit: o.preedit,
    };
    let mut f = Frame::default();
    f.build(&input, &mut Fake);
    f
}

fn build(t: &Terminal) -> Frame {
    build_with(t, &Theme::default(), Opts::default())
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
        let f = build_with(&t.state, &theme, Opts { focused, ..Opts::default() });
        // term-core may round tiny sizes up.
        let (cols, lines) = (t.state.cols(), t.state.lines());
        prop_assert_eq!(decoded(&f, cols, lines, (0, 0)), grid_text(&t.state));
    }
}

#[test]
fn glyph_text_matches_simple_screen() {
    let t = term(6, 2, "ab cd\r\n漢x·y");
    let f = build(&t.state);
    assert_eq!(decoded(&f, 6, 2, (0, 0)), ["ab cd ", "漢 x y "]);
}

// ---- background runs -----------------------------------------------------

#[test]
fn default_background_emits_nothing() {
    let t = term(10, 3, &format!("{NO_CURSOR}hello\r\n\x1b[1mworld"));
    let f = build(&t.state);
    assert!(f.rects.is_empty(), "{:?}", f.rects);
    assert_eq!(f.clear, Theme::default().background);
}

#[test]
fn same_colour_cells_merge_into_one_run() {
    let t = term(10, 2, &format!("{NO_CURSOR}\x1b[41mabc\x1b[0m  \x1b[42mde\x1b[0m\r\n\x1b[41mx"));
    let theme = Theme::default();
    let f = build(&t.state);
    assert_eq!(
        f.rects,
        [
            rect(0, 0, 3, theme.palette[1]),
            rect(5, 0, 2, theme.palette[2]),
            rect(0, 1, 1, theme.palette[1])
        ]
    );
}

#[test]
fn run_breaks_at_selection() {
    let mut t = term(10, 1, &format!("{NO_CURSOR}\x1b[41mabcdef"));
    let line = t.state.point_at(0, 0).line;
    t.state
        .set_selection(Some(Selection::new(Point { line, col: 3 }, Point { line, col: 2 })));
    let theme = Theme::default();
    let f = build(&t.state);
    assert_eq!(
        f.rects,
        [
            rect(0, 0, 2, theme.palette[1]),
            rect(2, 0, 2, theme.selection),
            rect(4, 0, 2, theme.palette[1])
        ]
    );
}

#[test]
fn run_breaks_at_inverse() {
    let t = term(10, 1, &format!("{NO_CURSOR}\x1b[41mab\x1b[7mcd\x1b[27mef"));
    let theme = Theme::default();
    let f = build(&t.state);
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
fn run_breaks_at_cursor() {
    let t = term(10, 1, "\x1b[41mabcdef\x1b[4G");
    let theme = Theme::default();
    let f = build(&t.state);
    assert_eq!(
        f.rects,
        [
            rect(0, 0, 3, theme.palette[1]),
            rect(3, 0, 1, theme.cursor),
            rect(4, 0, 2, theme.palette[1])
        ]
    );
}

// ---- styles --------------------------------------------------------------

fn glyph_at(f: &Frame, col: i16, line: i16) -> Option<&GlyphInstance> {
    f.glyphs.iter().find(|g| g.x == col * CW && g.y == line * CH)
}

#[test]
fn inverse_swaps_colours() {
    let t = term(10, 1, &format!("{NO_CURSOR}\x1b[7ma\x1b[0m\x1b[31;42;7mb"));
    let theme = Theme::default();
    let f = build(&t.state);
    assert_eq!(glyph_at(&f, 0, 0).unwrap().color, theme.background);
    assert_eq!(glyph_at(&f, 1, 0).unwrap().color, theme.palette[2]);
    assert_eq!(f.rects, [rect(0, 0, 1, theme.foreground), rect(1, 0, 1, theme.palette[1])]);
}

#[test]
fn dim_lowers_alpha() {
    let t = term(10, 1, "a\x1b[2mb");
    let theme = Theme::default();
    let f = build(&t.state);
    let (a, b) = (glyph_at(&f, 0, 0).unwrap().color, glyph_at(&f, 1, 0).unwrap().color);
    assert_eq!(a, theme.foreground);
    assert_eq!(b & 0x00FF_FFFF, theme.foreground & 0x00FF_FFFF, "same colour");
    assert!(b >> 24 < 255 && b >> 24 > 0, "lower alpha: {b:#x}");
}

#[test]
fn hidden_emits_no_glyph() {
    let t = term(10, 1, "a\x1b[8mbc\x1b[28md");
    let f = build(&t.state);
    assert_eq!(decoded(&f, 10, 1, (0, 0)), ["a  d      "]);
}

#[test]
fn wide_glyph_emitted_once() {
    let t = term(10, 1, "漢字😀");
    let f = build(&t.state);
    assert_eq!(f.glyphs.len(), 3);
    assert_eq!(decoded(&f, 10, 1, (0, 0)), ["漢 字 😀     "]);
}

// ---- decorations ---------------------------------------------------------

#[test]
fn underline_positions_from_metrics() {
    let t = term(10, 2, &format!("{NO_CURSOR}\r\n\x1b[4ma\x1b[24;21mb\x1b[24;9mc"));
    let fg = Theme::default().foreground;
    let f = build(&t.state);
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
fn decorations_follow_thicker_metrics() {
    let m = Metrics {
        underline_thickness: 3,
        underline_offset: 4,
        ..M
    };
    let t = term(4, 1, &format!("{NO_CURSOR}\x1b[21ma"));
    let theme = Theme::default();
    let f = build_with(
        &t.state,
        &theme,
        Opts {
            metrics: m,
            ..Opts::default()
        },
    );
    let ys: Vec<_> = f.rects.iter().map(|r| (r.y, r.h)).collect();
    assert_eq!(ys, [(19, 3), (25, 3)]);
}

// ---- cursor --------------------------------------------------------------

fn cursor_rects(f: &Frame, theme: &Theme) -> Vec<RectInstance> {
    f.rects.iter().copied().filter(|r| r.color == theme.cursor).collect()
}

#[test]
fn block_cursor_swaps_cell_colours() {
    let t = term(10, 2, "ab\x1b[1G");
    let theme = Theme::default();
    let f = build(&t.state);
    assert_eq!(f.rects, [rect(0, 0, 1, theme.cursor)]);
    assert_eq!(glyph_at(&f, 0, 0).unwrap().color, theme.cursor_text);
    assert_eq!(glyph_at(&f, 1, 0).unwrap().color, theme.foreground);
}

#[test]
fn bar_cursor() {
    let t = term(10, 2, "ab\x1b[6 q");
    let theme = Theme::default();
    let f = build(&t.state);
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
    let t = term(10, 2, "\r\nab\x1b[4 q");
    let theme = Theme::default();
    let f = build(&t.state);
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
        let t = term(10, 2, &format!("ab{shape}"));
        let f = build_with(
            &t.state,
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
    let t = term(10, 2, "ab\x1b[?25l");
    for focused in [true, false] {
        let f = build_with(
            &t.state,
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
    let blinking = term(10, 2, "ab\x1b[1 q");
    assert!(blinking.state.cursor_blink());
    let off = |t: &Term, focused| {
        build_with(
            &t.state,
            &theme,
            Opts {
                cursor_on: false,
                focused,
                ..Opts::default()
            },
        )
    };
    assert!(off(&blinking, true).rects.is_empty(), "blink phase off hides it");
    assert_eq!(cursor_rects(&off(&blinking, false), &theme).len(), 4, "unfocused doesn't blink");
    let steady = term(10, 2, "ab\x1b[2 q");
    assert_eq!(
        off(&steady, true).rects,
        [rect(2, 0, 1, theme.cursor)],
        "steady cursor ignores the phase"
    );
}

#[test]
fn cursor_hidden_when_scrolled_out_of_view() {
    let theme = Theme::default();
    let mut t = term(10, 3, "1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7");
    let f = build(&t.state);
    assert_eq!(cursor_rects(&f, &theme), [rect(1, 2, 1, theme.cursor)]);
    t.state.scroll_view(1);
    let f = build(&t.state);
    assert!(cursor_rects(&f, &theme).is_empty(), "cursor line pushed below the view");
    t.state.scroll_view(-1);
    assert_eq!(cursor_rects(&build(&t.state), &theme).len(), 1);
}

#[test]
fn cursor_on_wide_char_is_two_cells() {
    let theme = Theme::default();
    let t = term(10, 2, "a漢b\x1b[2G");
    assert_eq!(t.state.cursor().col(), 1);
    let f = build(&t.state);
    assert_eq!(f.rects, [rect(1, 0, 2, theme.cursor)], "block covers both halves");
    assert_eq!(glyph_at(&f, 1, 0).unwrap().color, theme.cursor_text);

    let f = build_with(
        &t.state,
        &theme,
        Opts {
            focused: false,
            ..Opts::default()
        },
    );
    assert_eq!(cursor_rects(&f, &theme), hollow_box(1, 0, 2, theme.cursor));

    let t = term(10, 2, "a漢b\x1b[2G\x1b[4 q");
    let f = build(&t.state);
    assert_eq!(cursor_rects(&f, &theme)[0].w, 2 * CW as u16);
}

// ---- input-method text ---------------------------------------------------

#[test]
fn preedit_suppresses_glyphs_under_it() {
    let theme = Theme::default();
    let t = term(10, 2, "\r\nabcdef\x1b[2G");
    let f = build_with(
        &t.state,
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
        decoded(&build_with(&t.state, &theme, opts()), 10, 2, (0, 0)),
        ["3         ", "abxy      "]
    );
    // Scrolled back by one row, the cursor's row is out of view: no preedit.
    t.state.scroll_view(1);
    let f = build_with(&t.state, &theme, opts());
    assert_eq!(decoded(&f, 10, 2, (0, 0)), ["2         ", "3         "]);
    assert!(f.rects.iter().all(|r| r.color != theme.background), "no preedit background");
}

#[test]
fn preedit_clipped_at_right_edge() {
    let t = term(5, 1, "abc");
    let theme = Theme::default();
    let f = build_with(
        &t.state,
        &theme,
        Opts {
            preedit: Some("xyz"),
            ..Opts::default()
        },
    );
    assert_eq!(decoded(&f, 5, 1, (0, 0)), ["abcxy"]);
    // A wide character that doesn't fit is dropped whole.
    let t = term(5, 1, "abcd");
    let f = build_with(
        &t.state,
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
    let t = term(8, 1, "abcdefg\x1b[1G");
    let f = build_with(
        &t.state,
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
    let t = term(8, 3, screen);
    let pad = (7u32, 9u32);
    let plain = build_with(
        &t.state,
        &theme,
        Opts {
            preedit: Some("q"),
            ..Opts::default()
        },
    );
    let padded = build_with(
        &t.state,
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
    let t = term(cols, lines, &text);
    let theme = Theme::default();
    let f = build_with(
        &t.state,
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
    let t = term(
        10,
        1,
        &format!("{NO_CURSOR}\x1b[38;2;1;2;3;48;2;250;128;7ma\x1b[38;5;67;48;5;244mb"),
    );
    let theme = Theme::default();
    let f = build(&t.state);
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
    let t = term(4, 1, "");
    let f = build_with(&t.state, &theme, Opts::default());
    assert_eq!(f.clear, (theme.background & 0x00FF_FFFF) | 128 << 24);
}
