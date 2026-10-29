use proptest::prelude::*;

use super::*;
use crate::term::Term;

fn q(text: &str, ignore_case: bool, word: WordMatch) -> Query {
    Query {
        text: text.into(),
        ignore_case,
        word,
    }
}

/// Run a search to the end, as the app's worker and main thread would.
fn search(t: &mut Term, query: Query) -> Vec<Hit> {
    let mut job = t.state.start_search(query);
    let mut batches = Vec::new();
    job.run(|b| batches.push(b));
    assert!(batches.last().is_some_and(|b| b.is_done()));
    for b in batches {
        t.state.add_search_hits(b);
    }
    t.state.search_hits()
}

/// The reference: every place in the whole text where the query matches,
/// scalar by scalar.
fn naive(t: &Term, query: &Query) -> Vec<Hit> {
    let cells = t.state.search_text_for_test();
    let text = std::str::from_utf8(&cells.text).expect("UTF-8");
    let want: Vec<char> = query.text.chars().collect();
    let same = |a: char, b: char| if query.ignore_case { fold(a) == fold(b) } else { a == b };
    let mut out = Vec::new();
    if want.is_empty() {
        return out;
    }
    for (s, _) in text.char_indices() {
        let mut it = text[s..].char_indices();
        let mut end = s;
        let mut ok = true;
        for &w in &want {
            match it.next() {
                Some((i, c)) if same(c, w) => end = s + i + c.len_utf8(),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let before = text[..s].chars().next_back();
        let after = text[end..].chars().next();
        let edge = |c: Option<char>| !c.is_some_and(is_word_char);
        let word_ok = match query.word {
            WordMatch::Contains => true,
            WordMatch::StartsWith => edge(before),
            WordMatch::EndsWith => edge(after),
            WordMatch::FullWord => edge(before) && edge(after),
        };
        if !word_ok {
            continue;
        }
        if let (Some(a), Some(b)) = (cells.start_at(s), cells.end_at(end)) {
            out.push(Hit::from_parts(a, b));
        }
    }
    out
}

#[test]
fn fold_equivalences() {
    for (a, b) in [
        ('σ', 'Σ'),
        ('ς', 'σ'),
        ('θ', 'ϑ'),
        ('ϴ', 'θ'),
        ('ǆ', 'ǅ'),
        ('Ǆ', 'ǆ'),
        ('É', 'é'),
        ('ẞ', 'ß'),
        ('A', 'a'),
    ] {
        assert_eq!(fold(a), fold(b), "{a} {b}");
    }
    // Kept apart, so ASCII text stays on the byte path.
    assert_ne!(fold('\u{212A}'), 'k');
    assert_ne!(fold('\u{017F}'), 's');
    assert_ne!(fold('ß'), fold('s'));
}

#[test]
fn no_non_ascii_character_folds_to_ascii() {
    for c in (0x80..=0x10FFFFu32).filter_map(char::from_u32) {
        assert!(!fold(c).is_ascii(), "{c:?} folds to {:?}", fold(c));
    }
}

#[test]
fn word_rule() {
    for c in ['a', 'Z', '7', '_', 'é', '中', '\u{301}'] {
        assert!(is_word_char(c), "{c:?}");
    }
    for c in [' ', '/', '-', '.', '\n', '!'] {
        assert!(!is_word_char(c), "{c:?}");
    }
}

#[test]
fn matcher_paths() {
    let find = |query: Query, hay: &str| {
        let mut out = Vec::new();
        Matcher::new(&query)
            .unwrap()
            .find(hay.as_bytes(), None, None, &mut Vec::new(), &mut out);
        out
    };
    assert_eq!(find(q("aa", false, WordMatch::Contains), "aaa"), [(0, 2), (1, 3)]);
    assert_eq!(find(q("Foo", false, WordMatch::Contains), "foo Foo"), [(4, 7)]);
    assert_eq!(find(q("Foo", true, WordMatch::Contains), "foo FOO"), [(0, 3), (4, 7)]);
    assert_eq!(find(q("σ", true, WordMatch::Contains), "Σς"), [(0, 2), (2, 4)]);
    assert_eq!(find(q("aσ", true, WordMatch::Contains), "xAΣ"), [(1, 4)]);
    assert_eq!(find(q("12", true, WordMatch::Contains), "x12"), [(1, 3)]);
    assert_eq!(
        find(q("foo", false, WordMatch::FullWord), "foo foobar barfoo foo_ foo"),
        [(0, 3), (23, 26)]
    );
    assert_eq!(find(q("foo", false, WordMatch::StartsWith), "foo foobar barfoo"), [(0, 3), (4, 7)]);
    assert_eq!(find(q("foo", false, WordMatch::EndsWith), "foo foobar barfoo"), [(0, 3), (14, 17)]);
    let m = Matcher::new(&q("ab", false, WordMatch::FullWord)).unwrap();
    let mut out = Vec::new();
    m.find(b"ab", Some('x'), None, &mut Vec::new(), &mut out);
    assert!(out.is_empty(), "the scalar before the text counts");
    assert!(Matcher::new(&q("", false, WordMatch::Contains)).is_none());
}

#[test]
fn fold_by_ascii_stretch() {
    let find = |query: &str, hay: &str| {
        let mut out = Vec::new();
        let m = Matcher::new(&q(query, true, WordMatch::Contains)).unwrap();
        assert!(matches!(m.kind, Kind::FoldAt { .. }), "{query}");
        m.find(hay.as_bytes(), None, None, &mut Vec::new(), &mut out);
        out
    };
    assert_eq!(find("Σigma", "σIGMA ΣIgma sigma ςigmaΣigma"), [(0, 6), (7, 13), (20, 26), (26, 32)]);
    // The stretch after two non-ASCII scalars, one ASCII before them.
    assert_eq!(find("xÉσab", "XéΣAB éσab xéςab"), [(0, 7), (15, 22)]);
    // The stretch at the start of the text: nothing before it to walk back over.
    assert_eq!(find("σab", "ab σab"), [(3, 7)]);
    // Overlapping matches.
    assert_eq!(find("éaéa", "éaéaéa"), [(0, 6), (3, 9)]);
}

#[test]
fn hits_on_screen_and_across_a_wrap() {
    let mut t = Term::new(10, 4, 100, crate::ImagePool::process());
    t.advance(b"hello world, hello");
    let hits = search(&mut t, q("hello", false, WordMatch::Contains));
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].start(), crate::Point { line: 0, col: 0 });
    // "hello worl" | "d, hello": the second is at column 3 of row 1.
    assert_eq!((hits[1].line, hits[1].col, hits[1].end_lines, hits[1].end_col), (1, 3, 0, 8));
    let hits = search(&mut t, q("world", false, WordMatch::Contains));
    assert_eq!((hits[0].line, hits[0].col, hits[0].end_lines, hits[0].end_col), (0, 6, 1, 1));
}

