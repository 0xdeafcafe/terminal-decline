//! Terminal state: the visible screen, scrollback, cursor and modes, driven by
//! escape codes from the `vte` parser.

use std::borrow::Cow;
use std::collections::VecDeque;

use unicode_width::UnicodeWidthChar;
use vte::{Params, Perform};

use crate::cell::{Cell, Color, Row, flags};
use crate::scrollback::Scrollback;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    /// The style new characters are drawn with.
    pub pen: Cell,
    /// Set after writing the last column; the next character wraps first.
    pub pending_wrap: bool,
}

#[derive(Clone, Copy, Debug)]
struct SavedCursor {
    cursor: Cursor,
    origin: bool,
    line_drawing: bool,
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

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub anchor: Point,
    pub head: Point,
}

impl Selection {
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
    alt_active: bool,
    /// Lines that scrolled off the top (primary screen only). Its total
    /// count of lines ever pushed is the base for [`Point`] line numbers.
    scrollback: Scrollback,
    /// How many rows the view is scrolled back by (0 = following output).
    display_offset: usize,
    /// Buffer line number ([`Point::line`]) of the screen's first row: the
    /// count of rows that have scrolled off its top. On the primary screen
    /// the scrollback holds the lines just before it.
    top_line: u64,
    /// The same for the other screen.
    other_top_line: u64,

    pub cursor: Cursor,
    saved_primary: Option<SavedCursor>,
    saved_alt: Option<SavedCursor>,
    scroll_top: usize,
    scroll_bottom: usize,
    tabs: Vec<bool>,
    /// G0 and G1 character sets: true = DEC line drawing.
    charsets: [bool; 2],
    /// Which of G0/G1 is in use (SI/SO).
    active_charset: usize,
    line_drawing: bool,
    last_char: Option<char>,
    /// Kitty keyboard protocol flags, a stack per screen.
    kitty_flags: [Vec<u8>; 2],

