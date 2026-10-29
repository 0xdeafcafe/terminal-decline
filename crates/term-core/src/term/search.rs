//! The terminal's side of search (§13.4, §13.8): start a search, add the
//! worker's hits, keep up with output, find next and previous, and the
//! hits the snapshot highlights. The engine is in `crate::search`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{Point, Selection, Terminal, ViewTop};
use crate::cell::Row;
use crate::search::{Cells, HIT_CAP, Hit, HitBatch, Mapper, Matcher, Query, SearchJob, Seg, WordMatch, char_at, scan_seg};

/// Where a search is, for the find bar's count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchCount {
    /// Hits found so far (on the alternate screen, the screen's only).
    pub total: usize,
    /// Which of them is the current one (0-based), if any.
    pub current: Option<usize>,
    /// History is still being searched ("340+").
    pub running: bool,
    /// The hit cap was reached.
    pub capped: bool,
}

/// One search, kept by the terminal while the find bar is open.
pub(crate) struct SearchState {
    matcher: Option<Arc<Matcher>>,
    generation: u64,
    cancel: Arc<AtomicU64>,
    /// History's hits per block (its first line), blocks in order, hits
    /// in text order: freeing a block drops its entry.
    blocks: VecDeque<(u64, Vec<Hit>)>,
    /// Hits that end on the screen (some start in history's last line).
    screen: Vec<Hit>,
    /// What the screen hits were found on: each row's `seq`, its first
    /// line, and which screen.
    screen_seqs: Vec<u64>,
    screen_top: u64,
    screen_alt: bool,
    /// History lines below this have been searched (by the job or the
    /// live rule).
    history_to: u64,
    /// The first visible history line when hits were last brought up to
    /// date: hiding lines before it, or showing them again, can change
    /// whether its start is a word's start; lines shown again are new.
    first_visible: u64,
    /// Where history's text ended (after its last cell or line end) when
    /// the screen was last searched, and when the job was taken. Ends With
    /// and Full Word look at the scalar after a match, so a match ending
    /// there depends on the screen: the screen's search keeps those up to
    /// date, and the job's are dropped.
    history_end: Option<(u64, u16)>,
    job_end: Option<(u64, u16)>,
    /// Lowest line popped since the search started: worker hits at or
    /// after it are dropped (those lines are searched again as output).
    popped_low: Option<u64>,
    epoch: u64,
    current: Option<Hit>,
    /// A find next (true) or previous waiting for the worker.
    pending: Option<bool>,
    done: bool,
    capped: bool,
}

/// Where a hit ends: its line, and the column after it.
fn end(h: &Hit) -> (u64, u16) {
    (h.end_line(), h.end_col)
}

/// Whether the query's word check looks at the scalar after a match.
fn looks_after(m: &Matcher) -> bool {
    matches!(m.word(), WordMatch::EndsWith | WordMatch::FullWord)
}

/// Process-wide search generations: a newer one cancels older jobs.
static GENERATIONS: AtomicU64 = AtomicU64::new(1);

impl SearchState {
    fn total(&self) -> usize {
        self.screen.len() + self.blocks.iter().map(|b| b.1.len()).sum::<usize>()
    }

    /// Every hit, in order (history's only on the primary screen).
    fn all(&self, alt: bool) -> impl Iterator<Item = &Hit> {
        let history = if alt {
            None
        } else {
            Some(self.blocks.iter().flat_map(|b| b.1.iter()))
        };
        history.into_iter().flatten().chain(self.screen.iter())
    }

    /// Drop history's hit that starts at `at`, if there is one.
    fn drop_starting_at(&mut self, at: (u64, u16)) {
        let at = Point {
            line: at.0,
            col: at.1 as usize,
        };
        for b in self.blocks.iter_mut().rev() {
            b.1.retain(|h| h.start() != at);
            if b.0 <= at.line {
                break;
            }
        }
        self.blocks.retain(|b| !b.1.is_empty());
        if self.current.is_some_and(|c| c.start() == at) {
            self.current = None;
        }
    }

