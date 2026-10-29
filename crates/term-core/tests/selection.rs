//! Selection, view scrolling and the reply queue: what is copied, how a
//! selection follows its text as output scrolls, how far the view scrolls,
//! and the order replies to queries go back to the program in.

use insta::assert_snapshot;
use term_core::{Selection, SelectionKind, Term, ViewTop};

/// Select between two visible (line, col) positions.
fn select(t: &mut Term, anchor: (usize, usize), head: (usize, usize)) {
    let anchor = t.state.point_at(anchor.0, anchor.1);
    let head = t.state.point_at(head.0, head.1);
    t.state.set_selection(Some(Selection::new(anchor, head)));
}

fn copied(t: &Term) -> String {
    t.state.selection_text().expect("a selection")
}

#[test]
fn selection_spans_scrollback_and_screen() {
    let mut t = Term::new(10, 2, 100);
    t.advance(b"one\r\ntwo\r\nthree");
    assert_eq!(t.state.scrollback_len(), 1);
    t.state.scroll_view(1);
    let anchor = t.state.point_at(0, 1);
    t.state.scroll_view(-1);
    let head = t.state.point_at(1, 2);
    t.state.set_selection(Some(Selection::new(anchor, head)));
    assert_eq!(copied(&t), "ne\ntwo\nthr");
}

#[test]
fn reversed_anchor_and_head_copy_the_same_text() {
    let mut t = Term::new(10, 3, 100);
    t.advance(b"alpha\r\nbeta\r\ngamma");
    select(&mut t, (0, 2), (2, 1));
    let forward = copied(&t);
    select(&mut t, (2, 1), (0, 2));
    assert_eq!(copied(&t), forward);
    assert_eq!(forward, "pha\nbeta\nga");
}

#[test]
fn selection_stays_on_its_text_as_it_scrolls_into_history() {
    let mut t = Term::new(10, 2, 3);
    t.advance(b"one\r\ntwo");
    select(&mut t, (1, 0), (1, 2));
    assert_eq!(copied(&t), "two");
    t.advance(b"\r\nthree\r\nfour");
    assert_eq!(t.state.scrollback_len(), 2);
    assert_eq!(copied(&t), "two");
    // Gone once it drops out of the scrollback (limit 3).
    t.advance(b"\r\nfive\r\nsix");
    assert_eq!(t.state.scrollback_len(), 3);
    assert_eq!(copied(&t), "two");
    t.advance(b"\r\nseven");
    assert_eq!(copied(&t), "");
}

#[test]
fn selection_stays_put_while_the_view_scrolls() {
    let mut t = Term::new(10, 2, 100);
    t.advance(b"a\r\nb\r\nc\r\nd");
    select(&mut t, (0, 0), (0, 0));
    assert_eq!(copied(&t), "c");
    t.state.scroll_view(2);
    assert_eq!(copied(&t), "c");
}

#[test]
fn resize_moves_the_selection_with_its_text() {
    let mut t = Term::new(10, 2, 100);
    t.advance(b"hello");
    select(&mut t, (0, 0), (0, 4));
    t.state.resize(10, 2);
    assert_eq!(copied(&t), "hello", "same size is not a resize");
    t.state.resize(12, 3);
    assert_eq!(copied(&t), "hello");
    t.state.resize(3, 3);
    assert_eq!(copied(&t), "hello");
    t.state.resize(10, 2);
    assert_eq!(copied(&t), "hello");
}

#[test]
fn wide_chars_are_copied_once() {
    let mut t = Term::new(10, 2, 100);
    t.advance("a漢b".as_bytes());
    select(&mut t, (0, 0), (0, 3));
    assert_eq!(copied(&t), "a漢b");
    // Ending on the left half takes the whole character.
    select(&mut t, (0, 0), (0, 1));
    assert_eq!(copied(&t), "a漢");
    // Starting on the right half skips it.
    select(&mut t, (0, 2), (0, 3));
    assert_eq!(copied(&t), "b");
    // Also once it has moved into scrollback.
    t.advance(b"\r\n\r\n");
    t.state.scroll_view(1);
    select(&mut t, (0, 0), (0, 9));
    assert_eq!(copied(&t), "a漢b");
}

