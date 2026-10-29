//! term-core's Unicode table and grapheme segmentation (cells design §6.2,
//! §6.3): widths against `unicode-width`, cluster boundaries against
//! GraphemeBreakTest.txt and `unicode-segmentation`, the cluster width
//! rule against the design's table, and the terminal's own segmentation,
//! fed whole and one byte at a time.

use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{Config, RngSeed};
use term_core::Term;
use term_core::unicode::{self, Gcb, cluster_width, graphemes, props};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

/// Scalars whose width deliberately differs from `unicode-width`'s:
/// (scalar, `unicode-width`, decline).
const WIDTH_EXCEPTIONS: &[(char, usize, u8)] = &[
    // KHMER SIGN BEYYAL: a cell is 1 or 2 wide; `string-width` says 1.
    ('\u{17D8}', 3, 1),
];

#[test]
fn table_and_unicode_width_are_the_same_version() {
    assert_eq!(unicode::UNICODE_VERSION, unicode_width::UNICODE_VERSION);
}

/// Every scalar's width, and its width where ambiguous characters are wide,
/// equal `unicode-width`'s `width` and `width_cjk`, except for the listed
/// exceptions. Controls are the scalars `unicode-width` gives no width.
#[test]
fn widths_equal_unicode_width_per_scalar() {
    let mut wrong = Vec::new();
    for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
        let p = props(c);
        if let Some(&(_, uw, ours)) = WIDTH_EXCEPTIONS.iter().find(|e| e.0 == c) {
            assert_eq!(c.width(), Some(uw), "{c:?}: unicode-width changed");
            assert_eq!(p.width(), ours, "{c:?}");
            continue;
        }
        let ours = (!p.is_control()).then(|| (p.width() as usize, p.width_with(2) as usize));
        let theirs = c.width().map(|w| (w, c.width_cjk().unwrap()));
        if ours != theirs {
            wrong.push(format!("U+{:04X}: ours {ours:?}, unicode-width {theirs:?}", c as u32));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} differences:\n{}",
        wrong.len(),
        wrong[..wrong.len().min(40)].join("\n")
    );
}

#[test]
fn ambiguous_flag_marks_narrow_characters_wide_in_cjk() {
    for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
        let p = props(c);
        assert_eq!(
            p.ambiguous(),
            !p.is_control() && c.width() == Some(1) && c.width_cjk() == Some(2),
            "{c:?}"
        );
    }
    for c in ['§', '°', '±', '①', '─', '│', '\u{E000}'] {
        assert!(props(c).ambiguous(), "{c:?}");
    }
    // Letters are narrow in unicode-width's set, though their East Asian
    // Width is Ambiguous: Greek and Cyrillic stay one cell.
    for c in ['a', '漢', 'é', '\u{0301}', 'α', 'Ж', '❗'] {
        assert!(!props(c).ambiguous(), "{c:?}");
    }
}

/// The 128-entry ASCII table (used so ASCII never touches the big table)
/// agrees with the big table.
#[test]
fn ascii_table_matches_the_big_table() {
    for b in 0u8..0x80 {
        let c = b as char;
        assert_eq!(props(c), unicode::table_props_for_tests(c), "{c:?}");
    }
    for c in ['#', '*', '0', '5', '9'] {
        assert!(props(c).variation_base(), "{c:?} is a keycap base");
    }
    assert_eq!(props('\r').gcb(), Gcb::Cr);
    assert_eq!(props('\n').gcb(), Gcb::Lf);
}

#[test]
fn emoji_properties() {
    assert!(props('😀').extended_pictographic() && props('😀').emoji_presentation());
    assert!(props('👍').emoji_modifier_base());
    assert!(props('\u{1F3FD}').emoji_modifier());
    assert!(props('❤').variation_base() && !props('❤').emoji_presentation());
    assert_eq!(props('\u{1F1EC}').gcb(), Gcb::RegionalIndicator);
    assert_eq!(props('\u{200D}').gcb(), Gcb::Zwj);
    assert_eq!(props('\u{0600}').gcb(), Gcb::Prepend);
    assert_eq!(props('\u{093F}').gcb(), Gcb::SpacingMark);
}