    /// Drop history's hits that end at `at`.
    fn drop_ending_at(&mut self, at: (u64, u16)) {
        for b in self.blocks.iter_mut().rev() {
            b.1.retain(|h| end(h) != at);
            if b.0 <= at.0 {
                break;
            }
        }
        self.blocks.retain(|b| !b.1.is_empty());
        if self.current.is_some_and(|c| end(&c) == at) {
            self.current = None;
        }
    }

    fn add(&mut self, block: u64, mut hits: Vec<Hit>) {
        if hits.is_empty() {
            return;
        }
        let room = HIT_CAP.saturating_sub(self.total());
        if hits.len() > room {
            hits.truncate(room);
            self.capped = true;
        }
        let i = self.blocks.partition_point(|b| b.0 < block);
        match self.blocks.get_mut(i) {
            Some(b) if b.0 == block => {
                b.1.extend(hits);
                b.1.sort_unstable();
                b.1.dedup_by_key(|h| h.start());
            }
            _ => self.blocks.insert(i, (block, hits)),
        }
    }
}

impl Terminal {
    /// Under the lock: start searching for `q` (§13.4). The screen is
    /// searched now and its hits are drawn in the next frame; the job
    /// returned searches history on the search worker, its batches going
    /// to [`Terminal::add_search_hits`]. Starting another search, or
    /// [`Terminal::stop_search`], cancels this one's job. An empty query
    /// clears the search.
    pub fn start_search(&mut self, q: Query) -> SearchJob {
        let generation = GENERATIONS.fetch_add(1, Ordering::Relaxed);
        let cancel = self
            .search
            .as_ref()
            .map_or_else(|| Arc::new(AtomicU64::new(0)), |s| s.cancel.clone());
        cancel.store(generation, Ordering::Relaxed);
        let matcher = Matcher::new(&q).map(Arc::new);
        let _ = self.scrollback.take_search_popped();
        let mut state = SearchState {
            matcher: matcher.clone(),
            generation,
            cancel: cancel.clone(),
            blocks: VecDeque::new(),
            screen: Vec::new(),
            screen_seqs: Vec::new(),
            screen_top: 0,
            screen_alt: false,
            history_to: self.scrollback.total(),
            first_visible: self.scrollback.first_visible(),
            history_end: None,
            job_end: None,
            popped_low: None,
            epoch: self.history_epoch,
            current: None,
            pending: None,
            done: matcher.is_none(),
            capped: false,
        };
        self.search_screen(&mut state);
        state.job_end = state.history_end;
        let (mut segs, mut start, mut after) = (Vec::new(), 0, None);
        if matcher.is_some() && !self.alt_active {
            let (blocks, visible) = self.scrollback.search_blocks(0);
            segs = blocks.into_iter().map(|(d, len)| Seg::new(d, len, visible)).collect();
            let top = self.top_line - self.display_offset as u64;
            start = segs.partition_point(|s| s.data.first() <= top).saturating_sub(1);
            after = self.screen_first_char();
        }
        if segs.is_empty() {
            state.done = true;
        }
        self.search = Some(Box::new(state));
        self.touch();
        SearchJob::new(matcher, segs, start, after, generation, cancel)
    }

    /// Close the search: its job stops, its hits go.
    pub fn stop_search(&mut self) {
        if let Some(s) = self.search.take() {
            s.cancel.store(0, Ordering::Relaxed);
            self.touch();
        }
    }

    /// Under the lock, briefly: add a batch of history hits from the
    /// worker. Batches from an older search are ignored. Returns whether
    /// the view should be redrawn (a hit is in view, or a find next
    /// waiting for it jumped).
    pub fn add_search_hits(&mut self, batch: HitBatch) -> bool {
        self.refresh_search();
        let Some(mut s) = self.search.take() else { return false };
        if batch.generation != s.generation {
            self.search = Some(s);
            return false;
        }
        let first = self.scrollback.first_visible();
        let popped = s.popped_low;
        let mut hits = batch.hits;
        let job_end = s.job_end.filter(|_| s.matcher.as_deref().is_some_and(looks_after));
        hits.retain(|h| h.line >= first && popped.is_none_or(|p| h.end_line() < p) && Some(end(h)) != job_end);
        let in_view = hits
            .iter()
            .any(|h| self.view_position(h.start()).is_some() || self.view_position(h.last()).is_some());
        s.add(batch.block, hits);
        s.done |= batch.done;
        let pending = s.pending;
        self.search = Some(s);
        let mut jumped = false;
        if let Some(forward) = pending {
            jumped = self.find_hit(forward, true);
        }
        if in_view || jumped || batch.done {
            self.touch();
        }
        in_view || jumped
    }

