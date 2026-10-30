//! Conformance tests comparing term-core's behaviour against xterm for the
//! escape sequences real programs use. Expected values were checked against
//! the xterm sources (cursor.c, charproc.c, screen.c, util.c, ctlseqs).

use term_core::{Attrs, Color, CursorShape, Selection, Style, Term, Underline, Width};

fn new(cols: usize, lines: usize) -> Term {
    Term::new(cols, lines, 1000, term_core::ImagePool::process())
}

/// Visible row text with trailing blanks trimmed (wide spacers dropped).
fn text(t: &Term, line: usize) -> String {
    let row = t.state.visible_row(line);
    let s: String = row.cells().filter(|c| !c.is_spacer()).map(|c| c.ch()).collect();
    s.trim_end().to_string()
}

fn screen(t: &Term) -> Vec<String> {
    (0..t.state.lines()).map(|l| text(t, l)).collect()
}

fn pos(t: &Term) -> (usize, usize) {
    (t.state.cursor().row(), t.state.cursor().col())
}

/// Text of scrollback row `i` (0 = oldest), via the scroll view.
fn history(t: &mut Term, i: usize) -> String {
    let sb = t.state.scrollback_len();
    assert!(i < sb, "scrollback has {sb} rows, wanted {i}");
    t.state.scroll_view(sb as isize);
    let s = text(t, i);
    t.state.scroll_view_to_bottom();
    s
}

fn history_wrapped(t: &mut Term, i: usize) -> bool {
    let sb = t.state.scrollback_len();
    assert!(i < sb);
    t.state.scroll_view(sb as isize);
    let w = t.state.visible_row(i).wrapped();
    t.state.scroll_view_to_bottom();
    w
}

// ---------------------------------------------------------------- cursor