/// Every line of GraphemeBreakTest.txt (Unicode 18.0.0, the table's version).
#[test]
fn grapheme_break_test() {
    let data = include_str!("data/GraphemeBreakTest.txt");
    let mut n = 0;
    for line in data.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let mut expected = Vec::new();
        let mut text = String::new();
        let mut current = String::new();
        for token in line.split_whitespace() {
            match token {
                "÷" => {
                    if !current.is_empty() {
                        expected.push(std::mem::take(&mut current));
                    }
                }
                "×" => {}
                hex => {
                    let c = char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap();
                    current.push(c);
                    text.push(c);
                }
            }
        }
        let ours: Vec<&str> = graphemes(&text).collect();
        assert_eq!(ours, expected, "{line}");
        n += 1;
    }
    assert!(n > 700, "only {n} test lines");
}

/// Pieces that exercise every rule, all assigned in Unicode 17 with the
/// same properties in 18 (`unicode-segmentation` is on 17.0). Unicode 18
/// changed GB9c to no longer need a consonant before the linker, so a
/// virama only comes straight after a consonant here; GraphemeBreakTest.txt
/// covers the new rule.
fn interesting() -> Vec<&'static str> {
    vec![
        "a",
        "1",
        "#",
        "*",
        " ",
        "é",
        "漢",
        "ア",
        "\u{0301}",
        "\u{0308}",
        "\u{20E3}",
        "\u{FE0F}",
        "\u{FE0E}",
        "\u{200D}",
        "\u{200C}",
        "\u{200B}",
        "\u{00AD}",
        "\u{2028}",
        "\u{0600}",
        "\u{0605}",
        "\u{110BD}",
        "\u{0915}",
        "\u{0915}\u{094D}",
        "\u{0937}",
        "\u{093F}",
        "\u{093E}",
        "\u{0E01}",
        "\u{0E33}",
        "\u{1100}",
        "\u{1161}",
        "\u{11A8}",
        "\u{AC00}",
        "\u{AC01}",
        "\u{1F1EC}",
        "\u{1F1E7}",
        "\u{1F1FA}",
        "😀",
        "👍",
        "\u{1F3FD}",
        "👨",
        "👩",
        "👧",
        "❤",
        "⌚",
        "©",
        "🔥",
        "\u{E0067}",
        "\u{E007F}",
        "\u{0E31}",
        "\u{1780}",
        "\u{AA00}",
    ]
}

fn config() -> Config {
    Config {
        rng_seed: RngSeed::Fixed(0x6EA9_4E5E),
        failure_persistence: None,
        cases: 512,
        ..Config::default()
    }
}

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(select(interesting()), 1..24).prop_map(|ps| ps.concat())
}

/// What a terminal segments `s` into when it is printed, fed whole or one
/// byte per `advance` call, with bulk ASCII on or off.
fn terminal_graphemes(s: &str, byte_at_a_time: bool, bulk: bool) -> Vec<String> {
    let mut t = Term::new(7, 3, 10);
    t.state.set_bulk_ascii(bulk);
    t.state.record_graphemes(true);
    if byte_at_a_time {
        for b in s.as_bytes() {
            t.advance(std::slice::from_ref(b));
        }
    } else {
        t.advance(s.as_bytes());
    }
    t.state.take_graphemes()
}

proptest! {
    #![proptest_config(config())]

    /// Incremental segmentation equals `unicode-segmentation`'s extended
    /// grapheme clusters, both on its own and inside the terminal.
    #[test]
    fn segmentation_equals_unicode_segmentation(s in text()) {
        let expected: Vec<&str> = s.graphemes(true).collect();
        let ours: Vec<&str> = graphemes(&s).collect();
        prop_assert_eq!(&ours, &expected);
        for (byte_at_a_time, bulk) in [(false, true), (true, true), (false, false), (true, false)] {
            prop_assert_eq!(&terminal_graphemes(&s, byte_at_a_time, bulk), &expected, "bytes {} bulk {}", byte_at_a_time, bulk);
        }
    }
}

