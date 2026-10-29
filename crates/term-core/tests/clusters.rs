//! Grapheme clusters on the screen (§6 of the cells design): what each
//! operation does to cluster cells, the width rule's changes in place,
//! mode 2027 and legacy mode, REP by value, and clusters in selection
//! text and `dump()`. The ambiguous-width setting is in tests/unicode.rs.

use term_core::{Content, Point, Selection, Term, Width};

fn term(cols: usize, lines: usize, input: &str) -> Term {
    let mut t = Term::new(cols, lines, 100);
    t.advance(input.as_bytes());
    t
}

/// Row `line`'s cells as (text, width), spacers included.
fn cells(t: &Term, line: usize) -> Vec<(String, Width)> {
    let row = t.state.visible_row(line);
    row.cells()
        .map(|c| {
            let mut s = String::new();
            if c.content() != Content::Empty {
                c.push_text(&mut s);
            }
            (s, c.width())
        })
        .collect()
}

fn cell(t: &Term, line: usize, col: usize) -> (String, Width) {
    cells(t, line).swap_remove(col)
}

fn cursor(t: &Term) -> (usize, usize, bool) {
    let c = t.state.cursor();
    (c.row(), c.col(), c.pending_wrap())
}

fn select_all_text(t: &mut Term) -> String {
    t.state.select_all();
    t.state.selection_text().unwrap().trim_end_matches('\n').to_string()
}

#[test]
fn a_combining_mark_joins_the_cell_before() {
    let t = term(10, 2, "cafe\u{301}!");
    assert_eq!(cell(&t, 0, 3), ("e\u{301}".into(), Width::Narrow));
    assert_eq!(cell(&t, 0, 4), ("!".into(), Width::Narrow));
    assert_eq!(cursor(&t), (0, 5, false));
    assert!(matches!(t.state.visible_row(0).cell(3).content(), Content::Cluster("e\u{301}")));
    assert_eq!(t.state.visible_row(0).cell(3).ch(), 'e');
    assert!(t.state.dump().starts_with("cafe\u{301}!|"));
}

#[test]
fn widths_follow_the_rule() {
    // (input, cells it takes)
    for (s, w) in [
        ("❤\u{FE0F}", 2),
        ("👍\u{1F3FD}", 2),
        ("🇬🇧", 2),
        ("1\u{FE0F}\u{20E3}", 2),
        ("1\u{20E3}", 2),
        ("1\u{FE0F}", 1),
        ("a\u{FE0F}", 1),
        ("⌚\u{FE0E}", 1),
        ("👨\u{200D}👩\u{200D}👧", 2),
        ("❤\u{200D}🔥", 2),
        ("\u{0915}\u{094D}\u{0937}", 1),
        ("\u{0E01}\u{0E33}", 1),
        ("\u{0600}1", 1),
        ("\u{1100}\u{1161}\u{11A8}", 2),
    ] {
        let t = term(10, 1, s);
        assert_eq!(cursor(&t).1, w, "{s:?}");
        let expected = if w == 2 { Width::Wide } else { Width::Narrow };
        assert_eq!(cell(&t, 0, 0), (s.to_string(), expected), "{s:?}");
        t.state.check_invariants().unwrap();
    }
}

#[test]
fn a_keycap_widens_only_when_u20e3_arrives() {
    let mut t = term(10, 1, "1\u{FE0F}");
    assert_eq!(cursor(&t), (0, 1, false));
    t.advance("\u{20E3}x".as_bytes());
    assert_eq!(
        cells(&t, 0)[..3],
        [
            ("1\u{FE0F}\u{20E3}".into(), Width::Wide),
            (String::new(), Width::Spacer),
            ("x".into(), Width::Narrow)
        ]
    );
}

#[test]
fn widening_takes_the_next_cell_and_erases_a_wide_character_there_whole() {
    let t = term(6, 1, "a漢\r❤\u{FE0F}");
    assert_eq!(
        cells(&t, 0)[..3],
        [
            ("❤\u{FE0F}".into(), Width::Wide),
            (String::new(), Width::Spacer),
            (String::new(), Width::Narrow)
        ]
    );
    assert_eq!(cursor(&t), (0, 2, false));
}

#[test]
fn vs15_narrows_a_wide_cell_and_moves_the_cursor_back() {
    let t = term(6, 1, "⌚\u{FE0E}x");
    assert_eq!(
        cells(&t, 0)[..2],
        [("⌚\u{FE0E}".into(), Width::Narrow), ("x".into(), Width::Narrow)]
    );
    assert_eq!(cursor(&t), (0, 2, false));
}

