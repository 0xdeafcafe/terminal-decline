//! OSC 8 hyperlinks and OSC 133 prompt marks (§5.5, §11.6 of the cells
//! design): what cells get a link, which pieces share one, who holds a
//! link through sweeps and saved cursors, the hovered link in the
//! snapshot, and the marks shells leave for jumping between prompts and
//! selecting a command's output.

use term_core::{HighlightKind, LinkValue, Mark, MarkKind, Point, Snapshot, Term};

fn link(uri: &str, id: Option<&str>) -> Option<LinkValue> {
    Some(LinkValue {
        uri: uri.into(),
        id: id.map(Into::into),
    })
}

/// The link at a visible (line, col).
fn at(t: &Term, line: usize, col: usize) -> Option<LinkValue> {
    t.state.link_at(t.state.point_at(line, col))
}

fn check(t: &Term) {
    t.state.check_invariants().unwrap();
}

#[test]
fn osc8_links_the_cells_printed_while_it_is_set() {
    let mut t = Term::new(30, 3, 100);
    t.advance(b"a\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ b");
    assert_eq!(at(&t, 0, 0), None);
    for col in 1..5 {
        assert_eq!(at(&t, 0, col), link("https://example.com", None), "col {col}");
    }
    assert_eq!(at(&t, 0, 5), None);
    assert_eq!(at(&t, 0, 6), None);
    assert_eq!(t.state.pen_link(), None);
    // BEL ends the string too; the `id=` parameter comes with the link.
    t.advance(b"\r\n\x1b]8;id=x;file:///tmp\x07f");
    assert_eq!(at(&t, 1, 0), link("file:///tmp", Some("x")));
    assert_eq!(t.state.pen_link(), link("file:///tmp", Some("x")).as_ref());
    check(&t);
}

#[test]
fn a_uri_may_hold_semicolons() {
    let mut t = Term::new(30, 2, 0);
    t.advance(b"\x1b]8;;https://x/?a=1;b=2;c\x1b\\x");
    assert_eq!(at(&t, 0, 0), link("https://x/?a=1;b=2;c", None));
    // Other parameters than `id` are skipped; an empty id is none.
    t.advance(b"\x1b]8;foo=1:id=7:bar=2;u\x1b\\y\x1b]8;id=;v\x1b\\z");
    assert_eq!(at(&t, 0, 1), link("u", Some("7")));
    assert_eq!(at(&t, 0, 2), link("v", None));
}

#[test]
fn links_over_the_limits_are_ignored() {
    let mut t = Term::new(10, 2, 0);
    let long = "a".repeat(4097);
    t.advance(format!("\x1b]8;;https://ok\x1b\\a\x1b]8;;{long}\x1b\\b").as_bytes());
    assert_eq!(at(&t, 0, 0), link("https://ok", None));
    assert_eq!(at(&t, 0, 1), None, "a URI over 4 KB ends the link");
    let id = "i".repeat(257);
    t.advance(format!("\x1b]8;;https://ok\x1b\\c\x1b]8;id={id};u\x1b\\d").as_bytes());
    assert_eq!(at(&t, 0, 3), None, "an id over 256 bytes");
    // 16 parts: vte's most, so the string may have been cut short.
    t.advance(format!("\x1b]8;;https://ok\x1b\\e\x1b]8;;u{}\x1b\\f", ";".repeat(13)).as_bytes());
    assert_eq!(at(&t, 0, 5), None);
}

#[test]
fn sgr_leaves_the_link_alone_and_erasing_drops_it() {
    let mut t = Term::new(20, 2, 0);
    t.advance(b"\x1b]8;;https://e\x1b\\\x1b[1;31ma\x1b[0mb\x1b[44mc");
    for col in 0..3 {
        assert_eq!(at(&t, 0, col), link("https://e", None), "col {col}");
    }
    assert!(t.state.visible_row(0).cell(0).style().attrs.contains(term_core::Attrs::BOLD));
    // Erased cells take the erase style, which has no link.
    t.advance(b"\x1b[2D\x1b[K");
    assert_eq!(at(&t, 0, 0), link("https://e", None));
    assert_eq!(at(&t, 0, 1), None);
    // ECH and DCH blanks too.
    t.advance(b"\rxyz\x1b[3D\x1b[X\x1b[P");
    assert_eq!(at(&t, 0, 0), link("https://e", None));
    assert_eq!(at(&t, 0, 2), None);
    check(&t);
}