/// Anything but a print ends the cluster: a mark after a cursor move or SGR
/// starts a new one.
#[test]
fn non_print_ends_the_cluster() {
    for between in ["\x1b[C", "\x1b[1m", "\r", "\x1b]0;t\x07", "\x1bP$q\x1b\\"] {
        let mut t = Term::new(20, 3, 10);
        t.state.record_graphemes(true);
        t.advance(format!("e{between}\u{0301}").as_bytes());
        assert_eq!(t.state.take_graphemes(), ["e", "\u{0301}"], "{between:?}");
    }
    let mut t = Term::new(20, 3, 10);
    t.state.record_graphemes(true);
    t.advance("e\u{0301}".as_bytes());
    assert_eq!(t.state.take_graphemes(), ["e\u{0301}"]);
}

/// After `write_ascii`, the state is on the run's last cell, so a mark or a
/// keycap joins it; and a Prepend scalar takes the first byte of a run.
#[test]
fn ascii_runs_leave_the_state_on_their_last_cell() {
    let mut t = Term::new(20, 3, 10);
    t.state.record_graphemes(true);
    t.advance("abc1\u{FE0F}\u{20E3}xe\u{0301}".as_bytes());
    assert_eq!(t.state.take_graphemes(), ["a", "b", "c", "1\u{FE0F}\u{20E3}", "x", "e\u{0301}"]);
    // The keycap is two cells wide.
    let g = t.state.grapheme_state();
    assert_eq!(g.at, Some((0, 6)));

    t.advance("\u{0600}12".as_bytes());
    assert_eq!(t.state.take_graphemes(), ["\u{0600}1", "2"]);

    // At the end of a row, before the wrap.
    let mut t = Term::new(4, 3, 10);
    t.advance(b"abcd");
    assert_eq!(t.state.grapheme_state().at, Some((0, 3)));
    assert!(t.state.cursor().pending_wrap());
}

/// The width rule (§6.3) against the design's table, with what
/// `unicode-width` 0.2's string width gives. `string-width` 8.3.0 gives
/// the values in the comments (checked with node).
#[test]
fn cluster_width_table() {
    // (cluster, unicode-width, decline)
    let cases: &[(&str, usize, u8)] = &[
        ("क्ष", 2, 1),                     // string-width 1
        ("กำ", 2, 1),                     // string-width 1
        ("\u{0600}1", 2, 1),              // string-width 1
        ("e\u{0301}", 1, 1),              // string-width 1
        ("👨\u{200D}👩\u{200D}👧", 2, 2), // string-width 2
        ("👍\u{1F3FD}", 2, 2),            // string-width 2
        ("🇬🇧", 2, 2),                     // string-width 2
        ("1\u{FE0F}\u{20E3}", 2, 2),      // string-width 2
        ("1\u{20E3}", 1, 2),              // string-width 2
        ("❤\u{FE0F}", 2, 2),              // string-width 2
        ("❤\u{200D}🔥", 3, 2),            // string-width 2
        ("⌚\u{FE0E}", 1, 1),             // string-width 2: it ignores VS15
        ("☺\u{FE0E}", 1, 1),              // string-width 1
        ("©\u{FE0F}", 2, 2),              // string-width 2
        ("1\u{FE0F}", 2, 1),              // string-width 1: not a keycap yet
        ("a\u{FE0F}", 1, 1),              // string-width 1
        ("🇬", 1, 1),                      // string-width 1
        ("\u{0600}", 1, 1),               // string-width 0; a cell is at least 1
    ];
    for &(s, uw, ours) in cases {
        assert_eq!(graphemes(s).count(), 1, "{s:?} is one cluster");
        assert_eq!(s.width(), uw, "{s:?}: unicode-width");
        assert_eq!(cluster_width(s, 1), ours, "{s:?}");
    }
}

/// With ambiguous characters wide, a cluster that starts with one is 2;
/// emoji sequences stay 2 and text presentation sequences 1.
#[test]
fn cluster_width_with_ambiguous_wide() {
    assert_eq!(cluster_width("§", 1), 1);
    assert_eq!(cluster_width("§", 2), 2);
    assert_eq!(cluster_width("±\u{0301}", 2), 2);
    assert_eq!(cluster_width("α\u{0301}", 2), 1);
    assert_eq!(cluster_width("─", 2), 2);
    // ↔ is ambiguous and a variation-sequence base.
    assert_eq!(cluster_width("↔\u{FE0E}", 2), 1); // string-width 2
    assert_eq!(cluster_width("↔\u{FE0F}", 2), 2);
    assert_eq!(cluster_width("↔\u{FE0F}", 1), 2);
    assert_eq!(cluster_width("👨\u{200D}👩", 2), 2);
}

