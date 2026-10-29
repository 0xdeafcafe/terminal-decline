//! Search in a pane (roadmap E.7, cells design §13): what the find bar
//! needs from a search engine ([`Search`]), and [`CoreSearch`], term-core's
//! own (block search off the lock, §13.4), the only engine there is.
//!
//! Hits are term-core [`Point`]s, which stay right while output scrolls.
//! Every call takes the terminal locked.

use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use term_core::{HitBatch, Point, SearchJob, Terminal};

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
    /// clears. The view may move to show a current hit, clear of the
    /// `covered` rows the bar is over.
    fn start(&mut self, t: &mut Terminal, q: &Query, covered: usize);
    /// Stop and forget the hits (the bar closed).
    fn cancel(&mut self, t: &mut Terminal);
    /// Once per frame while the bar is open: take in results and keep up
    /// with output.
    fn poll(&mut self, t: &mut Terminal) -> Polled;
    fn status(&self, t: &Terminal) -> Status;
    /// Make the next hit (`forward`) or the previous current, wrapping
    /// around at either end, and scroll to it. False when there is none
    /// (or none yet: it jumps when the hit arrives).
    fn step(&mut self, t: &mut Terminal, forward: bool, covered: usize) -> bool;
    fn current(&self, t: &Terminal) -> Option<Hit>;
}

/// If `at` is in view but under the `covered` rows the find bar is over,
/// scroll back just far enough to bring it out from under.
pub fn keep_clear(t: &mut Terminal, at: Point, covered: usize) {
    if let Some((row, _)) = t.view_position(at)
        && row < covered
    {
        t.scroll_view((covered - row) as isize);
    }
}

/// Hands a batch of history hits to the main thread, which adds them
/// with the terminal locked ([`Terminal::add_search_hits`]).
pub type Deliver = Arc<dyn Fn(HitBatch) + Send + Sync>;

/// term-core's search (cells design §13.4): the screen is searched under
/// the lock as the query changes, history on the one search worker, its
/// batches going back to the main thread through `deliver`. Hits and the
/// current one live in the terminal, which puts them in the snapshot.
pub struct CoreSearch {
    deliver: Deliver,
}

impl CoreSearch {
    pub fn new(deliver: Deliver) -> CoreSearch {
        CoreSearch { deliver }
    }
}

fn keep_clear_of_current(t: &mut Terminal, covered: usize) {
    if let Some(h) = t.current_hit() {
        keep_clear(t, h.start(), covered);
    }
}

fn core_query(q: &Query) -> term_core::Query {
    term_core::Query {
        text: q.text.clone(),
        ignore_case: q.ignore_case,
        word: match q.word {
            WordMatch::Contains => term_core::WordMatch::Contains,
            WordMatch::StartsWith => term_core::WordMatch::StartsWith,
            WordMatch::FullWord => term_core::WordMatch::FullWord,
        },
    }
}

fn core_hit(h: term_core::Hit) -> Hit {
    Hit {
        start: h.start(),
        end: Point {
            line: h.end_line(),
            col: h.end_col as usize,
        },
    }
}

impl Search for CoreSearch {
    fn start(&mut self, t: &mut Terminal, q: &Query, covered: usize) {
        let job = t.start_search(core_query(q));
        if q.text.is_empty() {
            return;
        }
        // A hit on the screen at or after the view's top becomes current
        // as you type; history's arrive from the worker.
        if t.search_count().total > 0 && t.search_next() {
            keep_clear_of_current(t, covered);
        }
        run_on_worker(job, self.deliver.clone());
    }

    fn cancel(&mut self, t: &mut Terminal) {
        t.stop_search();
    }

    fn poll(&mut self, _t: &mut Terminal) -> Polled {
        // The terminal keeps its hits up with output itself; the count is
        // compared by the caller.
        Polled::default()
    }

    fn status(&self, t: &Terminal) -> Status {
        let c = t.search_count();
        Status {
            count: c.total,
            current: c.current,
            running: c.running,
            capped: c.capped,
        }
    }

