//! Property tests: the terminal's invariants hold under random escape-code
//! streams, resizes, view scrolling and selections; scrollback rows round
//! trip; the scrollback keeps exactly the newest `limit` lines.
//!
//! Runs with a fixed seed so results are repeatable; set `PROPTEST_CASES`
//! to run more cases (the default is 256).

use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{Config, RngSeed};
use std::collections::HashMap;
use term_core::scrollback::Scrollback;
use term_core::term::GraphemeState;

use term_core::{
    Attrs, Cell, Color, Content, LinkValue, Mark, Row, RowFlags, RowKey, RowRef, ScreenId, Selection, Snapshot, Style, Term, Underline,
    Width,
};

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
        Just("8;id=a;https://example.com/a;b".to_string()),
        Just("8;id=b;file:///tmp".to_string()),
        Just("8;;".to_string()),
        Just("133;A".to_string()),
        Just("133;A;k=s".to_string()),
        Just("133;B".to_string()),
        Just("133;C".to_string()),
        Just("133;D;0".to_string()),
        Just("133;D;1".to_string()),
        Just("133;D".to_string()),
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
        "1\u{FE0F}\u{20E3}",
        "#\u{20E3}",
        "1\u{FE0F}",
        "👍\u{1F3FD}",
        "😀\u{FE0E}",
        "⌚\u{FE0E}",
        "🇬🇧🇺",
        "\u{0915}\u{094D}\u{0937}",
        "e\u{0301}\u{0308}\u{0301}\u{0308}",
        "\u{0600}1",
        "\u{1100}\u{1161}\u{11A8}",
        "§±①",
        "❤\u{200D}🔥",
        "\u{1F600}\u{1F3FD}\u{1F3FD}",
        "a\u{0301}\x1b[3b",
        "👨\u{200D}👩\x1b[2b",
        "\x1b[?2027l",
        "\x1b[?2027h",
        "\x1b[?2027$p",
        // kitty placeholders (images are on in
        // `invariants_hold_under_random_input`).
        "\u{10EEEE}\u{305}\u{30D}",
        "\u{10EEEE}\u{10EEEE}",
        "\u{10EEEE}\u{30E}\u{301}\u{305}\u{1D244}\u{310}",
        "\x1b[38;2;0;0;7m\u{10EEEE}\u{30E}\u{305}\u{310}\u{10EEEE}",
        "\x1b[38;5;9;58;5;3m\u{10EEEE}\u{1D244}\u{10EEEE}",
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

/// kitty graphics commands (images.md §3): transmits (raw, chunked,
/// id-less, broken), places, deletes of every kind and queries, with
/// small images so many fit.
fn graphics() -> impl Strategy<Value = Vec<u8>> {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let id = prop_oneof![
        Just(String::new()),
        (1u32..5).prop_map(|i| format!(",i={i}")),
        (1u32..3).prop_map(|n| format!(",I={n}"))
    ];
    let action = select(vec!["a=t", "a=T", "a=p", "a=q", "a=d", "a=T,U=1", "a=T,C=1", "a=p,p=2", "a=f"]);
    let delete = select(vec![
        "",
        ",d=a",
        ",d=A",
        ",d=i",
        ",d=I",
        ",d=c",
        ",d=p,x=2,y=2",
        ",d=x,x=1",
        ",d=y,y=3",
        ",d=z,z=0",
        ",d=r,x=1,y=3",
        ",d=N",
    ]);
    let size = (1u32..40, 1u32..60, select(vec![24u32, 32]));
    let more = select(vec!["", ",m=1", ",q=1", ",q=2"]);
    let payload =
        prop::collection::vec(any::<u8>(), 0..40).prop_map(|b| b.iter().map(|&x| B64[(x & 63) as usize] as char).collect::<String>());
    let end = select(vec!["\x1b\\", "\x18", "\x1b\\\x1b[c"]);
    (id, action, delete, size, more, payload, end).prop_map(|(id, action, delete, (w, h, f), more, payload, end)| {
        let delete = if action == "a=d" { delete } else { "" };
        format!("\x1b_G{action}{id}{delete},f={f},s={w},v={h}{more};{payload}{end}").into_bytes()
    })
}

