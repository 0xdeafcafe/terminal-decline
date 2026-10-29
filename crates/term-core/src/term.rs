//! Terminal state: the visible screen, scrollback, cursor and modes, driven by
//! escape codes from the `vte` parser.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;

use vte::{OscLarge, Params, Perform};

use crate::cell::{Cell, Color, Mark, MarkKind, Row, Width};
pub use crate::links::LinkValue;
use crate::rows::{Content, RowRef};
use crate::scrollback::{HistoryStats, Scrollback};
use crate::style::{Attrs, LinkId, Style, Underline};
use crate::style_table::{StyleStats, StyleTable};
use crate::unicode::{self, Segmenter};

mod history;
mod reflow;
mod replies;
mod search;
mod snapshot;

pub use replies::DynamicColor;
pub use search::SearchCount;
pub use snapshot::{Highlight, HighlightKind, RowFlags, RowKey, ScreenId, SnapCursor, SnapRow, Snapshot};

/// A style table holding more ids than this gives its memory back when
/// the screen is cleared.
const GIVE_BACK_AT: usize = 256;

/// Titles `CSI 22 t` keeps, as xterm.
const TITLE_STACK: usize = 10;

/// A desktop notification a program asked for: OSC 9 (`9;body`, iTerm2's
/// form, no title) or OSC 777 (`777;notify;title;body`, rxvt's).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Notification {
    pub title: Option<String>,
    pub body: String,
}

/// Progress a program reported with OSC 9;4 (ConEmu's form): a percentage
/// from 0 to 100 where the state has one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Progress {
    Clear,
    Normal(u8),
    Error(u8),
    Indeterminate,
    Paused(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Enough to decide whether the next scalar printed joins the last printed
/// cell's grapheme cluster, and what that cluster's width becomes (§6.2).
///
/// Clusters aren't stored yet: the cells are written as before (a joining
/// scalar with a width still takes its own cell, and zero-width ones are
/// dropped apart from VS16's widening), and this state only follows along.
#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GraphemeState {
    /// Screen row and column of the cell holding the current cluster, if it
    /// has one. Cleared by anything but a print.
    pub at: Option<(usize, usize)>,
    /// UAX #29 state of the current cluster.
    pub seg: Segmenter,
}

impl GraphemeState {
    #[inline]
    fn clear(&mut self) {
        self.at = None;
        self.seg.reset();
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    pub(crate) row: usize,
    pub(crate) col: usize,
    /// The style new characters are drawn with. Held by value, so a saved
    /// cursor holds no style id: an id belongs to one screen's table, and
    /// that table may free it.
    pub(crate) pen: Style,
    /// Set after writing the last column; the next character wraps first.
    pub(crate) pending_wrap: bool,
}

impl Cursor {
    /// Screen row, 0 = the top of the screen (not of the view).
    pub fn row(&self) -> usize {
        self.row
    }

    pub fn col(&self) -> usize {
        self.col
    }

    /// The last column was just written; the next character wraps first.
    pub fn pending_wrap(&self) -> bool {
        self.pending_wrap
    }

    /// The style new characters are drawn with.
    pub fn pen(&self) -> Style {
        self.pen
    }
}

#[derive(Clone, Debug)]
struct SavedCursor {
    cursor: Cursor,
    /// The pen's OSC 8 link, by value: the saved pen's `link` id belongs
    /// to one screen's link table, so restoring looks the link up again.
    link: Option<Arc<LinkValue>>,
    origin: bool,
    charset: Charset,
}

/// A character set that can be designated as G0 or G1 (SCS, `ESC ( F`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Charset {
    #[default]
    Ascii,
    /// `ESC ( 0`: DEC Special Graphics, the line drawing set.
    DecGraphics,
    /// `ESC ( A`: British, `#` is `£`.
    British,
}

impl Charset {
    fn map(self, c: char) -> char {
        match self {
            Charset::Ascii => c,
            Charset::DecGraphics => dec_line_drawing(c),
            Charset::British if c == '#' => '£',
            Charset::British => c,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MouseMode {
    #[default]
    Off,
    /// 1000: report presses and releases.
    Click,
    /// 1002: also report motion while a button is held.
    Drag,
    /// 1003: report all motion.
    Motion,
}

#[derive(Clone, Copy, Debug)]
pub struct Modes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub origin: bool,
    pub autowrap: bool,
    pub insert: bool,
    pub linefeed_newline: bool,
    pub cursor_visible: bool,
    pub bracketed_paste: bool,
    pub focus_events: bool,
    pub mouse: MouseMode,
    pub mouse_sgr: bool,
    /// 2026: the app is mid-update; hold off drawing until it ends.
    pub synchronized: bool,
    /// 2027, on by default: scalars join grapheme clusters by the UAX #29
    /// rules, and a cluster's width follows §6.3 of the cells design. Off
    /// (legacy): only zero-width scalars join the cell before them, and a
    /// cell keeps its first scalar's width.
    pub grapheme_clusters: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            app_cursor: false,
            app_keypad: false,
            origin: false,
            autowrap: true,
            insert: false,
            linefeed_newline: false,
            cursor_visible: true,
            bracketed_paste: false,
            focus_events: false,
            mouse: MouseMode::Off,
            mouse_sgr: false,
            synchronized: false,
            grapheme_clusters: true,
        }
    }
}

/// A point in the whole buffer. `line` counts every line that has scrolled
/// off the top of the screen, stored in scrollback or not, so a point stays
/// attached to its text while new output scrolls in. Lines no longer stored
/// (dropped from scrollback, or scrolled off the alternate screen) have no
/// text.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Point {
    pub line: u64,
    pub col: usize,
}

/// What a selection grows by when it is extended.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelectionKind {
    /// Cell by cell (a click and drag).
    #[default]
    Normal,
    /// Whole words (a double-click and drag).
    Word,
    /// Whole logical lines (a triple-click and drag).
    Line,
}

/// A selection from `anchor` (where it started) to `head` (where it
/// ends now), both included, in text order whichever comes first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selection {
    pub anchor: Point,
    pub head: Point,
    pub kind: SelectionKind,
}

/// Where the view is: following output, or showing history from a fixed
/// place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewTop {
    /// Following output: the view shows the screen.
    Bottom,
    /// Scrolled back. `point` is the first cell shown (column 0 of its row);
    /// `offset` is that cell's offset in its logical line, counted in cells
    /// at the current width, so the place can be found again after the
    /// width changes.
    At { point: Point, offset: usize },
}

impl Selection {
    /// A plain, cell-by-cell selection.
    pub fn new(anchor: Point, head: Point) -> Selection {
        Selection {
            anchor,
            head,
            kind: SelectionKind::Normal,
        }
    }

    pub fn ordered(&self) -> (Point, Point) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn contains(&self, p: Point) -> bool {
        let (a, b) = self.ordered();
        a <= p && p <= b
    }
}

pub struct Terminal {
    cols: usize,
    lines: usize,
    /// A ring of rows, so scrolling the whole screen moves the top row to the
    /// bottom without shifting the others.
    screen: VecDeque<Row>,
    /// The other screen: the alternate one while on the primary, and vice versa.
    other_screen: VecDeque<Row>,
    /// The styles `screen`'s cells use, and `other_screen`'s.
    styles: StyleTable,
    other_styles: StyleTable,
    /// The pen's style interned in `styles`: the template cells are
    /// written with. Dropped by every SGR, screen switch and cursor
    /// restore; otherwise kept alive by the sweep.
    pen_cell: Option<Cell>,
    /// The erase style (the pen's background alone) interned in `styles`,
    /// for the background `erase_bg`. Dropped on every screen switch.
    erase_cell: Option<Cell>,
    erase_bg: Color,
    /// Where style table epochs come from: terminal-wide, and carried
    /// across a reset, so no epoch repeats.
    epochs: u64,
    /// Test aid: sweep tables once they hold this many ids.
    sweep_at_test: Option<u32>,
    alt_active: bool,
    /// The window was resized while the alternate screen was active: the
    /// primary screen keeps its old size until the program leaves it, and
    /// is reflowed once then (§12.1).
    primary_stale: bool,
    /// Lines that scrolled off the top (primary screen only). Its total
    /// count of lines ever pushed is the base for [`Point`] line numbers.
    scrollback: Scrollback,
    /// The first cell the view shows when scrolled back (the start of a
    /// row of history at the current width, §12.3); `None` while following
    /// output.
    view: Option<Point>,
    /// History laid out at the current width, kept between reads.
    layouts: history::LayoutCache,
    /// Buffer line number ([`Point::line`]) of the screen's first row: the
    /// count of rows that have scrolled off its top. On the primary screen
    /// the scrollback holds the lines just before it.
    top_line: u64,
    /// The same for the other screen.
    other_top_line: u64,

    cursor: Cursor,
    /// The OSC 8 link new characters get, by value (§5.5), and its id in
    /// the active screen's link table once a print has interned it (0
    /// until then). The id is dropped whenever the link changes, the
    /// screen switches or the cursor is restored; while set, the sweep
    /// keeps it.
    pen_link: Option<Arc<LinkValue>>,
    pen_link_id: LinkId,
    /// Where the app's pointer is, so the snapshot highlights the
    /// hyperlink under it (§9.2).
    hover: Option<Point>,
    saved_primary: Option<SavedCursor>,
    saved_alt: Option<SavedCursor>,
    scroll_top: usize,
    scroll_bottom: usize,
    tabs: Vec<bool>,
    /// G0 and G1 character sets.
    charsets: [Charset; 2],
    /// Which of G0/G1 is in use (SI/SO).
    active_charset: usize,
    /// The set in use, `charsets[active_charset]` unless a cursor restore
    /// brought back another.
    charset: Charset,
    /// The last grapheme printed, for REP: its character, or None when it
    /// is a cluster, which is then `last_cluster` (kept by value, at most
    /// 128 bytes, so REP works after its cell has been overwritten), of
    /// `last_cluster_width` cells, with the segmenter state after it.
    last_char: Option<char>,
    last_cluster: String,
    last_cluster_width: u8,
    last_seg: Segmenter,
    /// East Asian Ambiguous characters are this many cells wide (1 or 2),
    /// a setting (§6.3).
    ambiguous: u8,
    /// Grapheme segmentation of what is being printed (§6.2 of the cells
    /// design).
    grapheme: GraphemeState,
    /// The last character printed, when it is a plain one (see
    /// `Props::is_plain`) that started a cluster and was written in the
    /// cell(s) just before the cursor with autowrap on: `grapheme` is out
    /// of date while this is set. Lets printing most text, and a bulk
    /// ASCII run, update the state with one store; `sync_grapheme` brings
    /// `grapheme` up to date.
    grapheme_plain: Option<char>,
    /// Test aid: when set, every grapheme cluster as segmented.
    grapheme_log: Option<Vec<String>>,
    /// Kitty keyboard protocol flags, a stack per screen.
    kitty_flags: [Vec<u8>; 2],

    modes: Modes,
    cursor_shape: CursorShape,
    cursor_blink: bool,
    title: Option<String>,
    title_changed: bool,
    bell: bool,
    selection: Option<Selection>,
    /// Text a program asked to put on the clipboard (OSC 52).
    clipboard: Option<String>,
    /// A notification (OSC 9 / OSC 777) and progress (OSC 9;4) not yet taken.
    notification: Option<Notification>,
    progress: Option<Progress>,
    /// The program's working directory, if it reports it (OSC 7).
    cwd: Option<String>,
    /// The palette and foreground, background and cursor colours: the
    /// theme's, with what programs set (OSC 4, 10–12) on top.
    colors: Box<replies::ColorTable>,
    colors_changed: bool,
    /// A DCS query being collected (DECRQSS, XTGETTCAP).
    dcs: Option<replies::Dcs>,
    /// Titles saved by `CSI 22 t`, for `CSI 23 t` to put back.
    title_stack: Vec<Option<String>>,
    /// Cell size in pixels, for answering size queries.
    cell_pixels: (u16, u16),
    /// Bytes the terminal must send back to the program (replies to queries).
    replies: Vec<u8>,
    /// Bumped on any change that needs a redraw.
    generation: u64,
    /// Row change counters (§9.1 of the cells design): the last `seq`
    /// given to a row, and the first one given since the last snapshot.
    /// Neither goes back, even across a reset.
    counter: u64,
    batch_start: u64,
    /// Unique in the process: a new terminal (after a caught parser panic,
    /// say) never repeats an old one's counters or line numbers.
    terminal_id: u64,
    /// Bumped when history is cleared (ED 3, RIS). (The lowest line
    /// pulled back from history since the last snapshot, `popped_from`,
    /// is kept by the scrollback.)
    history_epoch: u64,
    /// Write printable ASCII runs in bulk ([`Terminal::write_ascii`]); off
    /// only in tests, as the reference the bulk path is compared against.
    bulk_ascii: bool,
    /// The search, while the find bar is open (§13.4).
    search: Option<Box<search::SearchState>>,
}

