//! Replies programs use to find out what the terminal supports: DECID,
//! DECXCPR, DECREQTPARM; and the conformance fixes found by vttest and
//! esctest2. Expected values follow xterm (ctlseqs) and esctest2.

use term_core::Term;

fn new(cols: usize, lines: usize) -> Term {
    Term::new(cols, lines, 100)
}

/// Send `input`, return the replies as a string and clear them.
fn ask(t: &mut Term, input: &str) -> String {
    t.advance(input.as_bytes());
    String::from_utf8(t.state.take_replies()).unwrap()
}

fn text(t: &Term, line: usize) -> String {
    let row = t.state.visible_row(line);
    let s: String = row.cells().filter(|c| !c.is_spacer()).map(|c| c.ch()).collect();
    s.trim_end().to_string()
}

// ------------------------------------------------------------ reports

#[test]
fn decid_answers_as_primary_da() {
    let mut t = new(10, 2);
    assert_eq!(ask(&mut t, "\x1bZ"), "\x1b[?62;22c");
}

#[test]
fn decxcpr() {
    let mut t = new(10, 10);
    assert_eq!(ask(&mut t, "\x1b[5;6H\x1b[?6n"), "\x1b[?5;6R");
    assert_eq!(ask(&mut t, "\x1b[3;8r\x1b[?6h\x1b[2;4H\x1b[?6n"), "\x1b[?2;4R", "origin mode");
}

#[test]
fn decreqtparm() {
    let mut t = new(10, 2);
    assert_eq!(ask(&mut t, "\x1b[x"), "\x1b[2;1;1;128;128;1;0x");
    assert_eq!(ask(&mut t, "\x1b[1x"), "\x1b[3;1;1;128;128;1;0x");
    assert_eq!(ask(&mut t, "\x1b[2x"), "");
}

// --------------------------------------------- E.19 conformance fixes

#[test]
fn dec_graphics_control_pictures() {
    let mut t = new(12, 1);
    t.advance(b"\x1b(0bcdehi_a\x1b(B_");
    assert_eq!(text(&t, 0), "␉␌␍␊␤␋ ▒_");
}

#[test]
fn british_charset() {
    let mut t = new(10, 1);
    t.advance(b"\x1b)A#\x0e#\x0f#\x1b(A#\x1b(B#");
    assert_eq!(text(&t, 0), "#£#£#");
}

#[test]
fn rep_after_rep_repeats_nothing() {
    let mut t = new(20, 1);
    t.advance(b"+\x1b[1b\x1b[10b");
    assert_eq!(text(&t, 0), "++", "vttest iso6429-rep");
}

#[test]
fn decaln_resets_margins_and_homes_the_cursor() {
    let mut t = new(10, 10);
    t.advance(b"\x1b[4;5r\x1b[5;5H\x1b#8");
    assert_eq!((t.state.cursor().row(), t.state.cursor().col()), (0, 0));
    assert_eq!(text(&t, 9), "EEEEEEEEEE");
    // The margins are gone: CUU from row 4 goes past row 4's top margin.
    t.advance(b"\x1b[4;2H\x1b[A\x1b[?6n");
    assert_eq!(String::from_utf8(t.state.take_replies()).unwrap(), "\x1b[?3;2R");
}
