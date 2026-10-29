//! Row change counters and the snapshot (§9 of the cells design): which
//! rows keep their counter and which get a new one, what a snapshot
//! copies for a renderer that caches rows, the style updates it carries,
//! highlight ranges and the cursor. The property test in `props.rs` checks
//! that a frame rebuilt from snapshots always equals the live screen.

use std::collections::HashSet;

use term_core::{Color, CursorShape, HighlightKind, Point, RowFlags, RowKey, ScreenId, Selection, Snapshot, Style, Term};

/// A renderer's row cache, only as far as keys go.
#[derive(Default)]
struct Cache {
    snap: Snapshot,
    keys: HashSet<RowKey>,
}

impl Cache {
    /// Take a snapshot and return the view rows that were copied.
    fn take(&mut self, t: &mut Term) -> Vec<usize> {
        let keys = &self.keys;
        t.state.snapshot(&mut self.snap, |k| keys.contains(&k));
        let mut copied = Vec::new();
        for (i, row) in self.snap.rows.iter().enumerate() {
            if row.cells.is_some() {
                copied.push(i);
                self.keys.insert(row.key);
            }
        }
        copied
    }
}

fn filled(cols: usize, lines: usize) -> Term {
    let mut t = Term::new(cols, lines, 100, term_core::ImagePool::process());
    let text: Vec<String> = (0..lines).map(|i| format!("line {i}")).collect();
    t.advance(text.join("\r\n").as_bytes());
    t
}

#[test]
fn every_row_has_its_own_counter_from_the_start() {
    let t = Term::new(10, 5, 0, term_core::ImagePool::process());
    let seqs = t.state.row_seqs();
    assert_eq!(seqs.len(), 5);
    assert_eq!(seqs.iter().collect::<HashSet<_>>().len(), 5, "{seqs:?}");
    assert!(seqs.iter().all(|&s| s > 0 && s <= t.state.change_counter()));
}

#[test]
fn first_snapshot_copies_everything_then_nothing() {
    let mut t = filled(10, 4);
    let mut c = Cache::default();
    assert_eq!(c.take(&mut t), [0, 1, 2, 3]);
    assert_eq!(c.snap.cells.len(), 40);
    assert_eq!(c.take(&mut t), Vec::<usize>::new());
    assert!(c.snap.cells.is_empty());
}

#[test]
fn a_spinner_copies_one_row() {
    let mut t = filled(10, 4);
    let mut c = Cache::default();
    c.take(&mut t);
    for frame in ["|", "/", "-", "\\"] {
        t.advance(format!("\x1b[2;1H{frame}").as_bytes());
        assert_eq!(c.take(&mut t), [1]);
        assert_eq!(c.snap.cells.len(), 10);
    }
}

#[test]
fn rows_changed_twice_between_snapshots_count_once() {
    let mut t = filled(10, 4);
    let mut c = Cache::default();
    c.take(&mut t);
    let before = t.state.change_counter();
    t.advance(b"\x1b[1;1Ha");
    t.advance(b"\x1b[1;2Hb\x1b[1;3Hc");
    assert_eq!(
        t.state.change_counter(),
        before + 1,
        "one new counter for the row, not one per change"
    );
    c.take(&mut t);
    t.advance(b"\x1b[1;1Hd");
    assert_eq!(
        t.state.change_counter(),
        before + 2,
        "after a snapshot the row changes counter again"
    );
}

