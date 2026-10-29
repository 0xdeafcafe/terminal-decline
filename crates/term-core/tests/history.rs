//! History at another width (§12.3 of the cells design): stored lines keep
//! their width, and the view shows them laid out at the current one, keeps
//! its place on the same text across width changes, extends `FILL` lines
//! to the new width and maps Points (selection, links, search) through
//! the layout. The property test reads all text through the view at random
//! widths and checks it against the text as stored.

use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};
use term_core::{Color, Point, RowFlags, RowKey, RowRef, Selection, Snapshot, Term, ViewTop, Width};

fn row_text(r: &RowRef) -> String {
    let mut s = String::new();
    for c in r.cells() {
        match c.width() {
            Width::Spacer | Width::EdgeSpacer => {}
            _ => s.push(c.ch()),
        }
    }
    s
}

fn view(t: &Term) -> Vec<String> {
    (0..t.state.lines())
        .map(|l| row_text(&t.state.visible_row(l)).trim_end().to_string())
        .collect()
}

/// A 20-column terminal, two rows, with "0123…j" (20 cells, a whole row)
/// and "x" in history, then shrunk to 10 columns: history isn't touched.
fn shrunk() -> Term {
    let mut t = Term::new(20, 2, 100);
    t.advance(b"0123456789abcdefghij\r\nx\r\ny\r\nz");
    assert_eq!(t.state.scrollback_len(), 2);
    t.state.resize(10, 2);
    assert_eq!(t.state.scrollback_len(), 2, "history isn't rewritten");
    t
}

#[test]
fn a_wide_line_shows_as_two_rows_at_half_the_width() {
    let mut t = shrunk();
    t.state.scroll_view(3);
    assert_eq!(t.state.display_offset(), 3);
    assert_eq!(view(&t), ["0123456789", "abcdefghij"]);
    assert!(t.state.visible_row(0).wrapped());
    assert!(!t.state.visible_row(1).wrapped());
    t.state.scroll_view(-1);
    assert_eq!(view(&t), ["abcdefghij", "x"]);
    t.state.scroll_view(-1);
    assert_eq!(view(&t), ["x", "y"]);
    t.state.scroll_view(-1);
    assert_eq!(view(&t), ["y", "z"]);
    assert_eq!(t.state.view_top(), ViewTop::Bottom);
    // Scrolling past the top stops at the first row.
    t.state.scroll_view(100);
    assert_eq!(view(&t), ["0123456789", "abcdefghij"]);
}

#[test]
fn points_come_from_the_stored_line() {
    let mut t = shrunk();
    t.state.scroll_view(3);
    let first = t.state.point_at(0, 0).line;
    assert_eq!(t.state.point_at(1, 3), Point { line: first, col: 13 });
    assert_eq!(t.state.view_position(Point { line: first, col: 13 }), Some((1, 3)));
    // A selection in stored columns 5..=14 shows on both rows.
    t.state
        .set_selection(Some(Selection::new(Point { line: first, col: 5 }, Point { line: first, col: 14 })));
    assert_eq!(t.state.selected_cols(0), 5..10);
    assert_eq!(t.state.selected_cols(1), 0..5);
    assert_eq!(t.state.selection_text().as_deref(), Some("56789abcde"));
}

#[test]
fn the_view_keeps_its_place_across_width_changes() {
    let mut t = shrunk();
    t.state.scroll_view(2);
    assert_eq!(view(&t), ["abcdefghij", "x"]);
    let top = t.state.view_top();
    let ViewTop::At { point, offset } = top else {
        panic!("scrolled back")
    };
    assert_eq!(point.col, 10);
    assert_eq!(offset, 10);
    // Narrower: the same cell is the start of a row.
    t.state.resize(5, 2);
    assert_eq!(view(&t), ["abcde", "fghij"]);
    // Wider: rounded down to the row it is in.
    t.state.resize(20, 2);
    assert_eq!(view(&t), ["0123456789abcdefghij", "x"]);
    t.state.resize(10, 2);
    assert_eq!(view(&t), ["0123456789", "abcdefghij"]);
}

#[test]
fn output_underneath_leaves_the_view_alone() {
    let mut t = shrunk();
    t.state.scroll_view(2);
    let before = t.state.view_top();
    t.advance(b"\r\nmore\r\nand more");
    assert_eq!(t.state.view_top(), before);
    assert_eq!(view(&t), ["abcdefghij", "x"]);
    assert_eq!(t.state.display_offset(), 4);
}

#[test]
fn fill_lines_reach_the_new_width() {
    let mut t = Term::new(10, 1, 100);
    t.advance(b"\x1b[41mred\x1b[K\x1b[0m\r\nplain");
    t.state.resize(20, 1);
    t.state.scroll_view(1);
    let row = t.state.visible_row(0);
    assert_eq!(row_text(&row).trim_end(), "red");
    assert_eq!(row.len(), 20);
    assert_eq!(row.cell(19).style().bg, Color::indexed(1));
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert!(snap.rows[0].flags.contains(RowFlags::FILL));
}

#[test]
fn a_wrapped_fill_line_fills_its_last_row_at_the_new_width() {
    let mut t = Term::new(4, 1, 100);
    t.advance(b"\x1b[44mabcdef\x1b[K\x1b[0m\r\nz");
    // "abcd" wrapped, "ef" filled: two stored lines at 4 columns.
    assert_eq!(t.state.scrollback_len(), 2);
    t.state.resize(6, 1);
    t.state.scroll_view(isize::MAX / 2);
    let row = t.state.visible_row(0);
    assert_eq!(row_text(&row).trim_end(), "abcdef");
    t.state.resize(10, 1);
    t.state.scroll_view(isize::MAX / 2);
    let row = t.state.visible_row(0);
    assert_eq!(row_text(&row).trim_end(), "abcdef");
    assert_eq!(row.cell(9).style().bg, Color::indexed(4));
}