    pub modes: Modes,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub title: Option<String>,
    pub title_changed: bool,
    pub bell: bool,
    pub selection: Option<Selection>,
    /// Text a program asked to put on the clipboard (OSC 52).
    pub clipboard: Option<String>,
    /// The program's working directory, if it reports it (OSC 7).
    pub cwd: Option<String>,
    /// Default colours as 0xRRGGBB, for answering colour queries.
    pub default_colors: [u32; 3],
    /// Cell size in pixels, for answering size queries.
    pub cell_pixels: (u16, u16),
    /// Bytes the terminal must send back to the program (replies to queries).
    pub replies: Vec<u8>,
    /// Bumped on any change that needs a redraw.
    pub generation: u64,
    /// Write printable ASCII runs in bulk ([`Terminal::write_ascii`]); off
    /// only in tests, as the reference the bulk path is compared against.
    bulk_ascii: bool,
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
            alt_active: false,
            scrollback: Scrollback::new(scrollback_limit),
            display_offset: 0,
            top_line: 0,
            other_top_line: 0,
            cursor: Cursor::default(),
            saved_primary: None,
            saved_alt: None,
            scroll_top: 0,
            scroll_bottom: lines - 1,
            tabs: default_tabs(cols),
            charsets: [false; 2],
            active_charset: 0,
            line_drawing: false,
            last_char: None,
            kitty_flags: [Vec::new(), Vec::new()],
            modes: Modes::default(),
            cursor_shape: CursorShape::Block,
            cursor_blink: false,
            title: None,
            title_changed: false,
            bell: false,
            selection: None,
            clipboard: None,
            cwd: None,
            default_colors: [0xd8dee9, 0x16181c, 0xd8dee9],
            cell_pixels: (8, 16),
            replies: Vec::new(),
            generation: 1,
            bulk_ascii: true,
        }
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

    pub fn display_offset(&self) -> usize {
        self.display_offset
    }

    /// Scroll the view by `delta` lines (positive = back into history).
    pub fn scroll_view(&mut self, delta: isize) {
        if self.alt_active {
            return;
        }
        let max = self.scrollback.len() as isize;
        let new = (self.display_offset as isize + delta).clamp(0, max) as usize;
        if new != self.display_offset {
            self.display_offset = new;
            self.touch();
        }
    }

    pub fn scroll_view_to_bottom(&mut self) {
        if self.display_offset != 0 {
            self.display_offset = 0;
            self.touch();
        }
    }

    /// The row shown at visible line `line`, taking the scroll position into
    /// account. Rows from scrollback may be shorter than `cols`.
    /// History lines are expanded from their compact form, so they come back
    /// owned; screen lines are borrowed.
    pub fn visible_row(&self, line: usize) -> Cow<'_, Row> {
        let sb = self.scrollback.len();
        let idx = sb - self.display_offset + line;
        if idx < sb {
            Cow::Owned(self.scrollback.get(idx).unwrap_or_default())
        } else {
            Cow::Borrowed(&self.screen[idx - sb])
        }
    }

    /// Bytes used by scrollback, for measuring.
    pub fn scrollback_bytes(&self) -> usize {
        self.scrollback.memory_bytes()
    }

    /// The buffer position of visible line `line`.
    pub fn visible_point(&self, line: usize, col: usize) -> Point {
        Point {
            line: self.top_line - self.display_offset as u64 + line as u64,
            col,
        }
    }

    /// The row at buffer line `line`, or None if it is not stored (any more).
    fn row_at(&self, line: u64) -> Option<Cow<'_, Row>> {
        if line >= self.top_line {
            return self.screen.get((line - self.top_line) as usize).map(Cow::Borrowed);
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
        self.scrollback.get(sb - back as usize).map(Cow::Owned)
    }

    pub fn selection_text(&self) -> Option<String> {
        let sel = self.selection?;
        let (a, b) = sel.ordered();
        let mut out = String::new();
        for line in a.line..=b.line {
            let Some(row) = self.row_at(line) else { continue };
            let start = if line == a.line { a.col } else { 0 };
            let end = if line == b.line { b.col + 1 } else { self.cols };
            let mut text = String::new();
            for col in start..end.min(row.cells.len().max(start)) {
                let cell = row.get(col);
                if !cell.has(flags::WIDE_SPACER) {
                    text.push(cell.ch());
                }
            }
            let wrapped = row.wrapped && line != b.line;
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
        // On the primary screen, rows pushed off the top go into scrollback so
        // the cursor stays visible.
        let removed = resize_screen(&mut self.screen, &mut self.cursor, cols, lines);
        self.top_line += removed.len() as u64;
        if self.alt_active {
            // The primary screen is hidden; its cursor is the saved one.
            let mut cursor = self.saved_primary.map(|s| s.cursor).unwrap_or_default();
            let mut other = std::mem::take(&mut self.other_screen);
            let removed = resize_screen(&mut other, &mut cursor, cols, lines);
            self.other_screen = other;
            self.other_top_line += removed.len() as u64;
            for row in removed {
                self.push_scrollback(&row);
            }
            if let Some(saved) = &mut self.saved_primary {
                saved.cursor.row = cursor.row;
                saved.cursor.col = cursor.col.min(cols - 1);
            }
        } else {
            for row in removed {
                self.push_scrollback(&row);
            }
            resize_screen(&mut self.other_screen, &mut Cursor::default(), cols, lines);
        }

        self.cols = cols;
        self.lines = lines;
        self.cursor.col = self.cursor.col.min(cols - 1);
        self.cursor.row = self.cursor.row.min(lines - 1);
        self.cursor.pending_wrap = false;
        self.scroll_top = 0;
        self.scroll_bottom = lines - 1;
        self.tabs = default_tabs(cols);
        self.display_offset = self.display_offset.min(self.scrollback.len());
        self.selection = None;
        self.touch();
    }

    #[inline]
    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn blank(&self) -> Cell {
        Cell::erased(self.cursor.pen.bg)
    }

    /// Copy a row into scrollback. The row itself is left for the caller to
    /// reuse as the new blank line. Callers also advance the screen's
    /// `top_line`, which counts rows whether or not they were stored.
    fn push_scrollback(&mut self, row: &Row) {
        if self.scrollback.limit() == 0 {
            return;
        }
        self.scrollback.push(row);
        if self.display_offset > 0 {
            // Keep the view still while output arrives underneath it.
            self.display_offset = (self.display_offset + 1).min(self.scrollback.len());
        }
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
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        let n = n.min(bottom - top + 1);
        self.screen.make_contiguous()[top..=bottom].rotate_right(n);
        let blank = self.blank();
        for row in self.screen.range_mut(top..top + n) {
            row.reset(self.cols, blank);
        }
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
        let cells = &mut self.screen[row].cells;
        let end = range.end.min(cells.len());
        let start = range.start.min(end);
        cells[start..end].fill(blank);
        // Don't leave half of a wide character behind.
        if start > 0 && cells[start - 1].has(flags::WIDE) {
            cells[start - 1] = blank;
        }
        if end < cells.len() && cells[end].has(flags::WIDE_SPACER) {
            cells[end] = blank;
        }
    }

    fn save_cursor(&mut self) {
        let saved = SavedCursor {
            cursor: self.cursor,
            origin: self.modes.origin,
            line_drawing: self.line_drawing,
        };
        if self.alt_active {
            self.saved_alt = Some(saved)
        } else {
            self.saved_primary = Some(saved)
        }
    }

    fn restore_cursor(&mut self) {
        let saved = if self.alt_active { self.saved_alt } else { self.saved_primary };
        match saved {
            Some(s) => {
                self.cursor = s.cursor;
                self.modes.origin = s.origin;
                self.line_drawing = s.line_drawing;
            }
            None => {
                self.cursor = Cursor::default();
                self.modes.origin = false;
            }
        }
        self.cursor.row = self.cursor.row.min(self.lines - 1);
        self.cursor.col = self.cursor.col.min(self.cols - 1);
        if self.modes.origin {
            self.cursor.row = self.cursor.row.clamp(self.scroll_top, self.scroll_bottom);
        }
    }

    /// After shifting cells sideways, blank any half of a wide character
    /// that lost its other half.
    fn repair_wide(&mut self, row: usize) {
        let blank = self.blank();
        let cells = &mut self.screen[row].cells;
        for i in 0..cells.len() {
            let c = cells[i];
            let orphan_lead = c.has(flags::WIDE) && !cells.get(i + 1).is_some_and(|n| n.has(flags::WIDE_SPACER));
            let orphan_spacer = c.has(flags::WIDE_SPACER) && (i == 0 || !cells[i - 1].has(flags::WIDE));
            if orphan_lead || orphan_spacer {
                cells[i] = blank;
            }
        }
    }

    fn set_alt_screen(&mut self, on: bool, clear: bool) {
        if on == self.alt_active {
            return;
        }
        std::mem::swap(&mut self.screen, &mut self.other_screen);
        std::mem::swap(&mut self.top_line, &mut self.other_top_line);
        if on {
            // Number the alternate screen's rows after the primary's, so a
            // point on one can't be taken for a point on the other.
            self.top_line = self.top_line.max(self.other_top_line + self.lines as u64);
        }
        self.alt_active = on;
        if on && clear {
            let blank = self.blank();
            for row in &mut self.screen {
                row.reset(self.cols, blank);
            }
        }
        self.display_offset = 0;
        self.selection = None;
    }

    fn reset(&mut self) {
        let mut fresh = Terminal::new(self.cols, self.lines, self.scrollback.limit());
        fresh.generation = self.generation.wrapping_add(1);
        fresh.replies = std::mem::take(&mut self.replies);
        fresh.default_colors = self.default_colors;
        fresh.cell_pixels = self.cell_pixels;
        fresh.bulk_ascii = self.bulk_ascii;
        // Like xterm, a full reset clears scrollback too.
        fresh.scrollback = std::mem::replace(&mut self.scrollback, Scrollback::new(0));
        fresh.scrollback.clear();
        // Keep line numbers counting up from the primary screen's.
        let primary_top = if self.alt_active { self.other_top_line } else { self.top_line };
        fresh.top_line = primary_top;
        fresh.other_top_line = primary_top;
        *self = fresh;
    }

    fn write_char(&mut self, c: char) {
        let c = if self.line_drawing { dec_line_drawing(c) } else { c };
        let width = if (' '..='~').contains(&c) {
            1
        } else {
            match c.width() {
                Some(w) => w,
                None => return,
            }
        };
        if width == 0 {
            if c == '\u{FE0F}' {
                self.widen_previous();
            }
            // Other combining marks need grapheme storage, which cells don't have yet.
            return;
        }
        if self.cursor.pending_wrap && self.modes.autowrap {
            self.screen[self.cursor.row].wrapped = true;
            self.cursor.col = 0;
            self.linefeed();
        }
        if width == 2 && self.cursor.col + 1 >= self.cols {
            if !self.modes.autowrap {
                return;
            }
            self.erase_cells(self.cursor.row, self.cursor.col..self.cols);
            self.screen[self.cursor.row].wrapped = true;
            self.cursor.col = 0;
            self.linefeed();
        }

        let (row, col) = (self.cursor.row, self.cursor.col);
        if self.modes.insert {
            let cells = &mut self.screen[row].cells;
            cells[col..].rotate_right(width.min(self.cols - col));
            self.repair_wide(row);
        }
        let pen = self.cursor.pen;
        let style = pen.flags() & flags::STYLE_MASK;
        let cells = &mut self.screen[row].cells;
        // Only overwriting part of a wide character needs cleanup.
        let touches_wide =
            cells[col].has(flags::WIDE | flags::WIDE_SPACER) || (width == 2 && cells[col + 1].has(flags::WIDE | flags::WIDE_SPACER));
        if touches_wide {
            self.erase_cells(row, col..col + width);
        }
        let cells = &mut self.screen[row].cells;
        cells[col] = Cell {
            packed: c as u32 | style | if width == 2 { flags::WIDE } else { 0 },
            fg: pen.fg,
            bg: pen.bg,
        };
        if width == 2 {
            cells[col + 1] = Cell {
                packed: style | flags::WIDE_SPACER,
                fg: pen.fg,
                bg: pen.bg,
            };
        }

        if col + width >= self.cols {
            self.cursor.col = self.cols - 1;
            self.cursor.pending_wrap = self.modes.autowrap;
        } else {
            self.cursor.col = col + width;
        }
        self.last_char = Some(c);
    }

    /// Write a run of printable ASCII (0x20..=0x7E): the same result as
    /// [`Terminal::write_char`] for each byte, but one row segment at a time:
    /// the pen is read once, wrapping is handled only at segment ends, and
    /// the cells are written in one pass. Segments that overwrite part of a
    /// wide character, and the modes that change what a character does
    /// (insert, DEC line drawing, autowrap off), go one character at a time.
    fn write_ascii(&mut self, bytes: &[u8]) {
        let Some(&last) = bytes.last() else { return };
        if self.modes.insert || self.line_drawing || !self.modes.autowrap {
            for &b in bytes {
                self.write_char(b as char);
            }
            return;
        }
        let pen = self.cursor.pen;
        let style = pen.flags() & flags::STYLE_MASK;
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
            let cells = &mut self.screen[row].cells[col..col + n];
            if cells.iter().fold(0, |a, c| a | c.packed) & (flags::WIDE | flags::WIDE_SPACER) != 0 {
                // Overwriting part of a wide character: write_char cleans up.
                for &b in segment {
                    self.write_char(b as char);
                }
                continue;
            }
            for (cell, &b) in cells.iter_mut().zip(segment) {
                *cell = Cell {
                    packed: b as u32 | style,
                    fg: pen.fg,
                    bg: pen.bg,
                };
            }
            if col + n == cols {
                self.cursor.col = cols - 1;
                self.cursor.pending_wrap = true;
            } else {
                self.cursor.col = col + n;
            }
        }
        self.last_char = Some(last as char);
    }

    /// U+FE0F after a narrow symbol asks for emoji presentation, which is two
    /// cells wide (what Ghostty, kitty and WezTerm do, and what `string-width`
    /// based TUIs expect).
    fn widen_previous(&mut self) {
        if self.cursor.pending_wrap || self.cursor.col == 0 {
            return;
        }
        let (row, col) = (self.cursor.row, self.cursor.col - 1);
        let cell = self.screen[row].cells[col];
        if cell.has(flags::WIDE | flags::WIDE_SPACER) || cell.codepoint() < 0x2000 {
            return;
        }
        // The spacer goes where the next cell is; if that is the left half of
        // a wide character, erase the whole of it rather than half.
        if self.screen[row].cells[col + 1].has(flags::WIDE) {
            self.erase_cells(row, col + 1..col + 2);
        }
        let cells = &mut self.screen[row].cells;
        cells[col].packed |= flags::WIDE;
        cells[col + 1] = Cell {
            packed: (cell.flags() & flags::STYLE_MASK) | flags::WIDE_SPACER,
            fg: cell.fg,
            bg: cell.bg,
        };
        if col + 2 >= self.cols {
            self.cursor.col = self.cols - 1;
            self.cursor.pending_wrap = self.modes.autowrap;
        } else {
            self.cursor.col = col + 2;
        }
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
            // Unicode core: VS16 widening is always on.
            2027 => 3,
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
        self.cursor.pen = Cell::BLANK;
        self.cursor.pending_wrap = false;
        self.scroll_top = 0;
        self.scroll_bottom = self.lines - 1;
        self.charsets = [false; 2];
        self.active_charset = 0;
        self.line_drawing = false;
        self.saved_primary = None;
        self.saved_alt = None;
    }

    fn reply_color(&mut self, osc: &[u8], rgb: u32) {
        let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
        let os = String::from_utf8_lossy(osc);
        let reply = format!("\x1b]{os};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x1b\\");
        self.replies.extend_from_slice(reply.as_bytes());
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
                    let blank = self.blank();
                    for row in &mut self.screen {
                        row.reset(self.cols, blank);
                    }
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
            _ => {}
        }
    }

    fn sgr(&mut self, params: &Params) {
        let pen = &mut self.cursor.pen;
        let mut iter = params.iter();
        if params.is_empty() {
            *pen = Cell::BLANK;
            return;
        }
        while let Some(p) = iter.next() {
            match p[0] {
                0 => *pen = Cell::BLANK,
                1 => pen.packed |= flags::BOLD,
                2 => pen.packed |= flags::DIM,
                3 => pen.packed |= flags::ITALIC,
                4 => {
                    pen.packed &= !(flags::UNDERLINE | flags::DOUBLE_UNDERLINE);
                    match p.get(1) {
                        Some(0) => {}
                        Some(2) => pen.packed |= flags::DOUBLE_UNDERLINE,
                        _ => pen.packed |= flags::UNDERLINE,
                    }
                }
                5 | 6 => pen.packed |= flags::BLINK,
                7 => pen.packed |= flags::INVERSE,
                8 => pen.packed |= flags::HIDDEN,
                9 => pen.packed |= flags::STRIKE,
                21 => pen.packed |= flags::DOUBLE_UNDERLINE,
                22 => pen.packed &= !(flags::BOLD | flags::DIM),
                23 => pen.packed &= !flags::ITALIC,
                24 => pen.packed &= !(flags::UNDERLINE | flags::DOUBLE_UNDERLINE),
                25 => pen.packed &= !flags::BLINK,
                27 => pen.packed &= !flags::INVERSE,
                28 => pen.packed &= !flags::HIDDEN,
                29 => pen.packed &= !flags::STRIKE,
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
                58 => {
                    // Underline colour: parsed so its arguments are skipped.
                    let _ = extended_color(p, &mut iter);
                }
                n @ 90..=97 => pen.fg = Color::indexed((n - 90 + 8) as u8),
                n @ 100..=107 => pen.bg = Color::indexed((n - 100 + 8) as u8),
                _ => {}
            }
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
        row.cells.resize(cols, Cell::BLANK);
        if let Some(last) = row.cells.last_mut()
            && last.has(flags::WIDE)
        {
            *last = Cell::BLANK;
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
        'f' => '°',
        'g' => '±',
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
        c => c,
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

    fn execute(&mut self, byte: u8) {
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
                self.line_drawing = self.charsets[self.active_charset];
            }
            _ => return,
        }
        self.touch();
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
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
            ('@', []) => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                let n = a1.min(self.cols - col);
                let blank = self.blank();
                let cells = &mut self.screen[row].cells;
                cells[col..].rotate_right(n);
                cells[col..col + n].fill(blank);
                self.repair_wide(row);
            }
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
                    }
                    3 => {
                        self.scrollback.clear();
                        self.display_offset = 0;
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
            ('P', []) => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                let n = a1.min(self.cols - col);
                let blank = self.blank();
                let cells = &mut self.screen[row].cells;
                cells[col..].rotate_left(n);
                let len = cells.len();
                cells[len - n..].fill(blank);
                self.repair_wide(row);
            }
            ('X', []) => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                self.erase_cells(row, col..col + a1);
            }
            ('S', []) => self.scroll_up(a1),
            ('T', []) => self.scroll_down(a1),
            ('b', []) => {
                if let Some(c) = self.last_char {
                    for _ in 0..a1.min(self.cols * self.lines) {
                        self.write_char(c);
                    }
                }
            }
            ('c', []) => self.replies.extend_from_slice(b"\x1b[?62;22c"),
            ('c', [b'>']) => self.replies.extend_from_slice(b"\x1b[>1;10;0c"),
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
                let reply = format!("\x1bP>|decline {}\x1b\\", env!("CARGO_PKG_VERSION"));
                self.replies.extend_from_slice(reply.as_bytes());
            }
            ('t', []) => {
                let (cw, ch) = self.cell_pixels;
                let reply = match arg(0) {
                    14 => format!("\x1b[4;{};{}t", self.lines * ch as usize, self.cols * cw as usize),
                    16 => format!("\x1b[6;{ch};{cw}t"),
                    18 => format!("\x1b[8;{};{}t", self.lines, self.cols),
                    _ => String::new(),
                };
                self.replies.extend_from_slice(reply.as_bytes());
            }
            _ => return,
        }
        self.touch();
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
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
                self.charsets[g] = b == b'0';
                self.line_drawing = self.charsets[self.active_charset];
            }
            ([b'#'], b'8') => {
                for row in &mut self.screen {
                    row.cells.fill(Cell {
                        packed: 'E' as u32,
                        ..Cell::BLANK
                    });
                }
            }
            _ => return,
        }
        self.touch();
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        match params {
            [code @ (b"10" | b"11" | b"12"), b"?"] => {
                let i = (code[1] - b'0') as usize;
                let rgb = self.default_colors[i];
                self.reply_color(code, rgb);
            }
            [b"52", _target, data] if *data != b"?" => {
                if let Some(bytes) = base64_decode(data) {
                    self.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
                }
            }
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
    /// Check the rules the rest of the code relies on: the cursor is on the
    /// screen; both screens have `lines` rows of exactly `cols` cells; every
    /// WIDE cell is followed by a WIDE_SPACER and every spacer follows a WIDE
    /// cell; the view is not scrolled back further than the scrollback; the
    /// scroll region is inside the screen. Costs one pass over the screens.
    /// Turn the bulk ASCII path off (or back on), for tests that compare it
    /// with writing one character at a time.
    #[doc(hidden)]
    pub fn set_bulk_ascii(&mut self, on: bool) {
        self.bulk_ascii = on;
    }

    #[doc(hidden)]
    pub fn check_invariants(&self) -> Result<(), String> {
        let (cols, lines) = (self.cols, self.lines);
        if self.cursor.row >= lines || self.cursor.col >= cols {
            return Err(format!("cursor ({}, {}) outside {cols}x{lines}", self.cursor.row, self.cursor.col));
        }
        for (name, screen) in [("screen", &self.screen), ("other screen", &self.other_screen)] {
            if screen.len() != lines {
                return Err(format!("{name} has {} rows, expected {lines}", screen.len()));
            }
            for (r, row) in screen.iter().enumerate() {
                if row.cells.len() != cols {
                    return Err(format!("{name} row {r} has {} cells, expected {cols}", row.cells.len()));
                }
                for (c, cell) in row.cells.iter().enumerate() {
                    if cell.has(flags::WIDE) && !row.cells.get(c + 1).is_some_and(|n| n.has(flags::WIDE_SPACER)) {
                        return Err(format!("{name} row {r} col {c}: wide cell without a spacer after it"));
                    }
                    if cell.has(flags::WIDE_SPACER) && (c == 0 || !row.cells[c - 1].has(flags::WIDE)) {
                        return Err(format!("{name} row {r} col {c}: spacer without a wide cell before it"));
                    }
                }
            }
        }
        if self.display_offset > self.scrollback.len() {
            return Err(format!(
                "display offset {} beyond scrollback length {}",
                self.display_offset,
                self.scrollback.len()
            ));
        }
        if self.scroll_top > self.scroll_bottom || self.scroll_bottom >= lines {
            return Err(format!(
                "scroll region {}..={} invalid for {lines} lines",
                self.scroll_top, self.scroll_bottom
            ));
        }
        let primary_top = if self.alt_active { self.other_top_line } else { self.top_line };
        if primary_top < self.scrollback.len() as u64 {
            return Err(format!(
                "primary screen starts at line {primary_top}, before its {} scrollback lines",
                self.scrollback.len()
            ));
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
            let len = row.cells.iter().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
            let text: String = row.cells[..len]
                .iter()
                .filter(|c| !c.has(flags::WIDE_SPACER))
                .map(|c| c.ch())
                .collect();
            let kept = text.trim_end_matches(' ');
            out.push_str(kept);
            out.extend(std::iter::repeat_n('·', text.len() - kept.len()));
            out.push(if row.wrapped { '⏎' } else { '|' });
            out.push('\n');
        }
        out.push_str(&format!(
            "cursor=({},{}) alt={} offset={}",
            self.cursor.row, self.cursor.col, self.alt_active, self.display_offset
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
    }
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
