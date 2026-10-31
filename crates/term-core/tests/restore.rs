//! History saved across a restart and put back into a new terminal
//! (`Terminal::save_history` / `restore_history`): every line comes back
//! as it was, styles, links, prompt marks, wide characters, clusters and
//! soft wraps included, and the screen's rows come back onto the screen.

use term_core::{RowRef, Term};

/// One row, all it shows: text, each cell's style (link ids, which are
/// the table's own, as the link's value), its marks and whether it wraps.
fn describe(r: &RowRef) -> String {
    let mut s = r.text().trim_end().to_string();
    let mut last = String::new();
    // Up to the last cell that shows: a history row may keep fewer blank
    // cells than a screen row.
    let shown = r.cells().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
    for (col, c) in r.cells().enumerate().take(shown) {
        let mut style = c.style();
        style.link = 0;
        let here = format!("{style:?} {:?} {:?}", r.link(col), c.width());
        if here != last {
            s.push_str(&format!("\n  @{col}: {here}"));
            last = here;
        }
    }
    let marks: Vec<_> = r.marks().collect();
    format!("{s}\n  marks {marks:?} wrapped {}", r.wrapped())
}

/// Every row of history and the screen, oldest first, without the blank
/// rows at the end.
fn all_rows(t: &mut Term) -> Vec<String> {
    let lines = t.state.lines();
    let mut out = Vec::new();
    t.state.scroll_view_to_bottom();
    t.state.scroll_view(isize::MAX / 2);
    let back = t.state.display_offset();
    for k in (1..=back).rev() {
        t.state.scroll_view_to_bottom();
        t.state.scroll_view(k as isize);
        out.push(describe(&t.state.visible_row(0)));
    }
    t.state.scroll_view_to_bottom();
    for l in 0..lines {
        out.push(describe(&t.state.visible_row(l)));
    }
    while out.last().is_some_and(|r| r.starts_with('\n')) {
        out.pop();
    }
    out
}

fn sample() -> Term {
    let mut t = Term::new(20, 8, 10_000, term_core::ImagePool::process());
    for i in 0..400 {
        t.advance(format!("plain line {i}\r\n").as_bytes());
    }
    // Some of history compressed, some not.
    t.state.shrink_history();
    for i in 0..100 {
        t.advance(format!("\x1b[1;31mred {i}\x1b[0m \x1b[48;5;22mbg\x1b[0m\r\n").as_bytes());
    }
    t.advance(b"\x1b]8;;https://example.com/a\x1b\\a link\x1b]8;;\x1b\\ after\r\n");
    t.advance(b"\x1b]8;id=x;https://example.com/b\x1b\\linked by id\x1b]8;;\x1b\\\r\n");
    t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07");
    t.advance("日本語 テスト\r\ne\u{301} 👨‍👩‍👧 🇬🇧 x\r\n".as_bytes());
    t.advance(b"a long line that wraps over the edge of twenty columns\r\n");
    t.advance(b"\x1b[4:3;58;2;1;2;3mcurly\x1b[0m \x1b[7minverse\x1b[0m\r\n");
    t.advance(b"last on screen");
    t
}

#[test]
fn every_line_comes_back_as_it_was() {
    let mut t = sample();
    let before = all_rows(&mut t);
    // What is checked is there to check.
    for needle in ["example.com/a", "id: Some(\"x\")", "Wide", "👨‍👩‍👧", "wrapped true", "Curly"] {
        assert!(before.iter().any(|r| r.contains(needle)), "no {needle}");
    }
    assert!(before.iter().filter(|r| !r.contains("marks []")).count() >= 3);
    let saved = t.state.save_history(10_000, 1_700_000_000).expect("primary screen");
    let mut back = Term::new(20, 8, 10_000, term_core::ImagePool::process());
    let r = back.state.restore_history(&saved).unwrap();
    assert_eq!(r.saved_at, 1_700_000_000);
    back.state.check_history().unwrap();
    back.state.check_invariants().unwrap();
    let after = all_rows(&mut back);
    assert_eq!(after.len(), before.len());
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        assert_eq!(a, b, "row {i}");
    }
    // The screen's rows are back on the screen, the cursor below them.
    let dump = back.state.dump();
    assert!(dump.lines().nth(6).unwrap().starts_with("last on screen"), "{dump}");
    assert!(dump.ends_with("cursor=(7,0) alt=false offset=0"), "{dump}");
    // And output carries on below.
    back.advance(b"\r\nnew shell $ ");
    back.state.check_invariants().unwrap();
    assert!(back.state.dump().contains("new shell $"));
}

#[test]
fn only_the_newest_lines_up_to_the_cap() {
    let mut t = sample();
    let saved = t.state.save_history(50, 0).unwrap();
    let mut back = Term::new(20, 8, 10_000, term_core::ImagePool::process());
    let r = back.state.restore_history(&saved).unwrap();
    // 50 lines shown in all; the screen's 7 rows came back onto it.
    assert_eq!(r.lines, 50);
    assert_eq!(back.state.scrollback_len() + 7, 50);
    let before = all_rows(&mut t);
    let after = all_rows(&mut back);
    assert_eq!(after[..], before[before.len() - 50..]);
}

#[test]
fn a_smaller_window_shows_what_fits_and_keeps_the_rest_in_history() {
    let mut t = sample();
    let before = all_rows(&mut t);
    let saved = t.state.save_history(10_000, 0).unwrap();
    let mut back = Term::new(20, 4, 10_000, term_core::ImagePool::process());
    back.state.restore_history(&saved).unwrap();
    back.state.check_invariants().unwrap();
    let after = all_rows(&mut back);
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        assert_eq!(a, b, "row {i}");
    }
    assert_eq!(after.len(), before.len());
    assert!(back.state.dump().ends_with("cursor=(3,0) alt=false offset=0"));
}

#[test]
fn nothing_is_saved_from_the_alternate_screen_and_only_into_a_new_terminal() {
    let mut t = sample();
    t.advance(b"\x1b[?1049h secret editor buffer");
    assert!(t.state.save_history(10_000, 0).is_none());
    t.advance(b"\x1b[?1049l");
    let saved = t.state.save_history(10_000, 0).unwrap();
    let mut used = Term::new(20, 8, 10_000, term_core::ImagePool::process());
    used.advance(b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8\r\n9\r\n");
    assert!(used.state.restore_history(&saved).is_err());
    // A damaged file restores nothing and leaves the terminal as it was.
    let mut fresh = Term::new(20, 8, 10_000, term_core::ImagePool::process());
    assert!(fresh.state.restore_history(&saved[..saved.len() - 10]).is_err());
    assert_eq!(fresh.state.scrollback_len(), 0);
    fresh.state.check_invariants().unwrap();
}
