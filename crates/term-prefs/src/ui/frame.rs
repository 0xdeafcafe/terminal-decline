//! A cell grid drawn each frame and compared with the last one, so only
//! what changed is sent to the terminal.

use std::fmt::Write as _;

use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
    /// The terminal's default foreground or background.
    #[default]
    Default,
    /// One of the 16 ANSI colours.
    Ansi(u8),
    Rgb(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Style {
    pub fn fg(self, fg: Color) -> Style {
        Style { fg, ..self }
    }
    pub fn bg(self, bg: Color) -> Style {
        Style { bg, ..self }
    }
    pub fn bold(self) -> Style {
        Style { bold: true, ..self }
    }
    pub fn reverse(self) -> Style {
        Style { reverse: true, ..self }
    }
    pub fn underline(self) -> Style {
        Style { underline: true, ..self }
    }
    pub fn dim(self) -> Style {
        Style { dim: true, ..self }
    }

    fn sgr(&self, out: &mut String) {
        out.push_str("\x1b[0");
        for (on, code) in [(self.bold, ";1"), (self.dim, ";2"), (self.underline, ";4"), (self.reverse, ";7")] {
            if on {
                out.push_str(code);
            }
        }
        let colour = |out: &mut String, c: Color, base: u8| match c {
            Color::Default => {}
            Color::Ansi(n) if n < 8 => {
                let _ = write!(out, ";{}", base + n);
            }
            Color::Ansi(n) => {
                let _ = write!(out, ";{}", base + 60 + (n & 7));
            }
            Color::Rgb(rgb) => {
                let _ = write!(out, ";{};2;{};{};{}", base + 8, rgb >> 16 & 0xff, rgb >> 8 & 0xff, rgb & 0xff);
            }
        };
        colour(out, self.fg, 30);
        colour(out, self.bg, 40);
        out.push('m');
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    ch: char,
    /// 1 or 2; 0 for the right half of a wide character.
    width: u8,
    style: Style,
}

impl Cell {
    fn blank(style: Style) -> Cell {
        Cell { ch: ' ', width: 1, style }
    }
}

/// The width of `c` in cells, as the terminal counts it (term-core's table
/// equals `unicode-width`'s), ambiguous characters as 1. `None` for
/// controls.
pub fn char_width(c: char) -> Option<usize> {
    c.width()
}

/// The width of `s` in cells.
pub fn str_width(s: &str) -> usize {
    s.chars().filter_map(char_width).sum()
}

/// `s` cut to at most `max` cells, with `…` if anything was cut.
pub fn truncate(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = char_width(c).unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    if max > 0 {
        out.push('…');
    }
    out
}

/// One frame: what the screen should show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    cells: Vec<Cell>,
    /// Where the terminal's real cursor goes, if shown.
    pub cursor: Option<(usize, usize)>,
}

impl Frame {
    pub fn new(width: usize, height: usize, base: Style) -> Frame {
        Frame {
            width,
            height,
            cells: vec![Cell::blank(base); width * height],
            cursor: None,
        }
    }

    /// Fills a rectangle with blanks of `style`.
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, style: Style) {
        for row in y..(y + h).min(self.height) {
            for col in x..(x + w).min(self.width) {
                self.set(col, row, Cell::blank(style));
            }
        }
    }

    fn set(&mut self, x: usize, y: usize, cell: Cell) {
        // Never leave half a wide character behind.
        let i = y * self.width + x;
        if self.cells[i].width == 0 && x > 0 {
            let s = self.cells[i - 1].style;
            self.cells[i - 1] = Cell::blank(s);
        }
        if self.cells[i].width == 2 && x + 1 < self.width {
            let s = self.cells[i + 1].style;
            self.cells[i + 1] = Cell::blank(s);
        }
        self.cells[i] = cell;
    }

    /// Writes `s` from `(x, y)`, stopping before column `end`. Returns the
    /// column after the text.
    pub fn text(&mut self, x: usize, y: usize, end: usize, s: &str, style: Style) -> usize {
        if y >= self.height {
            return x;
        }
        let end = end.min(self.width);
        let mut col = x;
        for c in s.chars() {
            let Some(w) = char_width(c) else { continue };
            if w == 0 {
                continue;
            }
            if col + w > end {
                break;
            }
            self.set(
                col,
                y,
                Cell {
                    ch: c,
                    width: w as u8,
                    style,
                },
            );
            if w == 2 {
                self.set(col + 1, y, Cell { ch: ' ', width: 0, style });
            }
            col += w;
        }
        col
    }

    /// A box with rounded corners and a title in its top edge.
    #[allow(clippy::too_many_arguments)]
    pub fn boxed(&mut self, x: usize, y: usize, w: usize, h: usize, title: &str, style: Style, title_style: Style) {
        if w < 2 || h < 2 || x + w > self.width || y + h > self.height {
            return;
        }
        let right = x + w - 1;
        let bottom = y + h - 1;
        self.text(x, y, x + 1, "╭", style);
        self.text(right, y, right + 1, "╮", style);
        self.text(x, bottom, x + 1, "╰", style);
        self.text(right, bottom, right + 1, "╯", style);
        for col in x + 1..right {
            self.text(col, y, col + 1, "─", style);
            self.text(col, bottom, col + 1, "─", style);
        }
        for row in y + 1..bottom {
            self.text(x, row, x + 1, "│", style);
            self.text(right, row, right + 1, "│", style);
        }
        if !title.is_empty() && w > 4 {
            let t = truncate(title, w - 4);
            let end = self.text(x + 2, y, right, &t, title_style);
            if end < right {
                self.text(end, y, end + 1, " ", style);
            }
            self.text(x + 1, y, x + 2, " ", style);
        }
    }

    /// A horizontal rule from `x` to `end`.
    pub fn rule(&mut self, x: usize, y: usize, end: usize, style: Style) {
        for col in x..end.min(self.width) {
            self.text(col, y, col + 1, "─", style);
        }
    }

    /// The text of row `y`, wide characters once, for tests.
    pub fn row_text(&self, y: usize) -> String {
        self.cells[y * self.width..(y + 1) * self.width]
            .iter()
            .filter(|c| c.width != 0)
            .map(|c| c.ch)
            .collect()
    }

    /// Whether the cell at `(x, y)` is drawn in reverse video, for tests.
    pub fn is_reverse(&self, x: usize, y: usize) -> bool {
        self.cells[y * self.width + x].style.reverse
    }
}

