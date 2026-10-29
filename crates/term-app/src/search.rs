//! Search in a pane (roadmap E.7, cells design §13): what the find bar
//! needs from a search engine ([`Search`]), and [`TextSearch`], a simple
//! engine that reads the screen and history through term-core's public
//! API. term-core's own engine (block search off the lock, §13.4) plugs in
//! later behind the same trait; the bar doesn't change.
//!
//! Hits are kept as term-core [`Point`]s, which stay right while output
//! scrolls. Every call takes the terminal locked; the fallback does all its
//! work there, so it reads at most [`MAX_ROWS`] rows nearest the bottom.

use std::ops::Range;
use std::time::{Duration, Instant};

use term_core::{Content, HighlightKind, Point, Snapshot, Terminal, ViewTop, Width};

/// How a match must sit against the words around it (the bar's options
/// menu, cells design §13.8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordMatch {
    #[default]
    Contains,
    StartsWith,
    FullWord,
}

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub ignore_case: bool,
    pub word: WordMatch,
}

/// One match: from `start` to `end`, `end` being the column just past its
/// last cell, on the line it ends on (a match can cross a soft wrap).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit {
    pub start: Point,
    pub end: Point,
}

/// What the bar shows: "3 of 120", or "120+" while it is still running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub count: usize,
    /// The current hit, from 0 in text order.
    pub current: Option<usize>,
    /// Still searching: more hits may come.
    pub running: bool,
    /// Stopped at the hit limit.
    pub capped: bool,
}

/// What a [`Search::poll`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Polled {
    /// The hits or the count changed: draw them, update the bar.
    pub changed: bool,
    /// Poll again after this long even if nothing is drawn (the text
    /// changed, and searching it again waits a little).
    pub again: Option<Duration>,
}

impl Status {
    /// The bar's count: "3 of 120", "120+" while running or at the
    /// limit, "Not Found"; nothing before a search.
    pub fn text(&self, searched: bool) -> String {
        match (self.count, self.current) {
            _ if !searched => String::new(),
            (0, _) if self.running => "Searching…".into(),
            (0, _) => "Not Found".into(),
            (n, _) if self.running || self.capped => format!("{n}+"),
            (n, Some(i)) => format!("{} of {n}", i + 1),
            (n, None) => format!("{n} found"),
        }
    }
}

/// A search engine as the find bar uses it. Every call is made with the
/// pane's terminal locked.
pub trait Search {
    /// Search for `q` instead of whatever was searched for; an empty text
    /// clears. The current hit is the first at or after the view's top
    /// (the last before it when there is none after).
    fn start(&mut self, t: &mut Terminal, q: &Query);
    /// Stop and forget the hits (the bar closed).
    fn cancel(&mut self, t: &mut Terminal);
    /// Once per frame while the bar is open: take in results and keep up
    /// with output.
    fn poll(&mut self, t: &mut Terminal) -> Polled;
    fn status(&self) -> Status;
    /// Make the next hit (`forward`) or the previous current, wrapping
    /// around at either end; returns it.
    fn step(&mut self, forward: bool) -> Option<Hit>;
    fn current(&self) -> Option<Hit>;
    /// The hits in view, for the snapshot's highlights: (view row,
    /// columns, kind). An engine that puts its hits in the snapshot
    /// itself leaves `out` alone.
    fn view_highlights(&self, t: &Terminal, out: &mut Vec<(u16, Range<u16>, HighlightKind)>);
}

/// Scroll the view so `hit` is in it: not at all when it already is
/// (below the `covered` rows the find bar is over), else with its line in
/// the middle (as near as history allows).
pub fn scroll_to(t: &mut Terminal, hit: Hit, covered: usize) {
    let top = t.point_at(0, 0).line;
    let lines = t.lines() as u64;
    if (top + covered as u64..top + lines).contains(&hit.start.line) {
        return;
    }
    let screen_top = top + t.display_offset() as u64;
    let want = hit.start.line.saturating_sub(lines / 2);
    if want >= screen_top {
        t.scroll_view_to_bottom();
    } else {
        t.set_view_top(ViewTop::At {
            point: Point { line: want, col: 0 },
            offset: 0,
        });
    }
}