impl Terminal {
    pub fn new(cols: usize, lines: usize, scrollback_limit: usize) -> Terminal {
        let cols = cols.max(2);
        let lines = lines.max(1);
        Terminal {
            cols,
            lines,
            screen: blank_screen(cols, lines),
            other_screen: blank_screen(cols, lines),
            styles: StyleTable::new(1, cols * lines, None),
            other_styles: StyleTable::new(2, cols * lines, None),
            pen_cell: Some(Cell::BLANK),
            erase_cell: None,
            erase_bg: Color::DEFAULT,
            epochs: 2,
            sweep_at_test: None,
            alt_active: false,
            primary_stale: false,
            scrollback: Scrollback::with_widths_as_written(scrollback_limit),
            view: None,
            layouts: Default::default(),
            top_line: 0,
            other_top_line: 0,
            cursor: Cursor::default(),
            pen_link: None,
            pen_link_id: 0,
            hover: None,
            saved_primary: None,
            saved_alt: None,
            scroll_top: 0,
            scroll_bottom: lines - 1,
            tabs: default_tabs(cols),
            charsets: [Charset::Ascii; 2],
            active_charset: 0,
            charset: Charset::Ascii,
            last_char: None,
            last_cluster: String::new(),
            last_cluster_width: 1,
            last_seg: Segmenter::default(),
            ambiguous: 1,
            grapheme: GraphemeState::default(),
            grapheme_plain: None,
            grapheme_log: None,
            kitty_flags: [Vec::new(), Vec::new()],
            modes: Modes::default(),
            cursor_shape: CursorShape::Block,
            cursor_blink: false,
            title: None,
            title_changed: false,
            bell: false,
            selection: None,
            clipboard: None,
            notification: None,
            progress: None,
            cwd: None,
            colors: Box::new(replies::ColorTable::new([0xd8dee9, 0x16181c, 0xd8dee9])),
            colors_changed: false,
            dcs: None,
            title_stack: Vec::new(),
            cell_pixels: (8, 16),
            replies: Vec::new(),
            generation: 1,
            counter: 0,
            batch_start: 1,
            terminal_id: snapshot::next_terminal_id(),
            history_epoch: 0,
            bulk_ascii: true,
            search: None,
        }
        .with_rows_touched()
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn lines(&self) -> usize {
        self.lines
    }

    pub fn alt_screen_active(&self) -> bool {
        self.alt_active
    }

    /// Current kitty keyboard protocol flags (0 = legacy keys).
    pub fn kitty_keyboard_flags(&self) -> u8 {
        self.kitty_flags[self.alt_active as usize].last().copied().unwrap_or(0)
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// How many rows the view is scrolled back by (0 = following output),
    /// counted at the current width.
    pub fn display_offset(&self) -> usize {
        match self.view_start() {
            None => 0,
            Some((u, r)) => self.rows_below(&u, r),
        }
    }

    /// Scroll the view by `delta` rows (positive = back into history).
    pub fn scroll_view(&mut self, delta: isize) {
        if self.alt_active || delta == 0 || self.scrollback.is_empty() {
            return;
        }
        let d = delta.unsigned_abs();
        let new = match (self.view_start(), delta > 0) {
            (None, false) => None,
            (None, true) => {
                let u = self.unit_at(self.top_line - 1);
                let r = u.rows() - 1;
                let (u, r) = self.rows_back(u, r, d - 1);
                Some(u.row_start(r))
            }
            (Some((u, r)), true) => {
                let (u, r) = self.rows_back(u, r, d);
                Some(u.row_start(r))
            }
            (Some((u, r)), false) => self.rows_forward(u, r, d).map(|(u, r)| u.row_start(r)),
        };
        if new != self.view {
            self.view = new;
            self.touch();
        }
    }

    pub fn scroll_view_to_bottom(&mut self) {
        if self.view.is_some() {
            self.view = None;
            self.touch();
        }
    }

    /// The row shown at visible line `line`, taking the scroll position into
    /// account. History rows are laid out at the current width.
    ///
    /// History lines are expanded from their compact form, so they come back
    /// owned, with their own list of styles; screen lines are borrowed.
    pub fn visible_row(&self, line: usize) -> RowRef<'_> {
        self.view_row(self.view_row_at(line))
    }

    /// The columns of view row `line` the selection covers.
    pub fn selected_cols(&self, line: usize) -> std::ops::Range<usize> {
        let Some((a, b)) = self.selection.map(|s| s.ordered()) else {
            return 0..0;
        };
        let at = self.view_row_at(line);
        let mut pieces = Vec::new();
        self.view_pieces(at, &mut pieces);
        let mut out: Option<std::ops::Range<usize>> = None;
        for p in &pieces {
            if p.line < a.line || p.line > b.line {
                continue;
            }
            let start = if p.line == a.line { a.col } else { 0 };
            let end = if p.line == b.line { b.col + 1 } else { usize::MAX };
            if let Some(r) = p.view_cols(p.line, start, end) {
                out = Some(match out {
                    None => r,
                    Some(o) => o.start.min(r.start)..o.end.max(r.end),
                });
            }
        }
        out.map_or(0..0, |r| r.start..r.end.min(self.cols))
    }

    fn screen_row(&self, r: usize) -> RowRef<'_> {
        RowRef::new(Cow::Borrowed(&self.screen[r]), Cow::Borrowed(self.styles.styles()))
            .with_links(Cow::Borrowed(self.styles.links().as_slice()))
    }

