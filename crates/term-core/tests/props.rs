//! Property tests: the terminal's invariants hold under random escape-code
//! streams, resizes, view scrolling and selections; scrollback rows round
//! trip; the scrollback keeps exactly the newest `limit` lines.
//!
//! Runs with a fixed seed so results are repeatable; set `PROPTEST_CASES`
//! to run more cases (the default is 256).

use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{Config, RngSeed};
use term_core::scrollback::Scrollback;
use term_core::{Cell, Color, Content, Row, RowRef, Selection, Style, Term, Width, flags};

fn config() -> Config {
    Config {
        rng_seed: RngSeed::Fixed(0xDEC1_14E5),
        failure_persistence: None,
        ..Config::default()
    }
}

// ---------------------------------------------------------------------------
// Generators for terminal input.

/// Printable ASCII.
fn ascii() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(0x20u8..0x7f, 1..40)
}

/// UTF-8 text weighted towards what the terminal treats specially: wide
/// characters, combining marks, VS15/VS16, ZWJ, emoji and zero-width chars,
/// in random order.
fn unicode() -> impl Strategy<Value = Vec<u8>> {
    let special = select(vec![
        'é',
        '漢',
        '字',
        '界',
        'ア',
        '\u{3000}',
        '😀',
        '👍',
        '⚠',
        '☺',
        '❤',
        '✔',
        '↔',
        '\u{0301}',
        '\u{0308}',
        '\u{FE0F}',
        '\u{FE0E}',
        '\u{200D}',
        '\u{200B}',
        '\u{00AD}',
        '\u{1F1FA}',
        '\u{1F1F8}',
        'Ω',
        '─',
        '\u{E0B0}',
        '\u{10FFFD}',
    ]);
    let ch = prop_oneof![4 => special, 1 => any::<char>()];
    prop::collection::vec(ch, 1..12).prop_map(|cs| cs.into_iter().collect::<String>().into_bytes())
}

/// C0 controls (and DEL), including a lone ESC.
fn c0() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop_oneof![0u8..0x20, Just(0x7f)], 1..4)
}

/// A CSI parameter: mostly small, sometimes a mode number, sometimes huge.
fn param() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => (0u32..10).prop_map(|n| n.to_string()),
        3 => select(vec![
            1u32, 2, 3, 4, 5, 6, 7, 12, 14, 16, 18, 20, 25, 38, 48, 58, 47, 1000, 1002, 1003, 1004, 1006, 1047, 1048, 1049, 2004, 2026, 2027,
        ])
        .prop_map(|n| n.to_string()),
        2 => (0u32..300).prop_map(|n| n.to_string()),
        1 => any::<u16>().prop_map(|n| n.to_string()),
        1 => Just(String::new()),
    ]
}

fn csi() -> impl Strategy<Value = Vec<u8>> {
    let prefix = select(vec!["", "", "", "?", "?", ">", "<", "="]);
    let params = prop::collection::vec((param(), prop_oneof![5 => Just(';'), 1 => Just(':')]), 0..7);
    let intermediate = select(vec!["", "", "", "", " ", "!", "$", "\""]);
    let finals = select("@ABCDEFGHIJKLMPSTXZ`abcdefghlmnpqrstux".chars().collect::<Vec<_>>());
    (prefix, params, intermediate, finals).prop_map(|(prefix, params, inter, fin)| {
        let mut s = format!("\x1b[{prefix}");
        for (i, (p, sep)) in params.iter().enumerate() {
            if i > 0 {
                s.push(*sep);
            }
            s.push_str(p);
        }
        s.push_str(inter);
        s.push(fin);
        s.into_bytes()
    })
}

/// SGR colours in all their forms, so pens get every colour kind.
fn sgr_colour() -> impl Strategy<Value = Vec<u8>> {
    (select(vec![38u8, 48, 58]), any::<u8>(), any::<(u8, u8, u8)>(), 0..4u8).prop_map(|(which, i, (r, g, b), form)| {
        match form {
            0 => format!("\x1b[{which};5;{i}m"),
            1 => format!("\x1b[{which};2;{r};{g};{b}m"),
            2 => format!("\x1b[{which}:2::{r}:{g}:{b}m"),
            _ => format!("\x1b[{}m", 30 + i % 80),
        }
        .into_bytes()
    })
}