/// Put `extra` (view row, columns, kind; in row order) into `snap`'s
/// highlights, keeping each row's left to right.
pub fn add_highlights(snap: &mut Snapshot, extra: &[(u16, Range<u16>, HighlightKind)]) {
    if extra.is_empty() {
        return;
    }
    let old = std::mem::take(&mut snap.highlights);
    let mut out = Vec::with_capacity(old.len() + extra.len());
    let mut next = 0;
    for (r, row) in snap.rows.iter_mut().enumerate() {
        let start = out.len();
        out.extend_from_slice(&old[row.highlights.start as usize..row.highlights.end as usize]);
        while next < extra.len() && (extra[next].0 as usize) < r {
            next += 1;
        }
        while next < extra.len() && extra[next].0 as usize == r {
            let (_, cols, kind) = &extra[next];
            out.push(term_core::Highlight {
                cols: cols.clone(),
                kind: *kind,
            });
            next += 1;
        }
        out[start..].sort_by_key(|h| h.cols.start);
        row.highlights = start as u32..out.len() as u32;
    }
    snap.highlights = out;
}

/// The fallback reads at most this many rows, nearest the bottom.
pub const MAX_ROWS: usize = 50_000;
/// It keeps at most this many hits.
pub const MAX_HITS: usize = 100_000;
/// While output changes the text, it searches again at most this often.
const AGAIN_AFTER: Duration = Duration::from_millis(300);

/// The fallback engine: reads the rows' text with the terminal locked and
/// matches it there, once when the query changes and again after output.
#[derive(Debug, Default)]
pub struct TextSearch {
    query: Option<Query>,
    hits: Vec<Hit>,
    current: Option<usize>,
    capped: bool,
    /// The terminal's generation the hits were found at, and when.
    searched: Option<(u64, Instant)>,
}

impl TextSearch {
    pub fn new() -> TextSearch {
        TextSearch::default()
    }

    /// The hits, in text order (for tests).
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    fn run(&mut self, t: &mut Terminal) {
        self.hits.clear();
        self.capped = false;
        self.searched = Some((t.generation(), Instant::now()));
        let Some(q) = &self.query else { return };
        let needle: Vec<char> = q.text.chars().map(|c| fold(c, q.ignore_case)).collect();
        if needle.is_empty() {
            return;
        }
        let (hits, capped) = (&mut self.hits, &mut self.capped);
        each_logical_line(t, |units| {
            if !*capped {
                find_in(units, &needle, q, hits);
                if hits.len() >= MAX_HITS {
                    hits.truncate(MAX_HITS);
                    *capped = true;
                }
            }
        });
    }

    /// The first hit at or after the view's top, else the last.
    fn pick_current(&mut self, t: &Terminal) {
        let top = t.point_at(0, 0);
        self.current = match self.hits.partition_point(|h| h.start < top) {
            _ if self.hits.is_empty() => None,
            i if i < self.hits.len() => Some(i),
            _ => Some(self.hits.len() - 1),
        };
    }
}

impl Search for TextSearch {
    fn start(&mut self, t: &mut Terminal, q: &Query) {
        self.query = (!q.text.is_empty()).then(|| q.clone());
        self.run(t);
        self.pick_current(t);
    }

    fn cancel(&mut self, _t: &mut Terminal) {
        *self = TextSearch::default();
    }

    fn poll(&mut self, t: &mut Terminal) -> Polled {
        let Some((generation, at)) = self.searched else {
            return Polled::default();
        };
        if self.query.is_none() || generation == t.generation() {
            return Polled::default();
        }
        if at.elapsed() < AGAIN_AFTER {
            return Polled {
                changed: false,
                again: Some(AGAIN_AFTER - at.elapsed()),
            };
        }
        let was = self.current.map(|i| self.hits[i]);
        let before = (self.hits.len(), self.capped);
        let old = std::mem::take(&mut self.hits);
        self.run(t);
        // The same hit stays current, or the next one after where it was.
        self.current = match was {
            None => None,
            Some(_) if self.hits.is_empty() => None,
            Some(h) => Some(self.hits.partition_point(|x| x.start < h.start).min(self.hits.len() - 1)),
        };
        if was.is_none() && !self.hits.is_empty() {
            self.pick_current(t);
        }
        Polled {
            changed: old != self.hits || before != (self.hits.len(), self.capped),
            again: None,
        }
    }