#[test]
fn clusters_and_wide_characters() {
    let mut t = Term::new(20, 3, 100, crate::ImagePool::process());
    t.advance("e\u{301}x e 中文e".as_bytes());
    let hits = search(&mut t, q("e", false, WordMatch::Contains));
    // Not the first half of e + U+0301.
    let cols: Vec<u16> = hits.iter().map(|h| h.col).collect();
    assert_eq!(cols, [3, 9]);
    let hits = search(&mut t, q("文", false, WordMatch::Contains));
    assert_eq!((hits[0].col, hits[0].end_col), (7, 9));
}

#[test]
fn history_blocks_and_boundaries() {
    let mut t = Term::new(12, 3, 1000, crate::ImagePool::process());
    for i in 0..60 {
        t.advance(format!("line {i} needle\r\n").as_bytes());
        if i % 7 == 0 {
            t.state.seal_history_for_test();
        }
    }
    // A long line that wraps across a seal.
    t.advance(b"xxxxxxxxxneed");
    t.state.seal_history_for_test();
    t.advance(b"le yyyyyyyyyyyyyyyyyyyyyyyy\r\n\r\n\r\n\r\n");
    let query = q("needle", false, WordMatch::Contains);
    let hits = search(&mut t, query.clone());
    assert_eq!(hits, naive(&t, &query));
    assert_eq!(hits.len(), 61);
}

#[test]
fn live_output_is_searched_once() {
    let mut t = Term::new(16, 3, 1000, crate::ImagePool::process());
    t.advance(b"one foo\r\n");
    let query = q("foo", false, WordMatch::Contains);
    assert_eq!(search(&mut t, query.clone()).len(), 1);
    for i in 0..40 {
        t.advance(format!("{i} foo\r\n").as_bytes());
        if i == 20 {
            t.state.seal_history_for_test();
        }
        t.state.refresh_search();
    }
    let hits = t.state.search_hits();
    assert_eq!(hits, naive(&t, &query));
    assert_eq!(hits.len(), 41);
}