fn osc() -> impl Strategy<Value = Vec<u8>> {
    let body = prop_oneof![
        Just("0;title".to_string()),
        Just("2;a;b".to_string()),
        Just("10;?".to_string()),
        Just("11;?".to_string()),
        Just("12;?".to_string()),
        Just("52;c;aGVsbG8=".to_string()),
        Just("52;c;!!".to_string()),
        Just("52;c;?".to_string()),
        Just("7;file://host/tmp/a%20b".to_string()),
        Just("7;file://x".to_string()),
        Just("8;;https://example.com".to_string()),
        prop::collection::vec(0x20u8..0x7f, 0..20).prop_map(|b| String::from_utf8(b).unwrap()),
    ];
    let end = select(vec!["\x07", "\x1b\\", ""]);
    (body, end).prop_map(|(b, e)| format!("\x1b]{b}{e}").into_bytes())
}

fn esc() -> impl Strategy<Value = Vec<u8>> {
    select(vec![
        "7", "8", "D", "E", "H", "M", "c", "=", ">", "(0", "(B", ")0", ")B", "(A", "#8", "N", "O", "Z", "\\", "%G",
    ])
    .prop_map(|s| format!("\x1b{s}").into_bytes())
}

fn dcs() -> impl Strategy<Value = Vec<u8>> {
    select(vec![
        "\x1bP$qm\x1b\\",
        "\x1bP$q r\x1b\\",
        "\x1bP+q544e\x1b\\",
        "\x1bP1;2|17/ab\x1b\\",
        "\x1bPq#0;2;0;0;0#1!10~\x1b\\",
        "\x1bP=1s\x1b\\",
        "\x1bPunterminated",
    ])
    .prop_map(|s| s.as_bytes().to_vec())
}

/// One chunk of program output, made of several pieces.
/// Clusters the terminal handles specially, written whole: emoji
/// presentation (VS16), ZWJ sequences, flags, combining marks, wide runs.
fn cluster() -> impl Strategy<Value = Vec<u8>> {
    select(vec![
        "⚠\u{FE0F}",
        "❤\u{FE0F}",
        "↔\u{FE0F}",
        "☺\u{FE0F}",
        "a\u{FE0F}",
        "漢\u{FE0F}",
        "⚠\u{FE0E}",
        "👨\u{200D}👩\u{200D}👧",
        "🇺🇸",
        "e\u{0301}",
        "漢字界",
        "ｱｲ",
        "😀😀",
    ])
    .prop_map(|s| s.as_bytes().to_vec())
}

/// Cursor movement, so later output lands on top of earlier output.
fn movement() -> impl Strategy<Value = Vec<u8>> {
    let n = 0u16..12;
    prop_oneof![
        Just(b"\r".to_vec()),
        Just(b"\x08".to_vec()),
        Just(b"\n".to_vec()),
        Just(b"\t".to_vec()),
        n.clone().prop_map(|n| format!("\x1b[{n}D").into_bytes()),
        n.clone().prop_map(|n| format!("\x1b[{n}C").into_bytes()),
        n.clone().prop_map(|n| format!("\x1b[{n}G").into_bytes()),
        n.clone().prop_map(|n| format!("\x1b[{n}A").into_bytes()),
        (n.clone(), n).prop_map(|(r, c)| format!("\x1b[{r};{c}H").into_bytes()),
    ]
}

fn chunk() -> impl Strategy<Value = Vec<u8>> {
    let piece = prop_oneof![
        5 => ascii(),
        3 => unicode(),
        3 => cluster(),
        4 => movement(),
        3 => c0(),
        5 => csi(),
        1 => sgr_colour(),
        1 => osc(),
        2 => esc(),
        1 => dcs(),
        1 => prop::collection::vec(any::<u8>(), 1..16),
    ];
    prop::collection::vec(piece, 1..10).prop_map(|ps| ps.concat())
}

#[derive(Clone, Debug)]
enum Step {
    Output(Vec<u8>),
    Resize(usize, usize),
    ScrollView(isize),
    /// Visible (line, col) of anchor and head, reduced to the screen size.
    Select((usize, usize), (usize, usize)),
    ClearSelection,
    /// A double- or triple-click (1 or 2) at a visible (line, col), or a
    /// drag (0) to it.
    Gesture(u8, (usize, usize)),
    SelectAll,
    /// Leave the view where it is, through `view_top` and `set_view_top`.
    KeepView,
}