    fn history_row(&self, i: usize) -> RowRef<'_> {
        let mut row = Row::default();
        let (mut styles, mut links) = (Vec::new(), Vec::new());
        self.scrollback.get_at_width(i, self.cols, &mut row, &mut styles, &mut links);
        RowRef::new(Cow::Owned(row), Cow::Owned(styles)).with_links(Cow::Owned(links))
    }

    /// The history index (0 = oldest kept) of buffer line `line`, if it is
    /// a stored history line.
    fn history_index(&self, line: u64) -> Option<usize> {
        if self.alt_active || line >= self.top_line {
            return None;
        }
        let back = self.top_line - line;
        let sb = self.scrollback.len();
        (back <= sb as u64).then(|| sb - back as usize)
    }

    /// The cursor's position on the screen, and the pen it writes with.
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    pub fn modes(&self) -> &Modes {
        &self.modes
    }

    pub fn cursor_shape(&self) -> CursorShape {
        self.cursor_shape
    }

    /// The program asked for a blinking cursor.
    pub fn cursor_blink(&self) -> bool {
        self.cursor_blink
    }

    /// The window title the program set (OSC 0/2).
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The title, if the program set it since the last call.
    pub fn take_title(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.title_changed) {
            return None;
        }
        self.title.clone()
    }

    /// Whether the bell rang since the last call.
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.bell)
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    pub fn set_selection(&mut self, selection: Option<Selection>) {
        self.selection = selection;
    }

    /// Text a program asked to put on the clipboard (OSC 52), not yet taken.
    pub fn clipboard(&self) -> Option<&str> {
        self.clipboard.as_deref()
    }

    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// The last notification a program asked for (OSC 9 / OSC 777).
    pub fn take_notification(&mut self) -> Option<Notification> {
        self.notification.take()
    }

    /// The last progress a program reported (OSC 9;4).
    pub fn take_progress(&mut self) -> Option<Progress> {
        self.progress.take()
    }

    /// The program's working directory, if it reports it (OSC 7).
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// How many cells (1 or 2) East Asian Ambiguous characters take.
    pub fn ambiguous_width(&self) -> u8 {
        self.ambiguous
    }

    /// Set the East Asian Ambiguous width (the `ambiguous_width` setting,
    /// §6.3 of the cells design): 1 or 2 cells, for output from now on.
    /// Cells already written keep their width, so nothing moves; programs
    /// already running keep the width they worked out themselves. The
    /// grapheme state is cleared, and history's open block is sealed (if
    /// it holds any lines): each block records the setting its lines were
    /// written with, so they read back as written.
    pub fn set_ambiguous_width(&mut self, width: u8) {
        let width = width.clamp(1, 2);
        if width != self.ambiguous {
            // Cells written before keep their old width: history checks
            // each one's width from now on.
            self.scrollback.set_widths_as_written(false);
        }
        self.ambiguous = width;
        self.scrollback.set_ambiguous(self.ambiguous);
        self.clear_grapheme();
    }

    /// Cell size in pixels, for answering size queries.
    pub fn cell_pixels(&self) -> (u16, u16) {
        self.cell_pixels
    }

    pub fn set_cell_pixels(&mut self, size: (u16, u16)) {
        self.cell_pixels = size;
    }

    /// Bytes the terminal must send back to the program (replies to
    /// queries), not yet taken.
    pub fn replies(&self) -> &[u8] {
        &self.replies
    }

    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }

    /// Bumped on any change that needs a redraw.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Bytes used by scrollback, for measuring.
    pub fn scrollback_bytes(&self) -> usize {
        self.scrollback.memory_bytes()
    }

    /// What history holds, for measuring: bytes, and sealed blocks' bytes
    /// and lines.
    pub fn history_stats(&self) -> HistoryStats {
        self.scrollback.stats()
    }

    /// The scrollback limit in lines (the `scrollback` setting).
    pub fn scrollback_limit(&self) -> usize {
        self.scrollback.limit()
    }

    /// Change the scrollback limit on a running terminal (live config
    /// reload). A smaller limit drops the oldest lines at once, as output
    /// pushing them out would; 0 drops them all. A larger one keeps more
    /// from now on, and brings back lines still stored but hidden past
    /// the old limit. The view keeps its place as far as what is left
    /// allows. Line numbers ([`Point`]s) don't change.
    pub fn set_scrollback_limit(&mut self, limit: usize) {
        if limit == self.scrollback.limit() {
            return;
        }
        self.scrollback.set_limit(limit);
        self.touch();
    }

    /// The stable position of what is shown at view row `view_line` and
    /// column `col`. In history laid out at another width, that is the
    /// stored line and column the cell came from.
    pub fn point_at(&self, view_line: usize, col: usize) -> Point {
        match self.view_row_at(view_line) {
            history::ViewRow::History { lo, hi, row } => self.unit(lo, hi).point_at(row, col),
            history::ViewRow::Screen(r) => Point {
                line: self.top_line + r as u64,
                col,
            },
        }
    }

    /// Where the cursor is in the view (row, column), if it is in view.
    pub fn cursor_in_view(&self) -> Option<(usize, usize)> {
        let history = self
            .view_rows()
            .iter()
            .filter(|r| matches!(r, history::ViewRow::History { .. }))
            .count();
        let line = self.cursor.row + history;
        (line < self.lines).then_some((line, self.cursor.col))
    }

    /// Where the view is.
    pub fn view_top(&self) -> ViewTop {
        match self.view_start() {
            None => ViewTop::Bottom,
            Some((u, r)) => {
                let point = u.row_start(r);
                ViewTop::At {
                    point,
                    offset: u.offset_of(point, self.cols),
                }
            }
        }
    }

    /// Move the view: back to following output, or to show `point`'s row
    /// at the top (as near as the history allows). A point in the middle
    /// of a row (from another width) shows that row.
    pub fn set_view_top(&mut self, top: ViewTop) {
        match top {
            ViewTop::Bottom => self.scroll_view_to_bottom(),
            ViewTop::At { point, .. } => self.set_view_point(Some(point)),
        }
    }

    /// The OSC 8 hyperlink at `p`, by value (on either half of a wide
    /// character), on the screen or in history.
    pub fn link_at(&self, p: Point) -> Option<LinkValue> {
        self.row_at(p.line)?.link(p.col).cloned()
    }

    /// The pointer is at `p` (or off the terminal): the snapshot
    /// highlights every cell of the hyperlink there, compared by value,
    /// so all the pieces of a link light up together.
    pub fn set_hover(&mut self, p: Option<Point>) {
        if p != self.hover {
            self.hover = p;
            self.touch();
        }
    }

    pub fn hover(&self) -> Option<Point> {
        self.hover
    }

    /// The OSC 8 link new characters get, if any.
    pub fn pen_link(&self) -> Option<&LinkValue> {
        self.pen_link.as_deref()
    }

    /// The OSC 133 marks on line `line`, in the order they were written
    /// (by column; at the same column a command's output and end come
    /// before the next prompt), on the screen or in history.
    pub fn marks_at(&self, line: u64) -> Vec<Mark> {
        let mut marks: Vec<Mark> = match self.screen_line(line) {
            Some(row) => row.marks().collect(),
            None => self.history_index(line).map_or_else(Vec::new, |i| self.scrollback.marks(i)),
        };
        marks.sort_by_key(|m| (m.col, mark_rank(m.kind)));
        marks
    }

    /// The screen row at buffer line `line`, if it is on the screen.
    fn screen_line(&self, line: u64) -> Option<&Row> {
        let r = line.checked_sub(self.top_line)? as usize;
        self.screen.get(r)
    }

    /// History's lines with marks, as buffer lines, nearest first: before
    /// `from` going up, at or after it going down. The alternate screen
    /// has no history.
    fn history_marked(&self, from: u64, up: bool) -> impl Iterator<Item = u64> + '_ {
        let base = self.top_line.saturating_sub(self.scrollback.len() as u64);
        let mut at = (!self.alt_active && !(up && from <= base)).then(|| (from.clamp(base, self.top_line) - base) as usize);
        std::iter::from_fn(move || {
            let i = self.scrollback.marked(at?, up)?;
            at = Some(if up { i } else { i + 1 });
            Some(base + i as u64)
        })
    }

    /// Lines holding marks, nearest first: before `line` going up, or
    /// after it going down (`None`: every one, oldest first). History
    /// lines are found from their flags, skipping blocks without marks
    /// (§11.6).
    fn marked_lines(&self, line: Option<u64>, up: bool) -> Box<dyn Iterator<Item = u64> + '_> {
        let top = self.top_line;
        let hi = top + self.lines as u64;
        let has = move |l: &u64| self.screen_line(*l).is_some_and(Row::has_marks);
        match line {
            Some(line) if up => {
                let screen = (top..line.min(hi)).rev().filter(has);
                Box::new(screen.chain(self.history_marked(line, true)))
            }
            Some(line) => {
                let screen = (line.saturating_add(1).max(top)..hi).filter(has);
                Box::new(self.history_marked(line.saturating_add(1), false).chain(screen))
            }
            None => Box::new(self.history_marked(0, false).chain((top..hi).filter(has))),
        }
    }

    /// Where the nearest prompt (OSC 133 A) above line `line` starts.
    pub fn previous_prompt(&self, line: u64) -> Option<Point> {
        self.marked_lines(Some(line), true).find_map(|l| self.prompt_on(l))
    }

    /// Where the nearest prompt (OSC 133 A) below line `line` starts.
    pub fn next_prompt(&self, line: u64) -> Option<Point> {
        self.marked_lines(Some(line), false).find_map(|l| self.prompt_on(l))
    }

    fn prompt_on(&self, line: u64) -> Option<Point> {
        self.marks_at(line)
            .into_iter()
            .find(|m| m.kind == MarkKind::Prompt)
            .map(|m| Point { line, col: m.col as usize })
    }

    /// The output of the command `p` is in, its prompt and command line
    /// included in what counts as "in": from its OSC 133 C mark to just
    /// before the next D or A mark (or the cursor, while it runs), first
    /// and last cell. `None` if there is no C mark for it or the output
    /// is empty.
    pub fn command_output(&self, p: Point) -> Option<(Point, Point)> {
        // Every mark in the order written, history's then the screen's.
        let marks: Vec<(Point, MarkKind)> = self
            .marked_lines(None, false)
            .flat_map(|line| {
                self.marks_at(line)
                    .into_iter()
                    .map(move |m| (Point { line, col: m.col as usize }, m.kind))
            })
            .collect();
        // The nearest A or C at or before `p`.
        let mut i = marks
            .iter()
            .rposition(|&(q, k)| q <= p && matches!(k, MarkKind::Prompt | MarkKind::Output))?;
        let next = |from: usize, kinds: &[MarkKind]| (from + 1..marks.len()).find(|&j| kinds.contains(&marks[j].1));
        if marks[i].1 == MarkKind::Prompt {
            // `p` is on the prompt or command line: this command's output.
            i = next(i, &[MarkKind::Output, MarkKind::Prompt]).filter(|&j| marks[j].1 == MarkKind::Output)?;
        }
        let from = marks[i].0;
        let end = match next(i, &[MarkKind::End, MarkKind::Prompt]) {
            Some(j) => marks[j].0,
            // Still running: up to the cursor.
            None => Point {
                line: self.top_line + self.cursor.row as u64,
                col: self.cursor.col,
            },
        };
        // The cell before `end`.
        let last = match end.col {
            0 => Point {
                line: end.line.checked_sub(1)?,
                col: self.cols - 1,
            },
            c => Point {
                line: end.line,
                col: c - 1,
            },
        };
        (from <= last).then_some((from, last))
    }

    /// Select the output of the command `p` is in ([`Terminal::command_output`]).
    /// False, with the selection unchanged, if there is none.
    pub fn select_output(&mut self, p: Point) -> bool {
        let Some((a, b)) = self.command_output(p) else {
            return false;
        };
        self.set_selection(Some(Selection::new(a, b)));
        true
    }

    /// Select the word at `p`, on its logical line: a word that a soft wrap
    /// splits across rows is selected whole. Off a word, just that cell.
    pub fn select_word(&mut self, p: Point) {
        let (anchor, head) = self.word_span(p);
        self.set_selection(Some(Selection {
            anchor,
            head,
            kind: SelectionKind::Word,
        }));
    }

    /// Select the whole logical line at `p`, every row it wraps onto.
    pub fn select_line(&mut self, p: Point) {
        let (anchor, head) = self.line_span(p);
        self.set_selection(Some(Selection {
            anchor,
            head,
            kind: SelectionKind::Line,
        }));
    }

    /// Select everything stored: all of history and the screen.
    pub fn select_all(&mut self) {
        let first = if self.alt_active {
            self.top_line
        } else {
            self.top_line - self.scrollback.len() as u64
        };
        let last = Point {
            line: self.top_line + self.lines as u64 - 1,
            col: self.cols - 1,
        };
        self.set_selection(Some(Selection::new(Point { line: first, col: 0 }, last)));
    }

    /// Move the selection's head to `to`, keeping its anchor. Word and line
    /// selections grow by whole words or logical lines, and keep the word
    /// or line they started on. With no selection, starts one at `to`.
    pub fn extend_selection(&mut self, to: Point) {
        let Some(sel) = self.selection else {
            self.set_selection(Some(Selection::new(to, to)));
            return;
        };
        let span = |t: &Self, p: Point| match sel.kind {
            SelectionKind::Normal => (p, p),
            SelectionKind::Word => t.word_span(p),
            SelectionKind::Line => t.line_span(p),
        };
        let (start, end) = span(self, sel.anchor);
        let (to_start, to_end) = span(self, to);
        let (anchor, head) = match sel.kind {
            SelectionKind::Normal => (sel.anchor, to),
            _ if to < start => (end, to_start),
            _ => (start, to_end),
        };
        self.set_selection(Some(Selection { anchor, head, ..sel }));
    }

    /// The first and last line numbers of the logical line `line` is part
    /// of: the rows joined to it by soft wraps, as far as they are stored.
    fn logical_line(&self, line: u64) -> (u64, u64) {
        let wrapped = |l: u64| self.row_at(l).is_some_and(|r| r.wrapped());
        let mut start = line;
        while start > 0 && wrapped(start - 1) {
            start -= 1;
        }
        let mut end = line;
        while wrapped(end) && self.row_at(end + 1).is_some() {
            end += 1;
        }
        (start, end)
    }

    fn line_span(&self, p: Point) -> (Point, Point) {
        let (start, end) = self.logical_line(p.line);
        (
            Point { line: start, col: 0 },
            Point {
                line: end,
                col: self.cols - 1,
            },
        )
    }

    /// The first and last cell of the word at `p` on its logical line, or
    /// `(p, p)` when `p` isn't on a word.
    fn word_span(&self, p: Point) -> (Point, Point) {
        let (text, cells, at) = self.logical_text(p);
        match at.and_then(|i| crate::word::word_at(&text, i)) {
            Some((a, b)) => (cells[a].0, cells[b].1),
            None => (p, p),
        }
    }

    /// The plain URL at `p` (one written without OSC 8, starting
    /// `https://`, `http://`, `file://` or `mailto:`; see
    /// [`crate::word::url_at`]), on its logical line, so a URL that a soft
    /// wrap splits across rows is found whole: its first and last cell,
    /// and its text.
    pub fn url_at(&self, p: Point) -> Option<(Point, Point, String)> {
        let (text, cells, at) = self.logical_text(p);
        let (a, b) = crate::word::url_at(&text, at?)?;
        Some((cells[a].0, cells[b].1, text[a..=b].iter().collect()))
    }

    /// The text of the logical line `p` is on, one entry per character,
    /// with each character's first and last cell (a wide character's last
    /// cell is its spacer), and which character `p` is on.
    #[allow(clippy::type_complexity)]
    fn logical_text(&self, p: Point) -> (Vec<char>, Vec<(Point, Point)>, Option<usize>) {
        let (start, end) = self.logical_line(p.line);
        let mut text = Vec::new();
        let mut cells: Vec<(Point, Point)> = Vec::new();
        let mut at = None;
        for line in start..=end {
            let Some(row) = self.row_at(line) else { continue };
            // A wrapped row's trailing empty cells (where a wide character
            // didn't fit) don't break the line's text.
            let len = if row.wrapped() {
                row.cells()
                    .rposition(|c| c.content() != Content::Empty || c.is_spacer())
                    .map_or(0, |i| i + 1)
            } else {
                row.len().max(self.cols)
            };
            for col in 0..len {
                let cell = row.cell(col);
                let here = Point { line, col };
                if cell.width() == Width::EdgeSpacer {
                    // Padding where a wide character didn't fit.
                    continue;
                }
                if cell.is_spacer() {
                    if let Some(last) = cells.last_mut() {
                        last.1 = here;
                    }
                    if p == here {
                        at = cells.len().checked_sub(1);
                    }
                    continue;
                }
                if p == here {
                    at = Some(cells.len());
                }
                text.push(cell.ch());
                cells.push((here, here));
            }
        }
        (text, cells, at)
    }

    /// The row at buffer line `line`, or None if it is not stored (any more).
    fn row_at(&self, line: u64) -> Option<RowRef<'_>> {
        if line >= self.top_line {
            let r = (line - self.top_line) as usize;
            return (r < self.screen.len()).then(|| self.screen_row(r));
        }
        if self.alt_active {
            // Rows that scroll off the alternate screen are not kept.
            return None;
        }
        let back = self.top_line - line;
        let sb = self.scrollback.len();
        if back > sb as u64 {
            return None;
        }
        Some(self.history_row(sb - back as usize))
    }

    pub fn selection_text(&self) -> Option<String> {
        let sel = self.selection?;
        let (a, b) = sel.ordered();
        let mut out = String::new();
        for line in a.line..=b.line {
            let Some(row) = self.row_at(line) else { continue };
            let start = if line == a.line { a.col } else { 0 };
            let wrapped = row.wrapped() && line != b.line;
            // History rows keep the width they were written at, which may
            // be wider than the screen's now.
            let mut end = if line == b.line { b.col + 1 } else { self.cols.max(row.len()) };
            if wrapped {
                // Empty cells at the end of a wrapped row (where a wide
                // character didn't fit) aren't part of the line's text.
                end = end.min(
                    row.cells()
                        .rposition(|c| c.content() != Content::Empty || c.is_spacer())
                        .map_or(0, |i| i + 1),
                );
            }
            let mut text = String::new();
            for col in start..end.min(row.len().max(start)) {
                let cell = row.cell(col);
                if !cell.is_spacer() {
                    cell.push_text(&mut text);
                }
            }
            if !wrapped {
                text.truncate(text.trim_end_matches(' ').len());
            }
            out.push_str(&text);
            if line != b.line && !wrapped {
                out.push('\n');
            }
        }
        Some(out)
    }

    pub fn resize(&mut self, cols: usize, lines: usize) {
        let cols = cols.max(2);
        let lines = lines.max(1);
        if cols == self.cols && lines == self.lines {
            return;
        }
        self.clear_grapheme();
        // No sweeps while rows are half-built (§5.2): the tables may grow
        // past `sweep_at` for a moment, and are swept once at the end.
        self.styles.set_sweeps_blocked(true);
        self.other_styles.set_sweeps_blocked(true);
        if self.alt_active {
            // The alternate screen isn't reflowed (programs redraw it on
            // SIGWINCH); the primary one waits until the program leaves.
            resize_screen(&mut self.screen, &mut self.cursor, cols, lines);
            self.primary_stale = true;
            self.selection = None;
            self.cursor.pending_wrap = false;
        } else {
            // Rows, the cursor, the selection and the view move with the
            // text (§12.2).
            self.reflow(cols, lines);
            resize_screen(&mut self.other_screen, &mut Cursor::default(), cols, lines);
        }

        self.cols = cols;
        self.lines = lines;
        self.cursor.col = self.cursor.col.min(cols - 1);
        self.cursor.row = self.cursor.row.min(lines - 1);
        self.scroll_top = 0;
        self.scroll_bottom = lines - 1;
        self.tabs = default_tabs(cols);
        // The view's first cell, at the start of its row at the new width.
        self.drop_layouts();
        let view = self.view.take();
        self.set_view_point(view);

        self.styles.set_sweeps_blocked(false);
        self.other_styles.set_sweeps_blocked(false);
        self.styles.set_cells(cols * lines);
        self.other_styles.set_cells(cols * lines);
        if self.styles.needs_sweep() {
            self.sweep_styles(false);
        }
        if self.other_styles.needs_sweep() {
            // The other screen's templates were dropped when it was left.
            let epochs = &mut self.epochs;
            self.other_styles.sweep(
                &mut self.other_screen,
                &mut [],
                || {
                    *epochs += 1;
                    *epochs
                },
                false,
                // The other screen's pen link isn't interned there.
                &[],
            );
        }
        // Resizing changes every row (§9.1).
        self.touch_all_rows();
        self.touch();
    }

    #[inline]
    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// The cell erasing writes: empty, in the erase style (the pen's
    /// background alone, §5.4), interned once per background. Interning
    /// can sweep and renumber the table, so callers take it before any
    /// other template they hold in a local.
    #[inline]
    fn blank(&mut self) -> Cell {
        let bg = self.cursor.pen.bg;
        if bg == Color::DEFAULT {
            return Cell::BLANK;
        }
        match self.erase_cell {
            Some(c) if self.erase_bg == bg => c,
            _ => self.intern_erase(bg),
        }
    }

    #[cold]
    #[inline(never)]
    fn intern_erase(&mut self, bg: Color) -> Cell {
        // Drop the old one first, so the sweep doesn't keep it alive.
        self.erase_cell = None;
        let c = self.intern(Style { bg, ..Style::DEFAULT });
        self.erase_cell = Some(c);
        self.erase_bg = bg;
        c
    }

    /// The template the pen writes with: its style id and SE bit. Like
    /// [`Terminal::blank`], it may sweep the table when it interns, so take
    /// it after anything else that might intern.
    #[inline(always)]
    fn pen_template(&mut self) -> Cell {
        match self.pen_cell {
            Some(c) => c,
            None => self.intern_pen(),
        }
    }

    #[cold]
    #[inline(never)]
    fn intern_pen(&mut self) -> Cell {
        if let Some(link) = &self.pen_link
            && self.pen_link_id == 0
        {
            // Held by the sweep from here on, so interning the style
            // below can't free it.
            self.pen_link_id = self.styles.links_mut().intern(link);
        }
        self.cursor.pen.link = self.pen_link_id;
        let c = self.intern(self.cursor.pen);
        self.pen_cell = Some(c);
        c
    }

    /// OSC 8: new characters get `link` (or none). The pen holds it by
    /// value and interns it at the next print.
    fn set_link(&mut self, link: Option<LinkValue>) {
        if self.pen_link.as_deref() == link.as_ref() {
            return;
        }
        self.pen_link = link.map(Arc::new);
        self.drop_pen_link_id();
    }

    /// The pen's link id is no longer good (another link, another screen's
    /// table, a restored cursor): the next print looks it up again.
    fn drop_pen_link_id(&mut self) {
        self.pen_link_id = 0;
        self.cursor.pen.link = 0;
        self.pen_cell = None;
    }

    /// The pen's style changed: its template is out of date, and so is the
    /// erase template if the background changed (dropped, so the sweep
    /// doesn't keep its id alive).
    #[inline]
    fn pen_changed(&mut self) {
        self.pen_cell = None;
        if self.cursor.pen.bg != self.erase_bg {
            self.erase_cell = None;
        }
    }

    /// The template for `s` in the active screen's table, adding it (after
    /// a sweep, if the table is full) when it is new.
    fn intern(&mut self, s: Style) -> Cell {
        match self.styles.lookup(&s) {
            Ok(c) => c,
            Err(miss) => {
                if self.styles.needs_sweep() {
                    // Rebuilds the index: `insert` finds its own slot.
                    self.sweep_styles(false);
                }
                self.styles.insert(s, miss)
            }
        }
    }

    /// Sweep the active screen's table (§5.2): free the ids no cell and no
    /// template uses, renumbering if few are left. With `give_back`, the
    /// table's memory shrinks to what is left.
    fn sweep_styles(&mut self, give_back: bool) {
        let epochs = &mut self.epochs;
        let renumbered = self.styles.sweep(
            &mut self.screen,
            &mut [&mut self.pen_cell, &mut self.erase_cell],
            || {
                *epochs += 1;
                *epochs
            },
            give_back,
            &[self.pen_link_id],
        );
        if renumbered {
            // Every row's cells changed bits.
            self.touch_screen_rows();
            self.touch();
        }
    }

    /// The whole screen was just erased (`clear`, a program exiting): a
    /// big style table is swept now and gives its memory back, rather than
    /// waiting for it to fill again (§5.3).
    fn screen_cleared(&mut self) {
        // With a test's small `sweep_at`, clears renumber too.
        let at = self.sweep_at_test.map_or(GIVE_BACK_AT, |n| (n as usize).min(GIVE_BACK_AT));
        if self.styles.len() > at {
            self.sweep_styles(true);
        }
    }

    /// The next table epoch: terminal-wide, so none repeats.
    fn next_epoch(&mut self) -> u64 {
        self.epochs += 1;
        self.epochs
    }

    /// Copy a row of the primary screen into scrollback. The row itself is
    /// left for the caller to reuse as the new blank line. Callers also
    /// advance the screen's `top_line`; the scrollback counts the row
    /// whether or not it keeps it, so its line numbers stay the screen's.
    #[inline]
    fn push_scrollback(&mut self, row: &Row) {
        self.scrollback
            .push_keyed(row, self.styles.styles(), self.styles.links().as_slice(), self.styles.version());
        // The view holds a Point in history: output arriving underneath
        // leaves it where it is.
    }

    fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        let n = n.min(bottom - top + 1);
        let blank = self.blank();
        let to_history = top == 0 && !self.alt_active;
        if top == 0 {
            self.top_line += n as u64;
        }
        if top == 0 && bottom == self.lines - 1 {
            // Common case, the whole screen: each top row goes round the ring
            // to the bottom, and the others stay where they are.
            for _ in 0..n {
                let mut row = self.screen.pop_front().expect("screen has rows");
                if to_history {
                    self.push_scrollback(&row);
                }
                row.reset(self.cols, blank);
                snapshot::touch(&mut row, &mut self.counter, self.batch_start);
                self.screen.push_back(row);
            }
            return;
        }
        for i in 0..n {
            if to_history {
                let row = std::mem::take(&mut self.screen[top + i]);
                self.push_scrollback(&row);
                self.screen[top + i] = row;
            }
            self.screen[top + i].reset(self.cols, blank);
        }
        self.screen.make_contiguous()[top..=bottom].rotate_left(n);
        self.touch_rows(bottom + 1 - n..bottom + 1);
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        let n = n.min(bottom - top + 1);
        self.screen.make_contiguous()[top..=bottom].rotate_right(n);
        let blank = self.blank();
        for row in self.screen.range_mut(top..top + n) {
            row.reset(self.cols, blank);
        }
        self.touch_rows(top..top + n);
    }

    fn linefeed(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.scroll_bottom {
            self.scroll_up(1);
        } else if self.cursor.row + 1 < self.lines {
            self.cursor.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.scroll_top {
            self.scroll_down(1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    fn goto(&mut self, row: usize, col: usize) {
        let (min, max) = if self.modes.origin {
            (self.scroll_top, self.scroll_bottom)
        } else {
            (0, self.lines - 1)
        };
        self.cursor.row = (row + if self.modes.origin { self.scroll_top } else { 0 }).clamp(min, max);
        self.cursor.col = col.min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn goto_col(&mut self, col: usize) {
        self.cursor.col = col.min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn move_rows(&mut self, delta: isize) {
        // Up stops at the top margin if the cursor is at or below it; down
        // stops at the bottom margin if the cursor is at or above it.
        let min = if self.cursor.row >= self.scroll_top { self.scroll_top } else { 0 };
        let max = if self.cursor.row <= self.scroll_bottom {
            self.scroll_bottom
        } else {
            self.lines - 1
        };
        self.cursor.row = (self.cursor.row as isize + delta).clamp(min as isize, max as isize) as usize;
        self.cursor.pending_wrap = false;
    }

    fn erase_cells(&mut self, row: usize, range: std::ops::Range<usize>) {
        let blank = self.blank();
        self.touch_row(row);
        let r = &mut self.screen[row];
        let len = r.cells.len();
        let mut end = range.end.min(len);
        let mut start = range.start.min(end);
        // Don't leave half of a wide character behind.
        if start > 0 && r.cells[start - 1].is_wide() {
            start -= 1;
        }
        if end < len && r.cells[end].width() == Width::Spacer {
            end += 1;
        }
        r.free_clusters(start..end);
        r.cells[start..end].fill(blank);
        if start == 0 && end == len {
            // The whole row erased: its prompt marks go with it.
            r.clear_marks();
        }
    }

    /// OSC 133: a shell integration mark at the cursor. `kind` is A, B,
    /// C or D (N, kitty's "new command", counts as A); for D, `rest[0]`
    /// may be the exit status. Secondary and right prompts (`k=s`,
    /// `k=c`, `k=r` on A) aren't prompts to jump to, so are ignored.
    fn prompt_mark(&mut self, kind: &[u8], rest: &[&[u8]]) {
        let kind = match kind {
            b"A" | b"N" => {
                if rest.iter().any(|p| matches!(*p, b"k=s" | b"k=c" | b"k=r")) {
                    return;
                }
                MarkKind::Prompt
            }
            b"B" => MarkKind::Input,
            b"C" => MarkKind::Output,
            b"D" => MarkKind::End,
            _ => return,
        };
        let status = match kind {
            MarkKind::End => rest.first().and_then(|p| std::str::from_utf8(p).ok()?.parse::<u32>().ok()),
            _ => None,
        };
        let col = self.cursor.col.min(u16::MAX as usize) as u16;
        self.screen[self.cursor.row].set_mark(Mark { kind, col, status });
    }

    fn save_cursor(&mut self) {
        let saved = SavedCursor {
            cursor: self.cursor,
            link: self.pen_link.clone(),
            origin: self.modes.origin,
            charset: self.charset,
        };
        if self.alt_active {
            self.saved_alt = Some(saved)
        } else {
            self.saved_primary = Some(saved)
        }
    }

    fn restore_cursor(&mut self) {
        let saved = if self.alt_active { &self.saved_alt } else { &self.saved_primary };
        match saved.clone() {
            Some(s) => {
                // The saved pen is a style by value: it is interned again
                // in this screen's table at the next print, and so is its
                // link, from its URI.
                self.cursor = s.cursor;
                self.pen_link = s.link;
                self.modes.origin = s.origin;
                self.charset = s.charset;
            }
            None => {
                self.cursor = Cursor::default();
                self.pen_link = None;
                self.modes.origin = false;
            }
        }
        self.drop_pen_link_id();
        self.pen_changed();
        self.cursor.row = self.cursor.row.min(self.lines - 1);
        self.cursor.col = self.cursor.col.min(self.cols - 1);
        if self.modes.origin {
            self.cursor.row = self.cursor.row.clamp(self.scroll_top, self.scroll_bottom);
        }
    }

    /// ICH: insert `n` blank cells at the cursor, pushing the rest of the
    /// row right; cells pushed off the end free their clusters.
    #[inline(never)]
    fn insert_chars(&mut self, n: usize) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let n = n.min(self.cols - col);
        let blank = self.blank();
        let r = &mut self.screen[row];
        let len = r.cells.len();
        r.free_clusters(len - n..len);
        let cells = &mut r.cells;
        cells[col..].rotate_right(n);
        cells[col..col + n].fill(blank);
        self.repair_wide(row);
    }

    /// DCH: delete `n` cells at the cursor (freeing their clusters), pulling
    /// the rest of the row left and filling the end with blanks.
    #[inline(never)]
    fn delete_chars(&mut self, n: usize) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let n = n.min(self.cols - col);
        let blank = self.blank();
        let r = &mut self.screen[row];
        r.free_clusters(col..col + n);
        let cells = &mut r.cells;
        cells[col..].rotate_left(n);
        let len = cells.len();
        cells[len - n..].fill(blank);
        self.repair_wide(row);
    }

    /// After shifting cells sideways, blank any half of a wide character
    /// that lost its other half, and any spacer away from where it
    /// belongs: a spacer not after a wide cell, or an edge spacer not in
    /// the last column of a wrapped row.
    fn repair_wide(&mut self, row: usize) {
        let blank = self.blank();
        self.touch_row(row);
        let r = &mut self.screen[row];
        let len = r.cells.len();
        for i in 0..len {
            let c = r.cells[i];
            let misplaced = match c.width() {
                Width::Narrow => false,
                Width::Wide => r.cells.get(i + 1).is_none_or(|n| n.width() != Width::Spacer),
                Width::Spacer => i == 0 || !r.cells[i - 1].is_wide(),
                Width::EdgeSpacer => i + 1 != len || !r.wrapped,
            };
            if misplaced {
                r.free_clusters(i..i + 1);
                r.cells[i] = blank;
            }
        }
    }

    fn set_alt_screen(&mut self, on: bool, clear: bool) {
        if on == self.alt_active {
            return;
        }
        std::mem::swap(&mut self.screen, &mut self.other_screen);
        std::mem::swap(&mut self.styles, &mut self.other_styles);
        // The templates and the pen's link id name ids in the other
        // screen's tables.
        self.drop_pen_link_id();
        self.erase_cell = None;
        std::mem::swap(&mut self.top_line, &mut self.other_top_line);
        self.alt_active = on;
        if !on && self.primary_stale {
            // Resized while the alternate screen was up: reflow once now.
            self.primary_stale = false;
            self.clear_grapheme();
            self.styles.set_sweeps_blocked(true);
            // The cursor that belongs to the primary screen is the saved
            // one, if there is one (1049 restores it next): its line is
            // the one kept whole. The live cursor is moved with the text
            // like the other positions.
            let swap = |t: &mut Terminal| {
                if let Some(s) = &mut t.saved_primary {
                    std::mem::swap(&mut s.cursor.row, &mut t.cursor.row);
                    std::mem::swap(&mut s.cursor.col, &mut t.cursor.col);
                    std::mem::swap(&mut s.cursor.pending_wrap, &mut t.cursor.pending_wrap);
                }
            };
            swap(self);
            self.reflow(self.cols, self.lines);
            swap(self);
            self.styles.set_sweeps_blocked(false);
            if self.styles.needs_sweep() {
                self.sweep_styles(false);
            }
            self.touch_all_rows();
        }
        if on {
            // Number the alternate screen's rows after the primary's, so a
            // point on one can't be taken for a point on the other.
            self.top_line = self.top_line.max(self.other_top_line + self.lines as u64);
        }
        self.alt_active = on;
        if on && clear {
            self.clear_alt_screen();
        }
        self.view = None;
        self.selection = None;
    }

    /// Clear the alternate screen (the active one) with a fresh style
    /// table: no cell refers to the old one any more, so whatever a program
    /// left there (a gradient, say) is given back.
    fn clear_alt_screen(&mut self) {
        debug_assert!(self.alt_active);
        let epoch = self.next_epoch();
        self.styles = StyleTable::new(epoch, self.cols * self.lines, self.sweep_at_test);
        self.drop_pen_link_id();
        self.erase_cell = None;
        let blank = self.blank();
        for row in &mut self.screen {
            row.reset(self.cols, blank);
        }
        self.touch_screen_rows();
    }

    fn reset(&mut self) {
        let mut fresh = Terminal::new(self.cols, self.lines, self.scrollback.limit());
        // New tables, with epochs that carry on from these.
        let cells = self.cols * self.lines;
        fresh.styles = StyleTable::new(self.epochs + 1, cells, self.sweep_at_test);
        fresh.other_styles = StyleTable::new(self.epochs + 2, cells, self.sweep_at_test);
        fresh.epochs = self.epochs + 2;
        fresh.sweep_at_test = self.sweep_at_test;
        fresh.generation = self.generation.wrapping_add(1);
        fresh.replies = std::mem::take(&mut self.replies);
        // The theme's colours stay; what programs set goes.
        fresh.colors = Box::new(self.colors.reset_all());
        fresh.colors_changed = true;
        fresh.cell_pixels = self.cell_pixels;
        fresh.bulk_ascii = self.bulk_ascii;
        fresh.ambiguous = self.ambiguous;
        fresh.grapheme_log = self.grapheme_log.take();
        fresh.hover = self.hover;
        // The find bar stays open: its search carries on over the new state.
        fresh.search = self.search.take();
        // Like xterm, a full reset clears scrollback too.
        fresh.scrollback = std::mem::replace(&mut self.scrollback, Scrollback::new(0));
        fresh.scrollback.clear();
        // Every cell is new, written at the current setting.
        fresh.scrollback.set_widths_as_written(true);
        // Keep line numbers counting up from the primary screen's.
        let primary_top = if self.alt_active { self.other_top_line } else { self.top_line };
        fresh.top_line = primary_top;
        fresh.other_top_line = primary_top;
        // The same terminal to the renderer: counters carry on, and the
        // cleared history gets a new epoch (§9.1).
        fresh.terminal_id = self.terminal_id;
        fresh.counter = self.counter;
        fresh.batch_start = self.batch_start;
        fresh.history_epoch = self.history_epoch + 1;
        *self = fresh;
        self.touch_all_rows();
    }

    fn write_char(&mut self, c: char) {
        let c = if self.charset != Charset::Ascii { self.charset.map(c) } else { c };
        let p = unicode::props(c);
        // Most text: a plain character with a width starts a cluster
        // unless a Prepend one came before it.
        let width = p.width_with(self.ambiguous);
        if p.is_plain()
            && width != 0
            && self.modes.autowrap
            && self.grapheme_log.is_none()
            && (self.grapheme_plain.is_some() || !self.grapheme.seg.after_prepend())
        {
            let at = self.put_char(c, width as usize);
            self.grapheme_plain = at.map(|_| c);
            if at.is_none() {
                self.grapheme = GraphemeState {
                    at,
                    seg: Segmenter::start_plain(c, p),
                };
            }
            return;
        }
        self.write_char_rules(c, p);
    }

    /// [`Terminal::write_char`] for everything else: segment by the rules.
    #[inline(never)]
    fn write_char_rules(&mut self, c: char, p: unicode::Props) {
        if p.is_control() {
            // A C1 control that came as UTF-8: not printed.
            self.clear_grapheme();
            return;
        }
        self.sync_grapheme();
        let width = p.width_with(self.ambiguous) as usize;
        let rules_join = self.grapheme.seg.push(c, p);
        // Legacy (mode 2027 off): only zero-width scalars join.
        let joins = if self.modes.grapheme_clusters { rules_join } else { width == 0 };
        if let Some(log) = &mut self.grapheme_log {
            log_scalar(log, c, joins);
        }
        if joins {
            if let Some((row, col)) = self.grapheme.at {
                self.join_cell(row, col, c);
                return;
            }
            if width == 0 {
                // The cluster has no cell (it started with a zero-width
                // scalar): dropped.
                return;
            }
            // A scalar with a width joining a cluster with no cell: the
            // cell starts with it.
            self.grapheme.seg = Segmenter::start(c, p);
        } else if width == 0 {
            // A cluster that starts with a zero-width scalar has no cell.
            self.grapheme.at = None;
            return;
        }
        self.grapheme.at = self.put_char(c, width);
    }

    /// `c` joined the cluster in the cell at (`row`, `col`), the last one
    /// printed: add it to the cell, and fix the cell's width in place when
    /// the cluster's width changed (§6.3).
    #[inline(never)]
    fn join_cell(&mut self, row: usize, col: usize, c: char) {
        if !self.screen[row].push_to_cell(col, c) {
            // Past the caps (32 scalars, 128 bytes, 8 KB a row): dropped.
            return;
        }
        // The row changed without a cell being written: its counter must
        // change too (§9.1). This also covers widening or narrowing the
        // cell in place, which stays in this row.
        self.touch_row(row);
        // REP repeats the whole cluster. The cell held `last_char` (its
        // first scalar) or `last_cluster` until now, so the copy grows
        // with it. After a REP it holds nothing, and stays so.
        if let Some(first) = self.last_char.take() {
            self.last_cluster.clear();
            self.last_cluster.push(first);
        }
        if !self.last_cluster.is_empty() && self.last_cluster.len() + c.len_utf8() <= crate::clusters::MAX_BYTES {
            self.last_cluster.push(c);
        }
        let have = if self.screen[row].cells[col].is_wide() { 2 } else { 1 };
        let want = if self.modes.grapheme_clusters {
            self.grapheme.seg.width(self.ambiguous)
        } else {
            have
        };
        self.last_cluster_width = want;
        self.last_seg = self.grapheme.seg;
        if want > have {
            self.widen(row, col);
        } else if want < have {
            self.narrow(row, col);
        }
    }

    /// The narrow cluster at (`row`, `col`), just before the cursor, became
    /// two cells wide (VS16, a second regional indicator, a skin tone,
    /// U+20E3): the next cell becomes its spacer and the cursor moves one
    /// right. In the last column, with autowrap on, the cluster moves to
    /// the start of the next row and leaves an edge spacer; with autowrap
    /// off it stays narrow.
    #[cold]
    #[inline(never)]
    fn widen(&mut self, row: usize, col: usize) {
        let cols = self.cols;
        if col + 1 < cols {
            if self.modes.insert {
                // Make room rather than overwrite what follows.
                let r = &mut self.screen[row];
                r.free_clusters(cols - 1..cols);
                r.cells[cols - 1] = Cell::BLANK;
                r.cells[col + 1..].rotate_right(1);
                self.repair_wide(row);
            }
            // The spacer goes where the next cell is: erase a wide
            // character or cluster there whole.
            if self.screen[row].cells[col + 1].bits() & (Cell::WIDTH | Cell::KIND) != 0 {
                self.erase_cells(row, col + 1..col + 2);
            }
            // Read again: the erase may have renumbered the ids.
            let cells = &mut self.screen[row].cells;
            let cell = cells[col];
            cells[col] = cell.with_width(Width::Wide);
            cells[col + 1] = cell.style_part().with_width(Width::Spacer);
            if col + 2 >= cols {
                self.cursor.col = cols - 1;
                self.cursor.pending_wrap = self.modes.autowrap;
            } else {
                self.cursor.col = col + 2;
                self.cursor.pending_wrap = false;
            }
            return;
        }
        if !self.modes.autowrap {
            return;
        }
        let mut text = String::new();
        self.screen[row].text_at(col, &mut text);
        let first = text.chars().next().unwrap_or(' ');
        let blank = self.blank();
        let r = &mut self.screen[row];
        r.free_clusters(col..col + 1);
        r.cells[col] = blank.with_width(Width::EdgeSpacer);
        r.wrapped = true;
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
        self.linefeed();
        self.grapheme.at = self.put_char(first, 2);
        if let Some((r, c)) = self.grapheme.at
            && text.len() > first.len_utf8()
        {
            self.screen[r].set_cluster(c, &text);
        }
        self.last_char = None;
    }

    /// The wide cluster at (`row`, `col`) became one cell wide (VS15): its
    /// spacer becomes a blank in the same style, and the cursor moves one
    /// left.
    #[cold]
    #[inline(never)]
    fn narrow(&mut self, row: usize, col: usize) {
        let cells = &mut self.screen[row].cells;
        let cell = cells[col];
        cells[col] = cell.with_width(Width::Narrow);
        cells[col + 1] = cell.style_part();
        self.cursor.col = col + 1;
        self.cursor.pending_wrap = false;
    }

    /// Nothing for REP to repeat.
    fn forget_last_char(&mut self) {
        self.last_char = None;
        self.last_cluster.clear();
    }

    /// REP of a cluster: `n` copies of `last_cluster`, each in its own slot.
    #[cold]
    #[inline(never)]
    fn repeat_cluster(&mut self, n: usize) {
        let text = std::mem::take(&mut self.last_cluster);
        let Some(first) = text.chars().next() else { return };
        let width = self.last_cluster_width as usize;
        for _ in 0..n {
            let at = self.put_char(first, width);
            if let Some((r, c)) = at {
                self.screen[r].set_cluster(c, &text);
            }
            self.grapheme = GraphemeState { at, seg: self.last_seg };
        }
        self.last_char = None;
        self.last_cluster = text;
    }

    /// Write one character of `width` (1 or 2) cells at the cursor, wrapping
    /// first if needed, and move the cursor past it. Returns the cell it went
    /// in, or None if it didn't fit (a wide character in the last column with
    /// autowrap off).
    #[inline(always)]
    fn put_char(&mut self, c: char, width: usize) -> Option<(usize, usize)> {
        if self.cursor.pending_wrap && self.modes.autowrap {
            self.screen[self.cursor.row].wrapped = true;
            self.cursor.col = 0;
            self.linefeed();
        }
        if width == 2 && self.cursor.col + 1 >= self.cols {
            if !self.modes.autowrap {
                return None;
            }
            let row = self.cursor.row;
            self.erase_cells(row, self.cursor.col..self.cols);
            // The last column is padding: an edge spacer.
            let last = &mut self.screen[row].cells[self.cols - 1];
            *last = last.with_width(Width::EdgeSpacer);
            self.screen[row].wrapped = true;
            self.cursor.col = 0;
            self.linefeed();
        }

        let (row, col) = (self.cursor.row, self.cursor.col);
        if self.modes.insert {
            let cells = &mut self.screen[row].cells;
            cells[col..].rotate_right(width.min(self.cols - col));
            self.repair_wide(row);
        }
        let cells = &mut self.screen[row].cells;
        // Only overwriting part of a wide character, or a cluster, needs
        // cleanup.
        const PARTS: u64 = Cell::WIDTH | Cell::KIND;
        let touches_wide = cells[col].bits() & PARTS != 0 || (width == 2 && cells[col + 1].bits() & PARTS != 0);
        if touches_wide {
            self.erase_cells(row, col..col + width);
        }
        // After the erase: interning the erase style may renumber ids.
        let pen = self.pen_template();
        // The erase and repair above touch the row themselves.
        let line = &mut self.screen[row];
        snapshot::touch(line, &mut self.counter, self.batch_start);
        let cells = &mut line.cells;
        if width == 2 {
            cells[col] = pen.with_char(c).with_width(Width::Wide);
            cells[col + 1] = pen.with_width(Width::Spacer);
        } else {
            cells[col] = pen.with_char(c);
        }

        if col + width >= self.cols {
            self.cursor.col = self.cols - 1;
            self.cursor.pending_wrap = self.modes.autowrap;
        } else {
            self.cursor.col = col + width;
        }
        self.last_char = Some(c);
        Some((row, col))
    }

    /// Write a run of printable ASCII (0x20..=0x7E): the same result as
    /// [`Terminal::write_char`] for each byte, but one row segment at a time:
    /// the pen is read once, wrapping is handled only at segment ends, and
    /// the cells are written in one pass. Segments that overwrite part of a
    /// wide character, and the modes that change what a character does
    /// (insert, DEC line drawing, autowrap off), go one character at a time.
    fn write_ascii(&mut self, bytes: &[u8]) {
        if self.grapheme_plain.is_none() && self.grapheme.seg.after_prepend() {
            return self.write_ascii_after_prepend(bytes);
        }
        let Some(&last) = bytes.last() else { return };
        if self.modes.insert || self.charset != Charset::Ascii || !self.modes.autowrap {
            for &b in bytes {
                self.write_char(b as char);
            }
            return;
        }
        let cols = self.cols;
        let mut rest = bytes;
        while !rest.is_empty() {
            if self.cursor.pending_wrap {
                self.screen[self.cursor.row].wrapped = true;
                self.cursor.col = 0;
                self.linefeed();
            }
            let (row, col) = (self.cursor.row, self.cursor.col);
            let n = rest.len().min(cols - col);
            let (segment, tail) = rest.split_at(n);
            rest = tail;
            // Taken per segment: a linefeed's scroll may have interned the
            // erase style, and that may have renumbered the ids.
            let pen = self.pen_template().bits();
            // Touched through the borrow the cells are written through:
            // a separate `touch_row` call here made `seq` 30% slower.
            let line = &mut self.screen[row];
            snapshot::touch(line, &mut self.counter, self.batch_start);
            let cells = &mut line.cells[col..col + n];
            if cells.iter().fold(0, |a, c| a | c.bits()) & (Cell::WIDTH | Cell::KIND) != 0 {
                // Overwriting part of a wide character: put_char cleans up.
                for &b in segment {
                    self.put_char(b as char, 1);
                }
                continue;
            }
            for (cell, &b) in cells.iter_mut().zip(segment) {
                *cell = Cell::from_bits(pen | b as u64);
            }
            if col + n == cols {
                self.cursor.col = cols - 1;
                self.cursor.pending_wrap = true;
            } else {
                self.cursor.col = col + n;
            }
        }
        self.last_char = Some(last as char);
        self.segment_ascii(bytes);
    }

    /// The first byte joins the Prepend scalar before it (U+0600 + `1`): it
    /// goes into that cell, by the rules, and the rest as usual.
    #[cold]
    #[inline(never)]
    fn write_ascii_after_prepend(&mut self, bytes: &[u8]) {
        let Some((&first, rest)) = bytes.split_first() else { return };
        self.write_char(first as char);
        if !rest.is_empty() {
            self.write_ascii(rest);
        }
    }

    /// Bring the grapheme state up to date after `write_ascii` wrote `bytes`
    /// (printable ASCII, at least one) in bulk. Printable ASCII after
    /// anything but a Prepend scalar starts a cluster, and after ASCII
    /// always does, so only the first and last bytes matter: the first may
    /// join the cluster before it, and the last is the cluster now open
    /// (a keycap base, say, that a VS16 and U+20E3 can still join). In the
    /// usual case that is one store: the last byte, in `grapheme_plain`.
    #[inline]
    fn segment_ascii(&mut self, bytes: &[u8]) {
        let after_prepend = self.grapheme_plain.is_none() && self.grapheme.seg.after_prepend();
        if after_prepend || self.grapheme_log.is_some() {
            return self.segment_ascii_slow(bytes);
        }
        self.grapheme_plain = Some(bytes[bytes.len() - 1] as char);
    }

    #[cold]
    #[inline(never)]
    fn segment_ascii_slow(&mut self, bytes: &[u8]) {
        self.sync_grapheme();
        let first = bytes[0] as char;
        let last = bytes[bytes.len() - 1] as char;
        // Printable ASCII breaks from anything but Prepend (GB9b).
        let joins = self.grapheme.seg.after_prepend() && self.grapheme.seg.push(first, unicode::props(first));
        if let Some(log) = &mut self.grapheme_log {
            for (i, &b) in bytes.iter().enumerate() {
                log_scalar(log, b as char, i == 0 && joins);
            }
        }
        if joins && bytes.len() == 1 && self.grapheme.at.is_some() {
            return;
        }
        if !joins || bytes.len() > 1 {
            self.grapheme.seg = Segmenter::start_plain(last, unicode::props(last));
        }
        self.grapheme.at = self.cell_before_cursor(1);
    }

    /// The cell a character `width` cells wide was just written in, with
    /// autowrap on and the cursor right after it.
    #[inline]
    fn cell_before_cursor(&self, width: usize) -> Option<(usize, usize)> {
        let end = if self.cursor.pending_wrap { self.cols } else { self.cursor.col };
        end.checked_sub(width).map(|c| (self.cursor.row, c))
    }

    /// The grapheme state, with a pending plain character applied.
    #[inline]
    fn grapheme_now(&self) -> GraphemeState {
        let Some(c) = self.grapheme_plain else { return self.grapheme };
        let p = unicode::props(c);
        GraphemeState {
            at: self.cell_before_cursor(p.width_with(self.ambiguous) as usize),
            seg: Segmenter::start_plain(c, p),
        }
    }

    #[inline]
    fn sync_grapheme(&mut self) {
        if self.grapheme_plain.is_some() {
            self.grapheme = self.grapheme_now();
            self.grapheme_plain = None;
        }
    }

    #[inline]
    fn clear_grapheme(&mut self) {
        self.grapheme.clear();
        self.grapheme_plain = None;
    }

    /// Test aid: record every grapheme cluster as the terminal segments the
    /// printed text (on), and stop (off).
    #[doc(hidden)]
    pub fn record_graphemes(&mut self, on: bool) {
        self.grapheme_log = on.then(Vec::new);
    }

    /// Test aid: the clusters recorded since the last call.
    #[doc(hidden)]
    pub fn take_graphemes(&mut self) -> Vec<String> {
        self.grapheme_log.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Test aid: the grapheme segmentation state.
    #[doc(hidden)]
    pub fn grapheme_state(&self) -> GraphemeState {
        let g = self.grapheme_now();
        // A reset segmenter keeps stale fields; show it as the default.
        if g.seg.is_reset() { GraphemeState::default() } else { g }
    }

    fn mode_state(&self, mode: u16, private: bool) -> u8 {
        let set = |b: bool| if b { 1 } else { 2 };
        if !private {
            return match mode {
                4 => set(self.modes.insert),
                20 => set(self.modes.linefeed_newline),
                _ => 0,
            };
        }
        match mode {
            1 => set(self.modes.app_cursor),
            6 => set(self.modes.origin),
            7 => set(self.modes.autowrap),
            12 => set(self.cursor_blink),
            25 => set(self.modes.cursor_visible),
            47 | 1047 | 1049 => set(self.alt_active),
            1000 => set(self.modes.mouse == MouseMode::Click),
            1002 => set(self.modes.mouse == MouseMode::Drag),
            1003 => set(self.modes.mouse == MouseMode::Motion),
            1004 => set(self.modes.focus_events),
            1006 => set(self.modes.mouse_sgr),
            2004 => set(self.modes.bracketed_paste),
            2026 => set(self.modes.synchronized),
            2027 => set(self.modes.grapheme_clusters),
            _ => 0,
        }
    }

    /// DECSTR: reset modes and the pen, but keep the screen.
    fn soft_reset(&mut self) {
        let keep_mouse = (
            self.modes.mouse,
            self.modes.mouse_sgr,
            self.modes.focus_events,
            self.modes.bracketed_paste,
        );
        self.modes = Modes::default();
        (
            self.modes.mouse,
            self.modes.mouse_sgr,
            self.modes.focus_events,
            self.modes.bracketed_paste,
        ) = keep_mouse;
        self.cursor.pen = Style::DEFAULT;
        self.set_link(None);
        self.pen_changed();
        self.cursor.pending_wrap = false;
        self.scroll_top = 0;
        self.scroll_bottom = self.lines - 1;
        self.charsets = [Charset::Ascii; 2];
        self.active_charset = 0;
        self.charset = Charset::Ascii;
        self.saved_primary = None;
        self.saved_alt = None;
    }

    fn set_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.modes.app_cursor = on,
            6 => {
                self.modes.origin = on;
                self.goto(0, 0);
            }
            7 => self.modes.autowrap = on,
            12 => self.cursor_blink = on,
            25 => self.modes.cursor_visible = on,
            47 => self.set_alt_screen(on, false),
            // 1047 clears the alternate screen when leaving it, not on entry.
            1047 => {
                if !on && self.alt_active {
                    self.clear_alt_screen();
                }
                self.set_alt_screen(on, false);
            }
            1048 => {
                if on {
                    self.save_cursor()
                } else {
                    self.restore_cursor()
                }
            }
            1049 => {
                if on {
                    self.save_cursor();
                    self.set_alt_screen(true, true);
                } else {
                    self.set_alt_screen(false, false);
                    self.restore_cursor();
                }
            }
            1000 => self.modes.mouse = if on { MouseMode::Click } else { MouseMode::Off },
            1002 => self.modes.mouse = if on { MouseMode::Drag } else { MouseMode::Off },
            1003 => self.modes.mouse = if on { MouseMode::Motion } else { MouseMode::Off },
            1004 => self.modes.focus_events = on,
            1006 => self.modes.mouse_sgr = on,
            2004 => self.modes.bracketed_paste = on,
            2026 => self.modes.synchronized = on,
            2027 => self.modes.grapheme_clusters = on,
            _ => {}
        }
    }

    fn sgr(&mut self, params: &Params) {
        // SGR leaves the OSC 8 link alone, SGR 0 included.
        let link = self.cursor.pen.link;
        apply_sgr(&mut self.cursor.pen, params);
        self.cursor.pen.link = link;
        self.pen_changed();
    }
}

/// Apply an SGR sequence to `pen`. Stored: 1–9 (4 with `4:0`–`4:5`), 21,
/// 22–29, 30–49 (38/48 extended), 53/55, 58/59 and 90–107 (§5.1).
#[inline]
fn apply_sgr(pen: &mut Style, params: &Params) {
    if params.is_empty() {
        *pen = Style::DEFAULT;
        return;
    }
    let mut iter = params.iter();
    while let Some(p) = iter.next() {
        match p[0] {
            0 => *pen = Style::DEFAULT,
            1 => pen.attrs.insert(Attrs::BOLD),
            2 => pen.attrs.insert(Attrs::DIM),
            3 => pen.attrs.insert(Attrs::ITALIC),
            4 => {
                pen.underline = match p.get(1) {
                    None | Some(1) => Underline::Single,
                    Some(0) => Underline::None,
                    Some(2) => Underline::Double,
                    Some(3) => Underline::Curly,
                    Some(4) => Underline::Dotted,
                    Some(5) => Underline::Dashed,
                    // An unknown underline style: keep the current one.
                    Some(_) => pen.underline,
                }
            }
            5 | 6 => pen.attrs.insert(Attrs::BLINK),
            7 => pen.attrs.insert(Attrs::INVERSE),
            8 => pen.attrs.insert(Attrs::HIDDEN),
            9 => pen.attrs.insert(Attrs::STRIKE),
            21 => pen.underline = Underline::Double,
            22 => pen.attrs.remove(Attrs::BOLD | Attrs::DIM),
            23 => pen.attrs.remove(Attrs::ITALIC),
            24 => pen.underline = Underline::None,
            25 => pen.attrs.remove(Attrs::BLINK),
            27 => pen.attrs.remove(Attrs::INVERSE),
            28 => pen.attrs.remove(Attrs::HIDDEN),
            29 => pen.attrs.remove(Attrs::STRIKE),
            n @ 30..=37 => pen.fg = Color::indexed((n - 30) as u8),
            38 => {
                if let Some(c) = extended_color(p, &mut iter) {
                    pen.fg = c;
                }
            }
            39 => pen.fg = Color::DEFAULT,
            n @ 40..=47 => pen.bg = Color::indexed((n - 40) as u8),
            48 => {
                if let Some(c) = extended_color(p, &mut iter) {
                    pen.bg = c;
                }
            }
            49 => pen.bg = Color::DEFAULT,
            53 => pen.attrs.insert(Attrs::OVERLINE),
            55 => pen.attrs.remove(Attrs::OVERLINE),
            58 => {
                if let Some(c) = extended_color(p, &mut iter) {
                    pen.underline_color = c;
                }
            }
            59 => pen.underline_color = Color::DEFAULT,
            n @ 90..=97 => pen.fg = Color::indexed((n - 90 + 8) as u8),
            n @ 100..=107 => pen.bg = Color::indexed((n - 100 + 8) as u8),
            _ => {}
        }
    }
}

/// Parse `38;5;n`, `38;2;r;g;b` and their colon forms (`38:2::r:g:b`).
fn extended_color(first: &[u16], rest: &mut vte::ParamsIter<'_>) -> Option<Color> {
    if first.len() > 1 {
        return match first[1] {
            5 => first.get(2).map(|&i| Color::indexed(i as u8)),
            2 => {
                // With a colour-space id the RGB starts one later.
                let rgb = if first.len() >= 6 { &first[3..6] } else { first.get(2..5)? };
                Some(Color::rgb(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8))
            }
            _ => None,
        };
    }
    match rest.next()?[0] {
        5 => rest.next().map(|i| Color::indexed(i[0] as u8)),
        2 => {
            let r = rest.next()?[0];
            let g = rest.next()?[0];
            let b = rest.next()?[0];
            Some(Color::rgb(r as u8, g as u8, b as u8))
        }
        _ => None,
    }
}

fn blank_screen(cols: usize, lines: usize) -> VecDeque<Row> {
    (0..lines).map(|_| Row::new(cols, Cell::BLANK)).collect()
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c % 8 == 0).collect()
}

/// Returns the rows removed from the top.
fn resize_screen(screen: &mut VecDeque<Row>, cursor: &mut Cursor, cols: usize, lines: usize) -> Vec<Row> {
    let mut removed = Vec::new();
    for row in screen.iter_mut() {
        let old = row.cells.len();
        row.resize_cells(cols);
        // An edge spacer no longer in the last column is a blank.
        if cols > old && old > 0 && row.cells[old - 1].width() == Width::EdgeSpacer {
            row.cells[old - 1] = row.cells[old - 1].style_part();
        }
        if row.cells.last().is_some_and(|c| c.is_wide()) {
            row.free_clusters(cols - 1..cols);
            row.cells[cols - 1] = Cell::BLANK;
        }
    }
    while screen.len() > lines {
        // Remove blank rows below the cursor first, then rows from the top.
        if screen.len() - 1 > cursor.row && screen.back().is_some_and(|r| r.cells.iter().all(|c| c.is_blank())) {
            screen.pop_back();
        } else {
            removed.extend(screen.pop_front());
            cursor.row = cursor.row.saturating_sub(1);
        }
    }
    while screen.len() < lines {
        screen.push_back(Row::new(cols, Cell::BLANK));
    }
    removed
}

fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in input {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn dec_line_drawing(c: char) -> char {
    match c {
        '`' => '◆',
        'a' => '▒',
        'b' => '␉',
        'c' => '␌',
        'd' => '␍',
        'e' => '␊',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        '_' => ' ',
        c => c,
    }
}

/// How much of an OSC string over 4 KB to keep (§14 of the cells design);
/// `prefix` is what has arrived so far. Past its cap a string is dropped
/// whole, not dispatched.
/// - Clipboard copies (OSC 52) up to 4 MB, so a big yank (Neovim over SSH,
///   say) still arrives.
/// - iTerm2 file parts (`1337;FilePart=`) up to 1 MB each.
/// - Whole iTerm2 files (`1337;File=`) are dropped: they will stream into
///   the iTerm2 image reader when there is one, and nothing reads them now.
/// - Everything else up to 64 KB (vte's default).
fn osc_size_policy(prefix: &[u8]) -> OscLarge {
    const KB: usize = 1024;
    if prefix.starts_with(b"52;") {
        OscLarge::Buffer(4 * KB * KB)
    } else if prefix.starts_with(b"1337;FilePart=") {
        OscLarge::Buffer(KB * KB)
    } else if prefix.starts_with(b"1337;File=") {
        OscLarge::Drop
    } else {
        OscLarge::Buffer(64 * KB)
    }
}

/// Add a scalar to the recorded clusters: to the last one if it `joins`.
/// Where a mark sorts among marks at the same column. They can share one
/// when the output before them was empty: the previous command's output
/// (C), its end (D), then the next prompt (A) and its command line (B).
fn mark_rank(kind: MarkKind) -> u8 {
    match kind {
        MarkKind::Output => 0,
        MarkKind::End => 1,
        MarkKind::Prompt => 2,
        MarkKind::Input => 3,
    }
}

fn log_scalar(log: &mut Vec<String>, c: char, joins: bool) {
    match log.last_mut() {
        Some(last) if joins => last.push(c),
        _ => log.push(c.to_string()),
    }
}

impl Perform for Terminal {
    fn print(&mut self, c: char) {
        self.write_char(c);
        self.touch();
    }

    fn print_ascii(&mut self, bytes: &[u8]) {
        if self.bulk_ascii {
            self.write_ascii(bytes);
            self.touch();
        } else {
            for &b in bytes {
                self.print(b as char);
            }
        }
    }

    fn hook(&mut self, _params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.clear_grapheme();
        self.dcs_hook(intermediates, ignore, action);
    }

    fn put(&mut self, byte: u8) {
        self.dcs_put(&[byte]);
    }

    fn put_slice(&mut self, data: &[u8]) {
        self.dcs_put(data);
    }

    fn dcs_end(&mut self, complete: bool) {
        self.dcs_finish(complete);
    }

    fn apc_start(&mut self) {
        self.clear_grapheme();
    }

    fn execute(&mut self, byte: u8) {
        self.clear_grapheme();
        match byte {
            0x07 => self.bell = true,
            0x08 => {
                self.cursor.col = self.cursor.col.saturating_sub(1);
                self.cursor.pending_wrap = false;
            }
            0x09 => {
                let next = (self.cursor.col + 1..self.cols).find(|&c| self.tabs[c]).unwrap_or(self.cols - 1);
                self.goto_col(next);
            }
            0x0A..=0x0C => {
                self.linefeed();
                if self.modes.linefeed_newline {
                    self.cursor.col = 0;
                }
            }
            0x0D => self.goto_col(0),
            0x0E | 0x0F => {
                self.active_charset = (byte == 0x0E) as usize;
                self.charset = self.charsets[self.active_charset];
            }
            _ => return,
        }
        self.touch();
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.clear_grapheme();
        if ignore {
            return;
        }
        let mut args = [0u16; 16];
        let mut n = 0;
        for p in params.iter() {
            if n < args.len() {
                args[n] = p[0];
                n += 1;
            }
        }
        let args = &args[..n];
        // First argument, with 0 or missing treated as 1.
        let a1 = args.first().copied().filter(|&v| v != 0).unwrap_or(1) as usize;
        let arg = |i: usize| args.get(i).copied().unwrap_or(0) as usize;
        let private = intermediates.first() == Some(&b'?');

        if matches!(action, '@' | 'J' | 'K' | 'X' | 'P') {
            self.cursor.pending_wrap = false;
        }
        match (action, intermediates) {
            ('@', []) => self.insert_chars(a1),
            ('A', []) => self.move_rows(-(a1 as isize)),
            ('B' | 'e', []) => self.move_rows(a1 as isize),
            ('C' | 'a', []) => self.goto_col(self.cursor.col + a1),
            ('D', []) => self.goto_col(self.cursor.col.saturating_sub(a1)),
            ('E', []) => {
                self.move_rows(a1 as isize);
                self.goto_col(0);
            }
            ('F', []) => {
                self.move_rows(-(a1 as isize));
                self.goto_col(0);
            }
            ('G' | '`', []) => self.goto_col(a1 - 1),
            ('H' | 'f', []) => {
                let row = arg(0).max(1) - 1;
                let col = arg(1).max(1) - 1;
                self.goto(row, col);
            }
            ('d', []) => {
                let col = self.cursor.col;
                self.goto(a1 - 1, col);
            }
            ('I', []) => {
                for _ in 0..a1.min(self.cols) {
                    self.execute(0x09);
                }
            }
            ('Z', []) => {
                for _ in 0..a1.min(self.cols) {
                    let prev = (0..self.cursor.col).rev().find(|&c| self.tabs[c]).unwrap_or(0);
                    self.goto_col(prev);
                }
            }
            ('J', [] | [b'?']) => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                match arg(0) {
                    0 => {
                        self.erase_cells(row, col..self.cols);
                        self.screen[row].wrapped = false;
                        for r in row + 1..self.lines {
                            self.erase_cells(r, 0..self.cols);
                            self.screen[r].wrapped = false;
                        }
                        if (row, col) == (0, 0) {
                            self.screen_cleared();
                        }
                    }
                    1 => {
                        for r in 0..row {
                            self.erase_cells(r, 0..self.cols);
                        }
                        self.erase_cells(row, 0..col + 1);
                    }
                    2 => {
                        for r in 0..self.lines {
                            self.erase_cells(r, 0..self.cols);
                            self.screen[r].wrapped = false;
                        }
                        self.screen_cleared();
                    }
                    3 => {
                        self.scrollback.clear();
                        self.history_epoch += 1;
                        self.view = None;
                        self.selection = None;
                    }
                    _ => {}
                }
            }
            ('K', [] | [b'?']) => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                match arg(0) {
                    0 => {
                        self.erase_cells(row, col..self.cols);
                        self.screen[row].wrapped = false;
                    }
                    1 => self.erase_cells(row, 0..col + 1),
                    2 => {
                        self.erase_cells(row, 0..self.cols);
                        self.screen[row].wrapped = false;
                    }
                    _ => {}
                }
            }
            ('L' | 'M', []) => {
                if (self.scroll_top..=self.scroll_bottom).contains(&self.cursor.row) {
                    let top = self.scroll_top;
                    self.scroll_top = self.cursor.row;
                    if action == 'L' {
                        self.scroll_down(a1)
                    } else {
                        self.scroll_up_no_history(a1)
                    }
                    self.scroll_top = top;
                    self.cursor.col = 0;
                    self.cursor.pending_wrap = false;
                }
            }
            ('P', []) => self.delete_chars(a1),
            ('X', []) => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                self.erase_cells(row, col..col + a1);
            }
            ('S', []) => self.scroll_up(a1),
            ('T', []) => self.scroll_down(a1),
            ('b', []) => {
                let n = a1.min(self.cols * self.lines);
                if let Some(c) = self.last_char {
                    for _ in 0..n {
                        // Each copy is its own cluster: copies that could
                        // join (regional indicators, Hangul jamo, Prepend)
                        // mustn't merge (§6.4). The last copy keeps its
                        // state, so a mark printed next still joins it.
                        self.clear_grapheme();
                        self.write_char(c);
                    }
                } else if !self.last_cluster.is_empty() {
                    self.repeat_cluster(n);
                }
                // As in xterm, REP uses the character up: a second REP
                // straight after repeats nothing.
                self.forget_last_char();
            }
            ('c', []) => self.replies.extend_from_slice(b"\x1b[?62;22c"),
            ('c', [b'>']) => self.replies.extend_from_slice(b"\x1b[>1;10;0c"),
            // DECREQTPARM: no parity, 8 bits, 38400 baud both ways, clock
            // multiplier 1, no STP flags; solicited (3) for `1 x`.
            ('x', []) if arg(0) <= 1 => {
                let reply = format!("\x1b[{};1;1;128;128;1;0x", arg(0) + 2);
                self.replies.extend_from_slice(reply.as_bytes());
            }
            ('g', []) => match arg(0) {
                0 => self.tabs[self.cursor.col] = false,
                3 => self.tabs.fill(false),
                _ => {}
            },
            ('h' | 'l', _) => {
                let on = action == 'h';
                for &mode in args {
                    if private {
                        self.set_private_mode(mode, on);
                    } else {
                        match mode {
                            4 => self.modes.insert = on,
                            20 => self.modes.linefeed_newline = on,
                            _ => {}
                        }
                    }
                }
            }
            ('m', []) => self.sgr(params),
            ('n', []) => match arg(0) {
                5 => self.replies.extend_from_slice(b"\x1b[0n"),
                6 => {
                    let row = if self.modes.origin {
                        self.cursor.row.saturating_sub(self.scroll_top)
                    } else {
                        self.cursor.row
                    };
                    let reply = format!("\x1b[{};{}R", row + 1, self.cursor.col + 1);
                    self.replies.extend_from_slice(reply.as_bytes());
                }
                _ => {}
            },
            // DECXCPR: the cursor position. A VT400 adds the page (always
            // 1); we answer DA as a VT220, which doesn't, as xterm does then.
            ('n', [b'?']) if arg(0) == 6 => {
                let row = if self.modes.origin {
                    self.cursor.row.saturating_sub(self.scroll_top)
                } else {
                    self.cursor.row
                };
                let reply = format!("\x1b[?{};{}R", row + 1, self.cursor.col + 1);
                self.replies.extend_from_slice(reply.as_bytes());
            }
            ('r', []) => {
                let top = arg(0).max(1) - 1;
                let bottom = if arg(1) == 0 { self.lines } else { arg(1).min(self.lines) } - 1;
                if top < bottom {
                    self.scroll_top = top;
                    self.scroll_bottom = bottom;
                    self.goto(0, 0);
                }
            }
            ('s', []) => self.save_cursor(),
            ('u', []) => self.restore_cursor(),
            ('q', [b' ']) => {
                let (shape, blink) = match arg(0) {
                    0 | 1 => (CursorShape::Block, true),
                    2 => (CursorShape::Block, false),
                    3 => (CursorShape::Underline, true),
                    4 => (CursorShape::Underline, false),
                    5 => (CursorShape::Bar, true),
                    6 => (CursorShape::Bar, false),
                    _ => (self.cursor_shape, self.cursor_blink),
                };
                self.cursor_shape = shape;
                self.cursor_blink = blink;
            }
            ('p', [b'!']) => self.soft_reset(),
            ('p', [b'$']) | ('p', [b'?', b'$']) => {
                let private = intermediates[0] == b'?';
                let mode = arg(0) as u16;
                let state = self.mode_state(mode, private);
                let reply = format!("\x1b[{}{mode};{state}$y", if private { "?" } else { "" });
                self.replies.extend_from_slice(reply.as_bytes());
            }
            ('u', [b'?']) => {
                let reply = format!("\x1b[?{}u", self.kitty_keyboard_flags());
                self.replies.extend_from_slice(reply.as_bytes());
            }
            ('u', [b'>']) => {
                let stack = &mut self.kitty_flags[self.alt_active as usize];
                if stack.len() >= 16 {
                    stack.remove(0);
                }
                stack.push(arg(0) as u8 & 0x1F);
            }
            ('u', [b'<']) => {
                let stack = &mut self.kitty_flags[self.alt_active as usize];
                let n = a1.min(stack.len());
                stack.truncate(stack.len() - n);
            }
            ('u', [b'=']) => {
                let stack = &mut self.kitty_flags[self.alt_active as usize];
                let cur = stack.last().copied().unwrap_or(0);
                let f = arg(0) as u8 & 0x1F;
                let new = match args.get(1).copied().unwrap_or(1) {
                    2 => cur | f,
                    3 => cur & !f,
                    _ => f,
                };
                match stack.last_mut() {
                    Some(top) => *top = new,
                    None => stack.push(new),
                }
            }
            ('q', [b'>']) => {
                let reply = format!("\x1bP>|terminal-decline {}\x1b\\", env!("CARGO_PKG_VERSION"));
                self.replies.extend_from_slice(reply.as_bytes());
            }
            ('t', []) => {
                let (cw, ch) = self.cell_pixels;
                let reply = match arg(0) {
                    14 => format!("\x1b[4;{};{}t", self.lines * ch as usize, self.cols * cw as usize),
                    16 => format!("\x1b[6;{ch};{cw}t"),
                    18 => format!("\x1b[8;{};{}t", self.lines, self.cols),
                    // Push and pop the title (0 or 2: the window title;
                    // there is no icon label to keep).
                    22 if arg(1) != 1 => {
                        if self.title_stack.len() >= TITLE_STACK {
                            self.title_stack.remove(0);
                        }
                        self.title_stack.push(self.title.clone());
                        String::new()
                    }
                    23 if arg(1) != 1 => {
                        if let Some(title) = self.title_stack.pop() {
                            self.title = title;
                            self.title_changed = true;
                        }
                        String::new()
                    }
                    _ => String::new(),
                };
                self.replies.extend_from_slice(reply.as_bytes());
            }
            _ => return,
        }
        self.touch();
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        self.clear_grapheme();
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.linefeed();
                self.cursor.col = 0;
            }
            ([], b'H') => self.tabs[self.cursor.col] = true,
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([b'(' | b')'], b) => {
                let g = (intermediates[0] == b')') as usize;
                self.charsets[g] = match b {
                    b'0' => Charset::DecGraphics,
                    b'A' => Charset::British,
                    _ => Charset::Ascii,
                };
                self.charset = self.charsets[self.active_charset];
            }
            // DECALN: fill with `E`, reset the margins, cursor home.
            ([b'#'], b'8') => {
                for row in &mut self.screen {
                    row.fill(Cell::BLANK.with_char('E'));
                }
                self.touch_screen_rows();
                self.scroll_top = 0;
                self.scroll_bottom = self.lines - 1;
                self.cursor.row = 0;
                self.cursor.col = 0;
                self.cursor.pending_wrap = false;
            }
            // DECID: answered as primary DA.
            ([], b'Z') => self.replies.extend_from_slice(b"\x1b[?62;22c"),
            _ => return,
        }
        self.touch();
    }

    fn osc_large(&mut self, prefix: &[u8]) -> OscLarge {
        osc_size_policy(prefix)
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.clear_grapheme();
        if self.osc_color(params) {
            return;
        }
        match params {
            [b"52", _target, data] if *data != b"?" => {
                if let Some(bytes) = base64_decode(data) {
                    self.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
                }
            }
            // ConEmu's progress: `9;4;state;percent`.
            [b"9", b"4", state, rest @ ..] => {
                let pct = || {
                    rest.first()
                        .and_then(|p| std::str::from_utf8(p).ok()?.parse::<u8>().ok())
                        .unwrap_or(0)
                        .min(100)
                };
                self.progress = match *state {
                    b"0" => Some(Progress::Clear),
                    b"1" => Some(Progress::Normal(pct())),
                    b"2" => Some(Progress::Error(pct())),
                    b"3" => Some(Progress::Indeterminate),
                    b"4" => Some(Progress::Paused(pct())),
                    _ => self.progress,
                };
            }
            [b"9", body @ ..] | [b"777", b"notify", _, body @ ..] => {
                let body = body.iter().map(|p| String::from_utf8_lossy(p)).collect::<Vec<_>>().join(";");
                let title = match params {
                    [b"777", _, title, ..] => Some(String::from_utf8_lossy(title).into_owned()),
                    _ => None,
                };
                if !body.is_empty() || title.as_deref().is_some_and(|t| !t.is_empty()) {
                    self.notification = Some(Notification { title, body });
                }
            }
            [b"8", ..] => self.set_link(crate::links::parse_osc8(params)),
            [b"133", kind, rest @ ..] => self.prompt_mark(kind, rest),
            [b"7", url, ..] => {
                let url = String::from_utf8_lossy(url);
                if let Some(rest) = url.strip_prefix("file://") {
                    // Skip the host name.
                    let path = rest.find('/').map_or("", |i| &rest[i..]);
                    self.cwd = Some(percent_decode(path));
                }
            }
            [b"0" | b"2", title @ ..] => {
                let title = title.iter().map(|p| String::from_utf8_lossy(p)).collect::<Vec<_>>().join(";");
                self.title = Some(title);
                self.title_changed = true;
            }
            _ => {}
        }
    }
}

