//! Selection, view scrolling and the reply queue: what is copied, how a
//! selection follows its text as output scrolls, how far the view scrolls,
//! and the order replies to queries go back to the program in.

use insta::assert_snapshot;
use term_core::{Selection, Term};

/// Select between two visible (line, col) positions.
fn select(t: &mut Term, anchor: (usize, usize), head: (usize, usize)) {
    let anchor = t.state.visible_point(anchor.0, anchor.1);
    let head = t.state.visible_point(head.0, head.1);
    t.state.selection = Some(Selection { anchor, head });
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
    let anchor = t.state.visible_point(0, 1);
    t.state.scroll_view(-1);
    let head = t.state.visible_point(1, 2);
    t.state.selection = Some(Selection { anchor, head });
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
fn resize_clears_the_selection() {
    let mut t = Term::new(10, 2, 100);
    t.advance(b"hello");
    select(&mut t, (0, 0), (0, 4));
    t.state.resize(10, 2);
    assert!(t.state.selection.is_some(), "same size is not a resize");
    t.state.resize(12, 3);
    assert!(t.state.selection.is_none());
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
    let generation = t.state.generation;
    t.state.scroll_view(-5);
    assert_eq!(t.state.display_offset(), 0);
    assert_eq!(t.state.generation, generation, "no change, no redraw");
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
    let replies = String::from_utf8(t.state.replies.clone()).unwrap();
    assert_eq!(
        replies,
        "\x1b[0n\x1b[2;3R\x1b[?62;22c\x1b]11;rgb:1616/1818/1c1c\x1b\\\x1b[?2026;2$y\x1b[>1;10;0c\x1b[8;2;10t"
    );
}

#[test]
fn reset_keeps_pending_replies() {
    let mut t = Term::new(10, 2, 0);
    t.advance(b"\x1b[5n\x1b[2;2H\x1bc\x1b[6n");
    assert_eq!(t.state.replies, b"\x1b[0n\x1b[1;1R");
}