#[test]
fn cup_positions_and_homes() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[2;3H");
    assert_eq!(pos(&t), (1, 2));
    t.advance(b"\x1b[H");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn cup_zero_defaults_to_one() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[3;3H\x1b[0;0H");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn cup_clamps_to_screen() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[99;99H");
    assert_eq!(pos(&t), (3, 9));
}

#[test]
fn hvp_positions_like_cup() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[3;2f");
    assert_eq!(pos(&t), (2, 1));
}

#[test]
fn cuu_cud_clamp_at_screen_edges() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[2;2H\x1b[5A");
    assert_eq!(pos(&t), (0, 1));
    t.advance(b"\x1b[9B");
    assert_eq!(pos(&t), (3, 1));
    t.advance(b"\x1b[B");
    assert_eq!(pos(&t), (3, 1), "CUD at last row stays");
}

#[test]
fn cuf_cub_clamp_at_screen_edges() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[2;2H\x1b[50C");
    assert_eq!(pos(&t), (1, 9));
    t.advance(b"\x1b[99D");
    assert_eq!(pos(&t), (1, 0));
}

#[test]
fn cha_and_hpa_set_column() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[2;5H\x1b[3G");
    assert_eq!(pos(&t), (1, 2));
    t.advance(b"\x1b[7`");
    assert_eq!(pos(&t), (1, 6));
    t.advance(b"\x1b[99G");
    assert_eq!(pos(&t), (1, 9));
}

#[test]
fn vpa_sets_row_keeps_column() {
    let mut t = new(10, 5);
    t.advance(b"\x1b[2;5H\x1b[4d");
    assert_eq!(pos(&t), (3, 4));
    t.advance(b"\x1b[99d");
    assert_eq!(pos(&t), (4, 4));
}

#[test]
fn cnl_cpl_move_rows_and_carriage_return() {
    let mut t = new(10, 5);
    t.advance(b"\x1b[1;4H\x1b[E");
    assert_eq!(pos(&t), (1, 0));
    t.advance(b"\x1b[2E");
    assert_eq!(pos(&t), (3, 0));
    t.advance(b"\x1b[F");
    assert_eq!(pos(&t), (2, 0));
    t.advance(b"\x1b[9F");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn cuu_cud_clamp_inside_scroll_region() {
    let mut t = new(10, 5);
    t.advance(b"\x1b[2;4r\x1b[3;1H");
    assert_eq!(pos(&t), (2, 0));
    t.advance(b"\x1b[9A");
    assert_eq!(pos(&t), (1, 0), "CUU stops at top margin");
    t.advance(b"\x1b[9B");
    assert_eq!(pos(&t), (3, 0), "CUD stops at bottom margin");
}

#[test]
fn cuu_below_region_stops_at_top_margin() {
    let mut t = new(10, 5);
    t.advance(b"\x1b[2;4r\x1b[5;1H");
    assert_eq!(pos(&t), (4, 0));
    t.advance(b"\x1b[9A");
    // xterm CursorUp: min = top margin when the cursor is below it.
    assert_eq!(pos(&t), (1, 0));
}

#[test]
fn cud_above_region_stops_at_bottom_margin() {
    let mut t = new(10, 5);
    t.advance(b"\x1b[2;4r"); // homes the cursor to (0,0), above the region
    assert_eq!(pos(&t), (0, 0));
    t.advance(b"\x1b[9B");
    // xterm CursorDown: max = bottom margin when the cursor is above it.
    assert_eq!(pos(&t), (3, 0));
}

#[test]
fn cuf_cub_ignore_scroll_region() {
    let mut t = new(10, 5);
    t.advance(b"\x1b[2;4r\x1b[3;1H\x1b[99C");
    assert_eq!(pos(&t), (2, 9), "vertical region does not limit columns");
}

// ---------------------------------------------------------------- origin

#[test]
fn origin_mode_makes_cup_region_relative() {
    let mut t = new(10, 6);
    t.advance(b"\x1b[2;5r\x1b[?6h");
    assert_eq!(pos(&t), (1, 0), "setting origin homes into the region");
    t.advance(b"\x1b[2;2H");
    assert_eq!(pos(&t), (2, 1));
    t.advance(b"\x1b[99;99H");
    assert_eq!(pos(&t), (4, 9), "CUP clamps to bottom margin in origin mode");
}

#[test]
fn origin_mode_off_makes_cup_absolute() {
    let mut t = new(10, 6);
    t.advance(b"\x1b[2;5r\x1b[?6h\x1b[?6l");
    assert_eq!(pos(&t), (0, 0), "clearing origin homes the cursor");
    t.advance(b"\x1b[5;1H");
    assert_eq!(pos(&t), (4, 0), "absolute position below the region is fine");
}

// ---------------------------------------------------------------- erase

#[test]
fn ed0_erases_from_cursor_down() {
    let mut t = new(6, 3);
    t.advance(b"aaaaaa\r\nbbbbbb\r\ncccccc");
    t.advance(b"\x1b[2;3H\x1b[J");
    assert_eq!(screen(&t), ["aaaaaa", "bb", ""]);
}

#[test]
fn ed1_erases_up_to_cursor() {
    let mut t = new(6, 3);
    t.advance(b"aaaaaa\r\nbbbbbb\r\ncccccc");
    t.advance(b"\x1b[2;3H\x1b[1J");
    assert_eq!(screen(&t), ["", "   bbb", "cccccc"]);
}

#[test]
fn ed2_erases_screen_but_keeps_scrollback() {
    let mut t = new(5, 2);
    t.advance(b"aa\r\nbb\r\ncc\r\ndd");
    assert_eq!(t.state.scrollback_len(), 2);
    t.advance(b"\x1b[2J");
    assert_eq!(screen(&t), ["", ""]);
    assert_eq!(t.state.scrollback_len(), 2, "ED 2 must not touch scrollback");
}

#[test]
fn ed3_clears_scrollback_but_keeps_screen() {
    let mut t = new(5, 2);
    t.advance(b"aa\r\nbb\r\ncc\r\ndd");
    assert_eq!(t.state.scrollback_len(), 2);
    t.advance(b"\x1b[3J");
    assert_eq!(t.state.scrollback_len(), 0);
    assert_eq!(screen(&t), ["cc", "dd"], "ED 3 must not touch the screen");
}

#[test]
fn el_modes() {
    let mut t = new(6, 2);
    t.advance(b"abcdef\x1b[3G\x1b[K");
    assert_eq!(text(&t, 0), "ab");
    t.advance(b"\x1b[2;1Hfedcba\x1b[3G\x1b[1K");
    assert_eq!(text(&t, 1), "   cba");
    t.advance(b"\x1b[2K");
    assert_eq!(text(&t, 1), "");
}

#[test]
fn ech_erases_n_chars_and_clamps() {
    let mut t = new(6, 1);
    t.advance(b"abcdef\x1b[2G\x1b[2X");
    assert_eq!(text(&t, 0), "a  def");
    t.advance(b"\x1b[5G\x1b[99X");
    assert_eq!(text(&t, 0), "a  d");
}

#[test]
fn erase_uses_current_background() {
    let mut t = new(6, 2);
    t.advance(b"\x1b[44m\x1b[2K");
    let row = t.state.visible_row(0);
    let c = row.cell(0);
    assert_eq!(c.style().bg, Color::indexed(4));
    assert_eq!(c.ch(), ' ');
    t.advance(b"\x1b[42m\x1b[J");
    assert_eq!(t.state.visible_row(1).cell(5).style().bg, Color::indexed(2));
    t.advance(b"\x1b[49m\x1b[2K");
    assert_eq!(t.state.visible_row(0).cell(0).style().bg, Color::DEFAULT);
}

#[test]
fn erase_of_wide_left_half_blanks_both() {
    let mut t = new(5, 1);
    t.advance("漢x".as_bytes());
    t.advance(b"\x1b[1G\x1b[1X"); // erase the WIDE half
    assert_eq!(text(&t, 0), "  x");
    assert!(!t.state.visible_row(0).cell(1).is_spacer());
}

#[test]
fn erase_of_wide_right_half_blanks_both() {
    let mut t = new(5, 1);
    t.advance("x漢".as_bytes());
    t.advance(b"\x1b[3G\x1b[0K"); // erase from the spacer half rightwards
    assert_eq!(text(&t, 0), "x");
    assert!(t.state.visible_row(0).cell(1).width() != Width::Wide);
}

// ---------------------------------------------------------------- ich/dch

#[test]
fn ich_shifts_right_dropping_at_edge() {
    let mut t = new(8, 1);
    t.advance(b"abcdef\x1b[3G\x1b[2@");
    assert_eq!(text(&t, 0), "ab  cdef");
}

#[test]
fn ich_clamps_at_line_end() {
    let mut t = new(5, 1);
    t.advance(b"abcde\x1b[4G\x1b[99@");
    assert_eq!(text(&t, 0), "abc");
}

#[test]
fn dch_pulls_left() {
    let mut t = new(8, 1);
    t.advance(b"abcdef\x1b[3G\x1b[2P");
    assert_eq!(text(&t, 0), "abef");
}

#[test]
fn dch_clamps_at_line_end() {
    let mut t = new(5, 1);
    t.advance(b"abcde\x1b[4G\x1b[99P");
    assert_eq!(text(&t, 0), "abc");
}

#[test]
fn ich_splitting_wide_char_blanks_it() {
    let mut t = new(4, 1);
    t.advance("漢b".as_bytes());
    t.advance(b"\x1b[2G\x1b[@");
    // xterm ScrnInsertChar clears the damaged wide char before shifting.
    assert_eq!(text(&t, 0), "   b");
    let row = t.state.visible_row(0);
    assert!(row.cell(0).width() != Width::Wide, "no half of a wide char survives");
}

#[test]
fn dch_splitting_wide_char_blanks_it() {
    let mut t = new(4, 1);
    t.advance("漢a".as_bytes());
    t.advance(b"\x1b[2G\x1b[P"); // delete the spacer half
    // xterm ScrnDeleteChar clears the damaged wide char before shifting.
    assert_eq!(text(&t, 0), " a");
    let row = t.state.visible_row(0);
    for c in 0..row.len() {
        if row.cell(c).is_spacer() {
            assert!(c > 0 && row.cell(c - 1).width() == Width::Wide, "dangling spacer at {c}");
        }
    }
}

// ---------------------------------------------------------------- il/dl

#[test]
fn il_inserts_blank_lines_and_resets_column() {
    let mut t = new(5, 5);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne");
    t.advance(b"\x1b[2;4r\x1b[3;4H\x1b[L");
    assert_eq!(screen(&t), ["a", "b", "", "c", "e"], "d falls off the region bottom");
    assert_eq!(pos(&t), (2, 0));
}

#[test]
fn dl_deletes_lines_and_resets_column() {
    let mut t = new(5, 5);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne");
    t.advance(b"\x1b[2;4r\x1b[3;4H\x1b[M");
    assert_eq!(screen(&t), ["a", "b", "d", "", "e"]);
    assert_eq!(pos(&t), (2, 0));
}

#[test]
fn il_dl_ignored_outside_region() {
    let mut t = new(5, 5);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne");
    t.advance(b"\x1b[2;4r\x1b[1;1H\x1b[L\x1b[M"); // cursor above the region
    t.advance(b"\x1b[5;1H\x1b[L\x1b[M"); // and below it
    assert_eq!(screen(&t), ["a", "b", "c", "d", "e"]);
}

#[test]
fn dl_never_goes_to_scrollback() {
    let mut t = new(5, 3);
    t.advance(b"a\r\nb\r\nc");
    t.advance(b"\x1b[1;1H\x1b[M");
    assert_eq!(screen(&t), ["b", "c", ""]);
    assert_eq!(t.state.scrollback_len(), 0);
}

#[test]
fn dl_on_the_top_row_keeps_history_and_line_numbers() {
    // DL on row 0, of the whole screen or of a region starting there,
    // pulls the rows up without pushing the deleted one into scrollback;
    // line numbers stay, the deleted row's prompt mark goes with it and
    // the blank row that comes in has the pen's background.
    for region in ["", "\x1b[1;3r"] {
        let mut t = new(5, 4);
        t.advance(b"\x1b]133;A\x07$ x\r\n\x1b]133;A\x07$ y\r\nc\r\nd");
        let first = t.state.point_at(0, 0);
        t.advance(format!("{region}\x1b[44m\x1b[H\x1b[M").as_bytes());
        assert_eq!(pos(&t), (0, 0));
        assert_eq!(t.state.scrollback_len(), 0);
        assert_eq!(t.state.point_at(0, 0), first, "line numbers don't move");
        let (want, blank) = if region.is_empty() {
            (["$ y", "c", "d", ""], 3)
        } else {
            (["$ y", "c", "", "d"], 2)
        };
        assert_eq!(screen(&t), want);
        let below = t.state.point_at(3, 0);
        assert_eq!(t.state.previous_prompt(below.line), Some(first), "y's mark moved up to row 0");
        assert!(t.state.visible_row(blank).cells().all(|c| c.style().bg != Color::default()));
        t.state.check_invariants().unwrap();
    }
}

// ---------------------------------------------------------------- insert mode

#[test]
fn insert_mode_shifts_existing_text() {
    let mut t = new(6, 1);
    t.advance(b"abcdef\x1b[H\x1b[4hXY");
    assert_eq!(text(&t, 0), "XYabcd");
}

#[test]
fn replace_mode_overwrites() {
    let mut t = new(8, 1);
    t.advance(b"abcdef\x1b[HXY");
    assert_eq!(text(&t, 0), "XYcdef");
}

// ---------------------------------------------------------------- scroll

#[test]
fn ind_moves_down_and_scrolls_only_at_bottom_margin() {
    let mut t = new(5, 4);
    t.advance(b"a\r\nb\r\nc\r\nd");
    t.advance(b"\x1b[2;3r"); // region rows 2-3, cursor homed
    t.advance(b"\x1b[1;1H\x1bD"); // above the region: plain move down
    assert_eq!(screen(&t), ["a", "b", "c", "d"]);
    assert_eq!(pos(&t), (1, 0));
    t.advance(b"\x1bD"); // inside the region, not yet at the margin
    assert_eq!(screen(&t), ["a", "b", "c", "d"]);
    assert_eq!(pos(&t), (2, 0));
    t.advance(b"\x1bD"); // at bottom margin: region scrolls
    assert_eq!(screen(&t), ["a", "c", "", "d"]);
    assert_eq!(pos(&t), (2, 0));
    t.advance(b"\x1b[4;1H\x1bD"); // below the region, last row: stays put
    assert_eq!(pos(&t), (3, 0));
    assert_eq!(screen(&t), ["a", "c", "", "d"]);
}

#[test]
fn ri_moves_up_and_scrolls_only_at_top_margin() {
    let mut t = new(5, 4);
    t.advance(b"a\r\nb\r\nc\r\nd");
    t.advance(b"\x1b[2;3r\x1b[4;1H\x1bM"); // below the region: plain move up
    assert_eq!(pos(&t), (2, 0));
    assert_eq!(screen(&t), ["a", "b", "c", "d"]);
    t.advance(b"\x1b[2;1H\x1bM"); // at top margin: region scrolls down
    assert_eq!(screen(&t), ["a", "", "b", "d"]);
    assert_eq!(pos(&t), (1, 0));
    t.advance(b"\x1bM");
    assert_eq!(screen(&t), ["a", "", "", "d"]);
}

#[test]
fn nel_is_cr_then_lf() {
    let mut t = new(5, 3);
    t.advance(b"abc\x1bEde");
    assert_eq!(text(&t, 0), "abc");
    assert_eq!(text(&t, 1), "de");
    assert_eq!(pos(&t), (1, 2));
}

#[test]
fn su_scrolls_full_region_into_scrollback() {
    let mut t = new(5, 3);
    t.advance(b"one\r\ntwo\r\nsix");
    t.advance(b"\x1b[2S");
    assert_eq!(screen(&t), ["six", "", ""]);
    assert_eq!(t.state.scrollback_len(), 2);
    assert_eq!(history(&mut t, 0), "one");
    assert_eq!(history(&mut t, 1), "two");
}

#[test]
fn su_inside_region_drops_lines_without_scrollback() {
    let mut t = new(5, 4);
    t.advance(b"a\r\nb\r\nc\r\nd");
    t.advance(b"\x1b[2;3r\x1b[S");
    assert_eq!(screen(&t), ["a", "c", "", "d"]);
    assert_eq!(t.state.scrollback_len(), 0);
}

#[test]
fn sd_inserts_blanks_without_pulling_scrollback() {
    let mut t = new(5, 3);
    t.advance(b"aa\r\nbb\r\ncc\r\ndd\r\nee");
    assert_eq!(t.state.scrollback_len(), 2);
    t.advance(b"\x1b[T");
    assert_eq!(screen(&t), ["", "cc", "dd"]);
    assert_eq!(t.state.scrollback_len(), 2, "SD must not pull from scrollback");
}

#[test]
fn scrolled_rows_keep_wrapped_flag() {
    let mut t = new(5, 2);
    t.advance(b"abcdefgh\r\n");
    assert_eq!(history(&mut t, 0), "abcde");
    assert!(history_wrapped(&mut t, 0));
}

#[test]
fn scrollback_limit_evicts_oldest() {
    let mut t = Term::new(4, 1, 3, term_core::ImagePool::process());
    for i in 0..10 {
        t.advance(format!("{i}\r\n").as_bytes());
    }
    assert_eq!(t.state.scrollback_len(), 3);
    assert_eq!(history(&mut t, 0), "7");
    assert_eq!(history(&mut t, 2), "9");
}

#[test]
fn scroll_uses_current_background() {
    let mut t = new(4, 2);
    t.advance(b"\x1b[41maa\r\nbb\x1b[S");
    assert_eq!(t.state.visible_row(1).cell(0).style().bg, Color::indexed(1));
}

#[test]
fn alt_screen_scroll_never_reaches_scrollback() {
    let mut t = new(5, 2);
    t.advance(b"p1\r\np2");
    t.advance(b"\x1b[?1049ha\r\nb\r\nc\r\nd");
    assert_eq!(t.state.scrollback_len(), 0);
    t.advance(b"\x1b[?1049l");
    assert_eq!(screen(&t), ["p1", "p2"]);
    assert_eq!(t.state.scrollback_len(), 0);
}

// ---------------------------------------------------------------- autowrap

#[test]
fn last_column_sets_pending_wrap() {
    let mut t = new(5, 3);
    t.advance(b"abcde");
    assert_eq!(pos(&t), (0, 4));
    assert!(t.state.cursor().pending_wrap());
    assert!(!t.state.visible_row(0).wrapped(), "wrap is deferred");
}

#[test]
fn pending_wrap_wraps_the_next_char() {
    let mut t = new(5, 3);
    t.advance(b"abcdef");
    assert!(t.state.visible_row(0).wrapped());
    assert_eq!(text(&t, 1), "f");
    assert_eq!(pos(&t), (1, 1));
}

#[test]
fn pending_wrap_cleared_by_cursor_motion() {
    let mut t = new(5, 3);
    t.advance(b"abcde\x1b[H");
    assert!(!t.state.cursor().pending_wrap());
    t.advance(b"f");
    assert_eq!(text(&t, 0), "fbcde");
    assert!(!t.state.visible_row(0).wrapped(), "no spurious wrap flag");
}

#[test]
fn decawm_off_overwrites_last_column() {
    let mut t = new(5, 2);
    t.advance(b"\x1b[?7labcdefg");
    assert_eq!(text(&t, 0), "abcdg");
    assert_eq!(text(&t, 1), "");
    assert!(!t.state.cursor().pending_wrap());
    assert!(!t.state.visible_row(0).wrapped());
}

#[test]
fn decawm_off_drops_wide_char_at_last_column() {
    let mut t = new(5, 2);
    t.advance(b"\x1b[?7l");
    t.advance("abcd漢".as_bytes());
    assert_eq!(text(&t, 0), "abcd");
    assert_eq!(pos(&t), (0, 4));
    t.advance(b"e");
    assert_eq!(text(&t, 0), "abcde");
}

#[test]
fn wide_char_wraps_when_no_room() {
    let mut t = new(5, 2);
    t.advance("abcd漢".as_bytes());
    assert!(t.state.visible_row(0).wrapped());
    assert_eq!(text(&t, 0), "abcd");
    assert_eq!(text(&t, 1), "漢");
}

#[test]
fn wide_char_exact_fit_then_wrap() {
    let mut t = new(4, 2);
    t.advance("ab漢".as_bytes());
    assert_eq!(pos(&t), (0, 3));
    assert!(t.state.cursor().pending_wrap());
    assert!(!t.state.visible_row(0).wrapped());
    t.advance(b"x");
    assert!(t.state.visible_row(0).wrapped());
    assert_eq!(text(&t, 1), "x");
}

#[test]
fn overwrite_wide_first_half_blanks_second() {
    let mut t = new(5, 1);
    t.advance("漢".as_bytes());
    t.advance(b"\x1b[1Gx");
    assert_eq!(text(&t, 0), "x");
    assert!(!t.state.visible_row(0).cell(1).is_spacer());
}

#[test]
fn overwrite_wide_second_half_blanks_first() {
    let mut t = new(5, 1);
    t.advance("漢".as_bytes());
    t.advance(b"\x1b[2Gx");
    assert_eq!(text(&t, 0), " x");
    assert!(t.state.visible_row(0).cell(0).width() != Width::Wide);
}

// ---------------------------------------------------------------- tabs

#[test]
fn ht_uses_default_stops_every_8() {
    let mut t = new(24, 1);
    t.advance(b"\t");
    assert_eq!(pos(&t), (0, 8));
    t.advance(b"\t");
    assert_eq!(pos(&t), (0, 16));
}

#[test]
fn ht_past_last_stop_goes_to_last_column() {
    let mut t = new(10, 1);
    t.advance(b"\t");
    assert_eq!(pos(&t), (0, 8));
    t.advance(b"\t");
    assert_eq!(pos(&t), (0, 9));
}

#[test]
fn hts_sets_a_stop() {
    let mut t = new(24, 1);
    t.advance(b"\x1b[4G\x1bH\r\t");
    assert_eq!(pos(&t), (0, 3));
}

#[test]
fn tbc_clears_stop_at_cursor() {
    let mut t = new(24, 1);
    t.advance(b"\x1b[9G\x1b[g\r\t");
    assert_eq!(pos(&t), (0, 16), "stop at 8 cleared, next is 16");
}

#[test]
fn tbc_3_clears_all_stops() {
    let mut t = new(24, 1);
    t.advance(b"\x1b[3g\t");
    assert_eq!(pos(&t), (0, 23));
}

#[test]
fn cht_moves_forward_n_stops() {
    let mut t = new(30, 1);
    t.advance(b"\x1b[2I");
    assert_eq!(pos(&t), (0, 16));
}

#[test]
fn cbt_moves_backward_n_stops() {
    let mut t = new(30, 1);
    t.advance(b"\x1b[21G\x1b[Z");
    assert_eq!(pos(&t), (0, 16));
    t.advance(b"\x1b[2Z");
    assert_eq!(pos(&t), (0, 0));
}

// ---------------------------------------------------------------- save/restore

#[test]
fn decsc_decrc_restore_position() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[2;4H\x1b7\x1b[H\x1b8");
    assert_eq!(pos(&t), (1, 3));
}

#[test]
fn decsc_saves_attributes() {
    let mut t = new(10, 1);
    t.advance(b"\x1b[1;31m\x1b7\x1b[0m\x1b8x");
    let row = t.state.visible_row(0);
    let c = row.cell(0);
    assert_eq!(c.ch(), 'x');
    assert!(c.style().attrs.contains(Attrs::BOLD));
    assert_eq!(c.style().fg, Color::indexed(1));
}

#[test]
fn decsc_saves_origin_mode_and_charset() {
    let mut t = new(10, 6);
    t.advance(b"\x1b[2;5r\x1b[?6h\x1b(0\x1b7\x1b[?6l\x1b(B\x1b8");
    assert!(t.state.modes().origin, "origin mode restored");
    assert_eq!(pos(&t), (1, 0), "cursor restored to region home");
    t.advance(b"q");
    assert_eq!(text(&t, 1), "─", "line-drawing charset restored");
}

#[test]
fn csi_s_u_save_and_restore() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[3;3H\x1b[s\x1b[H\x1b[u");
    assert_eq!(pos(&t), (2, 2));
}

#[test]
fn decrc_without_save_goes_home() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[?6h\x1b[2;2H\x1b8");
    assert_eq!(pos(&t), (0, 0));
    assert!(!t.state.modes().origin);
}

#[test]
fn mode_1048_saves_and_restores_cursor() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[2;6H\x1b[?1048h\x1b[H\x1b[?1048l");
    assert_eq!(pos(&t), (1, 5));
}

#[test]
fn saved_cursors_are_per_screen() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[1;5H\x1b7"); // save (0,4) on primary
    t.advance(b"\x1b[?1049h"); // saves primary again, enters alt
    t.advance(b"\x1b[3;3H\x1b7"); // save (2,2) on alt
    t.advance(b"\x1b[?1049l");
    assert_eq!(pos(&t), (0, 4), "1049l restores the primary cursor");
    t.advance(b"\x1b[8");
    assert_eq!(pos(&t), (0, 4), "primary DECRC slot intact");
    t.advance(b"\x1b[?1049h\x1b8");
    assert_eq!(pos(&t), (2, 2), "alt DECRC slot intact");
}

// ---------------------------------------------------------------- sgr

#[test]
fn sgr_empty_and_zero_reset() {
    let mut t = new(4, 1);
    t.advance(b"\x1b[1;31mA\x1b[mB\x1b[1mC\x1b[0mD");
    let r = t.state.visible_row(0);
    assert!(r.cell(0).style().attrs.contains(Attrs::BOLD));
    assert!(!r.cell(1).style().attrs.contains(Attrs::BOLD));
    assert_eq!(r.cell(1).style().fg, Color::DEFAULT);
    assert!(r.cell(2).style().attrs.contains(Attrs::BOLD));
    assert!(!r.cell(3).style().attrs.contains(Attrs::BOLD));
}

#[test]
fn sgr_attributes_and_individual_resets() {
    let mut t = new(20, 1);
    t.advance(b"\x1b[1;2;3;4;5;7;8;9mA");
    let all = t.state.visible_row(0).cell(0).style();
    assert_eq!(all.underline, Underline::Single);
    for a in [
        Attrs::BOLD,
        Attrs::DIM,
        Attrs::ITALIC,
        Attrs::BLINK,
        Attrs::INVERSE,
        Attrs::HIDDEN,
        Attrs::STRIKE,
    ] {
        assert!(all.attrs.contains(a), "missing {a:?}");
    }
    // Each reset code clears only its own attribute.
    t.advance(b"\x1b[22mB");
    let row = t.state.visible_row(0);
    let c = row.cell(1);
    assert!(!c.style().attrs.contains(Attrs::BOLD) && !c.style().attrs.contains(Attrs::DIM));
    assert!(c.style().attrs.contains(Attrs::ITALIC));
    t.advance(b"\x1b[23;24;25;27;28;29mC");
    assert_eq!(t.state.visible_row(0).cell(2).style(), Style::DEFAULT, "all attributes reset");
}

#[test]
fn sgr_underline_styles() {
    let mut t = new(4, 1);
    t.advance(b"\x1b[4:1mA\x1b[4:2mB\x1b[21mC\x1b[4:0mD");
    let r = t.state.visible_row(0);
    assert!(r.cell(0).style().underline == Underline::Single);
    assert!(r.cell(1).style().underline == Underline::Double);
    assert!(r.cell(2).style().underline == Underline::Double, "SGR 21 is double underline");
    assert!(r.cell(3).style().underline == Underline::None);
}

#[test]
fn sgr_standard_and_bright_colors() {
    let mut t = new(6, 1);
    t.advance(b"\x1b[31mA\x1b[47mB\x1b[91mC\x1b[102mD\x1b[30mE\x1b[40mF");
    let r = t.state.visible_row(0);
    assert_eq!(r.cell(0).style().fg, Color::indexed(1));
    assert_eq!(r.cell(1).style().bg, Color::indexed(7));
    assert_eq!(r.cell(2).style().fg, Color::indexed(9));
    assert_eq!(r.cell(3).style().bg, Color::indexed(10));
    assert_eq!(r.cell(4).style().fg, Color::indexed(0));
    assert_eq!(r.cell(5).style().bg, Color::indexed(0));
}

#[test]
fn sgr_256_color_both_forms() {
    let mut t = new(4, 1);
    t.advance(b"\x1b[38;5;123mA\x1b[38:5:45mB\x1b[48;5;200mC\x1b[48:5:67mD");
    let r = t.state.visible_row(0);
    assert_eq!(r.cell(0).style().fg, Color::indexed(123));
    assert_eq!(r.cell(1).style().fg, Color::indexed(45));
    assert_eq!(r.cell(2).style().bg, Color::indexed(200));
    assert_eq!(r.cell(3).style().bg, Color::indexed(67));
}

#[test]
fn sgr_truecolor_both_forms() {
    let mut t = new(4, 1);
    t.advance(b"\x1b[38;2;10;20;30mA\x1b[38:2:40:50:60mB\x1b[38:2:0:70:80:90mC\x1b[48:2:1:2:3mD");
    let r = t.state.visible_row(0);
    assert_eq!(r.cell(0).style().fg, Color::rgb(10, 20, 30));
    assert_eq!(r.cell(1).style().fg, Color::rgb(40, 50, 60));
    assert_eq!(r.cell(2).style().fg, Color::rgb(70, 80, 90), "colour-space id skipped");
    assert_eq!(r.cell(3).style().bg, Color::rgb(1, 2, 3));
}

#[test]
fn sgr_39_49_restore_defaults() {
    let mut t = new(2, 1);
    t.advance(b"\x1b[31;42mA\x1b[39;49mB");
    let r = t.state.visible_row(0);
    assert_eq!(r.cell(1).style().fg, Color::DEFAULT);
    assert_eq!(r.cell(1).style().bg, Color::DEFAULT);
}

#[test]
fn sgr_combined_sequence() {
    let mut t = new(1, 1);
    t.advance(b"\x1b[1;4;31;42mA");
    let row = t.state.visible_row(0);
    let c = row.cell(0);
    assert!(c.style().attrs.contains(Attrs::BOLD) && c.style().underline == Underline::Single);
    assert_eq!(c.style().fg, Color::indexed(1));
    assert_eq!(c.style().bg, Color::indexed(2));
}

#[test]
fn sgr_unknown_codes_ignored_and_58_skipped() {
    let mut t = new(2, 1);
    t.advance(b"\x1b[999;1mA\x1b[58;2;1;2;3;27mB");
    let r = t.state.visible_row(0);
    assert!(r.cell(0).style().attrs.contains(Attrs::BOLD), "unknown SGR must not reset");
    assert!(r.cell(1).style().attrs.contains(Attrs::BOLD));
    assert!(!r.cell(1).style().attrs.contains(Attrs::INVERSE), "27 applies after 58's arguments");
    assert_eq!(r.cell(1).style().fg, Color::DEFAULT, "underline colour leaves fg alone");
}

#[test]
fn sgr_curly_dotted_dashed_underlines_are_stored() {
    let mut t = new(6, 1);
    t.advance(b"\x1b[4:3mA\x1b[4:4mB\x1b[4:5mC\x1b[4:9mD\x1b[24mE\x1b[4mF");
    let r = t.state.visible_row(0);
    let u: Vec<Underline> = (0..6).map(|i| r.cell(i).style().underline).collect();
    assert_eq!(
        u,
        [
            Underline::Curly,
            Underline::Dotted,
            Underline::Dashed,
            Underline::Dashed, // an unknown style keeps the current one
            Underline::None,
            Underline::Single,
        ]
    );
}

#[test]
fn sgr_overline_53_55() {
    let mut t = new(3, 1);
    t.advance(b"\x1b[53mA\x1b[1mB\x1b[55mC");
    let r = t.state.visible_row(0);
    assert!(r.cell(0).style().attrs.contains(Attrs::OVERLINE));
    assert!(r.cell(1).style().attrs.contains(Attrs::OVERLINE | Attrs::BOLD));
    assert!(!r.cell(2).style().attrs.contains(Attrs::OVERLINE));
    assert!(r.cell(2).style().attrs.contains(Attrs::BOLD), "55 clears only overline");
}

#[test]
fn sgr_underline_colour_58_59() {
    let mut t = new(5, 1);
    t.advance(b"\x1b[4;58;2;1;2;3mA\x1b[58:2::4:5:6mB\x1b[58;5;99mC\x1b[58:5:7mD\x1b[59mE");
    let r = t.state.visible_row(0);
    let c: Vec<Color> = (0..5).map(|i| r.cell(i).style().underline_color).collect();
    assert_eq!(
        c,
        [
            Color::rgb(1, 2, 3),
            Color::rgb(4, 5, 6),
            Color::indexed(99),
            Color::indexed(7),
            Color::DEFAULT
        ]
    );
    assert_eq!(r.cell(0).style().fg, Color::DEFAULT, "58 leaves fg alone");
    t.advance(b"\x1b[58;5;1m\x1b[0mF");
    assert_eq!(t.state.cursor().pen(), Style::DEFAULT, "SGR 0 resets the underline colour");
}

#[test]
fn erase_uses_the_pens_background_only() {
    let mut t = new(4, 2);
    t.advance(b"\x1b[1;4;31;44mAB\x1b[K\r\n\x1b[2;5;7mX\x1b[1X");
    let r = t.state.visible_row(0);
    let erased = r.cell(3).style();
    assert_eq!(
        erased,
        Style {
            bg: Color::indexed(4),
            ..Style::DEFAULT
        },
        "the erase style is the pen's background alone"
    );
    assert!(!r.cell(3).is_blank(), "a coloured erase shows");
    let r = t.state.visible_row(1);
    assert!(r.cell(0).style().attrs.contains(Attrs::INVERSE));
    assert_eq!(r.cell(1).style().bg, Color::indexed(4), "ECH at the cursor");
    assert!(r.cell(1).style().attrs.is_empty());
}

#[test]
fn saved_cursor_keeps_its_style_across_screens_and_sweeps() {
    let mut t = new(10, 3);
    t.state.set_sweep_at(Some(8));
    // Save a pen with every stored field, then churn both tables.
    t.advance(b"\x1b[1;4:3;53;38;2;9;8;7;48;5;3;58;5;2m\x1b7");
    t.advance(b"\x1b[?1049h");
    for i in 0..60u8 {
        t.advance(format!("\x1b[38;5;{i}m{}", (b'a' + i % 26) as char).as_bytes());
    }
    t.advance(b"\x1b[?1049l");
    for i in 0..60u8 {
        t.advance(format!("\x1b[48;5;{i}m\x1b[K").as_bytes());
    }
    t.advance(b"\x1b8Z");
    let saved = Style {
        fg: Color::rgb(9, 8, 7),
        bg: Color::indexed(3),
        underline_color: Color::indexed(2),
        attrs: Attrs::BOLD | Attrs::OVERLINE,
        underline: Underline::Curly,
        ..Style::DEFAULT
    };
    assert_eq!(t.state.cursor().pen(), saved);
    let (row, col) = (t.state.cursor().row(), t.state.cursor().col() - 1);
    assert_eq!(t.state.visible_row(row).cell(col).style(), saved);
    let (primary, alt) = t.state.style_stats();
    assert!(primary.sweeps > 0 && alt.sweeps > 0, "{primary:?} {alt:?}");
    t.state.check_invariants().unwrap();
    // A plain pen and a cleared screen leave only the default style: the
    // sweep renumbers, and the saved cursor still has its style.
    t.advance(b"\x1b[0m\x1b[2J");
    t.state.sweep_styles_now().unwrap();
    let (primary, _) = t.state.style_stats();
    assert!(primary.renumberings > 0 && primary.len == 1, "{primary:?}");
    t.advance(b"\x1b8Y");
    assert_eq!(t.state.visible_row(row).cell(col).style(), saved);
}

#[test]
fn sgr_incomplete_truecolor_leaves_pen() {
    let mut t = new(1, 1);
    t.advance(b"\x1b[31m\x1b[38;2;10mA");
    assert_eq!(t.state.visible_row(0).cell(0).style().fg, Color::indexed(1));
}

// ---------------------------------------------------------------- alt screen

#[test]
fn alt_47_roundtrip_preserves_contents() {
    let mut t = new(10, 2);
    t.advance(b"main");
    t.advance(b"\x1b[?47h\x1b[Halt\x1b[?47l");
    assert_eq!(text(&t, 0), "main");
    t.advance(b"\x1b[?47h");
    assert_eq!(text(&t, 0), "alt", "47 never clears the alt screen");
}

#[test]
fn alt_1047_entry_preserves_contents() {
    let mut t = new(10, 2);
    t.advance(b"\x1b[?47hkeepme\x1b[?47l");
    t.advance(b"\x1b[?1047h");
    assert_eq!(text(&t, 0), "keepme");
}

#[test]
fn alt_1047_exit_clears_contents() {
    let mut t = new(10, 2);
    t.advance(b"\x1b[?1047hjunk\x1b[?1047l");
    t.advance(b"\x1b[?47h");
    assert_eq!(text(&t, 0), "");
}

#[test]
fn alt_1049_saves_cursor_and_clears() {
    let mut t = new(10, 2);
    t.advance(b"main\x1b[2;3H");
    t.advance(b"\x1b[?1049h");
    assert_eq!(text(&t, 0), "", "alt screen starts cleared");
    assert_eq!(pos(&t), (1, 2), "cursor stays until restored");
    t.advance(b"x\x1b[?1049l");
    assert_eq!(text(&t, 0), "main");
    assert_eq!(pos(&t), (1, 2), "cursor restored");
}

#[test]
fn alt_1049_entry_always_clears() {
    let mut t = new(10, 2);
    t.advance(b"\x1b[?1049hold\x1b[?1049l\x1b[?1049h");
    assert_eq!(text(&t, 0), "", "1049h clears whatever the alt screen held");
}

#[test]
fn alt_47_does_not_save_cursor() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[1;5H");
    t.advance(b"\x1b[?47h");
    assert_eq!(pos(&t), (0, 4), "entering alt keeps the cursor");
    t.advance(b"\x1b[2;2H\x1b[?47l");
    assert_eq!(pos(&t), (1, 1), "leaving alt keeps the cursor where it was");
}

// ---------------------------------------------------------------- replies

#[test]
fn dsr_5_replies_ok() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[5n");
    assert_eq!(t.state.replies(), b"\x1b[0n");
}

#[test]
fn cpr_reports_1_based_position() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[3;5H\x1b[6n");
    assert_eq!(t.state.replies(), b"\x1b[3;5R");
}

#[test]
fn cpr_counts_pending_wrap_column() {
    let mut t = new(5, 2);
    t.advance(b"abcde\x1b[6n");
    assert_eq!(t.state.replies(), b"\x1b[1;5R");
}

#[test]
fn cpr_in_origin_mode_is_region_relative() {
    let mut t = new(10, 6);
    t.advance(b"\x1b[3;6r\x1b[?6h\x1b[2;4H\x1b[6n");
    assert_eq!(pos(&t), (3, 3));
    assert_eq!(t.state.replies(), b"\x1b[2;4R");
}

#[test]
fn da_replies() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[c\x1b[>c");
    assert_eq!(t.state.replies(), b"\x1b[?62;22c\x1b[>1;10;0c");
}

// ---------------------------------------------------------------- resize

#[test]
fn resize_shrink_width_keeps_content() {
    let mut t = new(10, 3);
    t.advance(b"hello\r\nworld");
    t.state.resize(5, 3);
    assert_eq!(screen(&t), ["hello", "world", ""]);
}

#[test]
fn resize_shrink_width_moves_wide_at_edge_to_the_next_row() {
    let mut t = new(5, 1);
    t.advance("ab漢".as_bytes());
    t.state.resize(3, 1);
    // The line wraps: "ab" and an edge spacer, then the wide character
    // whole on the next row.
    assert_eq!(text(&t, 0), "漢");
    assert_eq!(t.state.visible_row(0).cell(0).width(), Width::Wide);
    assert_eq!(history(&mut t, 0), "ab");
    assert_eq!(pos(&t), (0, 2));
}

#[test]
fn resize_shrink_height_sends_top_rows_to_scrollback() {
    let mut t = new(5, 5);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne");
    t.state.resize(5, 3);
    assert_eq!(screen(&t), ["c", "d", "e"]);
    assert_eq!(pos(&t), (2, 1), "cursor follows its content");
    assert_eq!(t.state.scrollback_len(), 2);
    assert_eq!(history(&mut t, 0), "a");
}

#[test]
fn resize_shrink_height_below_cursor_is_kept() {
    // xterm drops the rows below the cursor; we lose nothing (§12.2 of
    // the cells design): blank rows below the cursor go first, then rows
    // from the top go into history, the cursor's own row too.
    let mut t = new(5, 5);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne");
    t.advance(b"\x1b[2;1H"); // cursor on b
    t.state.resize(5, 3);
    assert_eq!(screen(&t), ["c", "d", "e"]);
    assert_eq!(pos(&t), (0, 0));
    assert_eq!(t.state.scrollback_len(), 2);
    // Blank rows below the cursor are dropped first.
    let mut t = new(5, 5);
    t.advance(b"a\r\nb\r\nc");
    t.state.resize(5, 3);
    assert_eq!(screen(&t), ["a", "b", "c"]);
    assert_eq!(pos(&t), (2, 1));
    assert_eq!(t.state.scrollback_len(), 0);
}

#[test]
fn resize_grow_height_pulls_from_scrollback() {
    let mut t = new(5, 5);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne");
    t.state.resize(5, 3);
    assert_eq!(t.state.scrollback_len(), 2);
    t.state.resize(5, 5);
    assert_eq!(screen(&t), ["a", "b", "c", "d", "e"]);
    assert_eq!(t.state.scrollback_len(), 0);
    assert_eq!(pos(&t), (4, 1), "cursor moves down with the pulled rows");
}

#[test]
fn resize_rewraps_lines_at_the_new_width() {
    let mut t = new(10, 3);
    t.advance(b"abcdefghijklmno");
    t.state.resize(5, 4);
    assert_eq!(screen(&t), ["abcde", "fghij", "klmno", ""]);
    // Just past the end of a full row: waiting to wrap.
    assert_eq!(pos(&t), (2, 4));
    assert!(t.state.cursor().pending_wrap());
    t.state.resize(20, 3);
    assert_eq!(screen(&t), ["abcdefghijklmno", "", ""]);
    assert_eq!(pos(&t), (0, 15));
    assert_eq!(t.state.scrollback_len(), 0);
    // Writing goes on where the line ends.
    t.advance(b"p");
    assert_eq!(text(&t, 0), "abcdefghijklmnop");
}

#[test]
fn resize_keeps_wide_characters_and_clusters_whole() {
    let mut t = new(10, 3);
    t.advance("ab👍\u{1F3FD}c漢".as_bytes());
    t.state.resize(3, 3);
    assert_eq!(screen(&t), ["ab", "👍c", "漢"]);
    let row = t.state.visible_row(0);
    assert_eq!(row.cell(2).width(), Width::EdgeSpacer);
    assert!(row.wrapped());
    let row = t.state.visible_row(1);
    let mut s = String::new();
    row.cell(0).push_text(&mut s);
    assert_eq!(s, "👍\u{1F3FD}");
    assert_eq!(row.cell(0).width(), Width::Wide);
    t.state.resize(10, 3);
    assert_eq!(screen(&t), ["ab👍c漢", "", ""]);
    t.state.check_invariants().unwrap();
}

#[test]
fn resize_keeps_cells_up_to_the_cursor() {
    let mut t = new(10, 3);
    t.advance(b"abc\x1b[1;9H");
    t.state.resize(4, 3);
    assert_eq!(screen(&t), ["abc", "", ""]);
    assert_eq!(pos(&t), (1, 3));
    assert!(t.state.cursor().pending_wrap());
    t.advance(b"x");
    assert_eq!(screen(&t), ["abc", "", "x"]);
}

#[test]
fn resize_carries_an_erased_tails_colour_to_the_new_row_end() {
    let mut t = new(10, 2);
    t.advance(b"\x1b[41m\x1b[K\x1b[0mab\r\n");
    t.state.resize(4, 2);
    assert_eq!(screen(&t), ["ab", ""]);
    let row = t.state.visible_row(0);
    assert!(!row.wrapped(), "the coloured tail isn't wrapped onto more rows");
    assert_eq!(row.cell(3).style().bg, Color::indexed(1));
    t.state.resize(8, 2);
    assert_eq!(t.state.visible_row(0).cell(7).style().bg, Color::indexed(1));
}

#[test]
fn resize_moves_the_view_with_its_text() {
    let mut t = new(10, 2);
    for i in 0..20 {
        t.advance(format!("\r\nL{i}").as_bytes());
    }
    t.state.scroll_view(5);
    let top = text(&t, 0);
    t.state.resize(4, 3);
    assert_eq!(text(&t, 0), top, "history isn't rewritten: the view stays on its line");
    t.state.resize(12, 2);
    assert_eq!(text(&t, 0), top);
}

#[test]
fn resize_on_alt_reflows_the_primary_once_on_leaving() {
    let mut t = new(10, 3);
    t.advance(b"hello world!");
    t.advance(b"\x1b[?1049h");
    t.state.resize(6, 3);
    t.advance(b"vim");
    t.state.resize(4, 3);
    t.state.check_invariants().unwrap();
    t.advance(b"\x1b[?1049l");
    assert_eq!(screen(&t), ["hell", "o wo", "rld!"]);
    assert_eq!(pos(&t), (2, 3), "the saved cursor follows its text");
    assert_eq!(t.state.scrollback_len(), 0);
    t.state.check_invariants().unwrap();
}

#[test]
fn resize_grow_width_pads_blanks() {
    let mut t = new(5, 2);
    t.advance(b"abcde");
    t.state.resize(10, 2);
    assert_eq!(text(&t, 0), "abcde");
    assert_eq!(t.state.visible_row(0).len(), 10);
    assert_eq!(pos(&t), (0, 5), "the cursor waiting to wrap goes on along its line");
}

#[test]
fn resize_on_alt_preserves_primary_and_scrollback() {
    let mut t = new(10, 4);
    t.advance(b"keep this\r\nline two");
    t.advance(b"\x1b[?1049h");
    t.state.resize(6, 3);
    t.advance(b"alt");
    t.state.resize(10, 4);
    t.advance(b"\x1b[?1049l");
    // The primary screen waited for the program to leave, and is back at
    // its own size: nothing changed.
    assert_eq!(text(&t, 0), "keep this");
    assert_eq!(text(&t, 1), "line two");
    assert_eq!(t.state.scrollback_len(), 0);
}

#[test]
fn resize_clamps_cursor_and_clears_pending_wrap() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[4;10H");
    t.advance(b"abcde"); // ends at last column: pending wrap... not quite
    t.state.resize(4, 2);
    // 9 blanks, "a", then "bcde": 14 cells, the cursor after them, at
    // column 2 of the line's fourth row.
    assert_eq!(pos(&t), (1, 2));
    assert!(!t.state.cursor().pending_wrap());
    // pending wrap on a shrink:
    let mut t = new(5, 2);
    t.advance(b"abcde");
    assert!(t.state.cursor().pending_wrap());
    t.state.resize(8, 2);
    assert!(!t.state.cursor().pending_wrap());
}

#[test]
fn resize_keeps_the_selection_on_its_text() {
    let mut t = new(10, 2);
    t.advance(b"hello world");
    let a = t.state.point_at(0, 0);
    let b = t.state.point_at(0, 7);
    t.state.set_selection(Some(Selection::new(a, b)));
    t.state.resize(5, 3);
    assert_eq!(t.state.selection_text().unwrap(), "hello wo");
    t.state.resize(20, 3);
    assert_eq!(t.state.selection_text().unwrap(), "hello wo");
}

// ---------------------------------------------------------------- rep/charset/reset

#[test]
fn rep_repeats_last_char() {
    let mut t = new(10, 1);
    t.advance(b"a\x1b[3b");
    assert_eq!(text(&t, 0), "aaaa");
}

#[test]
fn rep_wraps() {
    let mut t = new(5, 2);
    t.advance(b"ab\x1b[5b");
    assert_eq!(text(&t, 0), "abbbb");
    assert_eq!(text(&t, 1), "bb");
}

#[test]
fn rep_without_last_char_is_noop() {
    let mut t = new(5, 1);
    t.advance(b"\x1b[3b");
    assert_eq!(text(&t, 0), "");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn rep_uses_current_pen_not_original() {
    let mut t = new(4, 1);
    t.advance(b"a\x1b[31m\x1b[2b");
    let r = t.state.visible_row(0);
    assert_eq!(text(&t, 0), "aaa");
    assert_eq!(r.cell(0).style().fg, Color::DEFAULT);
    assert_eq!(r.cell(1).style().fg, Color::indexed(1));
}

#[test]
fn dec_line_drawing_charset() {
    let mut t = new(8, 1);
    t.advance(b"\x1b(0lqkxmjn\x1b(Bq");
    assert_eq!(text(&t, 0), "┌─┐│└┘┼q");
}

#[test]
fn ris_full_reset() {
    let mut t = new(5, 3);
    t.advance(b"aa\r\nbb\r\ncc\r\ndd"); // scrollback: aa
    t.advance(b"\x1b[1;31m\x1b[2;3r\x1b[?6h\x1b[4h\x1b[?7l");
    t.advance(b"\x1bc");
    assert_eq!(screen(&t), ["", "", ""]);
    assert_eq!(pos(&t), (0, 0));
    assert!(!t.state.modes().origin && !t.state.modes().insert && t.state.modes().autowrap);
    t.advance(b"z");
    let row = t.state.visible_row(0);
    let c = row.cell(0);
    assert_eq!(c.style(), Style::DEFAULT);
    assert_eq!(c.style().fg, Color::DEFAULT);
    // margins gone: linefeed on the last row scrolls the whole screen
    let before = t.state.scrollback_len();
    t.advance(b"\x1b[3;1H\n");
    assert_eq!(t.state.scrollback_len(), before + 1);
}

#[test]
fn ris_clears_scrollback() {
    let mut t = new(5, 3);
    t.advance(b"aa\r\nbb\r\ncc\r\ndd");
    assert_eq!(t.state.scrollback_len(), 1);
    t.advance(b"\x1bc");
    assert_eq!(t.state.scrollback_len(), 0);
}

#[test]
fn decstr_is_a_soft_reset() {
    let mut t = new(5, 3);
    t.advance(b"l1\r\nl2\r\nl3\r\nl4"); // scrollback: l1; screen: l2 l3 l4
    t.advance(b"\x1b[2;3r\x1b[?6h\x1b[4h\x1b[1m");
    t.advance(b"\x1b[!p");
    assert_eq!(screen(&t), ["l2", "l3", "l4"], "DECSTR keeps the screen");
    assert_eq!(t.state.scrollback_len(), 1, "DECSTR keeps scrollback");
    assert!(!t.state.modes().origin);
    assert!(!t.state.modes().insert);
    assert!(t.state.modes().autowrap, "DECSTR restores autowrap on");
    t.advance(b"z"); // lands where the cursor was left
    let row = t.state.visible_row(1);
    let z = row.cell(0);
    assert_eq!(z.ch(), 'z');
    assert!(!z.style().attrs.contains(Attrs::BOLD), "DECSTR resets attributes");
    // margins reset to the full screen
    t.advance(b"\x1b[3;1H\n");
    assert_eq!(t.state.scrollback_len(), 2, "no scroll region after DECSTR");
}

// ---------------------------------------------------------------- selection

#[test]
fn selection_text_single_line() {
    let mut t = new(10, 1);
    t.advance(b"abcdef");
    let a = t.state.point_at(0, 1);
    let b = t.state.point_at(0, 3);
    t.state.set_selection(Some(Selection::new(a, b)));
    assert_eq!(t.state.selection_text().unwrap(), "bcd");
}

#[test]
fn selection_text_multiline_uses_newlines() {
    let mut t = new(10, 3);
    t.advance(b"aaa\r\nbbb\r\nccc");
    let a = t.state.point_at(0, 1);
    let b = t.state.point_at(1, 1);
    t.state.set_selection(Some(Selection::new(a, b)));
    assert_eq!(t.state.selection_text().unwrap(), "aa\nbb");
}

#[test]
fn selection_text_trims_trailing_blanks() {
    let mut t = new(10, 2);
    t.advance(b"ab\r\ncd");
    let a = t.state.point_at(0, 0);
    let b = t.state.point_at(0, 9);
    t.state.set_selection(Some(Selection::new(a, b)));
    assert_eq!(t.state.selection_text().unwrap(), "ab");
}

#[test]
fn selection_text_joins_wrapped_without_newline() {
    let mut t = new(5, 3);
    t.advance(b"abcdefgh");
    let a = t.state.point_at(0, 2);
    let b = t.state.point_at(1, 1);
    t.state.set_selection(Some(Selection::new(a, b)));
    assert_eq!(t.state.selection_text().unwrap(), "cdefg");
}

#[test]
fn selection_text_reversed_is_ordered() {
    let mut t = new(10, 2);
    t.advance(b"abcdef");
    let a = t.state.point_at(0, 1);
    let b = t.state.point_at(0, 3);
    t.state.set_selection(Some(Selection::new(b, a)));
    assert_eq!(t.state.selection_text().unwrap(), "bcd");
}

#[test]
fn selection_text_wide_chars_once() {
    let mut t = new(5, 1);
    t.advance("a漢b".as_bytes());
    let a = t.state.point_at(0, 0);
    let b = t.state.point_at(0, 3);
    t.state.set_selection(Some(Selection::new(a, b)));
    assert_eq!(t.state.selection_text().unwrap(), "a漢b");
}

#[test]
fn selection_text_spans_scrollback() {
    let mut t = new(5, 2);
    t.advance(b"aa\r\nbb\r\ncc");
    assert_eq!(t.state.scrollback_len(), 1);
    t.state.scroll_view(1);
    let a = t.state.point_at(0, 1); // scrollback "aa", col 1
    t.state.scroll_view_to_bottom();
    let b = t.state.point_at(1, 1); // screen "cc", col 1
    t.state.set_selection(Some(Selection::new(a, b)));
    assert_eq!(t.state.selection_text().unwrap(), "a\nbb\ncc");
}

#[test]
fn selection_text_none_without_selection() {
    let t = new(5, 2);
    assert!(t.state.selection_text().is_none());
}

// ---------------------------------------------------------------- misc

#[test]
fn decaln_fills_screen_with_e() {
    let mut t = new(4, 2);
    t.advance(b"\x1b#8");
    assert_eq!(screen(&t), ["EEEE", "EEEE"]);
}

#[test]
fn bell_and_title_flags() {
    let mut t = new(4, 1);
    t.advance(b"\x07");
    assert!(t.state.take_bell());
    assert!(!t.state.take_bell(), "taken");
    t.advance(b"\x1b]0;my title\x07");
    assert_eq!(t.state.title(), Some("my title"));
    assert_eq!(t.state.take_title().as_deref(), Some("my title"));
    assert_eq!(t.state.take_title(), None, "taken");
    t.advance(b"\x1b]2;other\x1b\\");
    assert_eq!(t.state.title(), Some("other"));
}

#[test]
fn linefeed_newline_mode() {
    let mut t = new(6, 2);
    t.advance(b"ab\ncd");
    assert_eq!(text(&t, 1), "  cd", "plain LF keeps the column");
    let mut t = new(6, 2);
    t.advance(b"\x1b[20hab\ncd");
    assert_eq!(text(&t, 1), "cd", "LNM makes LF do CR+LF");
}

#[test]
fn backspace_moves_left_and_stops() {
    let mut t = new(6, 1);
    t.advance(b"ab\x08c");
    assert_eq!(text(&t, 0), "ac");
    t.advance(b"\x1b[H\x08\x08x");
    assert_eq!(text(&t, 0), "xc");
}

#[test]
fn private_modes_set_and_reset() {
    let mut t = new(10, 4);
    t.advance(b"\x1b[?1h\x1b[?25l\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?2026h\x1b=");
    assert!(t.state.modes().app_cursor);
    assert!(!t.state.modes().cursor_visible);
    assert!(t.state.modes().bracketed_paste);
    assert_eq!(t.state.modes().mouse, term_core::MouseMode::Drag);
    assert!(t.state.modes().mouse_sgr);
    assert!(t.state.modes().synchronized);
    assert!(t.state.modes().app_keypad);
    t.advance(b"\x1b[?1l\x1b[?25h\x1b[?2004l\x1b[?1002l\x1b>");
    assert!(!t.state.modes().app_cursor);
    assert!(t.state.modes().cursor_visible);
    assert!(!t.state.modes().bracketed_paste);
    assert_eq!(t.state.modes().mouse, term_core::MouseMode::Off);
    assert!(!t.state.modes().app_keypad);
}

#[test]
fn decscusr_sets_cursor_shape() {
    let mut t = new(4, 1);
    t.advance(b"\x1b[5 q");
    assert_eq!(t.state.cursor_shape(), CursorShape::Bar);
    assert!(t.state.cursor_blink());
    t.advance(b"\x1b[2 q");
    assert_eq!(t.state.cursor_shape(), CursorShape::Block);
    assert!(!t.state.cursor_blink());
}

/// The user's cursor (settings) is the cursor until a program sets one;
/// `CSI 0 SP q` and a reset go back to it, and a new setting changes a
/// cursor the program left alone but not one it set.
#[test]
fn cursor_default_is_used_until_a_program_sets_one() {
    let mut t = new(4, 1);
    t.state.set_cursor_default(CursorShape::Bar, true);
    assert_eq!((t.state.cursor_shape(), t.state.cursor_blink()), (CursorShape::Bar, true));
    t.advance(b"\x1b[4 q");
    assert_eq!((t.state.cursor_shape(), t.state.cursor_blink()), (CursorShape::Underline, false));
    t.state.set_cursor_default(CursorShape::Block, true);
    assert_eq!(t.state.cursor_shape(), CursorShape::Underline, "the program's stays");
    t.advance(b"\x1b[0 q");
    assert_eq!((t.state.cursor_shape(), t.state.cursor_blink()), (CursorShape::Block, true));
    t.advance(b"\x1b[6 q\x1bc");
    assert_eq!((t.state.cursor_shape(), t.state.cursor_blink()), (CursorShape::Block, true), "RIS");
    t.state.set_cursor_default(CursorShape::Underline, false);
    assert_eq!((t.state.cursor_shape(), t.state.cursor_blink()), (CursorShape::Underline, false));
}

#[test]
fn scroll_view_clamps_to_scrollback() {
    let mut t = new(5, 2);
    t.advance(b"aa\r\nbb\r\ncc");
    t.state.scroll_view(100);
    assert_eq!(t.state.display_offset(), 1);
    assert_eq!(text(&t, 0), "aa");
    t.state.scroll_view(-100);
    assert_eq!(t.state.display_offset(), 0);
    assert_eq!(text(&t, 0), "bb");
}

#[test]
fn view_stays_put_while_output_scrolls() {
    let mut t = new(5, 2);
    t.advance(b"aa\r\nbb\r\ncc");
    t.state.scroll_view(1);
    t.advance(b"\r\ndd\r\nee");
    assert_eq!(text(&t, 0), "aa", "view follows the same history row");
    t.state.scroll_view_to_bottom();
    assert_eq!(screen(&t), ["dd", "ee"]);
}

// OSC 8 and OSC 133 aren't in xterm. These follow the hyperlink spec
// (egmontkoblinger's "Hyperlinks in terminal emulators", which kitty,
// iTerm2, VTE and WezTerm implement) and FinalTerm's shell integration
// marks as kitty and WezTerm read them.

#[test]
fn osc8_is_independent_of_sgr_and_ended_by_an_empty_uri() {
    let mut t = new(20, 2);
    t.advance(b"\x1b]8;;http://a\x1b\\\x1b[0mx\x1b[1my\x1b]8;;\x1b\\z");
    let uri = |t: &Term, col| t.state.link_at(t.state.point_at(0, col)).map(|l| l.uri);
    assert_eq!(uri(&t, 0).as_deref(), Some("http://a"), "SGR 0 doesn't end a link");
    assert_eq!(uri(&t, 1).as_deref(), Some("http://a"));
    assert_eq!(uri(&t, 2), None);
    // A new link replaces the old one without an end in between.
    t.advance(b"\x1b]8;;http://b\x1b\\b");
    assert_eq!(uri(&t, 3).as_deref(), Some("http://b"));
}

#[test]
fn osc8_link_is_saved_and_restored_with_the_cursor() {
    let mut t = new(20, 2);
    t.advance(b"\x1b]8;;http://s\x1b\\\x1b[s\x1b]8;;\x1b\\a\x1b[ub");
    assert_eq!(t.state.link_at(t.state.point_at(0, 0)).map(|l| l.uri).as_deref(), Some("http://s"));
}

#[test]
fn osc133_reports_commands_starting_and_finishing() {
    use term_core::Command;
    let mut t = new(20, 4);
    t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07cargo build  \r\n\x1b]133;C\x07out\r\n");
    assert_eq!(
        t.state.take_commands(),
        [Command::Prompt, Command::Started(Some("cargo build".into()))]
    );
    assert_eq!(t.state.take_commands(), []);
    t.advance(b"\x1b]133;D;101\x07\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07\x1b]133;D\x07");
    assert_eq!(
        t.state.take_commands(),
        [
            Command::Finished(Some(101)),
            Command::Prompt,
            Command::Started(Some("ls".into())),
            Command::Finished(None)
        ]
    );
    // A command line that wrapped onto a second row is one line.
    let mut t = new(10, 4);
    t.advance(b"\x1b]133;B\x07echo 12345678\r\n\x1b]133;C\x07");
    assert_eq!(t.state.take_commands(), [Command::Started(Some("echo 12345678".into()))]);
    // No B mark: no text.
    t.advance(b"\x1b]133;C\x07");
    assert_eq!(t.state.take_commands(), [Command::Started(None)]);
}

#[test]
fn osc133_marks_where_the_cursor_is() {
    use term_core::{Mark, MarkKind};
    let mut t = new(20, 3);
    t.advance(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\x\r\n\x1b]133;C\x1b\\out\r\n\x1b]133;D;2\x1b\\");
    let line = |r| t.state.point_at(r, 0).line;
    let kinds = |r| t.state.marks_at(line(r)).into_iter().map(|m| (m.kind, m.col)).collect::<Vec<_>>();
    assert_eq!(kinds(0), [(MarkKind::Prompt, 0), (MarkKind::Input, 2)]);
    assert_eq!(kinds(1), [(MarkKind::Output, 0)]);
    assert_eq!(
        t.state.marks_at(line(2)),
        [Mark {
            kind: MarkKind::End,
            col: 0,
            status: Some(2)
        }]
    );
}