#[test]
fn scrolling_keeps_the_rows_that_only_moved() {
    let mut t = filled(10, 4);
    let mut c = Cache::default();
    c.take(&mut t);
    let seqs = t.state.row_seqs();
    // A whole-screen scroll: rows 1..3 move up, a new blank row arrives.
    t.advance(b"\r\nnew");
    let after = t.state.row_seqs();
    assert_eq!(after[..3], seqs[1..]);
    assert_eq!(c.take(&mut t), [3]);

    // A region scroll (DECSTBM 2..4, SU): the same within the region.
    c.take(&mut t);
    let seqs = t.state.row_seqs();
    t.advance(b"\x1b[2;4r\x1b[S\x1b[r");
    let after = t.state.row_seqs();
    assert_eq!(after[0], seqs[0]);
    assert_eq!(after[1..3], seqs[2..4]);
    assert_eq!(c.take(&mut t), [3]);

    // IL at row 2 pushes rows 2.. down; only the inserted row is new.
    let seqs = t.state.row_seqs();
    t.advance(b"\x1b[2;1H\x1b[L");
    let after = t.state.row_seqs();
    assert_eq!(after[0], seqs[0]);
    assert_eq!(after[2..], seqs[1..3]);
    assert_eq!(c.take(&mut t), [1]);

    // DL at row 1 pulls everything up; only the new bottom row is new.
    let seqs = t.state.row_seqs();
    t.advance(b"\x1b[1;1H\x1b[M");
    assert_eq!(t.state.row_seqs()[..3], seqs[1..]);
    assert_eq!(c.take(&mut t), [3]);

    // Scrolled into history: the old top row is now a history row, keyed by
    // its line number.
    t.state.scroll_view(1);
    let copied = c.take(&mut t);
    assert_eq!(copied, [0], "only the history row is new to the cache");
    let row = &c.snap.rows[0];
    assert_eq!(row.key, RowKey::History { line: 0, col: 0 });
    assert!(row.flags.contains(RowFlags::HISTORY));
}

#[test]
fn erases_inserts_and_wide_characters_change_their_row() {
    for seq in [
        &b"\x1b[2;3H\x1b[K"[..],
        b"\x1b[2;3H\x1b[1K",
        b"\x1b[2;3H\x1b[2X",
        b"\x1b[2;3H\x1b[2@",
        b"\x1b[2;3H\x1b[2P",
        b"\x1b[2;3H\x1b[4h!\x1b[4l",
        "\x1b[2;3H\u{6f22}".as_bytes(),
        "\x1b[2;3H\u{2764}\u{fe0f}".as_bytes(),
        b"\x1b[2;3Hx\x1b[3b",
    ] {
        let mut t = filled(10, 4);
        let mut c = Cache::default();
        c.take(&mut t);
        t.advance(seq);
        assert_eq!(c.take(&mut t), [1], "{seq:?}");
    }
    // Whole-screen ones change every row.
    for seq in [&b"\x1b[2J"[..], b"\x1b#8", b"\x1b[H\x1b[J"] {
        let mut t = filled(10, 4);
        let mut c = Cache::default();
        c.take(&mut t);
        t.advance(seq);
        assert_eq!(c.take(&mut t), [0, 1, 2, 3], "{seq:?}");
    }
    // Moving the cursor or setting a mode changes nothing.
    let mut t = filled(10, 4);
    let mut c = Cache::default();
    c.take(&mut t);
    t.advance(b"\x1b[3;3H\x1b[31m\x1b[?25l\x1b[?1h");
    assert_eq!(c.take(&mut t), Vec::<usize>::new());
}

#[test]
fn resize_and_renumbering_change_every_row() {
    let mut t = filled(10, 4);
    let mut c = Cache::default();
    c.take(&mut t);
    t.state.resize(12, 4);
    assert_eq!(c.take(&mut t), [0, 1, 2, 3]);

    // Many styles on the top row, then plain text over them: a sweep now
    // renumbers the table, which changes every row's bits.
    t.state.set_sweep_at(Some(8));
    for pass in 0..3 {
        t.advance(b"\x1b[1;1H");
        for i in 0..9 {
            t.advance(format!("\x1b[38;5;{}mx", pass * 9 + i).as_bytes());
        }
    }
    t.advance(b"\x1b[0m\x1b[1;1Hplain text");
    c.take(&mut t);
    let epoch = c.snap.styles_epoch;
    let before = t.state.style_stats().0.renumberings;
    t.state.sweep_styles_now().unwrap();
    assert!(t.state.style_stats().0.renumberings > before);
    assert_eq!(c.take(&mut t), [0, 1, 2, 3]);
    assert_ne!(c.snap.styles_epoch, epoch, "a renumbered table has a new epoch");
    assert_eq!(c.snap.style_updates, [(0, Style::DEFAULT)], "and comes whole");
}