    /// The count for the find bar.
    pub fn search_count(&self) -> SearchCount {
        let Some(s) = &self.search else { return SearchCount::default() };
        let total = if self.alt_active { s.screen.len() } else { s.total() };
        let current = s.current.and_then(|c| s.all(self.alt_active).position(|h| *h == c));
        SearchCount {
            total,
            current,
            running: !s.done,
            capped: s.capped,
        }
    }

    /// The current hit, if any.
    pub fn current_hit(&self) -> Option<Hit> {
        self.search.as_ref()?.current
    }

    /// Every hit found so far, in text order (for tests and the app).
    pub fn search_hits(&self) -> Vec<Hit> {
        self.search
            .as_ref()
            .map_or_else(Vec::new, |s| s.all(self.alt_active).copied().collect())
    }

    /// Find next (Cmd+G): the first hit after the current one, or after the
    /// view's top when there is none; it is selected and scrolled to.
    /// Wraps around at the end once history is searched through; until
    /// then a jump with nothing after waits for the worker's hits. Returns
    /// whether it jumped now.
    pub fn search_next(&mut self) -> bool {
        self.refresh_search();
        self.find_hit(true, false)
    }

    /// Find previous (Shift+Cmd+G): as [`Terminal::search_next`], backwards.
    pub fn search_previous(&mut self) -> bool {
        self.refresh_search();
        self.find_hit(false, false)
    }

    fn find_hit(&mut self, forward: bool, from_pending: bool) -> bool {
        let alt = self.alt_active;
        let top = self.point_at(0, 0);
        let Some(s) = self.search.as_mut() else { return false };
        let anchor = s.current.map(|h| h.start());
        let found = if forward {
            let after = |h: &&Hit| anchor.map_or(h.start() >= top, |a| h.start() > a);
            s.all(alt).find(after).copied()
        } else {
            let before = |h: &&Hit| anchor.map_or(h.start() < top, |a| h.start() < a);
            s.all(alt).filter(before).last().copied()
        };
        let found = match found {
            Some(h) => Some(h),
            None if s.done || alt => {
                if forward {
                    s.all(alt).next().copied()
                } else {
                    s.all(alt).last().copied()
                }
            }
            None => {
                s.pending = Some(forward);
                return false;
            }
        };
        if from_pending && found.is_none() {
            return false;
        }
        s.pending = None;
        let Some(h) = found else { return false };
        s.current = Some(h);
        self.selection = Some(Selection::new(h.start(), h.last()));
        if self.view_position(h.start()).is_none() {
            self.scroll_to(h.start());
        }
        self.touch();
        true
    }

    /// Where `p` is shown now, as (view row, column); None if it isn't in
    /// view.
    pub fn view_position(&self, p: Point) -> Option<(usize, usize)> {
        let top = self.point_at(0, 0).line;
        (p.line >= top && p.line < top + self.lines as u64).then(|| ((p.line - top) as usize, p.col))
    }

    /// Scroll so that `p`'s row is in view, in the middle when that means
    /// scrolling back.
    pub fn scroll_to(&mut self, p: Point) {
        if self.view_position(p).is_some() {
            return;
        }
        let want = p.line.saturating_sub(self.lines as u64 / 2);
        if want >= self.top_line {
            self.scroll_view_to_bottom();
        } else {
            self.set_view_top(ViewTop::At {
                point: Point { line: want, col: 0 },
                offset: 0,
            });
        }
    }