#[test]
fn both_halves_of_a_wide_character_have_its_link() {
    let mut t = Term::new(10, 2, 0);
    t.advance("\x1b]8;;https://w\x1b\\漢\x1b]8;;\x1b\\".as_bytes());
    assert_eq!(at(&t, 0, 0), link("https://w", None));
    assert_eq!(at(&t, 0, 1), link("https://w", None));
    assert_eq!(at(&t, 0, 2), None);
}

#[test]
fn pieces_of_one_link_share_an_id() {
    let mut t = Term::new(10, 3, 0);
    // The same id and URI in two colours, across a wrap: one link.
    t.advance(b"\x1b]8;id=1;https://p\x1b\\\x1b[31mabcdefgh\x1b[32mijkl\x1b]8;;\x1b\\");
    // The same URI with another id is another link.
    t.advance(b"\x1b]8;id=2;https://p\x1b\\m\x1b]8;;\x1b\\");
    let id_of = |line: usize, col: usize| t.state.visible_row(line).cell(col).style().link;
    assert_ne!(id_of(0, 0), 0);
    assert_eq!(id_of(0, 0), id_of(0, 9));
    assert_eq!(id_of(0, 0), id_of(1, 1));
    assert_ne!(id_of(0, 0), id_of(1, 2));
    assert_eq!(t.state.style_stats().0.links, 2);
}

#[test]
fn hovered_link_is_highlighted_in_every_piece() {
    let mut t = Term::new(10, 4, 0);
    t.advance(b"\x1b]8;id=1;https://p\x1b\\abc\x1b]8;;\x1b\\ x \x1b]8;id=1;https://p\x1b\\de\x1b]8;;\x1b\\");
    t.advance(b"\r\n\x1b]8;id=1;https://p\x1b\\\x1b[1mfg\x1b]8;;\x1b\\");
    t.advance(b"\r\n\x1b]8;;https://p\x1b\\other\x1b]8;;\x1b\\");
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert!(snap.highlights.is_empty());

    let generation = t.state.generation();
    let seqs = t.state.row_seqs();
    t.state.set_hover(Some(t.state.point_at(1, 1)));
    assert_ne!(t.state.generation(), generation, "a redraw is due");
    assert_eq!(t.state.row_seqs(), seqs, "hovering changes no row: it is an overlay");
    t.state
        .set_selection(Some(term_core::Selection::new(t.state.point_at(0, 4), t.state.point_at(0, 5))));
    t.state.snapshot(&mut snap, |_| true);
    let rows: Vec<Vec<(std::ops::Range<u16>, HighlightKind)>> = snap
        .rows
        .iter()
        .map(|r| snap.row_highlights(r).iter().map(|h| (h.cols.clone(), h.kind)).collect())
        .collect();
    use HighlightKind::{HoveredLink, Selection};
    assert_eq!(
        rows,
        vec![
            vec![(0..3, HoveredLink), (4..6, Selection), (6..8, HoveredLink)],
            vec![(0..2, HoveredLink)],
            vec![],
            vec![],
        ],
        "left to right; the link without an id is another link"
    );

    // Off the link, nothing is highlighted.
    t.state.set_selection(None);
    t.state.set_hover(Some(t.state.point_at(0, 4)));
    t.state.snapshot(&mut snap, |_| true);
    assert!(snap.highlights.is_empty());
    t.state.set_hover(None);
    assert_eq!(t.state.hover(), None);
}