#[test]
fn next_and_previous_wrap_and_select() {
    let mut t = Term::new(10, 3, 100, crate::ImagePool::process());
    t.advance(b"ab\r\nab\r\nab");
    search(&mut t, q("ab", false, WordMatch::Contains));
    assert!(t.state.search_next());
    assert_eq!(t.state.search_count().current, Some(0));
    assert!(t.state.search_next());
    assert!(t.state.search_next());
    assert!(t.state.search_next());
    assert_eq!(t.state.search_count().current, Some(0), "wraps around");
    assert!(t.state.search_previous());
    assert_eq!(t.state.search_count().current, Some(2));
    let sel = t.state.selection().unwrap();
    assert_eq!((sel.anchor.col, sel.head.col), (0, 1));
    let mut snap = crate::Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    let kinds: Vec<_> = snap.highlights.iter().map(|h| h.kind).collect();
    assert!(kinds.contains(&crate::HighlightKind::CurrentSearchHit));
    assert!(kinds.contains(&crate::HighlightKind::SearchHit));
}

#[test]
fn freed_and_cleared_history_drop_their_hits() {
    let mut t = Term::new(10, 2, 1000, crate::ImagePool::process());
    for _ in 0..30 {
        t.advance(b"zz\r\n");
    }
    t.state.seal_history_for_test();
    let query = q("zz", false, WordMatch::Contains);
    assert_eq!(search(&mut t, query.clone()).len(), 30);
    t.state.set_scrollback_limit(5);
    t.state.refresh_search();
    assert_eq!(t.state.search_hits(), naive(&t, &query));
    t.advance(b"\x1b[3J");
    t.state.refresh_search();
    assert_eq!(t.state.search_hits(), naive(&t, &query));
}

/// Found by `equals_naive_search` on CI: a row written again before the
/// next snapshot kept the change counter the screen's search recorded,
/// so "ς" stayed a whole-word hit after "a" was printed after it.
#[test]
fn screen_row_written_again_before_a_snapshot() {
    let mut t = Term::new(8, 3, 8, crate::ImagePool::process());
    let query = q("ς", false, WordMatch::FullWord);
    t.advance("ς".as_bytes());
    assert_eq!(search(&mut t, query.clone()).len(), 1);
    t.advance(b"a");
    t.state.refresh_search();
    assert_eq!(t.state.search_hits(), naive(&t, &query));
    assert!(t.state.search_hits().is_empty());
}

/// Found by `equals_naive_search`: when the limit hides the first row of
/// a wrapped line, the next row's start has nothing visible before it,
/// so a match there starts a word (as a new search would find).
#[test]
fn hiding_a_wrapped_rows_start_makes_a_word_start() {
    let mut t = Term::new(4, 2, 3, crate::ImagePool::process());
    t.advance("aaaaéb\r\n".as_bytes());
    let query = q("é", false, WordMatch::StartsWith);
    assert!(search(&mut t, query.clone()).is_empty());
    for _ in 0..8 {
        t.advance(b"x\r\n");
        t.state.refresh_search();
        assert_eq!(t.state.search_hits(), naive(&t, &query));
        if t.state.search_text_for_test().text.starts_with("é".as_bytes()) {
            assert_eq!(t.state.search_hits().len(), 1);
            return;
        }
    }
    panic!("the wrapped line's second row never became the first visible line");
}

/// A match at history's end is checked against the screen's first
/// scalar, so writing there changes whether it ends a word.
#[test]
fn history_hit_at_its_end_follows_the_screen() {
    // "y" ends history; the screen's rows are empty.
    let mut t = Term::new(8, 2, 100, crate::ImagePool::process());
    t.advance(b"y\r\n\r\n");
    let query = q("y\n", false, WordMatch::FullWord);
    assert_eq!(search(&mut t, query.clone()).len(), 1);
    for (write, hits) in [("c", 0), (" ", 1), ("d", 0)] {
        t.advance(format!("\x1b[H{write}").as_bytes());
        t.state.refresh_search();
        assert_eq!(t.state.search_hits(), naive(&t, &query), "after {write:?}");
        assert_eq!(t.state.search_hits().len(), hits, "after {write:?}");
    }
}