    /// The search highlights on buffer line `line`: (columns, current).
    pub(crate) fn search_highlights(&self, line: u64, out: &mut Vec<(std::ops::Range<usize>, bool)>) {
        let Some(s) = &self.search else { return };
        let cols = self.cols;
        let mut push = |h: &Hit| {
            if let Some(r) = h.cols_on(line, cols) {
                out.push((r, s.current == Some(*h)));
            }
        };
        // A hit spans at most a few lines: look from a little before.
        let from = line.saturating_sub(64);
        if !self.alt_active {
            let i = s.blocks.partition_point(|b| b.0 <= from).saturating_sub(1);
            for (_, hits) in s.blocks.range(i..) {
                let j = hits.partition_point(|h| h.line < from);
                for h in &hits[j..] {
                    if h.line > line {
                        break;
                    }
                    push(h);
                }
                if hits.last().is_some_and(|h| h.line > line) {
                    break;
                }
            }
        }
        let j = s.screen.partition_point(|h| h.line < from);
        for h in &s.screen[j..] {
            if h.line > line {
                break;
            }
            push(h);
        }
    }

    /// At snapshot time (and before using hits): keep up with output
    /// (§13.4's live rule). New history lines are searched (from a match's
    /// reach before them), changed screens searched again, hits on freed
    /// or popped lines dropped; after ED 3 or RIS, history's hits go.
    pub(crate) fn refresh_search(&mut self) {
        let Some(mut s) = self.search.take() else { return };
        if s.matcher.is_none() {
            self.search = Some(s);
            return;
        }
        let mut changed = false;
        if s.epoch != self.history_epoch {
            s.epoch = self.history_epoch;
            s.blocks.clear();
            s.history_to = self.scrollback.total();
            s.done = true;
            // The job's history is gone: stop it.
            s.generation = GENERATIONS.fetch_add(1, Ordering::Relaxed);
            s.cancel.store(s.generation, Ordering::Relaxed);
            changed = true;
        }
        if let Some(p) = self.scrollback.take_search_popped() {
            s.popped_low = Some(s.popped_low.map_or(p, |q| q.min(p)));
            s.history_to = s.history_to.min(p);
            for b in s.blocks.iter_mut().rev() {
                if b.0 < p && b.1.last().is_none_or(|h| h.end_line() < p) {
                    break;
                }
                b.1.retain(|h| h.end_line() < p);
            }
            s.blocks.retain(|b| !b.1.is_empty());
            changed = true;
        }
        // Freed and hidden lines take their hits with them.
        let first = self.scrollback.first_visible();
        while let Some(b) = s.blocks.front_mut() {
            if b.1.first().is_some_and(|h| h.line >= first) {
                break;
            }
            b.1.retain(|h| h.line >= first);
            changed = true;
            if b.1.is_empty() {
                s.blocks.pop_front();
            } else {
                break;
            }
        }
        if self.scrollback.total() > s.history_to {
            self.search_new_history(&mut s);
            changed = true;
        }
        let first_moved = s.first_visible != first;
        if first_moved {
            let old = s.first_visible;
            self.search_history_start(&mut s, old);
            changed = true;
        }
        s.first_visible = first;
        let screen_changed = first_moved
            || s.screen_alt != self.alt_active
            || s.screen_top != self.top_line
            || s.screen_seqs.len() != self.screen.len()
            || self.screen.iter().zip(&s.screen_seqs).any(|(r, &q)| r.seq != q);
        if screen_changed {
            self.search_screen(&mut s);
            changed = true;
        }
        if let Some(c) = s.current
            && !s.all(self.alt_active).any(|h| *h == c)
        {
            s.current = None;
        }
        self.search = Some(s);
        if changed {
            self.touch();
        }
    }