/// Testing aids: shared by the unit, property, conformance and fuzz tests.
impl Terminal {
    /// Turn the bulk ASCII path off (or back on), for tests that compare it
    /// with writing one character at a time.
    #[doc(hidden)]
    pub fn set_bulk_ascii(&mut self, on: bool) {
        self.bulk_ascii = on;
    }

    /// Test aid: every stored history line read through (its sections,
    /// text and the block's checkpoints), a pass over all of history.
    #[doc(hidden)]
    pub fn check_history(&self) -> Result<(), String> {
        self.scrollback.check_all()
    }

    /// Test aid: pull the newest `n` history lines back onto the top of
    /// the primary screen, as they were stored and without the reflow a
    /// resize does, dropping as many rows from the bottom. Their line numbers
    /// are handed out again as output pushes them back, which is what the
    /// snapshot's `popped_from` is for.
    #[doc(hidden)]
    pub fn pull_back_for_test(&mut self, n: usize) {
        if self.alt_active {
            return;
        }
        self.clear_grapheme();
        self.styles.set_sweeps_blocked(true);
        let (mut row, mut styles, mut links) = (Row::default(), Vec::new(), Vec::new());
        for _ in 0..n {
            if !self.scrollback.pop_into(self.cols, &mut row, &mut styles, &mut links) {
                break;
            }
            let ids: Vec<Cell> = styles
                .iter()
                .map(|s| {
                    let mut s = *s;
                    if let Some(Some(l)) = links.get(s.link as usize) {
                        s.link = self.styles.links_mut().intern(l);
                    }
                    if s == Style::DEFAULT { Cell::BLANK } else { self.intern(s) }
                })
                .collect();
            for c in &mut row.cells {
                *c = c.with_style(ids[c.style_id() as usize]);
            }
            // To the screen's width, as a resize without reflow does.
            let mut one = VecDeque::from([std::mem::take(&mut row)]);
            resize_screen(&mut one, &mut Cursor::default(), self.cols, 1);
            let mut back = one.pop_front().expect("one row");
            if back.cells.last().is_some_and(|c| c.width() == Width::EdgeSpacer) && !back.wrapped {
                let last = back.cells.len() - 1;
                back.cells[last] = back.cells[last].style_part();
            }
            back.seq = 0;
            snapshot::touch(&mut back, &mut self.counter, self.batch_start);
            self.screen.pop_back();
            self.screen.push_front(back);
            self.top_line -= 1;
            self.cursor.row = (self.cursor.row + 1).min(self.lines - 1);
        }
        self.styles.set_sweeps_blocked(false);
        if self.styles.needs_sweep() {
            self.sweep_styles(false);
        }
        self.drop_layouts();
        self.view = self.view_point();
        self.selection = None;
        self.touch();
    }