#[test]
fn widening_in_the_last_column_moves_the_cluster_to_the_next_row() {
    let mut t = term(4, 3, "abc❤\u{FE0F}x");
    assert_eq!(cell(&t, 0, 3), (String::new(), Width::EdgeSpacer));
    assert!(t.state.visible_row(0).wrapped());
    assert_eq!(
        cells(&t, 1)[..3],
        [
            ("❤\u{FE0F}".into(), Width::Wide),
            (String::new(), Width::Spacer),
            ("x".into(), Width::Narrow)
        ]
    );
    assert_eq!(cursor(&t), (1, 3, false));
    // The edge spacer isn't text: the line reads straight on.
    assert_eq!(select_all_text(&mut t), "abc❤\u{FE0F}x");
}

#[test]
fn widening_in_the_last_column_with_autowrap_off_stays_narrow() {
    let t = term(4, 2, "\x1b[?7labc❤\u{FE0F}");
    assert_eq!(cell(&t, 0, 3), ("❤\u{FE0F}".into(), Width::Narrow));
    assert_eq!(cursor(&t), (0, 3, false));
    assert!(!t.state.visible_row(0).wrapped());
}

#[test]
fn a_wide_character_that_doesnt_fit_leaves_an_edge_spacer() {
    let mut t = term(4, 2, "abc漢");
    assert_eq!(cell(&t, 0, 3), (String::new(), Width::EdgeSpacer));
    assert!(t.state.visible_row(0).wrapped());
    assert_eq!(select_all_text(&mut t), "abc漢");
    // Deleting a character moves the edge spacer left: it is blanked.
    t.advance(b"\x1b[1;1H\x1b[P");
    assert_eq!(
        cells(&t, 0),
        [
            ("b".into(), Width::Narrow),
            ("c".into(), Width::Narrow),
            (String::new(), Width::Narrow),
            (String::new(), Width::Narrow)
        ]
    );
    // Growing the row blanks one that is no longer in the last column.
    let mut t = term(4, 2, "abc漢");
    t.state.resize(6, 2);
    assert_eq!(cell(&t, 0, 3), (String::new(), Width::Narrow));
    t.state.check_invariants().unwrap();
}

#[test]
fn overwriting_and_erasing_free_the_slot() {
    // Every step is checked by `check_invariants` in debug builds: each
    // live slot is held by exactly one cell.
    let mut t = term(8, 2, "e\u{301}a\u{301}👍\u{1F3FD}");
    t.advance(b"\rX");
    assert_eq!(cell(&t, 0, 0), ("X".into(), Width::Narrow));
    t.advance(b"\x1b[2X");
    t.advance(b"\x1b[K");
    t.advance("\r\x1b[2C🇬🇧\x1b[1;1H\x1b[2P".as_bytes());
    assert_eq!(cell(&t, 0, 0), ("🇬🇧".into(), Width::Wide));
    t.advance(b"\x1b[3@");
    assert_eq!(cell(&t, 0, 3), ("🇬🇧".into(), Width::Wide));
    t.advance(b"\x1b#8");
    assert_eq!(cell(&t, 0, 3), ("E".into(), Width::Narrow));
}

#[test]
fn inserting_and_deleting_keep_clusters_with_their_cells() {
    let mut t = term(8, 1, "ae\u{301}b");
    t.advance(b"\r\x1b[2@");
    assert_eq!(cell(&t, 0, 3), ("e\u{301}".into(), Width::Narrow));
    t.advance(b"\x1b[3P");
    assert_eq!(cell(&t, 0, 0), ("e\u{301}".into(), Width::Narrow));
    // Cells pushed off the end free their slots.
    t.advance("\x1b[1;8H👍\u{1F3FD}".as_bytes());
    t.advance(b"\x1b[1;1H\x1b[8@");
    assert!(cells(&t, 0).iter().all(|(s, _)| s.is_empty()));
}

#[test]
fn rep_repeats_the_whole_cluster_by_value() {
    let mut t = term(12, 2, "👍\u{1F3FD}\x1b[2b");
    for col in [0, 2, 4] {
        assert_eq!(cell(&t, 0, col), ("👍\u{1F3FD}".into(), Width::Wide), "{col}");
    }
    assert_eq!(cursor(&t), (0, 6, false));
    // After the original is overwritten: still the cluster.
    t.advance("\r\x1b[2Ke\u{301}\x1b[1;1HZZ\x1b[1;5H".as_bytes());
    t.advance("a\u{301}".as_bytes());
    t.advance(b"\x1b[1;1H\x1b[KX\x1b[2b");
    assert_eq!(t.state.dump().lines().next().unwrap(), "XXX|");
    t.advance("\x1b[2;1Ha\u{301}\x1b[1;1H\x1b[K\x1b[2;3H\x1b[2b".as_bytes());
    assert_eq!(t.state.dump().lines().nth(1).unwrap(), "a\u{301} a\u{301}a\u{301}|");
}

