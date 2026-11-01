//! Selections kept to a program's panel (cells §13.2): finding the
//! panel a drag starts in, what such a selection copies and highlights,
//! dragging past its edge, and following its text as a region scrolls.

use term_core::{Panel, Point, Selection, Term};

/// Two boxes side by side, columns 0-5 and 6-11, rows 0-3.
const BOXES: &str = "┌────┐┌────┐\r\n│abc ││def │\r\n│ghi ││jkl │\r\n└────┘└────┘";

fn term(text: &str) -> Term {
    let mut t = Term::new(20, 6, 100, term_core::ImagePool::process());
    t.advance(text.as_bytes());
    t
}

fn panel(t: &Term, line: usize, col: usize) -> Option<Panel> {
    t.state.panel_at(t.state.point_at(line, col))
}

/// Start a selection at view (line, col) kept to the panel there, and
/// drag it to (line, col).
fn drag(t: &mut Term, from: (usize, usize), to: (usize, usize)) {
    let at = t.state.point_at(from.0, from.1);
    let p = t.state.panel_at(at);
    t.state.set_selection(Some(Selection::new(at, at).in_panel(p)));
    t.state.extend_selection(t.state.point_at(to.0, to.1));
}

fn copied(t: &Term) -> String {
    t.state.selection_text().expect("a selection")
}

#[test]
fn borders_on_both_sides() {
    let t = term(BOXES);
    let left = Panel {
        left: 1,
        right: 4,
        rows: None,
    };
    for (line, col) in [(1, 1), (1, 3), (2, 4)] {
        assert_eq!(panel(&t, line, col), Some(left), "({line}, {col})");
    }
    assert_eq!(
        panel(&t, 2, 8),
        Some(Panel {
            left: 7,
            right: 10,
            rows: None
        })
    );
    // Right of both boxes: up to the screen's edge.
    assert_eq!(
        panel(&t, 1, 15),
        Some(Panel {
            left: 12,
            right: 19,
            rows: None
        })
    );
    // On a border itself, or on the top border row (no vertical border
    // either side of a cell between corners): none.
    assert_eq!(panel(&t, 1, 0), None);
    assert_eq!(panel(&t, 1, 5), None);
}

#[test]
fn a_border_on_one_side() {
    let t = term("left │ right\r\nmore │ text\r\nend  │");
    assert_eq!(
        panel(&t, 1, 1),
        Some(Panel {
            left: 0,
            right: 4,
            rows: None
        })
    );
    assert_eq!(
        panel(&t, 0, 8),
        Some(Panel {
            left: 6,
            right: 19,
            rows: None
        })
    );
}

#[test]
fn no_borders_no_panel() {
    let t = term("plain text\r\nmore of it");
    assert_eq!(panel(&t, 0, 3), None);
    assert_eq!(panel(&t, 1, 3), None);
}

/// A `│` in prose, with nothing above or below it in its column, is no
/// border; nor are two in different columns on rows one above the other.
#[test]
fn a_stray_bar_in_text_is_no_border() {
    let t = term("pipe a │ b here\r\nno bar on this row\r\nx │ y");
    assert_eq!(panel(&t, 0, 2), None);
    assert_eq!(panel(&t, 0, 10), None);
    assert_eq!(panel(&t, 2, 4), None);
    // A corner below joins it: a box's side.
    let t = term("a │ b\r\n  └──");
    assert!(panel(&t, 0, 0).is_some());
}

/// A scroll region narrower than the screen holds the panel's rows,
/// for a drag starting inside it.
#[test]
fn the_scroll_regions_rows() {
    let t = term("\x1b[2;4rtitle");
    let rows = Panel {
        left: 0,
        right: 19,
        rows: Some((1, 3)),
    };
    for line in 1..=3 {
        assert_eq!(panel(&t, line, 5), Some(rows), "row {line}");
    }
    assert_eq!(panel(&t, 0, 5), None, "above it");
    assert_eq!(panel(&t, 4, 5), None, "below it");
    // The whole screen's region is no panel.
    assert_eq!(panel(&term("\x1b[1;6rtext"), 2, 2), None);
    // With borders as well.
    let t = term(&format!("{BOXES}\x1b[2;5r"));
    assert_eq!(
        panel(&t, 1, 2),
        Some(Panel {
            left: 1,
            right: 4,
            rows: Some((1, 4))
        })
    );
}

/// At the shell's prompt (its last OSC 133 mark a prompt or a command's
/// end) there are no panels: one command's output isn't one. While a
/// command runs, or on the alternate screen, there are.
#[test]
fn none_at_the_shells_prompt() {
    let boxes = |mark: &str| term(&format!("{mark}{BOXES}"));
    assert_eq!(panel(&boxes("\x1b]133;A\x07"), 1, 2), None, "a prompt");
    assert_eq!(panel(&boxes("\x1b]133;A\x07\x1b]133;B\x07"), 1, 2), None, "the command line");
    assert_eq!(panel(&boxes("\x1b]133;C\x07\x1b]133;D;0\x07"), 1, 2), None, "a command's end");
    assert!(
        panel(&boxes("\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07"), 1, 2).is_some(),
        "running"
    );
    assert!(panel(&boxes("\x1b]133;A\x07\x1b[?1049h"), 1, 2).is_some(), "the alternate screen");
}