#[test]
fn reset_keeps_counting_and_clears_history_by_epoch() {
    let mut t = filled(10, 2);
    t.advance(b"\r\nmore\r\nlines");
    assert!(t.state.scrollback_len() > 0);
    let (id, counter, epoch) = (t.state.terminal_id(), t.state.change_counter(), t.state.history_epoch());
    let old: HashSet<u64> = t.state.row_seqs().into_iter().collect();
    t.advance(b"\x1bc");
    assert_eq!(t.state.terminal_id(), id, "RIS is the same terminal");
    assert!(t.state.change_counter() > counter, "the counter doesn't go back");
    assert!(t.state.history_epoch() > epoch);
    assert!(t.state.row_seqs().iter().all(|s| !old.contains(s) && *s > counter));

    let epoch = t.state.history_epoch();
    t.advance(b"\x1b[3J");
    assert_eq!(t.state.history_epoch(), epoch + 1, "ED 3 clears history");
    t.advance(b"\x1b[2J");
    assert_eq!(t.state.history_epoch(), epoch + 1, "ED 2 leaves history alone");
}

#[test]
fn a_new_terminal_has_a_new_id() {
    let a = Term::new(10, 4, 0, term_core::ImagePool::process());
    let b = Term::new(10, 4, 0, term_core::ImagePool::process());
    assert_ne!(a.state.terminal_id(), b.state.terminal_id());
}

/// A renderer that kept its cache across a change of terminal, history or
/// width is never told it has rows it must drop: they are copied again.
#[test]
fn rows_the_renderer_must_drop_are_copied_again() {
    let mut t = filled(10, 2);
    t.advance(b"\r\nthree\r\nfour");
    t.state.scroll_view(1);
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    // The renderer claims to have everything.
    t.state.snapshot(&mut snap, |_| true);
    assert!(snap.rows.iter().all(|r| r.cells.is_none()));

    // ED 3 empties history; new history arrives with new numbers anyway,
    // but cached history rows go with the epoch.
    t.advance(b"\x1b[3J\r\nfive\r\nsix");
    t.state.scroll_view(1);
    let epoch = snap.history_epoch;
    t.state.snapshot(&mut snap, |_| true);
    assert_ne!(snap.history_epoch, epoch);
    assert!(snap.rows[0].flags.contains(RowFlags::HISTORY));
    assert!(snap.rows[0].cells.is_some(), "history rows copied after ED 3");
    assert!(snap.rows[1].cells.is_none(), "screen rows still cached");

    // A different terminal: everything is copied.
    let mut other = filled(10, 2);
    other.state.snapshot(&mut snap, |_| true);
    assert!(snap.rows.iter().all(|r| r.cells.is_some()));
}

#[test]
fn style_updates_follow_each_screens_table() {
    let mut t = Term::new(10, 3, 0, term_core::ImagePool::process());
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.screen, ScreenId::Primary);
    assert_eq!(snap.style_updates, [(0, Style::DEFAULT)], "a new table comes whole");
    let primary = snap.styles_epoch;

    t.advance(b"\x1b[31mred\x1b[0m");
    t.state.snapshot(&mut snap, |_| false);
    let red = Style {
        fg: Color::indexed(1),
        ..Style::DEFAULT
    };
    assert_eq!(snap.style_updates.iter().map(|u| u.1).collect::<Vec<_>>(), [red]);
    assert_eq!(snap.styles_epoch, primary);
    t.state.snapshot(&mut snap, |_| false);
    assert!(snap.style_updates.is_empty(), "steady state copies no styles");

    // The alternate screen has its own table, made new on entry.
    t.advance(b"\x1b[?1049h\x1b[32mgreen");
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.screen, ScreenId::Alternate);
    assert_ne!(snap.styles_epoch, primary);
    assert_eq!(snap.style_updates.len(), 2, "{:?}", snap.style_updates);

    // Back on the primary screen: nothing new there.
    t.advance(b"\x1b[?1049l");
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!((snap.screen, snap.styles_epoch), (ScreenId::Primary, primary));
    assert!(snap.style_updates.is_empty());
}