#[test]
fn plain_urls_are_found_across_wraps_and_hovered_like_links() {
    let mut t = Term::new(12, 4, 10);
    // Soft-wrapped onto a second row, then a full stop that isn't part of it.
    t.advance(b"go https://example.com/x. ok");
    let url = |t: &Term, line, col| t.state.url_at(t.state.point_at(line, col));
    let (a, b, u) = url(&t, 1, 3).unwrap();
    assert_eq!(u, "https://example.com/x");
    assert_eq!((a, b), (t.state.point_at(0, 3), t.state.point_at(1, 11)));
    assert_eq!(url(&t, 0, 4).map(|x| x.2), Some(u.clone()), "from either row");
    assert_eq!(url(&t, 0, 1), None, "not on the URL");
    assert_eq!(url(&t, 2, 0), None);

    let mut snap = Snapshot::default();
    t.state.set_hover(Some(t.state.point_at(1, 0)));
    t.state.snapshot(&mut snap, |_| true);
    let rows: Vec<Vec<(std::ops::Range<u16>, HighlightKind)>> = snap
        .rows
        .iter()
        .map(|r| snap.row_highlights(r).iter().map(|h| (h.cols.clone(), h.kind)).collect())
        .collect();
    use HighlightKind::HoveredLink;
    assert_eq!(rows, vec![vec![(3..12, HoveredLink)], vec![(0..12, HoveredLink)], vec![], vec![]]);

    // An OSC 8 link wins over the URL text it shows.
    let mut t = Term::new(40, 2, 0);
    t.advance(b"\x1b]8;;https://real\x1b\\https://shown\x1b]8;;\x1b\\");
    t.state.set_hover(Some(t.state.point_at(0, 2)));
    t.state.snapshot(&mut snap, |_| true);
    assert_eq!(snap.row_highlights(&snap.rows[0]).len(), 1);
    assert_eq!(snap.row_highlights(&snap.rows[0])[0].cols, 0..13);

    // History rows are text too.
    let mut t = Term::new(20, 2, 10);
    t.advance(b"see http://old.ex\r\n\r\nnew");
    t.state.scroll_view(1);
    assert_eq!(url(&t, 0, 6).map(|x| x.2).as_deref(), Some("http://old.ex"));
}

#[test]
fn a_saved_cursor_holds_its_link_by_value() {
    let mut t = Term::new(20, 3, 0);
    t.advance(b"\x1b]8;;https://saved\x1b\\\x1b7\x1b]8;;\x1b\\a\x1b8b");
    assert_eq!(at(&t, 0, 0), link("https://saved", None), "DECRC brought the link back");
    // Across the alternate screen (1049), whose table is another one: the
    // link is interned there, and again on the way back.
    t.advance(b"\r\n\x1b]8;;https://primary\x1b\\\x1b[?1049h");
    assert_eq!(t.state.pen_link(), link("https://primary", None).as_ref(), "the pen keeps it");
    t.advance(b"\x1b[Halt\x1b]8;;https://alt\x1b\\x");
    assert_eq!(at(&t, 0, 0), link("https://primary", None));
    assert_eq!(at(&t, 0, 3), link("https://alt", None));
    t.advance(b"\x1b[?1049l");
    assert_eq!(t.state.pen_link(), link("https://primary", None).as_ref());
    t.advance(b"c");
    assert_eq!(at(&t, 1, 0), link("https://primary", None));
    // Restoring with nothing saved gives no link.
    t.advance(b"\x1b[?1049h\x1b8d");
    assert_eq!(t.state.pen_link(), None);
    check(&t);
}

#[test]
fn resets_end_the_link() {
    let mut t = Term::new(20, 2, 0);
    t.advance(b"\x1b]8;;https://a\x1b\\\x1b[!p");
    assert_eq!(t.state.pen_link(), None, "DECSTR");
    t.advance(b"\x1b]8;;https://a\x1b\\x\x1bc");
    assert_eq!(t.state.pen_link(), None, "RIS");
    assert_eq!(at(&t, 0, 0), None);
    assert_eq!(t.state.style_stats().0.links, 0);
}