fn step() -> impl Strategy<Value = Step> {
    let point = (0usize..100, 0usize..250);
    prop_oneof![
        12 => chunk().prop_map(Step::Output),
        1 => (1usize..=200, 1usize..=60).prop_map(|(c, l)| Step::Resize(c, l)),
        2 => (-30isize..=30).prop_map(Step::ScrollView),
        2 => (point.clone(), point.clone()).prop_map(|(a, b)| Step::Select(a, b)),
        1 => Just(Step::ClearSelection),
        2 => (0u8..3, point).prop_map(|(k, p)| Step::Gesture(k, p)),
        1 => Just(Step::SelectAll),
        1 => Just(Step::KeepView),
    ]
}

fn apply(t: &mut Term, step: &Step) {
    match step {
        Step::Output(bytes) => t.advance(bytes),
        Step::Resize(c, l) => t.state.resize(*c, *l),
        Step::ScrollView(d) => t.state.scroll_view(*d),
        Step::Select(a, b) => {
            let (lines, cols) = (t.state.lines(), t.state.cols());
            let anchor = t.state.point_at(a.0 % lines, a.1 % cols);
            let head = t.state.point_at(b.0 % lines, b.1 % cols);
            t.state.set_selection(Some(Selection::new(anchor, head)));
        }
        Step::ClearSelection => t.state.set_selection(None),
        Step::Gesture(kind, (l, c)) => {
            let p = t.state.point_at(l % t.state.lines(), c % t.state.cols());
            match kind {
                0 => t.state.extend_selection(p),
                1 => t.state.select_word(p),
                _ => t.state.select_line(p),
            }
            let sel = t.state.selection().expect("a gesture leaves a selection");
            if *kind > 0 {
                assert!(sel.contains(p), "{sel:?} doesn't contain the click at {p:?}");
            }
        }
        Step::SelectAll => t.state.select_all(),
        Step::KeepView => {
            let (offset, top) = (t.state.display_offset(), t.state.view_top());
            t.state.set_view_top(top);
            assert_eq!(t.state.display_offset(), offset, "{top:?}");
        }
    }
}

/// Everything a reader of the terminal may call must work in any state.
fn check(t: &Term) -> Result<(), TestCaseError> {
    t.state.check_invariants().map_err(TestCaseError::fail)?;
    for line in 0..t.state.lines() {
        let _ = t.state.visible_row(line);
        let _ = t.state.point_at(line, 0);
    }
    let _ = t.state.selection_text();
    let dump = t.state.dump();
    prop_assert_eq!(dump.lines().count(), t.state.lines() + 1);
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn invariants_hold_under_random_input(
        cols in 1usize..=120,
        lines in 1usize..=40,
        limit in select(vec![0usize, 1, 2, 10, 1000]),
        steps in prop::collection::vec(step(), 1..40),
    ) {
        let mut t = Term::new(cols, lines, limit);
        for step in &steps {
            apply(&mut t, step);
            check(&t)?;
        }
    }

    /// The same input split into chunks at arbitrary points gives the same
    /// screen: the parser carries its state across `advance` calls.
    #[test]
    fn chunk_boundaries_do_not_matter(
        input in prop::collection::vec(chunk(), 1..8).prop_map(|c| c.concat()),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..6),
    ) {
        let mut whole = Term::new(20, 6, 50);
        whole.advance(&input);
        let mut parts = Term::new(20, 6, 50);
        let mut at: Vec<usize> = cuts.iter().map(|i| i.index(input.len() + 1)).collect();
        at.sort();
        let mut from = 0;
        for to in at.into_iter().chain([input.len()]) {
            parts.advance(&input[from..to]);
            from = to;
        }
        prop_assert_eq!(whole.state.dump(), parts.state.dump());
        prop_assert_eq!(&whole.state.replies(), &parts.state.replies());
    }
}

// ---------------------------------------------------------------------------
// The bulk ASCII path against one character at a time.