/// A renderer new to a terminal (a fresh `Snapshot`) is sent each screen's
/// whole table the first time it sees that screen, whatever earlier
/// snapshots were sent.
#[test]
fn a_new_renderer_is_sent_each_table_whole() {
    let mut t = Term::new(10, 3, 0, term_core::ImagePool::process());
    t.advance(b"\x1b[31mred\x1b[0m");
    let mut old = Snapshot::default();
    t.state.snapshot(&mut old, |_| false);
    t.advance(b"\x1b[?1049h\x1b[32mgreen");
    t.state.snapshot(&mut old, |_| false);
    let styles = |s: &Snapshot| s.style_updates.iter().map(|u| u.1.fg).collect::<Vec<_>>();
    let (red, green) = (Color::indexed(1), Color::indexed(2));

    let mut new = Snapshot::default();
    t.state.snapshot(&mut new, |_| false);
    assert_eq!(new.screen, ScreenId::Alternate);
    assert!(styles(&new).contains(&green), "{:?}", new.style_updates);
    t.state.snapshot(&mut new, |_| false);
    assert!(new.style_updates.is_empty(), "then only what's new");

    // The primary table was sent to the old renderer only.
    t.advance(b"\x1b[?1049l");
    t.state.snapshot(&mut new, |_| false);
    assert_eq!(new.screen, ScreenId::Primary);
    assert!(styles(&new).contains(&red), "{:?}", new.style_updates);
    t.state.snapshot(&mut new, |_| false);
    assert!(new.style_updates.is_empty());
}

#[test]
fn history_rows_bring_their_own_styles() {
    let mut t = Term::new(10, 2, 100, term_core::ImagePool::process());
    t.advance(b"\x1b[1;31mbold\x1b[0m plain\r\n\x1b[44mblue\x1b[0m\r\nlast\r\nend");
    t.state.scroll_view(2);
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    for (line, row) in snap.rows.iter().enumerate() {
        assert_eq!(row.key, RowKey::History { line: line as u64, col: 0 });
        let cells = snap.row_cells(row).unwrap();
        let shown = t.state.visible_row(line);
        assert_eq!(cells.len(), shown.len());
        for (i, c) in cells.iter().enumerate() {
            assert_eq!(
                snap.history_styles[c.style_id() as usize],
                shown.cell(i).style(),
                "row {line} col {i}"
            );
            assert_eq!(c.ch(), shown.cell(i).ch());
        }
    }
}

#[test]
fn selection_is_a_highlight_per_row() {
    let mut t = filled(10, 4);
    t.state
        .set_selection(Some(Selection::new(Point { line: 2, col: 7 }, Point { line: 0, col: 3 })));
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    let ranges: Vec<_> = snap
        .rows
        .iter()
        .map(|r| snap.row_highlights(r).iter().map(|h| (h.cols.clone(), h.kind)).collect::<Vec<_>>())
        .collect();
    assert_eq!(
        ranges,
        [
            vec![(3..10, HighlightKind::Selection)],
            vec![(0..10, HighlightKind::Selection)],
            vec![(0..8, HighlightKind::Selection)],
            vec![],
        ]
    );
    // Highlights are sent every time, cached row or not.
    t.state.snapshot(&mut snap, |_| true);
    assert_eq!(snap.highlights.len(), 3);
}

#[test]
fn cursor_shape_visibility_and_width() {
    let mut t = Term::new(10, 3, 0, term_core::ImagePool::process());
    t.advance("\u{6f22}\x1b[1;1H\x1b[5 q".as_bytes());
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.cursor.at, Some((0, 0)));
    assert!(snap.cursor.wide && snap.cursor.visible && snap.cursor.blinking);
    assert_eq!(snap.cursor.shape, CursorShape::Bar);
    t.advance(b"\x1b[?25l\x1b[2;4H");
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.cursor.at, Some((1, 3)));
    assert!(!snap.cursor.wide && !snap.cursor.visible);
}

#[test]
fn snapshot_dimensions_and_counter() {
    let mut t = filled(7, 3);
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!((snap.cols, snap.lines, snap.rows.len()), (7, 3, 3));
    assert_eq!(snap.counter, t.state.change_counter());
    assert_eq!(snap.terminal_id, t.state.terminal_id());
    assert_eq!(snap.popped_from, None, "no lines pulled back");
}