    /// Test aid: sweep the style tables once they hold `n` ids (at least
    /// 2), instead of 1.5 × the screen's cells; `None` goes back to that.
    /// With a small `n` sweeps and renumberings happen every few prints, so
    /// every holder of an id is exercised (§5.2).
    #[doc(hidden)]
    pub fn set_sweep_at(&mut self, n: Option<u32>) {
        self.sweep_at_test = n;
        let cells = self.cols * self.lines;
        self.styles.set_sweep_at_test(n, cells);
        self.other_styles.set_sweep_at_test(n, cells);
    }

    /// Test aid: the active screen's style table, and the other's.
    #[doc(hidden)]
    pub fn style_stats(&self) -> (StyleStats, StyleStats) {
        (self.styles.stats(), self.other_styles.stats())
    }

    /// Test aid: the active screen's table by id, `None` for free ids.
    #[doc(hidden)]
    pub fn live_styles(&self) -> Vec<Option<Style>> {
        (0..self.styles.styles().len() as u32)
            .map(|id| self.styles.is_live(id).then(|| *self.styles.get(id)))
            .collect()
    }

    /// Test aid: sweep the active screen's table now, and check that
    /// exactly the ids its cells and templates use are left, and that a
    /// renumbering left them dense.
    #[doc(hidden)]
    pub fn sweep_styles_now(&mut self) -> Result<(), String> {
        let before = self.styles.stats().renumberings;
        self.sweep_styles(true);
        let mut used: Vec<u32> = self.screen.iter().flat_map(|r| r.cells.iter().map(|c| c.style_id())).collect();
        used.extend(self.pen_cell.iter().chain(&self.erase_cell).map(|c| c.style_id()));
        used.push(0);
        used.sort_unstable();
        used.dedup();
        let live: Vec<u32> = (0..self.styles.styles().len() as u32)
            .filter(|&id| self.styles.is_live(id))
            .collect();
        if live != used {
            return Err(format!("after a sweep, live ids {live:?} but used {used:?}"));
        }
        let stats = self.styles.stats();
        if stats.renumberings > before && stats.len != stats.live {
            return Err(format!("renumbered to {} ids, {} live", stats.len, stats.live));
        }
        self.check_invariants()
    }