    fn status(&self) -> Status {
        Status {
            count: self.hits.len(),
            current: self.current,
            running: false,
            capped: self.capped,
        }
    }

    fn step(&mut self, forward: bool) -> Option<Hit> {
        let n = self.hits.len();
        if n == 0 {
            return None;
        }
        let i = match self.current {
            None if forward => 0,
            None => n - 1,
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
        };
        self.current = Some(i);
        Some(self.hits[i])
    }

    fn current(&self) -> Option<Hit> {
        self.current.map(|i| self.hits[i])
    }

    fn view_highlights(&self, t: &Terminal, out: &mut Vec<(u16, Range<u16>, HighlightKind)>) {
        let top = t.point_at(0, 0).line;
        let lines = t.lines() as u64;
        let cols = t.cols();
        let current = self.current();
        let first = self.hits.partition_point(|h| h.end.line < top);
        let mut rows: Vec<(u16, Range<u16>, HighlightKind)> = Vec::new();
        for h in self.hits[first..].iter().take_while(|h| h.start.line < top + lines) {
            let kind = if Some(*h) == current {
                HighlightKind::CurrentSearchHit
            } else {
                HighlightKind::SearchHit
            };
            for line in h.start.line.max(top)..=h.end.line.min(top + lines - 1) {
                let a = if line == h.start.line { h.start.col } else { 0 };
                let b = if line == h.end.line { h.end.col } else { cols };
                if a < b {
                    rows.push(((line - top) as u16, a as u16..b.min(cols) as u16, kind));
                }
            }
        }
        rows.sort_by_key(|(r, c, _)| (*r, c.start));
        out.extend(rows);
    }
}

/// One character of a logical line, and the cell it came from.
#[derive(Clone, Copy, Debug)]
struct Unit {
    ch: char,
    line: u64,
    col: u16,
    width: u8,
}

/// `f` on each logical line's characters (soft-wrapped rows joined), from
/// [`MAX_ROWS`] rows above the bottom of the screen to its end. Trailing
/// blank cells are left off.
///
/// term-core reads rows only relative to the view (`visible_row`), so the
/// view is scrolled all the way back to read them and put back after.
fn each_logical_line(t: &mut Terminal, mut f: impl FnMut(&[Unit])) {
    let lines = t.lines();
    let history = if t.alt_screen_active() { 0 } else { t.scrollback_len() };
    let offset = t.display_offset();
    // The line number of history's first row.
    let base = t.point_at(0, 0).line + offset as u64 - history as u64;
    if history > 0 {
        t.scroll_view(history as isize - offset as isize);
    }
    let total = history + lines;
    let first = total.saturating_sub(MAX_ROWS);
    let mut units: Vec<Unit> = Vec::new();
    // Units up to here are not trailing blanks.
    let mut kept = 0;
    // A logical line that started above the first row read is skipped.
    let mut skipping = first > 0 && t.visible_row(first - 1).wrapped();
    for i in first..total {
        let row = t.visible_row(i);
        let line = base + i as u64;
        if !skipping {
            for col in 0..row.len() {
                let cell = row.cell(col);
                if cell.is_spacer() {
                    continue;
                }
                let width = if cell.width() == Width::Wide { 2 } else { 1 };
                let unit = |ch| Unit {
                    ch,
                    line,
                    col: col as u16,
                    width,
                };
                match cell.content() {
                    Content::Char(c) => units.push(unit(c)),
                    Content::Cluster(s) => units.extend(s.chars().map(unit)),
                    _ => units.push(unit(' ')),
                }
                if !cell.is_blank() {
                    kept = units.len();
                }
            }
        }
        if row.wrapped() {
            // Blanks inside a logical line are part of it.
            kept = units.len();
        } else {
            if !skipping {
                units.truncate(kept);
                f(&units);
            }
            units.clear();
            kept = 0;
            skipping = false;
        }
    }
    if !units.is_empty() && !skipping {
        units.truncate(kept);
        f(&units);
    }
    if history > 0 {
        t.scroll_view(offset as isize - history as isize);
    }
}

fn fold(c: char, ignore_case: bool) -> char {
    if ignore_case { c.to_lowercase().next().unwrap_or(c) } else { c }
}