    /// The live rule for history: lines from `history_to` on, with the
    /// text a match could start in before them.
    fn search_new_history(&mut self, s: &mut SearchState) {
        let Some(m) = s.matcher.clone() else { return };
        let from_line = s.history_to;
        s.history_to = self.scrollback.total();
        let (blocks, visible) = self.scrollback.search_blocks(from_line.saturating_sub(1));
        let mut segs: Vec<Seg> = blocks.into_iter().map(|(d, len)| Seg::new(d, len, visible)).collect();
        let from_line = from_line.max(visible);
        let Some(j) = segs.iter().position(|g| g.data.first() + g.len as u64 > from_line) else {
            return;
        };
        let n = (from_line - segs[j].data.first()) as usize;
        let off = segs[j].line_start(n);
        let after = self.screen_first_char();
        let reach = m.reach();
        let (mut scratch, mut raw, mut found) = (Vec::new(), Vec::new(), Vec::new());
        // Where the old text ended, as a place in the block before when the
        // new lines start a block.
        let mut old_end = (j, off);
        if j > 0 {
            // The boundary's matches, and those ending at it (below).
            let (_, wb) = segs[j - 1].window();
            if n == 0 {
                old_end = (j - 1, wb);
            }
            let from = Some(wb.saturating_sub(reach));
            scan_seg(&m, &mut segs, j - 1, from, false, after, &mut scratch, &mut raw, &mut found);
        }
        scan_seg(
            &m,
            &mut segs,
            j,
            Some(off.saturating_sub(reach)),
            false,
            after,
            &mut scratch,
            &mut raw,
            &mut found,
        );
        for i in j + 1..segs.len() {
            scan_seg(&m, &mut segs, i, None, false, after, &mut scratch, &mut raw, &mut found);
        }
        // A match ending where history's text ended was checked against
        // the screen's first scalar: check it again against the new line.
        let at_old_end = looks_after(&m) && s.history_end.is_some();
        if at_old_end && let Some(e) = s.history_end {
            s.drop_ending_at(e);
        }
        found.retain(|&(_, b)| b > (j, off) || (at_old_end && b == old_end));
        let mut mapper = Mapper::default();
        let mut per: Vec<(u64, Vec<Hit>)> = Vec::new();
        for (a, b) in found {
            if let Some(h) = mapper.hit(&mut segs, a, b) {
                let block = segs[a.0].data.first();
                match per.last_mut() {
                    Some(p) if p.0 == block => p.1.push(h),
                    _ => per.push((block, vec![h])),
                }
            }
        }
        for (block, mut hits) in per {
            hits.sort_unstable();
            s.add(block, hits);
        }
    }

    /// The first visible history line moved from `old` (§11.2). Lines
    /// shown again (a pop lets lines hidden by the limit show) are
    /// searched; and whether a match at the start of the line that was
    /// first starts a word can change either way (§13.8), so that start is
    /// searched again: with nothing before it once lines are hidden, with
    /// the shown lines before it otherwise. The lines after are taken up
    /// to a match's reach; a match that runs on to the screen is the
    /// screen's.
    fn search_history_start(&mut self, s: &mut SearchState, old: u64) {
        let Some(m) = s.matcher.clone() else { return };
        let first = self.scrollback.first_visible();
        let starts_word = matches!(m.word(), WordMatch::StartsWith | WordMatch::FullWord);
        if first > old && !starts_word {
            return;
        }
        let sb = self.scrollback.len();
        let shown = old.saturating_sub(first).min(sb as u64) as usize;
        let (mut cells, mut row, mut styles) = (Cells::default(), Row::default(), Vec::new());
        let mut k = 0;
        let mut boundary = 0;
        while k < sb && (k <= shown || cells.text.len() < boundary + m.reach()) {
            if k == shown {
                boundary = cells.text.len();
            }
            self.scrollback.get_into(k, &mut row, &mut styles);
            cells.push_row(&row, first + k as u64);
            k += 1;
        }
        if k <= shown {
            // Every visible line was shown again.
            boundary = cells.text.len();
        }
        if cells.text.is_empty() {
            return;
        }
        if starts_word && let Some(p) = cells.start_at(boundary) {
            s.drop_starting_at(p);
        }
        // The scalar after these lines: the next one's first (its line end
        // if it is empty), or the screen's.
        let after = match self.scrollback.text(k) {
            Some(t) => Some(if t.is_empty() { '\n' } else { char_at(t, 0).0 }),
            None => self.screen_first_char(),
        };
        let (mut scratch, mut raw) = (Vec::new(), Vec::new());
        m.find(&cells.text, None, after, &mut scratch, &mut raw);
        let mut per: Vec<(u64, Vec<Hit>)> = Vec::new();
        for &(a, b) in &raw {
            if a > boundary {
                break;
            }
            let (Some(st), Some(en)) = (cells.start_at(a), cells.end_at(b)) else {
                continue;
            };
            let h = Hit::from_parts(st, en);
            let block = self.scrollback.block_first(h.line);
            match per.last_mut() {
                Some(p) if p.0 == block => p.1.push(h),
                _ => per.push((block, vec![h])),
            }
        }
        for (block, hits) in per {
            s.add(block, hits);
        }
    }