    /// Check the rules the rest of the code relies on: the cursor is on the
    /// screen; both screens have `lines` rows of exactly `cols` cells; every
    /// wide cell is followed by a spacer in its style and every spacer
    /// follows a wide cell; every cell's style id is live in its screen's
    /// table and its SE bit matches the style; the pen and erase templates
    /// are live in the active screen's table and stand for the pen and the
    /// erase style; each table is consistent on its own (§17.3); the view
    /// is not scrolled back further than the scrollback; the scroll region
    /// is inside the screen. Costs one pass over the screens.
    #[doc(hidden)]
    pub fn check_invariants(&self) -> Result<(), String> {
        let (cols, lines) = (self.cols, self.lines);
        self.check_counters()?;
        if self.cursor.row >= lines || self.cursor.col >= cols {
            return Err(format!("cursor ({}, {}) outside {cols}x{lines}", self.cursor.row, self.cursor.col));
        }
        for (name, screen, styles) in [
            ("screen", &self.screen, &self.styles),
            ("other screen", &self.other_screen, &self.other_styles),
        ] {
            styles.check().map_err(|e| format!("{name}'s style table: {e}"))?;
            if name == "other screen" && self.primary_stale {
                // The primary screen keeps its size until it is shown.
                continue;
            }
            if screen.len() != lines {
                return Err(format!("{name} has {} rows, expected {lines}", screen.len()));
            }
            for (r, row) in screen.iter().enumerate() {
                if row.cells.len() != cols {
                    return Err(format!("{name} row {r} has {} cells, expected {cols}", row.cells.len()));
                }
                for (c, &cell) in row.cells.iter().enumerate() {
                    check_cell(cell, styles).map_err(|e| format!("{name} row {r} col {c}: {e}"))?;
                    let next = row.cells.get(c + 1);
                    if cell.is_wide() && !next.is_some_and(|n| n.width() == Width::Spacer) {
                        return Err(format!("{name} row {r} col {c}: wide cell without a spacer after it"));
                    }
                    if cell.width() == Width::Spacer {
                        if c == 0 || !row.cells[c - 1].is_wide() {
                            return Err(format!("{name} row {r} col {c}: spacer without a wide cell before it"));
                        }
                        if cell.style_id() != row.cells[c - 1].style_id() {
                            return Err(format!("{name} row {r} col {c}: spacer in another style than its wide cell"));
                        }
                    }
                    if cell.width() == Width::EdgeSpacer && (c + 1 != cols || !row.wrapped) {
                        return Err(format!(
                            "{name} row {r} col {c}: edge spacer not in the last column of a wrapped row"
                        ));
                    }
                }
                check_clusters(row).map_err(|e| format!("{name} row {r}: {e}"))?;
                if let Some(m) = row.marks().find(|m| m.col as usize >= cols) {
                    return Err(format!("{name} row {r}: mark {m:?} past column {cols}"));
                }
            }
        }
        // The cached templates name live ids of the active screen's table,
        // with the styles they stand for.
        if let Some(t) = self.pen_cell {
            check_template(t, &self.cursor.pen, &self.styles).map_err(|e| format!("pen template: {e}"))?;
        }
        match (&self.pen_link, self.pen_link_id) {
            (None, 0) => {}
            (Some(link), id) if id == 0 || self.styles.links().get(id) == Some(link) => {}
            (link, id) => return Err(format!("pen link {link:?} has id {id}")),
        }
        if self.pen_cell.is_some() && self.cursor.pen.link != self.pen_link_id {
            return Err(format!(
                "pen style has link {}, pen link id {}",
                self.cursor.pen.link, self.pen_link_id
            ));
        }
        if let Some(t) = self.erase_cell {
            let erase = Style {
                bg: self.erase_bg,
                ..Style::DEFAULT
            };
            check_template(t, &erase, &self.styles).map_err(|e| format!("erase template: {e}"))?;
        }
        let (a, b) = (self.styles.epoch(), self.other_styles.epoch());
        if a == b || a.max(b) > self.epochs {
            return Err(format!("table epochs {a} and {b}, counter at {}", self.epochs));
        }
        if self.scroll_top > self.scroll_bottom || self.scroll_bottom >= lines {
            return Err(format!(
                "scroll region {}..={} invalid for {lines} lines",
                self.scroll_top, self.scroll_bottom
            ));
        }
        let primary_top = if self.alt_active { self.other_top_line } else { self.top_line };
        if primary_top != self.scrollback.total() {
            return Err(format!(
                "primary screen starts at line {primary_top}, history ends at {}",
                self.scrollback.total()
            ));
        }
        self.scrollback.check().map_err(|e| format!("history: {e}"))?;
        let primary = if self.alt_active { &self.other_screen } else { &self.screen };
        for (r, row) in primary.iter().enumerate() {
            self.scrollback
                .check_widths(row)
                .map_err(|e| format!("primary row {r}, as history would store it: {e}"))?;
        }
        if self.last_cluster.len() > crate::clusters::MAX_BYTES {
            return Err(format!("the last grapheme is {} bytes", self.last_cluster.len()));
        }
        if self.tabs.len() != cols {
            return Err(format!("{} tab stops for {cols} columns", self.tabs.len()));
        }
        Ok(())
    }