/// Sends frames to the terminal, each as the changes from the last.
#[derive(Default)]
pub struct Renderer {
    last: Option<Frame>,
}

impl Renderer {
    /// Forgets the last frame, so the next is drawn whole (Ctrl+L, a
    /// resize, back from `$EDITOR`).
    pub fn invalidate(&mut self) {
        self.last = None;
    }

    /// The bytes that turn the last frame into `frame`.
    pub fn render(&mut self, frame: &Frame) -> String {
        let mut out = String::new();
        let whole = self
            .last
            .as_ref()
            .is_none_or(|l| l.width != frame.width || l.height != frame.height);
        if whole {
            out.push_str("\x1b[0m\x1b[H\x1b[2J");
        }
        let mut pen: Option<Style> = None;
        for y in 0..frame.height {
            let row = &frame.cells[y * frame.width..(y + 1) * frame.width];
            let old = (!whole).then(|| self.last.as_ref().expect("checked").row(y));
            let mut x = 0;
            let mut at: Option<usize> = None;
            while x < frame.width {
                let cell = row[x];
                let span = if cell.width == 2 { 2 } else { 1 };
                let same = old.is_some_and(|o| o[x..(x + span).min(frame.width)] == row[x..(x + span).min(frame.width)]);
                if same || cell.width == 0 {
                    x += span.max(1);
                    continue;
                }
                if whole && cell == Cell::blank(Style::default()) {
                    // A cleared screen is already blank in the default style.
                    x += 1;
                    continue;
                }
                if at != Some(x) {
                    let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
                }
                if pen != Some(cell.style) {
                    cell.style.sgr(&mut out);
                    pen = Some(cell.style);
                }
                out.push(cell.ch);
                x += span;
                // After the last column the cursor doesn't move on.
                at = (x < frame.width).then_some(x);
            }
        }
        out.push_str("\x1b[0m");
        match frame.cursor {
            Some((x, y)) => {
                let _ = write!(out, "\x1b[{};{}H\x1b[?25h", y + 1, x + 1);
            }
            None => out.push_str("\x1b[?25l"),
        }
        self.last = Some(frame.clone());
        out
    }
}

impl Frame {
    fn row(&self, y: usize) -> &[Cell] {
        &self.cells[y * self.width..(y + 1) * self.width]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_frames_send_only_the_cursor() {
        let mut r = Renderer::default();
        let mut f = Frame::new(10, 2, Style::default());
        f.text(0, 0, 10, "hello", Style::default());
        f.cursor = Some((1, 0));
        let first = r.render(&f);
        assert!(first.contains("hello"));
        let second = r.render(&f);
        assert_eq!(second, "\x1b[0m\x1b[1;2H\x1b[?25h");
        f.text(0, 1, 10, "x", Style::default().bold());
        let third = r.render(&f);
        assert!(third.contains("\x1b[2;1H\x1b[0;1mx"), "{third:?}");
    }

    #[test]
    fn wide_characters_are_never_split() {
        let mut f = Frame::new(4, 1, Style::default());
        assert_eq!(f.text(0, 0, 4, "日本語", Style::default()), 4);
        assert_eq!(f.row_text(0), "日本");
        f.text(1, 0, 2, "x", Style::default());
        assert_eq!(f.row_text(0), " x本");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }
}