fn chunk() -> impl Strategy<Value = Vec<u8>> {
    let piece = prop_oneof![
        2 => graphics(),
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
    /// Change the ambiguous-width setting.
    Ambiguous(u8),
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
        1 => (1u8..=2).prop_map(Step::Ambiguous),
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
        Step::Ambiguous(w) => {
            // Cells already written keep their width (§17.3, 9).
            let before: Vec<String> = (0..t.state.lines())
                .map(|l| format!("{:?}", row_state(&t.state.visible_row(l))))
                .collect();
            t.state.set_ambiguous_width(*w);
            let after: Vec<String> = (0..t.state.lines())
                .map(|l| format!("{:?}", row_state(&t.state.visible_row(l))))
                .collect();
            assert_eq!(before, after, "changing the ambiguous width changed the screen");
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
        // Images on, so placeholders are cells.
        let mut t = Term::new(cols, lines, limit, term_core::ImagePool::new(32 << 20));
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
        let mut whole = Term::new(20, 6, 50, term_core::ImagePool::process());
        whole.advance(&input);
        let mut parts = Term::new(20, 6, 50, term_core::ImagePool::process());
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

proptest! {
    #![proptest_config(config())]

    /// Graphics streams with text, scrolling, resizes, ED and RIS, and
    /// snapshots: the image maps agree, every placement's image exists,
    /// and the pool counts what its entries hold, within the budget.
    #[test]
    fn graphics_streams_keep_the_image_invariants(
        steps in prop::collection::vec(prop_oneof![
            6 => graphics().prop_map(Step::Output),
            4 => chunk().prop_map(Step::Output),
            1 => select(vec![&b"\x1b[2J"[..], b"\x1b[3J", b"\x1bc", b"\x1b[?1049h", b"\x1b[?1049l", b"\x1b[2;4r\n\n\n\x1b[r", b"\n\n\n\n\n\n\n\n"])
                .prop_map(|b| Step::Output(b.to_vec())),
            1 => (1usize..=60, 1usize..=20).prop_map(|(c, l)| Step::Resize(c, l)),
            1 => (-10isize..=10).prop_map(Step::ScrollView),
        ], 1..40),
    ) {
        let pool = term_core::ImagePool::new(256 << 10);
        let mut t = Term::new(20, 6, 30, pool.clone());
        let mut snap = Snapshot::default();
        for step in &steps {
            apply(&mut t, step);
            check(&t)?;
            pool.check().map_err(TestCaseError::fail)?;
            prop_assert!(pool.used() <= pool.limit(), "pool holds {} of {}", pool.used(), pool.limit());
            t.state.snapshot(&mut snap, |_| false);
            for p in &snap.placements {
                prop_assert!(p.view_row + (p.rows as i32) > 0 && p.view_row < t.state.lines() as i32);
            }
        }
        drop(t);
        prop_assert_eq!(pool.used(), 0, "a dropped terminal gives every byte back");
    }
}

// ---------------------------------------------------------------------------
// The bulk ASCII path against one character at a time.

/// Input that steers printable-ASCII runs into the cases the bulk path must
/// get right: runs that wrap (often several times), runs over wide
/// characters, insert mode, DEC line drawing, autowrap off, scroll regions,
/// the alternate screen, repeat (REP) and history; and ASCII followed by a
/// combining mark, VS16 or U+20E3, which must join the run's last cell
/// (the grapheme state is compared too).
fn bulk_piece() -> impl Strategy<Value = Vec<u8>> {
    let joining = select(vec![
        "e\u{0301}",
        "\u{0301}",
        "\u{0308}\u{0301}",
        "\u{FE0F}",
        "\u{FE0E}",
        "\u{20E3}",
        "\u{FE0F}\u{20E3}",
        "1\u{FE0F}\u{20E3}",
        "#\u{20E3}",
        "a\u{FE0F}",
        "\u{200D}",
        "\u{0600}",
        "\u{0600}1",
        "\u{093F}",
        "\u{1F1EC}",
        "\u{1F3FD}",
        "\u{094D}\u{0937}",
        "\u{1161}",
    ]);
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
        "\x1b[?2027l",
        "\x1b[?2027h",
    ]);
    prop_oneof![
        8 => prop::collection::vec(0x20u8..0x7f, 1..300),
        3 => ascii(),
        4 => unicode(),
        3 => cluster(),
        5 => joining.prop_map(|s| s.as_bytes().to_vec()),
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
    Ambiguous(u8),
}

fn bulk_step() -> impl Strategy<Value = BulkStep> {
    prop_oneof![
        14 => prop::collection::vec(bulk_piece(), 1..12).prop_map(|ps| BulkStep::Output(ps.concat())),
        1 => (1usize..=90, 1usize..=14).prop_map(|(c, l)| BulkStep::Resize(c, l)),
        1 => (-20isize..=20).prop_map(BulkStep::ScrollView),
        1 => (1u8..=2).prop_map(BulkStep::Ambiguous),
    ]
}

/// Dump, rows cell by cell, cursor (row, col, pending wrap, pen and its
/// link), replies, scrollback length, grapheme state and each screen
/// line's prompt marks.
type ScreenState = (
    String,
    Vec<(Vec<ValueCell>, bool)>,
    (usize, usize, bool, Style, Option<LinkValue>),
    Vec<u8>,
    usize,
    GraphemeState,
    Vec<Vec<Mark>>,
);

/// One cell as a reader sees it: empty or not, its text (a cluster's
/// whole), width, style.
type CellState = (bool, String, Width, Style);

/// A cell by value: link ids differ between terminals (one sweeps more
/// often), so the style's link id is left out and the link's value put in.
type ValueCell = (bool, String, Width, Style, Option<LinkValue>);

fn row_value(r: &RowRef) -> (Vec<ValueCell>, bool) {
    let cells = r
        .cells()
        .enumerate()
        .map(|(col, c)| {
            let mut text = String::new();
            c.push_text(&mut text);
            let style = Style { link: 0, ..c.style() };
            (c.content() == Content::Empty, text, c.width(), style, r.link(col).cloned())
        })
        .collect();
    (cells, r.wrapped())
}

fn row_state(r: &RowRef) -> (Vec<CellState>, bool) {
    let cells = r
        .cells()
        .map(|c| {
            let mut text = String::new();
            c.push_text(&mut text);
            (c.content() == Content::Empty, text, c.width(), c.style())
        })
        .collect();
    (cells, r.wrapped())
}

/// Everything about the screen a reader can see, cell by cell.
fn screen_state(t: &Term) -> ScreenState {
    let rows = (0..t.state.lines()).map(|l| row_value(&t.state.visible_row(l))).collect();
    let c = t.state.cursor();
    let top = t.state.point_at(t.state.display_offset(), 0).line;
    let marks = (top..top + t.state.lines() as u64).map(|l| t.state.marks_at(l)).collect();
    (
        t.state.dump(),
        rows,
        (
            c.row(),
            c.col(),
            c.pending_wrap(),
            Style { link: 0, ..c.pen() },
            t.state.pen_link().cloned(),
        ),
        t.state.replies().to_vec(),
        t.state.scrollback_len(),
        t.state.grapheme_state(),
        marks,
    )
}

/// Every stored history line, read by scrolling the view through it.
fn history(t: &mut Term) -> Vec<(Vec<ValueCell>, bool)> {
    let mut out = Vec::new();
    t.state.scroll_view(isize::MAX / 2);
    loop {
        out.push(row_value(&t.state.visible_row(0)));
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
    /// character at a time gives, grapheme state included. The reference
    /// also gets its input one byte per `advance` call, so the parser's fast
    /// path is checked against its byte-at-a-time handling at the same time.
    #[test]
    fn bulk_ascii_matches_one_char_at_a_time(
        cols in 1usize..=90,
        lines in 1usize..=14,
        limit in select(vec![0usize, 3, 100]),
        steps in prop::collection::vec(bulk_step(), 1..16),
    ) {
        let mut fast = Term::new(cols, lines, limit, term_core::ImagePool::process());
        let mut slow = Term::new(cols, lines, limit, term_core::ImagePool::process());
        slow.state.set_bulk_ascii(false);
        // Recording clusters makes the reference keep its grapheme state
        // up to date after every character, where the fast one leaves the
        // last plain character to be worked out when it is needed.
        slow.state.record_graphemes(true);
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
                BulkStep::Ambiguous(w) => {
                    fast.state.set_ambiguous_width(*w);
                    slow.state.set_ambiguous_width(*w);
                }
            }
            prop_assert_eq!(screen_state(&fast), screen_state(&slow), "after {:?}", step);
            slow.state.take_graphemes();
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

/// Every attribute SGR can set.
const SGR_ATTRS: u16 = Attrs::BOLD.bits()
    | Attrs::DIM.bits()
    | Attrs::ITALIC.bits()
    | Attrs::OVERLINE.bits()
    | Attrs::STRIKE.bits()
    | Attrs::INVERSE.bits()
    | Attrs::HIDDEN.bits()
    | Attrs::BLINK.bits();

/// A style as the screen can hold it (no link yet).
fn style() -> impl Strategy<Value = Style> {
    let underline = select(vec![
        Underline::None,
        Underline::Single,
        Underline::Double,
        Underline::Curly,
        Underline::Dotted,
        Underline::Dashed,
    ]);
    (colour(), colour(), colour(), any::<u16>(), underline).prop_map(|(fg, bg, underline_color, attrs, underline)| Style {
        fg,
        bg,
        underline_color,
        attrs: Attrs::from_bits(attrs & SGR_ATTRS),
        underline,
        ..Style::DEFAULT
    })
}

/// A row's style list: the default style first, as a table has it.
fn styles() -> impl Strategy<Value = Vec<Style>> {
    prop::collection::vec(prop_oneof![1 => Just(Style::DEFAULT), 4 => style()], 0..8).prop_map(|mut s| {
        s.insert(0, Style::DEFAULT);
        s
    })
}

fn codepoint() -> impl Strategy<Value = u32> {
    // Never a C0 control: those are executed, not printed, and history's
    // text relies on it.
    prop_oneof![
        Just(0u32),
        Just(' ' as u32),
        (0x21u32..0x7f),
        any::<char>().prop_map(|c| c as u32).prop_filter("C0", |&c| c >= 0x20),
    ]
}

/// One narrow cell, or a wide cell and its spacer (which always share the
/// wide cell's style, as the terminal writes them), in style `id` of
/// `styles`.
fn cells(styles: Vec<Style>) -> impl Strategy<Value = Vec<Cell>> {
    let n = styles.len();
    (any::<bool>(), codepoint(), 0..n).prop_map(move |(wide, cp, id)| {
        let t = Cell::template(id as u32, styles[id].shows_empty());
        let c = Cell::from_bits(t.bits() | cp as u64);
        if wide {
            vec![c.with_width(Width::Wide), t.with_width(Width::Spacer)]
        } else {
            vec![c]
        }
    })
}

fn row() -> impl Strategy<Value = (Row, Vec<Style>)> {
    styles().prop_flat_map(|styles| {
        (prop::collection::vec(cells(styles.clone()), 0..60), any::<bool>(), Just(styles)).prop_map(|(cells, wrapped, styles)| {
            let mut row = Row::new(0, Cell::BLANK);
            row.cells = cells.concat();
            row.wrapped = wrapped;
            (row, styles)
        })
    })
}

/// What a row is expected to read back as, cell by cell (content, width,
/// style by value): spaces stored as empty cells, and trailing blank cells
/// dropped (but never half of a wide character).
fn normalise(row: &Row, styles: &[Style]) -> (Vec<(u32, Width, Style)>, bool) {
    let spaces_empty: Vec<Cell> = row
        .cells
        .iter()
        .map(|&c| {
            if c.codepoint() == ' ' as u32 {
                c.style_part().with_width(c.width())
            } else {
                c
            }
        })
        .collect();
    let mut len = spaces_empty.iter().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
    if len > 0 && len < spaces_empty.len() && spaces_empty[len - 1].is_wide() {
        len += 1;
    }
    let cells = spaces_empty[..len]
        .iter()
        .map(|c| (c.codepoint(), c.width(), styles[c.style_id() as usize]))
        .collect();
    (cells, row.wrapped)
}

fn numbered_row(i: usize) -> Row {
    let text = format!("line {i} {}", "x".repeat(60));
    let mut row = Row::new(80, Cell::BLANK);
    for (cell, ch) in row.cells.iter_mut().zip(text.chars()) {
        *cell = Cell::BLANK.with_char(ch);
    }
    row
}

fn number_of(row: &Row) -> usize {
    let text: String = row.cells.iter().map(|c| c.ch()).collect();
    text.split(' ').nth(1).and_then(|n| n.parse().ok()).expect("numbered line")
}

proptest! {
    #![proptest_config(config())]

    /// History stores styles by value: every cell reads back with the
    /// same style, whatever ids the screen gave it.
    #[test]
    fn scrollback_rows_round_trip(rows in prop::collection::vec(row(), 1..8)) {
        let mut sb = Scrollback::new(100);
        for (r, styles) in &rows {
            sb.push(r, styles);
        }
        for (i, (r, styles)) in rows.iter().enumerate() {
            let (back, back_styles) = sb.get(i).expect("stored row");
            prop_assert_eq!(normalise(&back, &back_styles), normalise(r, styles));
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
            sb.push(&numbered_row(i), &[Style::DEFAULT]);
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
                prop_assert_eq!(number_of(&sb.get(i).unwrap().0), first + i);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Style tables with forced sweeps (§5.2, §17.3).

/// SGR that sets every stored style field: colours in every form,
/// underline styles and colours, overline, double underline, resets.
fn sgr_style() -> impl Strategy<Value = Vec<u8>> {
    let simple = select(vec![
        "0", "1", "2", "3", "4", "4:0", "4:1", "4:2", "4:3", "4:4", "4:5", "5", "7", "8", "9", "21", "22", "23", "24", "25", "27", "28",
        "29", "39", "49", "53", "55", "59", "",
    ])
    .prop_map(|s| format!("\x1b[{s}m").into_bytes());
    prop_oneof![
        3 => simple,
        3 => sgr_colour(),
        // A few colours only, so styles repeat and ids are reused.
        2 => (select(vec![31u8, 32, 41, 42, 90, 100]), select(vec![1u8, 3, 4, 53]))
            .prop_map(|(c, a)| format!("\x1b[{c};{a}m").into_bytes()),
    ]
}

/// What makes ids change hands: styled text, erases in a coloured pen,
/// DECSC/DECRC (and 1048), screen switches, scrolls, RIS.
fn style_piece() -> impl Strategy<Value = Vec<u8>> {
    let holders = select(vec![
        "\x1b7",
        "\x1b8",
        "\x1b[s",
        "\x1b[u",
        "\x1b[?1048h",
        "\x1b[?1048l",
        "\x1b[?1049h",
        "\x1b[?1049l",
        "\x1b[?1047h",
        "\x1b[?1047l",
        "\x1b[?47h",
        "\x1b[?47l",
        "\x1bc",
        "\x1b[!p",
        "\x1b[K",
        "\x1b[2J",
        "\x1b[J",
        "\x1b[3X",
        "\x1b[2@",
        "\x1b[2P",
        "\x1b[L",
        "\x1b[M",
        "\x1b[S",
        "\x1b[T",
        "\x1bM",
        "\x1b#8",
        "\x1b[2;5r",
        "\x1b[r",
        "\x1b[4h",
        "\x1b[4l",
    ])
    .prop_map(|s| s.as_bytes().to_vec());
    // Links come and go: a few repeat, so pieces share ids, and each new
    // number makes a new link for the sweep to free.
    let links = prop_oneof![
        2 => select(vec!["https://a", "https://b;c", "file:///x"]).prop_map(|u| format!("\x1b]8;;{u}\x1b\\").into_bytes()),
        1 => (0u16..40).prop_map(|n| format!("\x1b]8;id={n};https://n/{n}\x07").into_bytes()),
        2 => Just(b"\x1b]8;;\x1b\\".to_vec()),
        1 => select(vec!["A", "B", "C", "D;0", "D;2"]).prop_map(|k| format!("\x1b]133;{k}\x07").into_bytes()),
    ];
    prop_oneof![
        6 => sgr_style(),
        3 => links,
        5 => prop::collection::vec(0x20u8..0x7f, 1..30),

        2 => unicode(),
        1 => cluster(),
        4 => holders,
        3 => movement(),
        1 => c0(),
    ]
}

#[derive(Clone, Debug)]
enum StyleStep {
    Output(Vec<u8>),
    Resize(usize, usize),
    /// Sweep now and check exactly the used ids are left.
    Sweep,
    /// Take a snapshot into the renderer's mirrors and row cache.
    Snapshot,
    /// Scroll the view (into history and back).
    View(isize),
    /// Pull history lines back onto the screen (their line numbers are
    /// handed out again).
    PullBack(usize),
    /// Change the scrollback limit.
    Limit(usize),
    /// Change the ambiguous-width setting (seals history's open block).
    Ambiguous(u8),
}

fn style_step() -> impl Strategy<Value = StyleStep> {
    prop_oneof![
        14 => prop::collection::vec(style_piece(), 1..12).prop_map(|ps| StyleStep::Output(ps.concat())),
        2 => (1usize..=40, 1usize..=12).prop_map(|(c, l)| StyleStep::Resize(c, l)),
        1 => Just(StyleStep::Sweep),
        4 => Just(StyleStep::Snapshot),
        1 => (-6isize..=6).prop_map(StyleStep::View),
        1 => (1usize..=4).prop_map(StyleStep::PullBack),
        1 => select(vec![0usize, 3, 5, 100]).prop_map(StyleStep::Limit),
        1 => (1u8..=2).prop_map(StyleStep::Ambiguous),
    ]
}

/// What a renderer keeps between frames (§9): a mirror of each screen's
/// style table, the epochs seen, and a row cache keyed by `RowKey`, filled
/// only from snapshots.
#[derive(Default)]
struct Mirrors {
    tables: [Vec<Option<Style>>; 2],
    epochs: [Option<u64>; 2],
    seen: Vec<u64>,
    snap: Snapshot,
    rows: HashMap<RowKey, Vec<CellState>>,
    /// The terminal, history epoch and width the cached rows belong to.
    terminal: Option<u64>,
    history_epoch: u64,
    cols: u16,
    /// Snapshots taken, and rows copied in them.
    snapshots: usize,
    copied: usize,
}

impl Mirrors {
    /// Take a snapshot, as a renderer would, and check that the frame it
    /// gives (cached rows plus copied ones, styles from the mirror) is
    /// what the terminal shows.
    fn take(&mut self, t: &mut Term) -> Result<(), TestCaseError> {
        let rows = &self.rows;
        t.state.snapshot(&mut self.snap, |k| rows.contains_key(&k));
        let snap = &self.snap;
        self.snapshots += 1;

        // The cache rules of §9.1.
        let new_terminal = self.terminal != Some(snap.terminal_id);
        if new_terminal || self.history_epoch != snap.history_epoch || self.cols != snap.cols {
            self.rows.retain(|k, _| !new_terminal && matches!(k, RowKey::Screen(_)));
        }
        if let Some(from) = snap.popped_from {
            self.rows.retain(|k, _| !matches!(k, RowKey::History { line, .. } if *line >= from));
        }
        (self.terminal, self.history_epoch, self.cols) = (Some(snap.terminal_id), snap.history_epoch, snap.cols);

        let i = (snap.screen == ScreenId::Alternate) as usize;
        prop_assert_eq!(i == 1, t.state.alt_screen_active());
        if self.epochs[i] != Some(snap.styles_epoch) {
            // A new table (renumbered, the alternate one made again, RIS):
            // its epoch was never seen before, and it comes whole.
            prop_assert!(!self.seen.contains(&snap.styles_epoch), "epoch {} repeats", snap.styles_epoch);
            self.seen.push(snap.styles_epoch);
            self.epochs[i] = Some(snap.styles_epoch);
            self.tables[i].clear();
        }
        for &(id, s) in &snap.style_updates {
            let id = id as usize;
            if self.tables[i].len() <= id {
                self.tables[i].resize(id + 1, None);
            }
            self.tables[i][id] = Some(s);
        }
        // Every live id is mirrored with its style.
        for (id, s) in t.state.live_styles().into_iter().enumerate() {
            if let Some(s) = s {
                prop_assert_eq!(self.tables[i].get(id).copied().flatten(), Some(s), "id {}", id);
            }
        }

        prop_assert_eq!(snap.rows.len(), t.state.lines());
        for (line, row) in snap.rows.iter().enumerate() {
            if let Some(cells) = snap.row_cells(row) {
                let styles: Vec<Style> = if row.flags.contains(RowFlags::HISTORY) {
                    snap.history_styles.clone()
                } else {
                    self.tables[i].iter().map(|s| s.unwrap_or_default()).collect()
                };
                let state = cells
                    .iter()
                    .map(|c| {
                        let text = if c.is_cluster() {
                            snap.cluster(row, c.codepoint()).to_string()
                        } else {
                            c.ch().to_string()
                        };
                        let empty = !c.is_cluster() && c.codepoint() == 0;
                        (empty, text, c.width(), styles[c.style_id() as usize])
                    })
                    .collect();
                self.copied += 1;
                prop_assert!(self.rows.insert(row.key, state).is_none(), "{:?} copied though cached", row.key);
            }
            let shown = row_state(&t.state.visible_row(line));
            let cached = self.rows.get(&row.key);
            prop_assert!(cached.is_some(), "row {} ({:?}) neither cached nor copied", line, row.key);
            prop_assert_eq!(cached.unwrap(), &shown.0, "row {} ({:?})", line, row.key);
            prop_assert_eq!(row.flags.contains(RowFlags::WRAPPED), shown.1, "row {}", line);
            if let RowKey::Screen(seq) = row.key {
                prop_assert!(seq <= snap.counter);
            }
        }
        Ok(())
    }
}

proptest! {
    #![proptest_config(config())]

    /// With a table that sweeps at 8 ids, sweeps and renumberings happen
    /// every few prints. Every holder of an id (cells, the pen and erase
    /// templates, saved cursors, the other screen) must survive them, and
    /// what a reader sees must be what a terminal that sweeps as usual
    /// shows.
    #[test]
    fn forced_sweeps_keep_every_style(
        cols in 1usize..=40,
        lines in 1usize..=12,
        limit in select(vec![0usize, 5, 100]),
        steps in prop::collection::vec(style_step(), 1..30),
    ) {
        let mut swept = Term::new(cols, lines, limit, term_core::ImagePool::process());
        swept.state.set_sweep_at(Some(8));
        let mut plain = Term::new(cols, lines, limit, term_core::ImagePool::process());
        let mut mirrors = Mirrors::default();
        let mut most_cells = swept.state.cols() * swept.state.lines();
        for step in &steps {
            match step {
                StyleStep::Output(bytes) => {
                    swept.advance(bytes);
                    plain.advance(bytes);
                }
                StyleStep::Resize(c, l) => {
                    swept.state.resize(*c, *l);
                    plain.state.resize(*c, *l);
                }
                StyleStep::Sweep => swept.state.sweep_styles_now().map_err(TestCaseError::fail)?,
                StyleStep::Snapshot => mirrors.take(&mut swept)?,
                StyleStep::View(d) => {
                    swept.state.scroll_view(*d);
                    plain.state.scroll_view(*d);
                }
                StyleStep::PullBack(n) => {
                    swept.state.pull_back_for_test(*n);
                    plain.state.pull_back_for_test(*n);
                }
                StyleStep::Limit(n) => {
                    swept.state.set_scrollback_limit(*n);
                    plain.state.set_scrollback_limit(*n);
                }
                StyleStep::Ambiguous(w) => {
                    swept.state.set_ambiguous_width(*w);
                    plain.state.set_ambiguous_width(*w);
                }
            }
            swept.state.check_invariants().map_err(TestCaseError::fail)?;
            swept.state.check_history().map_err(TestCaseError::fail)?;
            prop_assert_eq!(screen_state(&swept), screen_state(&plain), "after {:?}", step);
            let (a, b) = swept.state.style_stats();
            // A sweep leaves at most the screen's cells + 2 ids in use, so
            // a table holds at most that many plus the one being added (for
            // the largest screen so far: shrinking frees nothing by itself).
            most_cells = most_cells.max(swept.state.cols() * swept.state.lines());
            let cap = (most_cells + 3).max(8);
            prop_assert!(a.len <= cap && b.len <= cap, "{:?} {:?}", a, b);
        }
        prop_assert_eq!(history(&mut swept), history(&mut plain));
    }
}

/// A truecolour gradient (a new colour in every cell, as lolcat and
/// colour tests write) keeps the table bounded: it sweeps at 1.5 × the
/// screen's cells, never holds more than that, and renumbers smaller once
/// the gradient is gone.
#[test]
fn truecolour_gradient_keeps_the_table_bounded() {
    for (cols, lines) in [(80usize, 24usize), (300, 100)] {
        let cells = cols * lines;
        let sweep_at = (cells * 3 / 2).max(1024);
        let mut t = Term::new(cols, lines, 1000, term_core::ImagePool::process());
        let mut out = Vec::new();
        let mut n = 0u32;
        let (mut peak, mut peak_bytes) = (0, 0);
        for _frame in 0..10 {
            out.clear();
            out.extend_from_slice(b"\x1b[H");
            for _ in 0..cells {
                n += 1;
                let (r, g, b) = ((n >> 16) as u8, (n >> 8) as u8, n as u8);
                out.extend_from_slice(format!("\x1b[38;2;{r};{g};{b}m\u{2588}").as_bytes());
            }
            t.advance(&out);
            let (s, _) = t.state.style_stats();
            peak = peak.max(s.len);
            peak_bytes = peak_bytes.max(s.bytes);
            assert!(s.len <= sweep_at + 1, "{s:?}");
        }
        let (s, _) = t.state.style_stats();
        assert!(s.sweeps >= 5, "{s:?}");
        assert!(peak >= cells, "every cell had its own style: peak {peak}");
        // The last cell written still reads back with its own colour.
        let row = t.state.visible_row(lines - 1);
        let last = row.cell(cols - 1).style().fg;
        assert_eq!(last, Color::rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
        eprintln!(
            "gradient {cols}x{lines}: peak {peak} ids, {peak_bytes} bytes ({} B/id), {} sweeps",
            peak_bytes / peak,
            s.sweeps
        );

        // The program clears the screen and leaves: the next sweep renumbers.
        t.advance(b"\x1b[0m\x1b[2J\x1b[H");
        t.state.sweep_styles_now().unwrap();
        let (s, _) = t.state.style_stats();
        assert!(s.renumberings >= 1, "{s:?}");
        assert!(s.len <= 2, "{s:?}");
        assert!(s.bytes < 4096, "memory given back: {s:?}");
    }
}

// ---------------------------------------------------------------------------
// Reflow (§12.2): resizing moves text between rows and history, but never
// changes it.

/// Text, line breaks and the styles that change what a reflow keeps: wide
/// characters, clusters, coloured erases (FILL lines) and OSC 133 marks.
fn reflow_piece() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        6 => ascii(),
        3 => select(vec!["漢字", "a\u{301}", "👍\u{1F3FD}", "e\u{301}x", "界ア", "❤\u{FE0F}", "🇺🇸"]).prop_map(|s| s.as_bytes().to_vec()),
        3 => Just(b"\r\n".to_vec()),
        1 => Just(b"\x1b[41m\x1b[K\x1b[0m".to_vec()),
        1 => Just(b"\x1b[44mblue\x1b[0m".to_vec()),
        1 => Just(b"\x1b]133;A\x07$ ".to_vec()),
        1 => Just(b"\x1b]133;C\x07".to_vec()),
    ]
}

#[derive(Clone, Debug)]
enum ReflowStep {
    Output(Vec<u8>),
    Resize(usize, usize),
    ScrollView(isize),
}

fn reflow_step() -> impl Strategy<Value = ReflowStep> {
    prop_oneof![
        6 => prop::collection::vec(reflow_piece(), 1..8).prop_map(|p| ReflowStep::Output(p.concat())),
        3 => (2usize..=24, 1usize..=8).prop_map(|(c, l)| ReflowStep::Resize(c, l)),
        1 => (-6isize..=6).prop_map(ReflowStep::ScrollView),
    ]
}

/// All the text the terminal holds, history and screen, one logical line
/// per line (an empty cell is a space, trailing spaces dropped), with the
/// blank lines at the end dropped. Read row by row through the view, so
/// it doesn't depend on how selections treat the ends of wrapped rows.
fn all_text(t: &mut Term) -> String {
    let view = t.state.view_top();
    let mut rows = Vec::new();
    t.state.scroll_view(isize::MAX / 2);
    while t.state.display_offset() > 0 {
        rows.push(row_text(&t.state.visible_row(0)));
        t.state.scroll_view(-1);
    }
    for l in 0..t.state.lines() {
        rows.push(row_text(&t.state.visible_row(l)));
    }
    t.state.set_view_top(view);
    let mut out = String::new();
    let mut line = String::new();
    for (text, wrapped) in rows {
        line.push_str(&text);
        if !wrapped {
            out.push_str(line.trim_end_matches(' '));
            out.push('\n');
            line.clear();
        }
    }
    out.push_str(line.trim_end_matches(' '));
    out.trim_end_matches(['\n', ' ']).to_string()
}

fn row_text(r: &RowRef) -> (String, bool) {
    let mut text = String::new();
    for c in r.cells() {
        match c.width() {
            Width::Spacer | Width::EdgeSpacer => {}
            _ if c.content() == Content::Empty => text.push(' '),
            _ => c.push_text(&mut text),
        }
    }
    (text, r.wrapped())
}

proptest! {
    #![proptest_config(Config { cases: 128, ..config() })]

    /// Resizing back and forth, to any sizes, keeps all the text, in
    /// order, and the selection on the same text.
    #[test]
    fn resizing_keeps_the_text(
        start in (2usize..=24, 1usize..=8),
        steps in prop::collection::vec(reflow_step(), 1..30),
        sizes in prop::collection::vec((2usize..=24, 1usize..=8), 1..6),
    ) {
        let mut t = Term::new(start.0, start.1, 100_000, term_core::ImagePool::process());
        for step in &steps {
            match step {
                ReflowStep::Output(b) => t.advance(b),
                ReflowStep::Resize(c, l) => {
                    let before = all_text(&mut t);
                    t.state.resize(*c, *l);
                    t.state.check_invariants().map_err(TestCaseError::fail)?;
                    prop_assert_eq!(all_text(&mut t), before, "resize to {}x{}", c, l);
                }
                ReflowStep::ScrollView(d) => t.state.scroll_view(*d),
            }
        }
        // Select the middle of the screen, then resize back and forth.
        let (lines, cols) = (t.state.lines(), t.state.cols());
        let a = t.state.point_at(0, 0);
        let b = t.state.point_at(lines - 1, cols - 1);
        t.state.set_selection(Some(Selection::new(a, b)));
        let sel = |t: &Term| t.state.selection_text().map(|s| s.trim_end_matches('\n').to_string());
        let selected = sel(&t);
        let before = all_text(&mut t);
        let (c0, l0) = (t.state.cols(), t.state.lines());
        for &(c, l) in &sizes {
            t.state.resize(c, l);
            t.state.check_invariants().map_err(TestCaseError::fail)?;
            prop_assert_eq!(all_text(&mut t), before.clone(), "resize to {}x{}", c, l);
            prop_assert_eq!(sel(&t), selected.clone(), "selection at {}x{}", c, l);
        }
        t.state.resize(c0, l0);
        prop_assert_eq!(all_text(&mut t), before);
        prop_assert_eq!(sel(&t), selected);
        check(&t)?;
    }
}

proptest! {
    #![proptest_config(config())]

    /// A line (spaces and wide characters, so typed spaces and edge
    /// padding end wrapped rows) copies as written, trailing spaces
    /// trimmed: on the screen, in history, in compressed history, and
    /// reflowed to other widths.
    #[test]
    fn a_line_copies_as_written_wherever_it_is(
        chars in prop::collection::vec(select(vec!['a', 'b', ' ', ' ', '\u{6f22}']), 1..60),
        cols in 2usize..=20,
        widths in prop::collection::vec(2usize..=24, 1..4),
    ) {
        let text: String = chars.into_iter().collect();
        let want = text.trim_end_matches(' ').to_string();
        let mut t = Term::new(cols, 3, 1000, term_core::ImagePool::process());
        t.advance(text.as_bytes());
        // The line is the first one stored: at the top of the view
        // scrolled all the way back.
        let copy = |t: &mut Term| {
            t.state.scroll_view(isize::MAX / 2);
            let p = t.state.point_at(0, 0);
            t.state.select_line(p);
            let s = t.state.selection_text().unwrap_or_default();
            t.state.scroll_view_to_bottom();
            s
        };
        prop_assert_eq!(copy(&mut t), want.clone(), "on the screen");
        t.advance("\r\n".repeat(2 * text.len() / cols + 6).as_bytes());
        prop_assert_eq!(copy(&mut t), want.clone(), "in history");
        t.state.seal_history_for_test();
        t.state.shrink_history();
        prop_assert_eq!(copy(&mut t), want.clone(), "in compressed history");
        for &w in &widths {
            t.state.resize(w, 3);
            prop_assert_eq!(copy(&mut t), want.clone(), "at {} columns", w);
        }
        // Reflowed on the screen, never in history.
        let mut t = Term::new(cols, 30, 1000, term_core::ImagePool::process());
        t.advance(text.as_bytes());
        for &w in &widths {
            t.state.resize(w, 30);
            prop_assert_eq!(copy(&mut t), want.clone(), "on the screen at {} columns", w);
        }
    }
}

// ---------------------------------------------------------------------------
// Compressed history (R.7).

proptest! {
    #![proptest_config(config())]

    /// Compressing history at any point changes nothing a reader sees: the
    /// screen after any steps (resizes reflow history, scrolling reads it),
    /// every stored line, pulling lines back and searching. Both terminals
    /// seal their open block after every step, so there are many blocks.
    #[test]
    fn compressing_history_changes_nothing(
        cols in 1usize..=80,
        lines in 1usize..=20,
        limit in select(vec![2usize, 10, 1000]),
        steps in prop::collection::vec((step(), 0u8..4), 1..30),
    ) {
        let mut plain = Term::new(cols, lines, limit, term_core::ImagePool::process());
        let mut packed = Term::new(cols, lines, limit, term_core::ImagePool::process());
        for (step, extra) in &steps {
            apply(&mut plain, step);
            apply(&mut packed, step);
            plain.state.seal_history_for_test();
            packed.state.seal_history_for_test();
            match extra {
                0 => {
                    packed.state.shrink_history();
                }
                1 => {
                    // The idle job: some blocks at a time.
                    if let Some(mut job) = packed.state.compress_job(2) {
                        job.run();
                        packed.state.add_compressed(job);
                    }
                }
                2 => {
                    plain.state.pull_back_for_test(2);
                    packed.state.pull_back_for_test(2);
                }
                _ => {}
            }
            check(&packed)?;
            prop_assert_eq!(plain.state.dump(), packed.state.dump(), "after {:?}", step);
        }
        for q in ["a", "e ", "ab", "\u{1f600}"] {
            let query = term_core::Query { text: q.into(), ignore_case: true, word: term_core::WordMatch::Contains };
            prop_assert_eq!(search(&mut plain, query.clone()), search(&mut packed, query));
        }
        prop_assert_eq!(history(&mut plain), history(&mut packed));
    }
}

fn search(t: &mut Term, query: term_core::Query) -> Vec<term_core::Hit> {
    let mut job = t.state.start_search(query);
    let mut batches = Vec::new();
    job.run(|b| batches.push(b));
    for b in batches {
        t.state.add_search_hits(b);
    }
    t.state.search_hits()
}

// ---------------------------------------------------------------------------
// The selection check (cells §11): output clears the selection when it
// changes the text under it, found as cells are written, against reading
// the text before and after every parse call.

/// One piece of output, of the kinds that write and move cells most.
fn watch_piece() -> impl Strategy<Value = Vec<u8>> {
    let region = (0u16..8, 0u16..8).prop_map(|(t, b)| format!("\x1b[{t};{b}r").into_bytes());
    let same = select(vec!["menu", "ab", "a", " ", "  ", "漢", "漢字", "e\u{301}", "😀"]).prop_map(|s| s.as_bytes().to_vec());
    prop_oneof![
        5 => ascii(),
        4 => same,
        3 => unicode(),
        3 => cluster(),
        5 => movement(),
        2 => c0(),
        5 => csi(),
        2 => region,
        1 => esc(),
    ]
}

#[derive(Clone, Debug)]
enum WatchStep {
    /// One piece: the selection goes exactly when its text changed.
    One(Vec<u8>),
    /// Several in one parse call: text changed and put back may lose
    /// it, but a change that stays always clears it.
    Many(Vec<Vec<u8>>),
    Select((usize, usize), (usize, usize)),
    SelectAll,
    Resize(usize, usize),
}

fn watch_step() -> impl Strategy<Value = WatchStep> {
    let point = (0usize..100, 0usize..250);
    prop_oneof![
        10 => watch_piece().prop_map(WatchStep::One),
        3 => prop::collection::vec(watch_piece(), 2..8).prop_map(WatchStep::Many),
        4 => (point.clone(), point).prop_map(|(a, b)| WatchStep::Select(a, b)),
        1 => Just(WatchStep::SelectAll),
        1 => (1usize..=30, 1usize..=10).prop_map(|(c, l)| WatchStep::Resize(c, l)),
    ]
}

/// The first buffer line still kept (in history or on the screen).
fn first_kept(t: &Term) -> u64 {
    let top = t.state.point_at(0, 0).line;
    if t.state.alt_screen_active() {
        top
    } else {
        top - t.state.scrollback_len() as u64
    }
}

/// A cluster in `s` may grow wide after its first character was
/// written, moving to the next row in the last column: one piece of
/// output then changes a cell and puts it back.
fn widens(s: &str) -> bool {
    s.chars()
        .any(|c| matches!(c, '\u{FE0F}' | '\u{200D}' | '\u{20E3}' | '\u{1F1E6}'..='\u{1F1FF}' | '\u{1F3FB}'..='\u{1F3FF}'))
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn the_selection_goes_when_output_changes_its_text(
        cols in 2usize..=12,
        lines in 1usize..=6,
        limit in select(vec![0usize, 2, 1000]),
        steps in prop::collection::vec(watch_step(), 1..60),
    ) {
        let mut t = Term::new(cols, lines, limit, term_core::ImagePool::new(32 << 20));
        // Once a scroll region may be set, a run of text can write a
        // selected cell and then scroll another row under it.
        let mut region = false;
        for step in &steps {
            let (bytes, exact) = match step {
                WatchStep::One(b) => (b.clone(), true),
                WatchStep::Many(bs) => (bs.concat(), false),
                WatchStep::Select(a, b) => {
                    let (lines, cols) = (t.state.lines(), t.state.cols());
                    let anchor = t.state.point_at(a.0 % lines, a.1 % cols);
                    let head = t.state.point_at(b.0 % lines, b.1 % cols);
                    t.state.set_selection(Some(Selection::new(anchor, head)));
                    continue;
                }
                WatchStep::SelectAll => {
                    t.state.select_all();
                    continue;
                }
                WatchStep::Resize(c, l) => {
                    t.state.resize(*c, *l);
                    continue;
                }
            };
            let s = String::from_utf8_lossy(&bytes).into_owned();
            region |= s.contains("\x1b[") && s.contains('r');
            let Some(sel) = t.state.selection() else {
                t.advance(&bytes);
                continue;
            };
            let (a, b) = sel.ordered();
            let before = t.state.text_between(a, b);
            t.advance(&bytes);
            let after = t.state.text_between(a, b);
            let changed = after != before || a.line < first_kept(&t);
            let kept = t.state.selection() == Some(sel);
            if changed {
                prop_assert!(!kept, "{:?}: {:?} became {:?}, but the selection stayed", s, before, after);
            } else if exact && !kept && !widens(&s) && !region {
                // Only the parser's own clearing (a reset, the other
                // screen, history erased) may take an unchanged one.
                let own = s.contains("\x1bc") || s.contains("\x1b[?") || (s.contains("\x1b[3") && s.contains('J'));
                prop_assert!(own, "{:?}: the text {:?} didn't change, but the selection went", s, before);
            }
        }
    }
}