#[test]
fn sweeps_free_links_no_style_uses() {
    let mut t = Term::new(10, 3, 0);
    t.state.set_sweep_at(Some(8));
    for i in 0..200 {
        t.advance(format!("\x1b]8;;https://n/{i}\x1b\\\x1b[3{}mx\x1b]8;;\x1b\\\r", i % 8).as_bytes());
        check(&t);
    }
    // One cell still shows a link; the table holds about what the screen uses.
    assert_eq!(at(&t, 0, 0), link("https://n/199", None));
    let (s, _) = t.state.style_stats();
    assert!(s.links <= 10, "{s:?}");
    t.state.sweep_styles_now().unwrap();
    assert_eq!(t.state.style_stats().0.links, 1);
}

#[test]
fn the_pens_link_survives_a_sweep_before_it_is_printed() {
    let mut t = Term::new(10, 3, 0);
    t.state.set_sweep_at(Some(8));
    // Interned at the first print, then many new styles force sweeps
    // while the pen holds it and no cell does.
    t.advance(b"\x1b]8;;https://keep\x1b\\a\x1b[K");
    for i in 0..40 {
        t.advance(format!("\x1b[38;5;{i}mx\x1b[D").as_bytes());
        check(&t);
    }
    t.state.sweep_styles_now().unwrap();
    t.advance(b"\x1b[0mz");
    assert_eq!(at(&t, 0, 0), link("https://keep", None));
    check(&t);
}

#[test]
fn clearing_the_alternate_screen_frees_its_links() {
    let mut t = Term::new(10, 3, 0);
    t.advance(b"\x1b[?1049h\x1b]8;;https://alt\x1b\\abc\x1b]8;;\x1b\\");
    assert_eq!(t.state.style_stats().0.links, 1);
    t.advance(b"\x1b[?1049l\x1b[?1049h");
    assert_eq!(t.state.style_stats().0.links, 0);
    check(&t);
}

#[test]
fn history_has_no_links_yet() {
    // History v1 stores styles without their link: links come back with
    // scrollback v2's per-block link table (PR 7).
    let mut t = Term::new(10, 2, 100);
    t.advance(b"\x1b]8;;https://h\x1b\\old\x1b]8;;\x1b\\\r\n\r\nnew");
    assert_eq!(t.state.scrollback_len(), 1);
    t.state.scroll_view(1);
    assert_eq!(t.state.visible_row(0).text().trim_end(), "old");
    assert_eq!(at(&t, 0, 0), None);
}

// ---------------------------------------------------------------------------
// OSC 133.

fn mark(kind: MarkKind, col: u16) -> Mark {
    Mark { kind, col, status: None }
}

/// A zsh-like session: prompt `$ `, a command, its output, then the next
/// prompt, with marks the way the documented snippets send them.
fn session(t: &mut Term, cmd: &str, output: &str, status: u32) {
    t.advance(format!("\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07{output}\x1b]133;D;{status}\x07").as_bytes());
}

#[test]
fn marks_go_on_the_cursors_row_at_its_column() {
    let mut t = Term::new(20, 5, 0);
    session(&mut t, "ls", "a\r\nb\r\n", 0);
    let top = t.state.point_at(0, 0).line;
    let line = |r: usize| top + r as u64;
    assert_eq!(t.state.marks_at(line(0)), vec![mark(MarkKind::Prompt, 0), mark(MarkKind::Input, 2)]);
    assert_eq!(t.state.marks_at(line(1)), vec![mark(MarkKind::Output, 0)]);
    assert_eq!(t.state.marks_at(line(2)), vec![]);
    let end = Mark {
        kind: MarkKind::End,
        col: 0,
        status: Some(0),
    };
    assert_eq!(t.state.marks_at(line(3)), vec![end]);
    // The next prompt on the same row: the end comes before it.
    t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07");
    assert_eq!(
        t.state.marks_at(line(3)),
        vec![end, mark(MarkKind::Prompt, 0), mark(MarkKind::Input, 2)]
    );
    // D without a status; unknown kinds and secondary prompts are ignored.
    t.advance(b"\r\n\x1b]133;D\x07\x1b]133;A;k=s\x07\x1b]133;Z\x07\x1b]133;P;k=i\x07");
    assert_eq!(
        t.state.marks_at(line(4)),
        vec![Mark {
            kind: MarkKind::End,
            col: 0,
            status: None
        }]
    );
    check(&t);
}

