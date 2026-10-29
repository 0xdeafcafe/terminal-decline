//! Screen behaviour per feature: printing, wrapping, scrolling, colours,
//! modes, queries, plus regression tests for bugs found in review. Screen
//! state is compared through `Terminal::dump` with insta inline snapshots.

use insta::assert_snapshot;
use term_core::{Attrs, Color, Term, Width};

fn text(t: &Term, line: usize) -> String {
    let row = t.state.visible_row(line);
    let s: String = row.cells().filter(|c| !c.is_spacer()).map(|c| c.ch()).collect();
    s.trim_end().to_string()
}

#[test]
fn prints_and_wraps() {
    let mut t = Term::new(5, 3, 100);
    t.advance(b"abcdefg");
    assert_eq!(text(&t, 0), "abcde");
    assert_eq!(text(&t, 1), "fg");
    assert!(t.state.visible_row(0).wrapped());
    assert_eq!((t.state.cursor().row(), t.state.cursor().col()), (1, 2));
}

#[test]
fn last_column_defers_wrap() {
    let mut t = Term::new(5, 3, 100);
    t.advance(b"abcde\r\n");
    assert_eq!(text(&t, 0), "abcde");
    assert_eq!((t.state.cursor().row(), t.state.cursor().col()), (1, 0));
    assert!(!t.state.visible_row(0).wrapped());
}

#[test]
fn scrolls_into_history_compactly() {
    let mut t = Term::new(80, 2, 100);
    t.advance(b"one\r\ntwo\r\nthree");
    assert_eq!(t.state.scrollback_len(), 1);
    assert_eq!(text(&t, 0), "two");
    assert_eq!(text(&t, 1), "three");
    t.state.scroll_view(1);
    assert_eq!(text(&t, 0), "one");
    // Trailing blanks dropped on the way into scrollback.
    assert_eq!(t.state.visible_row(0).len(), 3);
}

#[test]
fn scrollback_limit() {
    let mut t = Term::new(10, 1, 3);
    for i in 0..10 {
        t.advance(format!("{i}\r\n").as_bytes());
    }
    assert_eq!(t.state.scrollback_len(), 3);
}

#[test]
fn sgr_colours() {
    let mut t = Term::new(10, 1, 0);
    t.advance(b"\x1b[1;31mA\x1b[38;5;200mB\x1b[38;2;1;2;3mC\x1b[38:2::4:5:6mD\x1b[0mE");
    let r = t.state.visible_row(0);
    assert!(r.cell(0).style().attrs.contains(Attrs::BOLD));
    assert_eq!(r.cell(0).style().fg, Color::indexed(1));
    assert_eq!(r.cell(1).style().fg, Color::indexed(200));
    assert_eq!(r.cell(2).style().fg, Color::rgb(1, 2, 3));
    assert_eq!(r.cell(3).style().fg, Color::rgb(4, 5, 6));
    assert_eq!(r.cell(4).style().fg, Color::DEFAULT);
    assert!(!r.cell(4).style().attrs.contains(Attrs::BOLD));
}

#[test]
fn alt_screen_restores_primary() {
    let mut t = Term::new(10, 2, 10);
    t.advance(b"hello\x1b[?1049h\x1b[Hvim");
    assert_eq!(text(&t, 0), "vim");
    t.advance(b"\x1b[?1049l");
    assert_eq!(text(&t, 0), "hello");
    assert_eq!(t.state.cursor().col(), 5);
}

#[test]
fn erase_and_cursor_report() {
    let mut t = Term::new(10, 3, 0);
    t.advance(b"abcdef\x1b[3D\x1b[K\x1b[6n");
    assert_eq!(text(&t, 0), "abc");
    assert_eq!(t.state.replies(), b"\x1b[1;4R");
}

#[test]
fn wide_chars() {
    let mut t = Term::new(5, 2, 0);
    t.advance("ab漢字".as_bytes());
    assert_eq!(text(&t, 0), "ab漢");
    assert_eq!(text(&t, 1), "字");
    let r = t.state.visible_row(0);
    assert!(r.cell(2).width() == Width::Wide);
    assert!(r.cell(3).is_spacer());
}