/// Search's word rule (cells design §13.8): letters, digits and `_`.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The matches of `needle` (already folded) in one logical line.
fn find_in(units: &[Unit], needle: &[char], q: &Query, out: &mut Vec<Hit>) {
    let n = needle.len();
    if units.len() < n {
        return;
    }
    let mut i = 0;
    while i + n <= units.len() {
        let matched = units[i..i + n].iter().zip(needle).all(|(u, c)| fold(u.ch, q.ignore_case) == *c);
        let words = matched
            && match q.word {
                WordMatch::Contains => true,
                WordMatch::StartsWith => i == 0 || !is_word(units[i - 1].ch),
                WordMatch::FullWord => (i == 0 || !is_word(units[i - 1].ch)) && units.get(i + n).is_none_or(|u| !is_word(u.ch)),
            };
        if !words {
            // Word checks need overlapping candidates.
            i += 1;
            continue;
        }
        let (a, b) = (units[i], units[i + n - 1]);
        out.push(Hit {
            start: Point {
                line: a.line,
                col: a.col as usize,
            },
            end: Point {
                line: b.line,
                col: b.col as usize + b.width as usize,
            },
        });
        // A cluster's characters share a cell: the match ends after it.
        i += n;
        while i < units.len() && units[i].line == b.line && units[i].col == b.col {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::Term;

    fn term(cols: usize, lines: usize, text: &str) -> Term {
        let mut t = Term::new(cols, lines, 1000);
        t.advance(text.as_bytes());
        t
    }

    fn q(text: &str) -> Query {
        Query {
            text: text.into(),
            ..Query::default()
        }
    }

    fn p(line: u64, col: usize) -> Point {
        Point { line, col }
    }

    #[test]
    fn finds_on_screen_and_in_history() {
        let mut out = String::new();
        for i in 0..30 {
            out.push_str(&format!("line {i} foo\r\n"));
        }
        let mut t = term(20, 5, &out);
        let mut s = TextSearch::new();
        s.start(&mut t.state, &q("foo"));
        assert_eq!(s.status().count, 30);
        assert_eq!(
            s.hits()[0],
            Hit {
                start: p(0, 7),
                end: p(0, 10)
            }
        );
        // The view is at the bottom: the first hit at or after its top.
        let top = t.state.point_at(0, 0).line;
        assert_eq!(s.current().unwrap().start.line, top);
        assert_eq!(t.state.display_offset(), 0, "the view is put back");
    }

    #[test]
    fn next_and_previous_wrap_and_scroll() {
        let mut out = String::new();
        for i in 0..40 {
            out.push_str(&format!("{}\r\n", if i % 10 == 0 { "hit" } else { "miss" }));
        }
        let mut t = term(20, 5, &out);
        let mut s = TextSearch::new();
        s.start(&mut t.state, &q("hit"));
        assert_eq!(s.status().count, 4);
        // No hit at or after the top (lines 36..41): the last is current.
        assert_eq!(s.status().current, Some(3));
        let h = s.step(true).unwrap();
        assert_eq!(h.start.line, 0);
        scroll_to(&mut t.state, h, 0);
        assert!(t.state.display_offset() > 0);
        let top = t.state.point_at(0, 0).line;
        assert!((top..top + 5).contains(&0));
        let h = s.step(false).unwrap();
        assert_eq!(h.start.line, 30);
        scroll_to(&mut t.state, h, 0);
        let top = t.state.point_at(0, 0).line;
        assert!((top..top + 5).contains(&30), "top {top}");
        // A hit under the find bar is scrolled out from under it.
        let before = t.state.display_offset();
        let under = Hit {
            start: p(top, 0),
            end: p(top, 1),
        };
        scroll_to(&mut t.state, under, 0);
        assert_eq!(t.state.display_offset(), before);
        scroll_to(&mut t.state, under, 2);
        assert!(t.state.display_offset() > before);
    }

    #[test]
    fn matches_cross_soft_wraps() {
        // 10 columns: "0123456789abc" wraps after 9.
        let mut t = term(10, 4, "xxxxxxxhello\r\n");
        let mut s = TextSearch::new();
        s.start(&mut t.state, &q("hello"));
        assert_eq!(
            s.hits(),
            &[Hit {
                start: p(0, 7),
                end: p(1, 2)
            }]
        );
        let mut hl = Vec::new();
        s.view_highlights(&t.state, &mut hl);
        assert_eq!(
            hl,
            vec![
                (0, 7..10, HighlightKind::CurrentSearchHit),
                (1, 0..2, HighlightKind::CurrentSearchHit)
            ]
        );
    }

    #[test]
    fn case_and_words() {
        let mut t = term(40, 3, "Foo foobar barfoo foo_x FOO\r\n");
        let mut s = TextSearch::new();
        s.start(&mut t.state, &q("foo"));
        assert_eq!(s.status().count, 3);
        let ci = |word| Query {
            text: "foo".into(),
            ignore_case: true,
            word,
        };
        s.start(&mut t.state, &ci(WordMatch::Contains));
        assert_eq!(s.status().count, 5);
        s.start(&mut t.state, &ci(WordMatch::StartsWith));
        let starts: Vec<usize> = s.hits().iter().map(|h| h.start.col).collect();
        assert_eq!(starts, vec![0, 4, 18, 24]);
        s.start(&mut t.state, &ci(WordMatch::FullWord));
        let starts: Vec<usize> = s.hits().iter().map(|h| h.start.col).collect();
        assert_eq!(starts, vec![0, 24]);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let mut t = term(20, 3, "a日本b\r\n");
        let mut s = TextSearch::new();
        s.start(&mut t.state, &q("本b"));
        assert_eq!(
            s.hits(),
            &[Hit {
                start: p(0, 3),
                end: p(0, 6)
            }]
        );
    }

    #[test]
    fn trailing_blanks_dont_match() {
        let mut t = term(20, 3, "ab\r\n");
        let mut s = TextSearch::new();
        s.start(&mut t.state, &q("b "));
        assert_eq!(s.status().count, 0);
        s.start(&mut t.state, &q(""));
        assert_eq!(s.status(), Status::default());
    }

    #[test]
    fn output_is_searched_again() {
        let mut t = term(20, 5, "one\r\n");
        let mut s = TextSearch::new();
        s.start(&mut t.state, &q("one"));
        assert_eq!(s.status().count, 1);
        t.advance(b"one again\r\n");
        let soon = s.poll(&mut t.state);
        assert!(!soon.changed && soon.again.is_some(), "not this soon");
        s.searched = s.searched.map(|(g, at)| (g, at - AGAIN_AFTER));
        assert!(s.poll(&mut t.state).changed);
        assert_eq!(s.status().count, 2);
        assert_eq!(s.status().current, Some(0), "the same hit stays current");
    }

    #[test]
    fn count_text() {
        let s = |count, current, running, capped| Status {
            count,
            current,
            running,
            capped,
        };
        assert_eq!(s(0, None, false, false).text(false), "");
        assert_eq!(s(0, None, false, false).text(true), "Not Found");
        assert_eq!(s(120, Some(2), false, false).text(true), "3 of 120");
        assert_eq!(s(120, Some(2), true, false).text(true), "120+");
        assert_eq!(s(MAX_HITS, Some(0), false, true).text(true), "100000+");
    }

    #[test]
    fn highlights_go_into_the_snapshot() {
        let mut t = term(20, 3, "ab foo\r\n");
        t.state.set_selection(Some(term_core::Selection::new(p(0, 0), p(0, 1))));
        let mut snap = Snapshot::default();
        t.state.snapshot(&mut snap, |_| false);
        add_highlights(
            &mut snap,
            &[(0, 3..6, HighlightKind::SearchHit), (2, 0..1, HighlightKind::CurrentSearchHit)],
        );
        let kinds = |r: usize| -> Vec<(Range<u16>, HighlightKind)> {
            snap.row_highlights(&snap.rows[r])
                .iter()
                .map(|h| (h.cols.clone(), h.kind))
                .collect()
        };
        assert_eq!(kinds(0), vec![(0..2, HighlightKind::Selection), (3..6, HighlightKind::SearchHit)]);
        assert!(kinds(1).is_empty());
        assert_eq!(kinds(2), vec![(0..1, HighlightKind::CurrentSearchHit)]);
    }
}