/// The ambiguous-width setting applies to output from then on: cells
/// already written keep their width, the cursor moves by the new one (what
/// Vim's `t_u7` check measures), letters stay narrow, and a reset keeps it.
#[test]
fn ambiguous_width_applies_to_new_output_only() {
    use term_core::Width::*;
    let mut t = Term::new(10, 1, 0);
    t.advance("§".as_bytes());
    assert_eq!(t.state.cursor().col(), 1);
    t.state.set_ambiguous_width(2);
    assert_eq!(t.state.ambiguous_width(), 2);
    t.advance("§─α\x1b[6n".as_bytes());
    let row = t.state.visible_row(0);
    let cells: Vec<_> = row.cells().take(7).map(|c| (c.ch(), c.width())).collect();
    assert_eq!(
        cells,
        [
            ('§', Narrow),
            ('§', Wide),
            (' ', Spacer),
            ('─', Wide),
            (' ', Spacer),
            ('α', Narrow),
            (' ', Narrow),
        ]
    );
    assert_eq!(t.state.take_replies(), b"\x1b[1;7R");
    // VS15 stays one cell, emoji two.
    t.advance("\r\x1b[K↔\u{FE0E}❤\u{FE0F}".as_bytes());
    assert_eq!(t.state.cursor().col(), 3);
    // A reset keeps the setting, which is 1 or 2.
    t.advance(b"\x1bc");
    assert_eq!(t.state.ambiguous_width(), 2);
    t.state.set_ambiguous_width(7);
    assert_eq!(t.state.ambiguous_width(), 2);
    t.state.set_ambiguous_width(0);
    assert_eq!(t.state.ambiguous_width(), 1);
}

/// What the cells should hold after printing `s` on one wide row: one cell
/// per cluster that has a scalar with a width (leading zero-width scalars
/// of a cluster have no cell and are dropped), its text within the caps
/// (32 scalars, 128 bytes; a scalar past them is dropped), and its width
/// by §6.3. With mode 2027 off (`legacy`), each scalar with a width starts
/// a cell of its own width and zero-width ones join the cell before.
fn expected_cells(s: &str, ambiguous: u8, legacy: bool) -> Vec<(String, u8)> {
    let w = |c: char| props(c).width_with(ambiguous);
    let push = |text: &mut String, c: char| {
        if text.chars().count() < 32 && text.len() + c.len_utf8() <= 128 {
            text.push(c);
        }
    };
    let mut out: Vec<(String, u8)> = Vec::new();
    if legacy {
        for c in s.chars() {
            if w(c) > 0 {
                out.push((c.to_string(), w(c)));
            } else if let Some((text, _)) = out.last_mut() {
                push(text, c);
            }
        }
        return out;
    }
    for g in graphemes(s) {
        let Some(i) = g.char_indices().find(|&(_, c)| w(c) > 0).map(|(i, _)| i) else {
            continue;
        };
        let mut text = String::new();
        for c in g[i..].chars() {
            push(&mut text, c);
        }
        out.push((text, cluster_width(&g[i..], ambiguous)));
    }
    out
}

/// The cells of row 0, up to the cursor: text and width.
fn printed_cells(s: &str, ambiguous: u8, legacy: bool, byte_at_a_time: bool, bulk: bool) -> Vec<(String, u8)> {
    let mut t = Term::new(400, 2, 10);
    t.state.set_bulk_ascii(bulk);
    t.state.set_ambiguous_width(ambiguous);
    if legacy {
        t.advance(b"\x1b[?2027l");
    }
    if byte_at_a_time {
        for b in s.as_bytes() {
            t.advance(std::slice::from_ref(b));
        }
    } else {
        t.advance(s.as_bytes());
    }
    let row = t.state.visible_row(0);
    let end = t.state.cursor().col();
    (0..end)
        .map(|col| row.cell(col))
        .filter(|c| !c.is_spacer())
        .map(|c| {
            let mut text = String::new();
            c.push_text(&mut text);
            (text, if c.width() == term_core::Width::Wide { 2 } else { 1 })
        })
        .collect()
}