    fn step(&mut self, t: &mut Terminal, forward: bool, covered: usize) -> bool {
        let stepped = if forward { t.search_next() } else { t.search_previous() };
        if stepped {
            keep_clear_of_current(t, covered);
        }
        stepped
    }

    fn current(&self, t: &Terminal) -> Option<Hit> {
        t.current_hit().map(core_hit)
    }
}

/// The one search worker (§13.4): jobs run one at a time, in order; a
/// newer search cancels the job before it, which then stops within a
/// block.
fn run_on_worker(job: SearchJob, deliver: Deliver) {
    static WORKER: OnceLock<Mutex<mpsc::Sender<(SearchJob, Deliver)>>> = OnceLock::new();
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(SearchJob, Deliver)>();
        std::thread::Builder::new()
            .name("search".into())
            .spawn(move || {
                for (mut job, deliver) in rx {
                    job.run(|batch| deliver(batch));
                }
            })
            .expect("search worker starts");
        Mutex::new(tx)
    });
    let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send((job, deliver));
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::{Content, Term};

    fn term(cols: usize, lines: usize, text: &str) -> Term {
        let mut t = Term::new(cols, lines, 1000, term_core::ImagePool::process());
        t.advance(text.as_bytes());
        t
    }

    fn q(text: &str) -> Query {
        Query {
            text: text.into(),
            ..Query::default()
        }
    }

    /// 50 lines of 30 cells, `hit N` in line N % 10 == 0, the rest `x`s:
    /// written 20 columns wide (two stored lines each), then read at
    /// `cols`.
    fn wide_lines_at(cols: usize) -> Term {
        let mut out = String::new();
        for i in 0..50 {
            let text = if i % 10 == 0 { format!("hit {i}") } else { String::new() };
            out.push_str(&format!("{text:x<30}\r\n"));
        }
        let mut t = term(20, 6, &out);
        t.state.resize(cols, 6);
        t
    }

    /// The text of view row `row` from column `col`.
    fn text_at(t: &Term, row: usize, col: usize) -> String {
        let r = t.state.visible_row(row);
        (col..r.len())
            .filter(|&c| !r.cell(c).is_spacer())
            .map(|c| match r.cell(c).content() {
                Content::Char(ch) => ch,
                _ => ' ',
            })
            .collect()
    }

    #[test]
    fn core_find_next_lands_on_the_hit_after_a_width_change() {
        for cols in [40, 12, 8] {
            let mut t = wide_lines_at(cols);
            let mut job = t.state.start_search(core_query(&q("hit")));
            let mut batches = Vec::new();
            job.run(|b| batches.push(b));
            for b in batches {
                t.state.add_search_hits(b);
            }
            assert_eq!(t.state.search_count().total, 5, "at {cols}");
            for _ in 0..5 {
                assert!(t.state.search_next());
                let h = core_hit(t.state.current_hit().unwrap());
                let (row, col) = t.state.view_position(h.start).expect("the hit is in view");
                let n = h.start.line / 2;
                assert!(
                    text_at(&t, row, col).starts_with(&format!("hit {n}")),
                    "at {cols}: {:?}",
                    text_at(&t, row, col)
                );
            }
        }
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
        assert_eq!(s(100_000, Some(0), false, true).text(true), "100000+");
    }

    #[test]
    fn core_search_keeps_the_current_hit_out_from_under_the_bar() {
        // 41 lines: the screen (5 rows) shows lines 37..41, and the hit
        // is on line 37, its top row.
        let mut out = String::new();
        for i in 0..41 {
            out.push_str(&format!("{}\r\n", if i == 37 { "hit" } else { "miss" }));
        }
        let search = |covered: usize| {
            let mut t = term(20, 5, &out);
            let mut s = CoreSearch::new(Arc::new(|_| {}));
            s.start(&mut t.state, &q("hit"), covered);
            let h = s.current(&t.state).expect("a current hit");
            let row = t.state.view_position(h.start).expect("in view").0;
            (row, t.state.display_offset())
        };
        // Not covered: where it is.
        assert_eq!(search(0), (0, 0));
        // Two rows covered: scrolled back until it is just below them.
        assert_eq!(search(2), (2, 2));
    }
}