/// Line-wise inside the panel's columns: the first row from where it
/// starts, the middle ones edge to edge, the last to where it ends; no
/// borders, nor the other panel's text, trailing spaces trimmed.
#[test]
fn what_a_panel_selection_copies_and_highlights() {
    let mut t = term("┌────┐┌────┐\r\n│abc ││def │\r\n│ghi ││jkl │\r\n│mno ││pqr │\r\n└────┘└────┘");
    drag(&mut t, (1, 2), (3, 2));
    assert_eq!(copied(&t), "bc\nghi\nmn");
    assert_eq!(t.state.selected_cols(1), 2..5);
    assert_eq!(t.state.selected_cols(2), 1..5);
    assert_eq!(t.state.selected_cols(3), 1..3);
    assert_eq!(t.state.selected_cols(0), 0..0);
    // Dragged into the other box: stops at the edge.
    t.state.extend_selection(t.state.point_at(2, 9));
    assert_eq!(copied(&t), "bc\nghi");
    assert_eq!(t.state.selected_cols(2), 1..5);
    // Backwards, from the right box.
    drag(&mut t, (3, 9), (1, 8));
    assert_eq!(copied(&t), "ef\njkl\npqr");
    // The other box drawn again with other text keeps it; its own
    // text changing clears it.
    t.advance("\x1b[2;2HXYZW\x1b[3;2Hpane\x1b[4;2H1234".as_bytes());
    assert_eq!(copied(&t), "ef\njkl\npqr", "the other box redrawn");
    t.advance("\x1b[3;8Hjkx".as_bytes());
    assert_eq!(t.state.selection(), None, "its own text changed");

    // A plain selection over the same rows takes everything.
    t.state
        .set_selection(Some(Selection::new(t.state.point_at(1, 2), t.state.point_at(2, 8))));
    assert_eq!(copied(&t), "YZW││def │\n│pane││jk");
}

/// A drag past the scroll region's top or bottom stops at its edge.
#[test]
fn a_drag_stops_at_the_regions_edge() {
    let mut t = term("title\r\none\r\ntwo\r\nthree\r\nstatus\x1b[2;4r");
    drag(&mut t, (2, 1), (5, 3));
    assert_eq!(copied(&t), "wo\nthre");
    drag(&mut t, (2, 1), (0, 0));
    assert_eq!(copied(&t), "one\ntw");
}

/// A panel selection follows its text as the program scrolls its region,
/// cut at the region's edge (at the panel's columns), and cleared once
/// all of it has scrolled out.
#[test]
fn a_panel_selection_follows_a_region_scroll() {
    let mut t = term("┌────┐┌────┐\r\n│abc ││def │\r\n│ghi ││jkl │\r\n│mno ││pqr │\r\n│stu ││vwx │\r\n└────┘└────┘\x1b[2;5r");
    drag(&mut t, (2, 2), (3, 3));
    assert_eq!(copied(&t), "hi\nmno");
    // Scroll the region up one, redrawing the borders on the new row.
    t.advance("\x1b[S\x1b[5;1H│yz  ││    │".as_bytes());
    assert_eq!(copied(&t), "hi\nmno", "followed");
    let top = t.state.point_at(0, 0).line;
    let (a, _) = t.state.selection().unwrap().ordered();
    assert_eq!(a, Point { line: top + 1, col: 2 });
    t.advance(b"\x1b[S");
    assert_eq!(copied(&t), "mno", "cut at the region's top, at the panel's edge");
    let (a, _) = t.state.selection().unwrap().ordered();
    assert_eq!(a, Point { line: top + 1, col: 1 });
    t.advance(b"\x1b[S");
    assert_eq!(t.state.selection(), None, "scrolled out");
}

/// At a shell prompt nothing changes: a drag over boxes a command
/// printed selects across them, and output scrolling the primary screen
/// keeps it on its text.
#[test]
fn the_primary_screen_at_a_prompt_is_unchanged() {
    let mut t = term(&format!(
        "\x1b]133;A\x07$ \x1b]133;B\x07\x1b]133;C\x07\r\n{BOXES}\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ "
    ));
    drag(&mut t, (2, 2), (3, 8));
    assert_eq!(t.state.selection().unwrap().panel, None);
    assert_eq!(copied(&t), "bc ││def │\n│ghi ││jk");
    for i in 0..20 {
        t.advance(format!("\r\nline {i}").as_bytes());
    }
    assert_eq!(copied(&t), "bc ││def │\n│ghi ││jk");
}