#[test]
fn scroll_region_insert_delete_lines() {
    let mut t = Term::new(5, 4, 10);
    t.advance(b"1\r\n2\r\n3\r\n4\x1b[2;3r\x1b[2H\x1b[M");
    assert_eq!((0..4).map(|l| text(&t, l)).collect::<Vec<_>>(), ["1", "3", "", "4"]);
    assert_eq!(t.state.scrollback_len(), 0);
}

#[test]
fn line_drawing_charset() {
    let mut t = Term::new(5, 1, 0);
    t.advance(b"\x1b(0lqk\x1b(Bq");
    assert_eq!(text(&t, 0), "┌─┐q");
}

#[test]
fn selection_text_joins_wrapped_lines() {
    let mut t = Term::new(4, 3, 10);
    t.advance(b"abcdef\r\nxy");
    let a = t.state.visible_point(0, 0);
    let b = t.state.visible_point(2, 3);
    t.state.set_selection(Some(term_core::Selection { anchor: a, head: b }));
    assert_eq!(t.state.selection_text().unwrap(), "abcdef\nxy");
}

#[test]
fn origin_mode_restore_and_report() {
    let mut t = Term::new(80, 24, 0);
    t.advance(b"\x1b[?6h\x1b7\x1b[5;20r\x1b8\x1b[6n");
    assert_eq!(t.state.replies(), b"\x1b[1;1R");
}

#[test]
fn resize_in_alt_screen_keeps_shell_lines() {
    let mut t = Term::new(10, 6, 100);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne\r\n$ ");
    t.advance(b"\x1b[?1049h");
    t.state.resize(10, 3);
    t.advance(b"\x1b[?1049l");
    assert_eq!(text(&t, 2), "$");
    assert_eq!(text(&t, 1), "e");
    assert_eq!(t.state.scrollback_len(), 3);
}

#[test]
fn scrolled_view_stays_put_with_full_history() {
    let mut t = Term::new(10, 3, 5);
    for i in 0..20 {
        t.advance(format!("L{i}\r\n").as_bytes());
    }
    t.state.scroll_view(2);
    let before = text(&t, 0);
    t.advance(b"more\r\n");
    assert_eq!(text(&t, 0), before);
}

#[test]
fn insert_char_does_not_split_wide() {
    let mut t = Term::new(6, 1, 0);
    t.advance("界".as_bytes());
    t.advance(b"\x1b[2G\x1b[@");
    let r = t.state.visible_row(0);
    for (i, c) in r.cells().enumerate() {
        if c.is_spacer() {
            assert!(i > 0 && r.cell(i - 1).width() == Width::Wide);
        }
    }
}

#[test]
fn g1_line_drawing() {
    let mut t = Term::new(5, 1, 0);
    t.advance(b"\x1b)0\x0eq\x0fq");
    assert_eq!(text(&t, 0), "─q");
}

#[test]
fn soft_reset_keeps_screen() {
    let mut t = Term::new(5, 2, 10);
    t.advance(b"hi\x1b[1m\x1b[!p");
    assert_eq!(text(&t, 0), "hi");
    assert!(!t.state.cursor().pen().attrs.contains(Attrs::BOLD));
}

#[test]
fn erase_clears_pending_wrap() {
    let mut t = Term::new(10, 2, 0);
    t.advance(b"0123456789\x1b[KX");
    assert_eq!(text(&t, 0), "012345678X");
}

#[test]
fn queries() {
    let mut t = Term::new(10, 2, 0);
    t.advance(b"\x1b[?2026$p\x1b]11;?\x1b\\\x1b[?u");
    let r = String::from_utf8(t.state.replies().to_vec()).unwrap();
    assert!(r.contains("\x1b[?2026;2$y"), "{r:?}");
    assert!(r.contains("\x1b]11;rgb:1616/1818/1c1c\x1b\\"), "{r:?}");
    assert!(r.contains("\x1b[?0u"), "{r:?}");
}