#[test]
fn marks_move_with_their_rows_and_go_when_the_row_is_erased() {
    let mut t = Term::new(20, 3, 0);
    t.advance(b"\x1b]133;A\x07$ \r\n\r\n");
    let first = t.state.point_at(0, 0).line;
    assert_eq!(t.state.marks_at(first), vec![mark(MarkKind::Prompt, 0)]);
    // Scrolled up a row: the mark stays with its line number.
    t.advance(b"\r\n");
    assert_eq!(t.state.point_at(0, 0).line, first + 1);
    assert_eq!(t.state.marks_at(first), vec![], "scrolled off with no history");
    let mut t = Term::new(20, 3, 0);
    t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07ls");
    let line = t.state.point_at(0, 0).line;
    // A partial erase (zle redrawing after the prompt) keeps them.
    t.advance(b"\x1b[3G\x1b[K");
    assert_eq!(t.state.marks_at(line).len(), 2);
    // Erasing the whole row drops them: EL 2, ED 2, or ED from column 0.
    t.advance(b"\x1b[2K");
    assert_eq!(t.state.marks_at(line), vec![]);
    t.advance(b"\x1b]133;A\x07\x1b[H\x1b[J");
    assert_eq!(t.state.marks_at(line), vec![]);
    t.advance(b"\x1b]133;A\x07\x1b[2J");
    assert_eq!(t.state.marks_at(line), vec![]);
    // IL pushes a marked row down a line.
    t.advance(b"\x1b[H\x1b]133;A\x07\x1b[L");
    assert_eq!(t.state.marks_at(line + 1), vec![mark(MarkKind::Prompt, 0)]);
    // Narrowing drops marks past the new width.
    t.advance(b"\x1b[1;15H\x1b]133;C\x07");
    t.state.resize(10, 3);
    assert_eq!(t.state.marks_at(line + 1), vec![mark(MarkKind::Prompt, 0)]);
    check(&t);
}

#[test]
fn jump_between_prompts() {
    let mut t = Term::new(20, 12, 0);
    session(&mut t, "one", "1\r\n", 0);
    session(&mut t, "two", "2\r\n22\r\n", 1);
    t.advance(b"\x1b]133;A\x07$ ");
    let top = t.state.point_at(0, 0).line;
    let line = |r: usize| top + r as u64;
    let at = |r: usize| Point { line: line(r), col: 0 };
    assert_eq!(t.state.previous_prompt(line(11)), Some(at(5)));
    assert_eq!(t.state.previous_prompt(line(5)), Some(at(2)));
    assert_eq!(t.state.previous_prompt(line(2)), Some(at(0)));
    assert_eq!(t.state.previous_prompt(line(0)), None);
    assert_eq!(t.state.next_prompt(line(0)), Some(at(2)));
    assert_eq!(t.state.next_prompt(line(2)), Some(at(5)));
    assert_eq!(t.state.next_prompt(line(5)), None);
    // With no marks there is nowhere to go.
    let plain = Term::new(20, 4, 0);
    assert_eq!(plain.state.previous_prompt(3), None);
    assert_eq!(plain.state.next_prompt(0), None);
}