#[test]
fn legacy_mode_joins_only_zero_width_scalars() {
    let t = term(12, 1, "\x1b[?2027l❤\u{FE0F}e\u{301}🇬🇧1\u{20E3}⌚\u{FE0E}");
    assert_eq!(
        cells(&t, 0)[..9],
        [
            ("❤\u{FE0F}".into(), Width::Narrow),
            ("e\u{301}".into(), Width::Narrow),
            ("🇬".into(), Width::Narrow),
            ("🇧".into(), Width::Narrow),
            ("1\u{20E3}".into(), Width::Narrow),
            ("⌚\u{FE0E}".into(), Width::Wide),
            (String::new(), Width::Spacer),
            (String::new(), Width::Narrow),
            (String::new(), Width::Narrow),
        ]
    );
}

#[test]
fn decrqm_reports_mode_2027() {
    let mut t = term(10, 1, "\x1b[?2027$p");
    assert_eq!(t.state.take_replies(), b"\x1b[?2027;1$y");
    t.advance(b"\x1b[?2027l\x1b[?2027$p");
    assert_eq!(t.state.take_replies(), b"\x1b[?2027;2$y");
    t.advance(b"\x1b[?2027h\x1b[?2027$p");
    assert_eq!(t.state.take_replies(), b"\x1b[?2027;1$y");
}

#[test]
fn selection_and_dump_show_clusters() {
    let mut t = term(10, 2, "x👨\u{200D}👩\u{200D}👧y\r\n1\u{FE0F}\u{20E3}");
    t.state
        .set_selection(Some(Selection::new(Point { line: 0, col: 1 }, Point { line: 1, col: 1 })));
    assert_eq!(t.state.selection_text().unwrap(), "👨\u{200D}👩\u{200D}👧y\n1\u{FE0F}\u{20E3}");
    let dump = t.state.dump();
    assert!(dump.starts_with("x👨\u{200D}👩\u{200D}👧y|\n1\u{FE0F}\u{20E3}|\n"), "{dump}");
    assert_eq!(t.state.visible_row(0).text().trim_end(), "x👨\u{200D}👩\u{200D}👧y");
}

#[test]
fn history_keeps_whole_clusters() {
    // With mode 2027 on, then ❤ + VS16 written one cell wide with it off:
    // history keeps both widths as written.
    let mut t = term(10, 2, "e\u{301}👍\u{1F3FD}\r\n\x1b[?2027l\u{2764}\u{FE0F}x\r\n\r\n\r\n");
    t.state.scroll_view(3);
    assert_eq!(
        cells(&t, 0)[..3],
        [
            ("e\u{301}".into(), Width::Narrow),
            ("👍\u{1F3FD}".into(), Width::Wide),
            (String::new(), Width::Spacer)
        ]
    );
    assert_eq!(
        cells(&t, 1)[..2],
        [("\u{2764}\u{FE0F}".into(), Width::Narrow), ("x".into(), Width::Narrow)]
    );
}

#[test]
fn resizing_narrower_frees_cut_off_clusters() {
    let mut t = term(10, 1, "abcde\u{301}fg👍\u{1F3FD}");
    t.state.resize(4, 1);
    t.state.check_invariants().unwrap();
    t.state.resize(10, 1);
    t.state.check_invariants().unwrap();
    // A wide cluster in the new last column is blanked.
    let mut t = term(10, 1, "ab👍\u{1F3FD}");
    t.state.resize(3, 1);
    assert_eq!(
        cells(&t, 0),
        [
            ("a".into(), Width::Narrow),
            ("b".into(), Width::Narrow),
            (String::new(), Width::Narrow)
        ]
    );
    t.state.check_invariants().unwrap();
}

#[test]
fn caps_drop_further_marks() {
    let marks = "\u{301}".repeat(40);
    let t = term(10, 1, &format!("a{marks}b"));
    let (text, _) = cell(&t, 0, 0);
    assert_eq!(text.chars().count(), 32);
    assert_eq!(cell(&t, 0, 1), ("b".into(), Width::Narrow));
    // A row holds at most 8 KB of cluster text; past it, cells keep their
    // first scalar.
    let cluster = format!("😀{}", "\u{1F3FD}".repeat(31));
    let t = term(200, 1, &cluster.repeat(100));
    t.state.check_invariants().unwrap();
    let full = cells(&t, 0).iter().filter(|(s, _)| *s == cluster).count();
    assert_eq!(full, 8 * 1024 / 128);
}

#[test]
fn a_mark_after_a_cursor_move_has_no_cell() {
    let t = term(10, 1, "ab\x1b[D\u{301}c");
    assert_eq!(cells(&t, 0)[..2], [("a".into(), Width::Narrow), ("c".into(), Width::Narrow)]);
}