#[test]
fn kitty_keyboard_stack() {
    let mut t = Term::new(10, 2, 0);
    t.advance(b"\x1b[>1u");
    assert_eq!(t.state.kitty_keyboard_flags(), 1);
    t.advance(b"\x1b[<u");
    assert_eq!(t.state.kitty_keyboard_flags(), 0);
}

#[test]
fn vs16_widens_emoji() {
    let mut t = Term::new(10, 1, 0);
    t.advance("⚠\u{FE0F}x".as_bytes());
    assert!(t.state.visible_row(0).cell(0).width() == Width::Wide);
    assert_eq!(t.state.visible_row(0).cell(2).ch(), 'x');
}

#[test]
fn osc_clipboard_and_cwd() {
    let mut t = Term::new(10, 1, 0);
    t.advance(b"\x1b]52;c;aGVsbG8=\x07\x1b]7;file://host/Users/a%20b\x07");
    assert_eq!(t.state.clipboard(), Some("hello"));
    assert_eq!(t.state.cwd(), Some("/Users/a b"));
}

#[test]
fn dump_shows_wraps_trailing_spaces_and_cursor() {
    let mut t = Term::new(6, 4, 10);
    t.advance("ab  \r\n漢x wrapped\r\n\x1b[41m \x1b[0m".as_bytes());
    assert_snapshot!(t.state.dump(), @r"
    ab··|
    漢x wr⏎
    apped|
    ·|
    cursor=(3,1) alt=false offset=0
    ");
}

#[test]
fn dump_follows_view_and_alt_screen() {
    let mut t = Term::new(4, 2, 10);
    t.advance(b"1\r\n2\r\n3");
    t.state.scroll_view(1);
    assert_snapshot!(t.state.dump(), @r"
    1|
    2|
    cursor=(1,1) alt=false offset=1
    ");
    t.advance(b"\x1b[?1049h\x1b[Hvi");
    assert_snapshot!(t.state.dump(), @r"
    vi|
    |
    cursor=(0,2) alt=true offset=0
    ");
}

#[test]
fn vs16_does_not_break_the_next_wide_char() {
    // Regression: widening ⚠ wrote its spacer over the left half of 漢,
    // leaving 漢's spacer without its wide cell.
    let mut t = Term::new(6, 1, 0);
    t.advance("a漢\r⚠\u{FE0F}".as_bytes());
    assert_snapshot!(t.state.dump(), @r"
    ⚠|
    cursor=(0,2) alt=false offset=0
    ");
}

#[test]
fn utf8_split_across_chunks_keeps_the_text_after_it() {
    // Regression (vte 0.15, fixed in vendor/vte): completing a split
    // character next to an invalid byte dropped the valid text in between.
    let cases: &[&[&[u8]]] = &[
        &[&[0xC3], &[0xA9, b'a', 0xFF]],
        &[&[0xCC], &[0x81, b'0', 0xA9]],
        &[&[0xE6, 0xBC], &[0xA2, b'a', 0xFF]],
        &[&[0xE6], &[0xBC], &[0xA2, b'a', 0xFF]],
        &[&[0xF0, 0x9F], &[0x98], &[], &[0x80, b'b', 0xC3]],
    ];
    for chunks in cases {
        let mut whole = Term::new(10, 1, 0);
        whole.advance(&chunks.concat());
        let mut split = Term::new(10, 1, 0);
        for c in *chunks {
            split.advance(c);
        }
        assert_eq!(split.state.dump(), whole.state.dump(), "{chunks:?}");
    }
    let mut t = Term::new(10, 1, 0);
    t.advance(&[0xC3]);
    t.advance(&[0xA9, b'a', 0xFF]);
    assert_snapshot!(t.state.dump(), @r"
    éa�|
    cursor=(0,3) alt=false offset=0
    ");
}

fn select(t: &mut Term, from: (usize, usize), to: (usize, usize)) {
    let anchor = t.state.visible_point(from.0, from.1);
    let head = t.state.visible_point(to.0, to.1);
    t.state.set_selection(Some(term_core::Selection { anchor, head }));
}

#[test]
fn selection_without_scrollback_does_not_slide_to_the_next_line() {
    // Regression: with scrollback off, line numbers only counted stored
    // lines, so after a scroll the selection pointed at the next line.
    let mut t = Term::new(10, 2, 0);
    t.advance(b"aaa\r\nbbb");
    select(&mut t, (0, 0), (0, 2));
    assert_eq!(t.state.selection_text().unwrap(), "aaa");
    t.advance(b"\r\nccc");
    assert_eq!(t.state.selection_text().unwrap(), "");
    // A selection over both rows keeps only the part still on screen.
    select(&mut t, (0, 0), (1, 2));
    t.advance(b"\r\nddd");
    assert_eq!(t.state.selection_text().unwrap(), "ccc");
}

#[test]
fn selection_on_alt_screen_does_not_slide_to_the_next_line() {
    let mut t = Term::new(10, 2, 100);
    t.advance(b"shell\x1b[?1049h\x1b[Haaa\r\nbbb");
    select(&mut t, (0, 0), (0, 2));
    assert_eq!(t.state.selection_text().unwrap(), "aaa");
    t.advance(b"\r\nccc");
    assert_eq!(t.state.selection_text().unwrap(), "");
    assert_eq!(t.state.scrollback_len(), 0);
}

#[test]
fn selection_after_reset_and_alt_screen_round_trip() {
    let mut t = Term::new(10, 2, 100);
    t.advance(b"one\r\ntwo\r\nthree\x1bc");
    // A full reset clears scrollback, like xterm.
    assert_eq!(t.state.scrollback_len(), 0);
    t.advance(b"\x1b[?1049h\x1b[?1049lnew");
    select(&mut t, (0, 0), (0, 2));
    assert_eq!(t.state.selection_text().unwrap(), "new");
    t.advance(b"\r\nmore\r\nlast");
    t.state.scroll_view(1);
    select(&mut t, (0, 0), (1, 3));
    assert_eq!(t.state.selection_text().unwrap(), "new\nmore");
}

#[test]
fn region_scrolls_after_the_screen_ring_has_turned() {
    // The screen is a ring of rows that turns as output scrolls. Scrolling a
    // region, inserting and deleting lines and reverse index must work the
    // same whether or not it has turned: build the same screen by scrolling
    // (turned) and by cursor moves (not turned), then edit both alike.
    let edits: &[&[u8]] = &[
        b"\x1b[2;4r\x1b[2HM\x1b[2;1H\x1bM\x1bMRI",
        b"\x1b[2;4r\x1b[3H\x1b[L\x1b[2L+",
        b"\x1b[2;4r\x1b[2H\x1b[M\x1b[4H\x1b[3M-",
        b"\x1b[2;5r\x1b[5H\n\n\x1b[S\x1b[T\x1b[2T",
        b"\x1b[r\x1b[5H\n\n\x1bM\x1b[L\x1b[M",
    ];
    for (i, edit) in edits.iter().enumerate() {
        let mut turned = Term::new(8, 5, 0);
        turned.advance(b"line0");
        for n in 1..17 {
            turned.advance(format!("\r\nline{n}").as_bytes());
        }
        let mut fresh = Term::new(8, 5, 0);
        for (row, n) in (12..17).enumerate() {
            fresh.advance(format!("\x1b[{};1Hline{n}", row + 1).as_bytes());
        }
        assert_eq!(turned.state.dump(), fresh.state.dump(), "same start");
        turned.advance(edit);
        fresh.advance(edit);
        assert_eq!(turned.state.dump(), fresh.state.dump(), "edit {i}");
    }
}

#[test]
fn vs16_next_to_a_narrow_char_overwrites_it() {
    let mut t = Term::new(6, 1, 0);
    t.advance("abc\r⚠\u{FE0F}".as_bytes());
    assert_snapshot!(t.state.dump(), @r"
    ⚠c|
    cursor=(0,2) alt=false offset=0
    ");
}