/// Lines pulled back from history hand their numbers out again: the
/// snapshot names the lowest one popped, the history rows from there on
/// are copied again, and the ones before it stay cached.
#[test]
fn lines_pulled_back_are_copied_again() {
    let mut t = Term::new(10, 2, 100, term_core::ImagePool::process());
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne\r\nf");
    t.state.scroll_view(2);
    let mut cache = Cache::default();
    cache.take(&mut t);
    assert_eq!(cache.take(&mut t), Vec::<usize>::new(), "all cached");
    // Line 3 ("d") goes back on the screen and comes out again changed.
    t.state.pull_back_for_test(1);
    assert_eq!(t.state.history_stats().open_lines, 3);
    t.advance(b"\x1b[1;1HD\x1b[2;1H\r\n");
    t.state.scroll_view_to_bottom();
    t.state.scroll_view(2);
    let copied = cache.take(&mut t);
    assert_eq!(cache.snap.popped_from, Some(3));
    let rows: Vec<RowKey> = cache.snap.rows.iter().map(|r| r.key).collect();
    assert_eq!(
        rows[..2],
        [RowKey::History { line: 2, col: 0 }, RowKey::History { line: 3, col: 0 }]
    );
    assert_eq!(copied, [1], "line 2 kept, line 3 copied again");
    assert_eq!(t.state.visible_row(1).text().trim_end(), "D", "{}", t.state.dump());
    // The mark goes with the snapshot that carried it.
    cache.take(&mut t);
    assert_eq!(cache.snap.popped_from, None);
}

/// A history row whose last style filled its row says so, and carries
/// the style to the screen's width.
#[test]
fn full_width_backgrounds_fill_history_rows() {
    let mut t = Term::new(10, 2, 100, term_core::ImagePool::process());
    t.advance(b"\x1b[44m+1\x1b[K\x1b[0m\r\nplain\r\n\r\n");
    t.state.resize(14, 2);
    t.state.scroll_view(2);
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert!(snap.rows[0].flags.contains(RowFlags::FILL));
    assert!(!snap.rows[1].flags.contains(RowFlags::FILL));
    let cells = snap.row_cells(&snap.rows[0]).unwrap();
    assert_eq!(cells.len(), 14);
    assert_eq!(snap.history_styles[cells[13].style_id() as usize].bg, Color::indexed(4));
}

#[test]
fn a_scalar_joining_a_cluster_changes_its_row() {
    let mut t = Term::new(10, 2, 0, term_core::ImagePool::process());
    t.advance(b"e");
    let mut c = Cache::default();
    c.take(&mut t);
    // A combining mark, VS16 (widening in place) and a skin tone each
    // join the cell before without writing a new one.
    for (start, join) in [("", "\u{301}"), ("\x1b[3C❤", "\u{FE0F}"), ("\x1b[6C👍", "\u{1F3FD}")] {
        t.advance(start.as_bytes());
        c.take(&mut t);
        let before = t.state.row_seqs();
        t.advance(join.as_bytes());
        let after = t.state.row_seqs();
        assert_ne!(before[0], after[0], "{join:?}");
        assert_eq!(before[1], after[1], "{join:?}");
        assert_eq!(c.take(&mut t), [0], "{join:?}");
        t.advance(b"\r");
    }
}

#[test]
fn snapshots_carry_cluster_text() {
    let mut t = Term::new(10, 2, 0, term_core::ImagePool::process());
    t.advance("ae\u{301}👍\u{1F3FD}\r\nplain".as_bytes());
    let mut c = Cache::default();
    c.take(&mut t);
    let snap = &c.snap;
    let row = &snap.rows[0];
    let cells = snap.row_cells(row).unwrap();
    let texts: Vec<&str> = cells
        .iter()
        .filter(|c| c.is_cluster())
        .map(|c| snap.cluster(row, c.codepoint()))
        .collect();
    assert_eq!(texts, ["e\u{301}", "👍\u{1F3FD}"]);
    // A row without clusters has none.
    assert!(snap.rows[1].clusters.is_empty());
    // Taken again with nothing new: no cells and no clusters copied.
    c.take(&mut t);
    assert!(c.snap.cluster_spans.is_empty() && c.snap.cluster_text.is_empty());
}