#[test]
fn snapshot_rows_at_another_width_are_keyed_by_their_first_cell() {
    let mut t = shrunk();
    t.state.scroll_view(3);
    let first = t.state.point_at(0, 0).line;
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.rows[0].key, RowKey::History { line: first, col: 0 });
    assert_eq!(snap.rows[1].key, RowKey::History { line: first, col: 10 });
    assert!(snap.rows[0].flags.contains(RowFlags::WRAPPED));
    assert!(snap.rows[1].flags.contains(RowFlags::HISTORY));
    let cells = snap.rows[1].cells.clone().expect("copied");
    assert_eq!(cells.len(), 10);
    // Selection highlights land on the row's own columns.
    t.state
        .set_selection(Some(Selection::new(Point { line: first, col: 12 }, Point { line: first, col: 13 })));
    t.state.snapshot(&mut snap, |_| false);
    let h = &snap.highlights[snap.rows[1].highlights.start as usize];
    assert_eq!(h.cols, 2..4);
}

#[test]
fn wide_characters_move_whole_to_the_next_row() {
    let mut t = Term::new(6, 1, 100);
    t.advance("ab漢字\r\nz".as_bytes());
    t.state.resize(3, 1);
    t.state.scroll_view(isize::MAX / 2);
    assert_eq!(view(&t), ["ab"]);
    t.state.scroll_view(-1);
    assert_eq!(view(&t), ["漢"]);
    t.state.scroll_view(-1);
    assert_eq!(view(&t), ["字"]);
}

#[test]
fn links_in_history_are_found_at_the_new_columns() {
    let mut t = Term::new(20, 1, 100);
    t.advance(b"0123456789ab\x1b]8;;https://x.test\x1b\\link\x1b]8;;\x1b\\\r\nz");
    t.state.resize(10, 1);
    t.state.scroll_view(1);
    assert_eq!(view(&t), ["ablink"]);
    let p = t.state.point_at(0, 3);
    assert_eq!(p.col, 13);
    assert_eq!(t.state.link_at(p).map(|l| l.uri.to_string()).as_deref(), Some("https://x.test"));
    assert_eq!(t.state.link_at(t.state.point_at(0, 1)), None);
}

fn config() -> Config {
    Config {
        rng_seed: RngSeed::Fixed(0x12_3456),
        failure_persistence: None,
        cases: 96,
        ..Config::default()
    }
}

/// Lines of text: ASCII words, CJK and spaces, of any length.
fn lines() -> impl Strategy<Value = Vec<String>> {
    let piece = prop_oneof![
        "[a-z]{1,12}".prop_map(|s| s),
        Just(" ".to_string()),
        Just("漢字".to_string()),
        Just("é".to_string()),
    ];
    prop::collection::vec(prop::collection::vec(piece, 0..12).prop_map(|p| p.concat()), 1..30)
}

/// Every row of history and the screen read through the view, scrolling
/// a row at a time from the top, joined into logical lines, and the
/// number of history rows read.
fn read_through_view(t: &mut Term) -> (Vec<String>, usize) {
    t.state.scroll_view(isize::MAX / 2);
    let mut rows = Vec::new();
    let mut history = 0;
    let cols = t.state.cols();
    while t.state.display_offset() > 0 {
        let r = t.state.visible_row(0);
        assert!(r.len() <= cols, "a row of {} cells at {} columns", r.len(), cols);
        rows.push((row_text(&r), r.wrapped()));
        history += 1;
        t.state.scroll_view(-1);
    }
    for l in 0..t.state.lines() {
        let r = t.state.visible_row(l);
        rows.push((row_text(&r), r.wrapped()));
    }
    let mut out = Vec::new();
    let mut line = String::new();
    for (text, wrapped) in rows {
        line.push_str(&text);
        if !wrapped {
            out.push(line.trim_end().to_string());
            line.clear();
        }
    }
    out.push(line.trim_end().to_string());
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    (out, history)
}

proptest! {
    #![proptest_config(config())]

    /// History written at one width and read at another holds the same
    /// text, line for line, in rows no wider than the view; the view's
    /// count of rows back agrees with the rows read; and scrolling there
    /// and back lands on the same rows.
    #[test]
    fn history_reads_the_same_at_any_width(
        text in lines(),
        written in 2usize..=30,
        widths in prop::collection::vec(2usize..=40, 1..4),
    ) {
        let mut t = Term::new(written, 2, 10_000);
        let input = text.join("\r\n");
        t.advance(input.as_bytes());
        t.advance(b"\r\n\r\n\r\n");
        let (want, _) = read_through_view(&mut t);
        let stored: Vec<String> = text.iter().map(|l| l.trim_end().to_string()).collect();
        prop_assert_eq!(&want, &stored.iter().rev().skip_while(|l| l.is_empty()).collect::<Vec<_>>().into_iter().rev().cloned().collect::<Vec<_>>());
        for &w in &widths {
            t.state.resize(w, 2);
            t.state.check_invariants().map_err(TestCaseError::fail)?;
            let (got, history) = read_through_view(&mut t);
            prop_assert_eq!(&got, &want, "at {} columns", w);
            t.state.scroll_view(isize::MAX / 2);
            prop_assert_eq!(t.state.display_offset(), history);
            // Down k rows and back up k: the same place.
            let top = t.state.view_top();
            let k = history / 2;
            t.state.scroll_view(-(k as isize));
            prop_assert_eq!(t.state.display_offset(), history - k);
            t.state.scroll_view(k as isize);
            prop_assert_eq!(t.state.view_top(), top);
            t.state.scroll_view_to_bottom();
        }
    }
}