/// Input that steers printable-ASCII runs into the cases the bulk path must
/// get right: runs that wrap (often several times), runs over wide
/// characters, insert mode, DEC line drawing, autowrap off, scroll regions,
/// the alternate screen, repeat (REP) and history.
fn bulk_piece() -> impl Strategy<Value = Vec<u8>> {
    let modes = select(vec![
        "\x1b[4h",
        "\x1b[4l",
        "\x1b(0",
        "\x1b(B",
        "\x1b)0",
        "\x1b)B",
        "\x0e",
        "\x0f",
        "\x1b[?7l",
        "\x1b[?7h",
        "\x1b[?6h",
        "\x1b[?6l",
        "\x1b[?1049h",
        "\x1b[?1049l",
        "\x1b[?47h",
        "\x1b[?47l",
        "\x1b7",
        "\x1b8",
        "\x1bM",
        "\x1bD",
        "\x1bE",
        "\x1b[L",
        "\x1b[2M",
        "\x1b[2@",
        "\x1b[3P",
        "\x1b[5X",
        "\x1b[K",
        "\x1b[1K",
        "\x1b[2J",
        "\x1b[r",
        "\x1b[S",
        "\x1b[2T",
        "\x1b[!p",
        "\x1bc",
        "\x1b[1m",
        "\x1b[7m",
        "\x1b[4m",
        "\x1b[0m",
        "\x1b[6n",
        "\x1b[?25$p",
        "\x1b[3J",
    ]);
    prop_oneof![
        8 => prop::collection::vec(0x20u8..0x7f, 1..300),
        3 => ascii(),
        4 => unicode(),
        3 => cluster(),
        4 => movement(),
        2 => c0(),
        5 => modes.prop_map(|s| s.as_bytes().to_vec()),
        2 => (0u16..14, 0u16..14).prop_map(|(t, b)| format!("\x1b[{t};{b}r").into_bytes()),
        1 => (0u16..30).prop_map(|n| format!("\x1b[{n}b").into_bytes()),
        2 => sgr_colour(),
        1 => csi(),
        1 => prop::collection::vec(any::<u8>(), 1..16),
    ]
}

#[derive(Clone, Debug)]
enum BulkStep {
    Output(Vec<u8>),
    Resize(usize, usize),
    ScrollView(isize),
}

fn bulk_step() -> impl Strategy<Value = BulkStep> {
    prop_oneof![
        14 => prop::collection::vec(bulk_piece(), 1..12).prop_map(|ps| BulkStep::Output(ps.concat())),
        1 => (1usize..=90, 1usize..=14).prop_map(|(c, l)| BulkStep::Resize(c, l)),
        1 => (-20isize..=20).prop_map(BulkStep::ScrollView),
    ]
}

/// Dump, rows cell by cell, cursor (row, col, pending wrap, pen), replies
/// and scrollback length.
type ScreenState = (String, Vec<(Vec<CellState>, bool)>, (usize, usize, bool, Style), Vec<u8>, usize);

/// One cell as a reader sees it: empty or not, character, width, style.
type CellState = (bool, char, Width, Style);

fn row_state(r: &RowRef) -> (Vec<CellState>, bool) {
    let cells = r
        .cells()
        .map(|c| (c.content() == Content::Empty, c.ch(), c.width(), *c.style()))
        .collect();
    (cells, r.wrapped())
}

/// Everything about the screen a reader can see, cell by cell.
fn screen_state(t: &Term) -> ScreenState {
    let rows = (0..t.state.lines()).map(|l| row_state(&t.state.visible_row(l))).collect();
    let c = t.state.cursor();
    (
        t.state.dump(),
        rows,
        (c.row(), c.col(), c.pending_wrap(), c.pen()),
        t.state.replies().to_vec(),
        t.state.scrollback_len(),
    )
}

/// Every stored history line, read by scrolling the view through it.
fn history(t: &mut Term) -> Vec<(Vec<CellState>, bool)> {
    let mut out = Vec::new();
    t.state.scroll_view(isize::MAX / 2);
    loop {
        out.push(row_state(&t.state.visible_row(0)));
        if t.state.display_offset() == 0 {
            break;
        }
        t.state.scroll_view(-1);
    }
    out
}

proptest! {
    #![proptest_config(config())]

    /// Printable ASCII written in bulk gives exactly what writing it one
    /// character at a time gives. The reference also gets its input one byte
    /// per `advance` call, so the parser's fast path is checked against its
    /// byte-at-a-time handling at the same time.
    #[test]
    fn bulk_ascii_matches_one_char_at_a_time(
        cols in 1usize..=90,
        lines in 1usize..=14,
        limit in select(vec![0usize, 3, 100]),
        steps in prop::collection::vec(bulk_step(), 1..16),
    ) {
        let mut fast = Term::new(cols, lines, limit);
        let mut slow = Term::new(cols, lines, limit);
        slow.state.set_bulk_ascii(false);
        for step in &steps {
            match step {
                BulkStep::Output(bytes) => {
                    fast.advance(bytes);
                    for b in bytes {
                        slow.advance(std::slice::from_ref(b));
                    }
                }
                BulkStep::Resize(c, l) => {
                    fast.state.resize(*c, *l);
                    slow.state.resize(*c, *l);
                }
                BulkStep::ScrollView(d) => {
                    fast.state.scroll_view(*d);
                    slow.state.scroll_view(*d);
                }
            }
            prop_assert_eq!(screen_state(&fast), screen_state(&slow), "after {:?}", step);
        }
        prop_assert_eq!(history(&mut fast), history(&mut slow));
    }
}