#[test]
fn trailing_spaces_trimmed_except_on_wrapped_lines() {
    let mut t = Term::new(4, 4, 100);
    t.advance(b"ab  \r\ncd\r\nef  gh");
    assert_snapshot!(t.state.dump(), @r"
    ab··|
    cd|
    ef··⏎
    gh|
    cursor=(3,2) alt=false offset=0
    ");
    select(&mut t, (0, 0), (3, 3));
    assert_eq!(copied(&t), "ab\ncd\nef  gh");
    // The last line of a selection is trimmed even if it wraps.
    select(&mut t, (2, 0), (2, 3));
    assert_eq!(copied(&t), "ef");
}

#[test]
fn view_scroll_clamps_at_both_ends() {
    let mut t = Term::new(10, 2, 100);
    for i in 0..10 {
        t.advance(format!("{i}\r\n").as_bytes());
    }
    let max = t.state.scrollback_len();
    assert_eq!(max, 9);
    let generation = t.state.generation();
    t.state.scroll_view(-5);
    assert_eq!(t.state.display_offset(), 0);
    assert_eq!(t.state.generation(), generation, "no change, no redraw");
    t.state.scroll_view(3);
    assert_eq!(t.state.display_offset(), 3);
    t.state.scroll_view(1000);
    assert_eq!(t.state.display_offset(), max);
    assert_snapshot!(t.state.dump(), @r"
    0|
    1|
    cursor=(1,0) alt=false offset=9
    ");
    t.state.scroll_view(-1000);
    assert_eq!(t.state.display_offset(), 0);
    t.state.scroll_view(2);
    t.state.scroll_view_to_bottom();
    assert_eq!(t.state.display_offset(), 0);
}

#[test]
fn view_scroll_does_nothing_on_the_alt_screen() {
    let mut t = Term::new(10, 2, 100);
    t.advance(b"a\r\nb\r\nc\r\n");
    t.state.scroll_view(1);
    assert_eq!(t.state.display_offset(), 1);
    t.advance(b"\x1b[?1049h");
    assert_eq!(t.state.display_offset(), 0, "entering the alt screen follows output");
    t.state.scroll_view(1);
    assert_eq!(t.state.display_offset(), 0);
    t.advance(b"\x1b[?1049l");
    t.state.scroll_view(1);
    assert_eq!(t.state.display_offset(), 1);
}

#[test]
fn view_scroll_with_no_scrollback() {
    let mut t = Term::new(10, 2, 0);
    t.advance(b"a\r\nb\r\nc\r\n");
    t.state.scroll_view(5);
    assert_eq!(t.state.display_offset(), 0);
}

#[test]
fn replies_in_one_chunk_keep_their_order() {
    let mut t = Term::new(10, 2, 0);
    t.advance(b"\x1b[5n\x1b[2;3H\x1b[6n\x1b[c\x1b]11;?\x07\x1b[?2026$p\x1b[>c\x1b[18t");
    let replies = String::from_utf8(t.state.replies().to_vec()).unwrap();
    assert_eq!(
        replies,
        "\x1b[0n\x1b[2;3R\x1b[?62;22c\x1b]11;rgb:1616/1818/1c1c\x1b\\\x1b[?2026;2$y\x1b[>1;10;0c\x1b[8;2;10t"
    );
}

// ------------------------------------------------- gestures and the view

/// Double-click at a visible position; the selected text.
fn word(t: &mut Term, line: usize, col: usize) -> String {
    let p = t.state.point_at(line, col);
    t.state.select_word(p);
    assert_eq!(t.state.selection().unwrap().kind, SelectionKind::Word);
    copied(t)
}

#[test]
fn double_click_selects_a_word() {
    let mut t = Term::new(40, 3, 100);
    t.advance(b"see src/main.rs:12:5 (here)");
    assert_eq!(word(&mut t, 0, 0), "see");
    assert_eq!(word(&mut t, 0, 2), "see");
    assert_eq!(word(&mut t, 0, 9), "src/main.rs:12:5");
    assert_eq!(word(&mut t, 0, 23), "here");
    // Off a word: just that cell.
    assert_eq!(word(&mut t, 0, 3), "");
    assert_eq!(word(&mut t, 0, 21), "(");
    assert_eq!(word(&mut t, 0, 35), "");
    assert_eq!(word(&mut t, 2, 0), "");
}

#[test]
fn double_click_selects_a_word_split_by_a_soft_wrap() {
    let mut t = Term::new(10, 4, 100);
    // "averyverylongword" wraps after 6 of its letters.
    t.advance(b"one  averyverylongword two");
    assert_snapshot!(t.state.dump(), @r"
    one  avery⏎
    verylongwo⏎
    rd two|
    |
    cursor=(2,6) alt=false offset=0
    ");
    let whole = "averyverylongword";
    assert_eq!(word(&mut t, 0, 7), whole, "clicked in the first row");
    assert_eq!(word(&mut t, 1, 3), whole, "clicked in the middle row");
    assert_eq!(word(&mut t, 2, 0), whole, "clicked in the last row");
    let sel = t.state.selection().unwrap();
    assert_eq!(sel.anchor, t.state.point_at(0, 5));
    assert_eq!(sel.head, t.state.point_at(2, 1));
    // A word that only touches the wrap from one side.
    assert_eq!(word(&mut t, 2, 4), "two");
    assert_eq!(word(&mut t, 0, 1), "one");
}

#[test]
fn a_hard_line_break_ends_a_word() {
    let mut t = Term::new(10, 3, 100);
    t.advance(b"0123456789\r\nabc");
    assert!(!t.state.visible_row(0).wrapped());
    assert_eq!(word(&mut t, 0, 9), "0123456789");
    assert_eq!(word(&mut t, 1, 0), "abc");
}

#[test]
fn double_click_on_a_wrapped_word_in_history() {
    let mut t = Term::new(6, 2, 100);
    t.advance(b"xx abcdefgh\r\n\r\n\r\n");
    assert_eq!(t.state.scrollback_len(), 3);
    t.state.scroll_view(3);
    assert_eq!(word(&mut t, 1, 2), "abcdefgh");
    // Across the boundary between history and the screen.
    let mut t = Term::new(6, 2, 100);
    t.advance(b"\r\nxx abcdefgh");
    assert_eq!(t.state.scrollback_len(), 1);
    assert_eq!(word(&mut t, 1, 0), "abcdefgh", "from the screen");
    t.state.scroll_view(1);
    assert_eq!(word(&mut t, 1, 4), "abcdefgh", "from history");
}

#[test]
fn double_click_takes_wide_characters_whole() {
    let mut t = Term::new(10, 2, 100);
    t.advance("a漢字b c".as_bytes());
    for col in 0..=5 {
        assert_eq!(word(&mut t, 0, col), "a漢字b", "at {col}");
    }
    let sel = t.state.selection().unwrap();
    assert_eq!((sel.anchor.col, sel.head.col), (0, 5));
    // A wide character that didn't fit at the end of a row moved to the
    // next; the empty cell it left doesn't split the word.
    let mut t = Term::new(5, 2, 100);
    t.advance("abcd漢x".as_bytes());
    assert_snapshot!(t.state.dump(), @r"
    abcd⏎
    漢x|
    cursor=(1,3) alt=false offset=0
    ");
    assert_eq!(word(&mut t, 0, 1), "abcd漢x");
    assert_eq!(word(&mut t, 1, 1), "abcd漢x", "from the spacer");
}

#[test]
fn triple_click_selects_the_logical_line() {
    let mut t = Term::new(5, 5, 100);
    t.advance(b"first\r\nsecond line\r\nlast");
    let p = t.state.point_at(2, 2);
    t.state.select_line(p);
    let sel = t.state.selection().unwrap();
    assert_eq!(sel.kind, SelectionKind::Line);
    assert_eq!(copied(&t), "second line");
    assert_eq!(sel.ordered(), (t.state.point_at(1, 0), t.state.point_at(3, 4)));
    let p = t.state.point_at(0, 3);
    t.state.select_line(p);
    assert_eq!(copied(&t), "first");
    let p = t.state.point_at(4, 0);
    t.state.select_line(p);
    assert_eq!(copied(&t), "last");
}

#[test]
fn dragging_grows_word_and_line_selections_by_whole_units() {
    let mut t = Term::new(30, 3, 100);
    t.advance(b"alpha beta gamma\r\ndelta epsilon");
    // Double-click "beta", drag forward into "gamma": "beta gamma".
    let beta = t.state.point_at(0, 7);
    t.state.select_word(beta);
    t.state.extend_selection(t.state.point_at(0, 12));
    assert_eq!(copied(&t), "beta gamma");
    // Drag back before the word: from "alpha" to the end of "beta".
    t.state.extend_selection(t.state.point_at(0, 1));
    assert_eq!(copied(&t), "alpha beta");
    // Back inside the word it started on.
    t.state.extend_selection(t.state.point_at(0, 8));
    assert_eq!(copied(&t), "beta");
    // Onto the next line.
    t.state.extend_selection(t.state.point_at(1, 8));
    assert_eq!(copied(&t), "beta gamma\ndelta epsilon");

    // Triple-click then drag: whole lines.
    let p = t.state.point_at(1, 3);
    t.state.select_line(p);
    t.state.extend_selection(t.state.point_at(0, 2));
    assert_eq!(copied(&t), "alpha beta gamma\ndelta epsilon");
}

#[test]
fn dragging_a_plain_selection_moves_its_head() {
    let mut t = Term::new(20, 2, 100);
    t.advance(b"hello world");
    t.state.extend_selection(t.state.point_at(0, 2));
    let sel = t.state.selection().unwrap();
    assert_eq!(
        (sel.anchor, sel.head, sel.kind),
        (sel.head, t.state.point_at(0, 2), SelectionKind::Normal)
    );
    t.state.extend_selection(t.state.point_at(0, 7));
    assert_eq!(copied(&t), "llo wo");
    t.state.extend_selection(t.state.point_at(0, 0));
    assert_eq!(copied(&t), "hel");
}

#[test]
fn select_all_takes_history_and_the_whole_screen() {
    let mut t = Term::new(10, 2, 100);
    t.advance(b"one\r\ntwo\r\nthree\r\nfour");
    assert_eq!(t.state.scrollback_len(), 2);
    t.state.select_all();
    assert_eq!(copied(&t), "one\ntwo\nthree\nfour");
    // Scrolled back, it still ends at the bottom of the screen.
    t.state.scroll_view(2);
    t.state.select_all();
    assert_eq!(copied(&t), "one\ntwo\nthree\nfour");
    // On the alternate screen, just that screen.
    t.advance(b"\x1b[?1049h\x1b[Halt");
    t.state.select_all();
    assert_eq!(copied(&t), "alt\n");
}

#[test]
fn cursor_in_view_follows_the_view() {
    let mut t = Term::new(10, 3, 100);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne");
    assert_eq!(t.state.cursor_in_view(), Some((2, 1)));
    t.state.scroll_view(1);
    assert_eq!(t.state.cursor_in_view(), None, "scrolled off the bottom");
    t.state.scroll_view(-1);
    t.advance(b"\x1b[1;4H");
    t.state.scroll_view(2);
    assert_eq!(t.state.cursor_in_view(), Some((2, 3)));
}

#[test]
fn view_top_names_the_first_row_shown() {
    let mut t = Term::new(4, 2, 100);
    t.advance(b"0\r\nabcdefghij\r\nx");
    assert_eq!(t.state.scrollback_len(), 3);
    assert_eq!(t.state.view_top(), ViewTop::Bottom);
    t.state.scroll_view(1);
    // "efgh" is the second row of the logical line "abcdefghij".
    assert_eq!(
        t.state.view_top(),
        ViewTop::At {
            point: t.state.point_at(0, 0),
            offset: 4
        }
    );
    let top = t.state.view_top();
    t.state.scroll_view(1);
    assert_eq!(
        t.state.view_top(),
        ViewTop::At {
            point: t.state.point_at(0, 0),
            offset: 0
        }
    );
    // Output underneath doesn't move it.
    t.advance(b"\r\nmore\r\nlines");
    assert_eq!(t.state.visible_row(0).text().trim_end(), "abcd");
    t.state.set_view_top(top);
    assert_eq!(t.state.view_top(), top);
    assert_eq!(t.state.visible_row(0).text().trim_end(), "efgh");
    t.state.set_view_top(ViewTop::Bottom);
    assert_eq!(t.state.display_offset(), 0);
}

#[test]
fn reset_keeps_pending_replies() {
    let mut t = Term::new(10, 2, 0);
    t.advance(b"\x1b[5n\x1b[2;2H\x1bc\x1b[6n");
    assert_eq!(t.state.replies(), b"\x1b[0n\x1b[1;1R");
}