#[test]
fn select_a_commands_output() {
    let mut t = Term::new(20, 12, 0);
    session(&mut t, "one", "1\r\n", 0);
    session(&mut t, "two", "2\r\n22\r\n", 1);
    session(&mut t, "quiet", "", 0);
    t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07slow\r\n\x1b]133;C\x07still\r\ngoing");
    let top = t.state.point_at(0, 0).line;
    let p = |r: usize, c: usize| Point {
        line: top + r as u64,
        col: c,
    };
    let copy = |t: &mut Term, at: Point| t.state.select_output(at).then(|| t.state.selection_text().unwrap());
    // From anywhere in the output, or on its prompt or command line.
    assert_eq!(copy(&mut t, p(1, 0)), Some("1".into()));
    assert_eq!(copy(&mut t, p(0, 3)), Some("1".into()));
    assert_eq!(copy(&mut t, p(4, 1)), Some("2\n22".into()));
    assert_eq!(copy(&mut t, p(2, 0)), Some("2\n22".into()));
    // An empty output: nothing selected, the selection left alone.
    let before = t.state.selection();
    assert_eq!(copy(&mut t, p(5, 4)), None);
    assert_eq!(t.state.selection(), before);
    // A command still running: up to the cursor.
    assert_eq!(copy(&mut t, p(8, 2)), Some("still\ngoing".into()));
    assert_eq!(t.state.command_output(p(6, 8)), Some((p(7, 0), p(8, 4))));
    // Before any mark there is no command.
    let mut plain = Term::new(20, 3, 0);
    plain.advance(b"text");
    assert_eq!(plain.state.command_output(plain.state.point_at(0, 1)), None);
}

/// Marks scrolled into history are kept beside its lines: prompts there
/// are found going up and down, outputs there are selected, and they go
/// with their lines when history drops them or is cleared.
#[test]
fn marks_in_history() {
    let mut t = Term::new(20, 4, 6);
    session(&mut t, "one", "1\r\n", 0);
    session(&mut t, "two", "2\r\n22\r\n", 1);
    t.advance(b"\x1b]133;A\x07$ ");
    // Lines 0..7: prompts at 0, 2 and 5; the screen is lines 2..6.
    let top = t.state.point_at(0, 0).line;
    assert_eq!(top, 2);
    let at = |line: u64| Point { line, col: 0 };
    assert_eq!(t.state.marks_at(0), vec![mark(MarkKind::Prompt, 0), mark(MarkKind::Input, 2)]);
    assert_eq!(t.state.previous_prompt(5), Some(at(2)));
    assert_eq!(t.state.previous_prompt(2), Some(at(0)));
    assert_eq!(t.state.previous_prompt(0), None);
    assert_eq!(t.state.next_prompt(0), Some(at(2)));
    assert_eq!(t.state.next_prompt(1), Some(at(2)));
    let copy = |t: &mut Term, at: Point| t.state.select_output(at).then(|| t.state.selection_text().unwrap());
    assert_eq!(copy(&mut t, at(0)), Some("1".into()));
    assert_eq!(copy(&mut t, at(3)), Some("2\n22".into()));
    // History holds 6 lines: 6 more lines drop lines 0 and 1, the first
    // command's.
    t.advance(b"\r\n\r\n\r\n\r\n\r\n\r\n");
    assert_eq!(t.state.scrollback_len(), 6);
    assert_eq!(t.state.marks_at(0), vec![]);
    assert_eq!(t.state.marks_at(1), vec![]);
    assert_eq!(t.state.previous_prompt(5), Some(at(2)));
    assert_eq!(t.state.previous_prompt(2), None);
    // ED 3 clears history and its marks.
    t.advance(b"\x1b[3J");
    assert_eq!(t.state.previous_prompt(10), None);
    // The alternate screen has no history to jump into.
    let mut t = Term::new(20, 2, 10);
    session(&mut t, "one", "1\r\n", 0);
    t.advance(b"\x1b[?1049h");
    assert_eq!(t.state.previous_prompt(t.state.point_at(1, 0).line), None);
    check(&t);
}

#[test]
fn osc8_and_133_split_across_reads() {
    let input = b"\x1b]8;id=q;https://split/a;b\x1b\\link\x1b]8;;\x1b\\\x1b]133;A\x07$ ";
    let mut whole = Term::new(20, 2, 0);
    whole.advance(input);
    let mut bytes = Term::new(20, 2, 0);
    for b in input.iter() {
        bytes.advance(std::slice::from_ref(b));
    }
    for col in 0..6 {
        assert_eq!(at(&whole, 0, col), at(&bytes, 0, col));
    }
    assert_eq!(at(&bytes, 0, 0), link("https://split/a;b", Some("q")));
    let line = bytes.state.point_at(0, 0).line;
    assert_eq!(bytes.state.marks_at(line), vec![mark(MarkKind::Prompt, 4)]);
}