// ---------------------------------------------------------------------------
// Scrollback.

fn colour() -> impl Strategy<Value = Color> {
    prop_oneof![
        Just(Color::DEFAULT),
        Just(Color::indexed(0)),
        Just(Color::indexed(255)),
        any::<u8>().prop_map(Color::indexed),
        Just(Color::rgb(0, 0, 0)),
        Just(Color::rgb(255, 255, 255)),
        any::<(u8, u8, u8)>().prop_map(|(r, g, b)| Color::rgb(r, g, b)),
    ]
}

fn style() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), any::<u32>().prop_map(|b| b & flags::STYLE_MASK)]
}

fn codepoint() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), Just(' ' as u32), (0x21u32..0x7f), any::<char>().prop_map(|c| c as u32),]
}

/// One narrow cell, or a wide cell and its spacer (which always share the
/// wide cell's colours and style, as the terminal writes them).
fn cells() -> impl Strategy<Value = Vec<Cell>> {
    (any::<bool>(), codepoint(), style(), colour(), colour()).prop_map(|(wide, cp, style, fg, bg)| {
        if wide {
            vec![
                Cell {
                    packed: cp | style | flags::WIDE,
                    fg,
                    bg,
                },
                Cell {
                    packed: style | flags::WIDE_SPACER,
                    fg,
                    bg,
                },
            ]
        } else {
            vec![Cell {
                packed: cp | style,
                fg,
                bg,
            }]
        }
    })
}

fn row() -> impl Strategy<Value = Row> {
    (prop::collection::vec(cells(), 0..60), any::<bool>()).prop_map(|(cells, wrapped)| Row {
        cells: cells.concat(),
        wrapped,
    })
}

/// What a row is expected to read back as: spaces stored as empty cells,
/// and trailing blank cells dropped (but never half of a wide character).
fn normalise(row: &Row) -> (Vec<Cell>, bool) {
    let mut cells: Vec<Cell> = row
        .cells
        .iter()
        .map(|c| {
            let mut c = *c;
            if c.codepoint() == ' ' as u32 {
                c.packed &= !flags::CHAR_MASK;
            }
            c
        })
        .collect();
    let mut len = cells.iter().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
    if len > 0 && len < cells.len() && cells[len - 1].has(flags::WIDE) {
        len += 1;
    }
    cells.truncate(len);
    (cells, row.wrapped)
}

fn numbered_row(i: usize) -> Row {
    let text = format!("line {i} {}", "x".repeat(60));
    let mut row = Row::new(80, Cell::BLANK);
    for (cell, ch) in row.cells.iter_mut().zip(text.chars()) {
        cell.packed = ch as u32;
    }
    row
}

fn number_of(row: &Row) -> usize {
    let text: String = row.cells.iter().map(|c| c.ch()).collect();
    text.split(' ').nth(1).and_then(|n| n.parse().ok()).expect("numbered line")
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn scrollback_rows_round_trip(rows in prop::collection::vec(row(), 1..8)) {
        let mut sb = Scrollback::new(100);
        for r in &rows {
            sb.push(r);
        }
        for (i, r) in rows.iter().enumerate() {
            let back = sb.get(i).expect("stored row");
            prop_assert_eq!(normalise(&back), normalise(r));
        }
    }

    #[test]
    fn scrollback_keeps_the_newest_limit_lines(
        limit in prop_oneof![Just(0usize), Just(1), Just(2), 0usize..50, 500usize..1500],
        pushed in prop_oneof![0usize..10, 0usize..4000],
        probes in prop::collection::vec(any::<prop::sample::Index>(), 20),
    ) {
        let mut sb = Scrollback::new(limit);
        for i in 0..pushed {
            sb.push(&numbered_row(i));
        }
        let len = pushed.min(limit);
        prop_assert_eq!(sb.len(), len);
        prop_assert_eq!(sb.is_empty(), len == 0);
        prop_assert!(sb.get(len).is_none());
        if len > 0 {
            let first = pushed - len;
            let mut idx: Vec<usize> = probes.iter().map(|p| p.index(len)).collect();
            idx.extend([0, len - 1]);
            for i in idx {
                prop_assert_eq!(number_of(&sb.get(i).unwrap()), first + i);
            }
        }
    }
}