    /// The visible rows as text, for tests. One line per visible row (taking
    /// the view's scroll position into account), then a status line:
    ///
    /// ```text
    /// hello world··|
    /// a long line that wra⏎
    /// ps here|
    /// |
    /// cursor=(2,7) alt=false offset=0
    /// ```
    ///
    /// Each row is written up to its last non-blank cell ([`Cell::is_blank`]:
    /// a typed space or an erased cell with a coloured background is not
    /// blank). Spaces at the end of that span are shown as `·` so they can be
    /// seen; spaces before other text are left as spaces. Wide characters
    /// appear once (their spacer cell is skipped). The row ends with `⏎` if it
    /// soft-wraps onto the next row, otherwise `|`.
    #[doc(hidden)]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for line in 0..self.lines {
            let row = self.visible_row(line);
            let len = row.cells().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
            let mut text = String::new();
            for c in row.cells().take(len).filter(|c| !c.is_spacer()) {
                c.push_text(&mut text);
            }
            let kept = text.trim_end_matches(' ');
            out.push_str(kept);
            out.extend(std::iter::repeat_n('·', text.len() - kept.len()));
            out.push(if row.wrapped() { '⏎' } else { '|' });
            out.push('\n');
        }
        out.push_str(&format!(
            "cursor=({},{}) alt={} offset={}",
            self.cursor.row,
            self.cursor.col,
            self.alt_active,
            self.display_offset()
        ));
        out
    }
}