/// A query with line ends in it can start in history's last lines, not
/// only its last logical line, and run on to the screen.
#[test]
fn match_from_historys_line_ends_onto_the_screen() {
    for (out, query) in [("x\r\nab\r\n", "\nA"), ("x\r\n\r\nab\r\n", "\n\nA")] {
        let mut t = Term::new(8, 2, 100, crate::ImagePool::process());
        t.advance(out.as_bytes());
        let query = q(query, true, WordMatch::Contains);
        let hits = search(&mut t, query.clone());
        assert_eq!(hits, naive(&t, &query), "{out:?}");
        assert_eq!(hits.len(), 1, "{out:?}");
        assert_eq!((hits[0].line, hits[0].col), (0, 1), "{out:?}");
    }
}

/// A resize reflows the screen and the lines pulled back from history:
/// hits are then those of the reflowed text.
#[test]
fn resize_keeps_hits_equal_to_the_reflowed_text() {
    for query in [q("needle", false, WordMatch::Contains), q("needle", false, WordMatch::FullWord)] {
        let mut t = Term::new(12, 3, 1000, crate::ImagePool::process());
        for i in 0..20 {
            t.advance(format!("{i} xx needle yy needle\r\n").as_bytes());
        }
        t.advance(b"needle needle needle");
        assert_eq!(search(&mut t, query.clone()), naive(&t, &query));
        for (cols, lines) in [(7, 3), (5, 2), (16, 4), (9, 5)] {
            t.state.resize(cols, lines);
            t.state.refresh_search();
            assert_eq!(t.state.search_hits(), naive(&t, &query), "{cols}x{lines}");
            assert_eq!(t.state.search_hits().len(), 43, "{cols}x{lines}");
        }
    }
}

/// Found by `equals_naive_search`: a resize pulls lines back from
/// history, which lets lines hidden by the limit show again; their hits
/// are found.
#[test]
fn lines_shown_again_by_a_resize_are_searched() {
    let mut t = Term::new(6, 3, 8, crate::ImagePool::process());
    let query = q("A", false, WordMatch::Contains);
    search(&mut t, query.clone());
    t.advance(format!("aA\r\n{}\r\nab", "a".repeat(66)).as_bytes());
    t.state.refresh_search();
    assert_eq!(t.state.search_hits(), naive(&t, &query));
    assert!(t.state.search_hits().is_empty(), "\"aA\" is hidden by the limit");
    t.state.resize(14, 4);
    t.state.refresh_search();
    assert_eq!(t.state.search_hits(), naive(&t, &query));
    assert_eq!(t.state.search_hits().len(), 1);
}

const PIECES: &[&str] = &[
    "a", "b", "A", "B", "ab", "é", "É", "σ", "Σ", "ς", "e\u{301}", "中", " ", "_", "-", "\r\n", "aaa", "ß",
];

fn output() -> impl Strategy<Value = Vec<(usize, bool)>> {
    prop::collection::vec((0..PIECES.len(), prop::bool::weighted(0.04)), 0..400)
}

fn query() -> impl Strategy<Value = Query> {
    let word = prop::sample::select(vec![
        WordMatch::Contains,
        WordMatch::StartsWith,
        WordMatch::FullWord,
        WordMatch::EndsWith,
    ]);
    let pieces = prop::sample::select(vec!["a", "b", "A", "é", "É", "σ", "ς", " ", "e", "中", "\n", "_"]);
    (prop::collection::vec(pieces, 1..4), any::<bool>(), word).prop_map(|(t, ignore_case, word)| Query {
        text: t.concat(),
        ignore_case,
        word,
    })
}

fn feed(t: &mut Term, out: &[(usize, bool)]) {
    for &(p, seal) in out {
        t.advance(PIECES[p].as_bytes());
        if seal {
            t.state.seal_history_for_test();
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    /// Search finds what a naive search over the whole text finds, through
    /// block boundaries, hidden lines, output after it started and a
    /// resize (which reflows).
    #[test]
    fn equals_naive_search(
        cols in 6usize..14,
        limit in prop::sample::select(vec![8usize, 30, 1000]),
        before in output(),
        later in output(),
        query in query(),
        resize in prop::option::of((4usize..16, 1usize..6)),
    ) {
        let mut t = Term::new(cols, 3, limit, crate::ImagePool::process());
        feed(&mut t, &before);
        let hits = search(&mut t, query.clone());
        prop_assert_eq!(&hits, &naive(&t, &query));
        feed(&mut t, &later);
        t.state.refresh_search();
        prop_assert_eq!(t.state.search_hits(), naive(&t, &query));
        if let Some((cols, lines)) = resize {
            t.state.resize(cols, lines);
            t.state.refresh_search();
            prop_assert_eq!(t.state.search_hits(), naive(&t, &query));
        }
    }
}