proptest! {
    #![proptest_config(config())]

    /// The cells written equal the clusters the segmenter finds, with the
    /// widths of §6.3, at both ambiguous-width settings and in legacy mode;
    /// fed whole or one byte at a time, with bulk ASCII on or off.
    #[test]
    fn cells_hold_the_clusters(s in text(), ambiguous in 1u8..=2, legacy in any::<bool>()) {
        let expected = expected_cells(&s, ambiguous, legacy);
        for (byte_at_a_time, bulk) in [(false, true), (true, true), (false, false)] {
            prop_assert_eq!(
                &printed_cells(&s, ambiguous, legacy, byte_at_a_time, bulk),
                &expected,
                "bytes {} bulk {}", byte_at_a_time, bulk
            );
        }
    }
}

/// The emoji sequence shapes the design names (presentation, modifier,
/// flag, keycap, ZWJ and tag sequences) are each one cell two wide, at
/// both ambiguous-width settings.
#[test]
fn emoji_sequences_are_two_cells_at_both_settings() {
    for s in [
        "❤\u{FE0F}",
        "↔\u{FE0F}",
        "👍\u{1F3FD}",
        "🇬🇧",
        "1\u{FE0F}\u{20E3}",
        "#\u{20E3}",
        "👨\u{200D}👩\u{200D}👧",
        "🏳\u{FE0F}\u{200D}🌈",
        "🏴\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
    ] {
        for ambiguous in [1, 2] {
            assert_eq!(
                printed_cells(s, ambiguous, false, false, true),
                vec![(s.to_string(), 2)],
                "{s:?} at {ambiguous}"
            );
        }
    }
}

/// The sequences of emoji-test.txt (Unicode 18.0.0, the table's version)
/// with their statuses: `fq`, `mq`, `uq` (fully, minimally or not
/// qualified) and `c` (component).
fn emoji_test() -> Vec<(String, &'static str)> {
    include_str!("data/emoji-test.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|line| {
            let (points, status) = line.split_once(';').expect("`points;status`");
            let s = points
                .split(' ')
                .map(|h| char::from_u32(u32::from_str_radix(h, 16).expect("hex")).expect("a scalar"))
                .collect();
            (s, status)
        })
        .collect()
}

/// Every sequence in emoji-test.txt, printed alone at both ambiguous-width
/// settings, is one cell holding the whole sequence. Fully-qualified ones
/// and components are two cells wide; the others (a pictograph without its
/// VS16, say, which is text presentation) follow the width rule.
#[test]
fn every_emoji_test_sequence_is_one_cell() {
    let all = emoji_test();
    assert!(all.len() > 5000, "{} sequences", all.len());
    for (s, status) in &all {
        for ambiguous in [1, 2] {
            let want = match *status {
                "fq" | "c" => 2,
                _ => cluster_width(s, ambiguous),
            };
            assert_eq!(
                printed_cells(s, ambiguous, false, false, true),
                vec![(s.clone(), want)],
                "{s:?} ({status}) at {ambiguous}"
            );
        }
    }
}

/// The same sequences one after another, between letters and spaces, fed
/// whole and one byte at a time: the cells hold the clusters.
#[test]
fn emoji_test_sequences_in_a_stream() {
    let all = emoji_test();
    for chunk in all.chunks(60) {
        let s: String = chunk.iter().map(|(s, _)| format!("{s}a ")).collect();
        for ambiguous in [1, 2] {
            let expected = expected_cells(&s, ambiguous, false);
            for (byte_at_a_time, bulk) in [(false, true), (true, true), (false, false)] {
                assert_eq!(
                    printed_cells(&s, ambiguous, false, byte_at_a_time, bulk),
                    expected,
                    "{s:?} at {ambiguous}, bytes {byte_at_a_time} bulk {bulk}"
                );
            }
        }
    }
}