impl Terminal {
    /// Delete lines inside the scroll region; unlike output scrolling, these
    /// never go to scrollback.
    fn scroll_up_no_history(&mut self, n: usize) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        let n = n.min(bottom - top + 1);
        self.screen.make_contiguous()[top..=bottom].rotate_left(n);
        let blank = self.blank();
        for row in self.screen.range_mut(bottom + 1 - n..=bottom) {
            row.reset(self.cols, blank);
        }
        self.touch_rows(bottom + 1 - n..bottom + 1);
    }
}

/// A screen cell is a narrow, wide or spacer scalar, or a narrow or wide
/// cluster (no placeholders yet), its spare bits are zero, and its style
/// id is live with SE matching the style.
fn check_cell(cell: Cell, styles: &StyleTable) -> Result<(), String> {
    let bits = cell.bits();
    if bits & (Cell::PROTECTED | 0x1F << 27) != 0 || (bits & Cell::KIND != 0 && !cell.is_cluster()) {
        return Err(format!("cell {bits:#x} has a placeholder kind, protected or spare bits set"));
    }
    if !cell.is_cluster() && cell.codepoint() != 0 && char::from_u32(cell.codepoint()).is_none() {
        return Err(format!("cell {bits:#x} holds no scalar"));
    }
    // C0 controls are executed, never printed; history's text relies on
    // it (0x0A ends a stored line).
    if !cell.is_cluster() && (1..0x20).contains(&cell.codepoint()) {
        return Err(format!("cell {bits:#x} holds a C0 control"));
    }
    if cell.is_spacer() && (cell.codepoint() != 0 || cell.is_cluster()) {
        return Err(format!("spacer {bits:#x} holds a character"));
    }
    let id = cell.style_id();
    if !styles.is_live(id) {
        return Err(format!("style id {id} isn't live"));
    }
    if cell.shows_empty() != styles.get(id).shows_empty() {
        return Err(format!("SE bit of {bits:#x} doesn't match style {id}"));
    }
    Ok(())
}

/// A row's clusters (§17.3): every cluster cell's slot is live in the
/// row's store, each live slot is held by exactly one cell, the store is
/// consistent, and a row without a store has no cluster cells.
fn check_clusters(row: &Row) -> Result<(), String> {
    let mut slots: Vec<u32> = row.cells.iter().filter(|c| c.is_cluster()).map(|c| c.codepoint()).collect();
    let Some(store) = row.clusters() else {
        return match slots.first() {
            Some(s) => Err(format!("cluster cell (slot {s}) in a row without a store")),
            None => Ok(()),
        };
    };
    store.check()?;
    slots.sort_unstable();
    if let Some(w) = slots.windows(2).find(|w| w[0] == w[1]) {
        return Err(format!("slot {} held by two cells", w[0]));
    }
    let live: Vec<u32> = store.live_slots().collect();
    if slots != live {
        return Err(format!("cells hold slots {slots:?}, live slots are {live:?}"));
    }
    Ok(())
}

fn check_template(t: Cell, style: &Style, styles: &StyleTable) -> Result<(), String> {
    if t.style_part() != t {
        return Err(format!("{:#x} isn't a template", t.bits()));
    }
    check_cell(t, styles)?;
    if styles.get(t.style_id()) != style {
        return Err(format!("id {} is {:?}, not {style:?}", t.style_id(), styles.get(t.style_id())));
    }
    Ok(())
}

/// The parser plus the state it drives.
pub struct Term {
    parser: vte::Parser,
    pub state: Terminal,
}

impl Term {
    pub fn new(cols: usize, lines: usize, scrollback_limit: usize) -> Term {
        Term {
            parser: vte::Parser::new(),
            state: Terminal::new(cols, lines, scrollback_limit),
        }
    }

    /// Parse a chunk of program output. Escape sequences and UTF-8
    /// characters may be split across chunks anywhere: the parser carries
    /// its state from one call to the next.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.state, bytes);
        #[cfg(debug_assertions)]
        if let Err(e) = self.state.check_invariants() {
            panic!("terminal invariant broken: {e}");
        }
    }
}