    /// Search the screen again, with the end of history's last logical
    /// line before it (a match can start there and end on the screen):
    /// the screen's hits are replaced.
    fn search_screen(&mut self, s: &mut SearchState) {
        s.screen.clear();
        s.screen_seqs.clear();
        s.screen_seqs.extend(self.screen.iter().map(|r| r.seq));
        // A row's counter changes once per snapshot batch, so a row written
        // again in this batch would keep the counter just recorded and its
        // change would be missed: start a new batch, as a snapshot does.
        self.batch_start = self.counter + 1;
        s.screen_top = self.top_line;
        s.screen_alt = self.alt_active;
        let Some(m) = s.matcher.clone() else { return };
        let mut cells = Cells::default();
        let mut row = Row::default();
        let mut styles = Vec::new();
        if !self.alt_active {
            // History's last rows, newest first, until there are enough
            // bytes for a match's reach: whole lines too, as a query with
            // line ends in it can start in one and run on to the screen.
            let sb = self.scrollback.len();
            let mut k = sb;
            let mut bytes = 0;
            while k > 0 && bytes < m.reach() {
                k -= 1;
                bytes += self.scrollback.text(k).map_or(0, |t| t.len()) + 1;
            }
            let first = self.top_line - (sb - k) as u64;
            for i in k..sb {
                self.scrollback.get_into(i, &mut row, &mut styles);
                cells.push_row(&row, first + (i - k) as u64);
            }
        }
        let context = cells.text.len();
        let history_end = cells.end_at(context);
        s.history_end = history_end;
        for (r, row) in self.screen.iter().enumerate() {
            cells.push_row(row, self.top_line + r as u64);
        }
        let (mut scratch, mut raw) = (Vec::new(), Vec::new());
        m.find(&cells.text, None, None, &mut scratch, &mut raw);
        // History's matches that end where it does, checked against the
        // screen's first scalar now (they stay history's).
        let mut at_end = Vec::new();
        for &(a, b) in &raw {
            if b < context || (b == context && !looks_after(&m)) {
                continue;
            }
            if let (Some(st), Some(en)) = (cells.start_at(a), cells.end_at(b)) {
                if b == context {
                    at_end.push(Hit::from_parts(st, en));
                } else {
                    s.screen.push(Hit::from_parts(st, en));
                }
            }
        }
        if looks_after(&m)
            && let Some(e) = history_end
        {
            s.drop_ending_at(e);
            for h in at_end {
                s.add(self.scrollback.block_first(h.line), vec![h]);
            }
        }
    }

    /// The first scalar of the screen's text (what follows history).
    fn screen_first_char(&self) -> Option<char> {
        let mut cells = Cells::default();
        cells.push_row(self.screen.front()?, self.top_line);
        (!cells.text.is_empty()).then(|| char_at(&cells.text, 0).0)
    }

    /// Test aid: seal history's open block now.
    #[doc(hidden)]
    pub fn seal_history_for_test(&mut self) {
        self.scrollback.seal_for_test();
    }
}

#[cfg(test)]
impl Terminal {
    /// Test aid: the text of everything searched (visible history, then
    /// the screen), with where each cell's text starts.
    pub(crate) fn search_text_for_test(&self) -> Cells {
        let mut cells = Cells::default();
        let mut row = Row::default();
        let mut styles = Vec::new();
        if !self.alt_active {
            let sb = self.scrollback.len();
            for i in 0..sb {
                self.scrollback.get_into(i, &mut row, &mut styles);
                cells.push_row(&row, self.top_line - (sb - i) as u64);
            }
        }
        for (r, row) in self.screen.iter().enumerate() {
            cells.push_row(row, self.top_line + r as u64);
        }
        cells
    }
}
