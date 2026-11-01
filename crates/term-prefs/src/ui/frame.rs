//! A cell grid drawn each frame and compared with the last one, so only
//! what changed is sent to the terminal.

use std::fmt::Write as _;
use std::rc::Rc;

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

/// `s` on two lines of `max` cells: broken at the last space that fits,
/// the second cut with `…` if it is still too long. One line (and an
/// empty second) when it fits.
pub fn wrap2(s: &str, max: usize) -> (String, String) {
    if str_width(s) <= max {
        return (s.to_string(), String::new());
    }
    let (mut w, mut cut, mut last_space) = (0, 0, None);
    for (i, c) in s.char_indices() {
        let cw = char_width(c).unwrap_or(0);
        if w + cw > max {
            break;
        }
        if c == ' ' {
            last_space = Some(i);
        }
        w += cw;
        cut = i + c.len_utf8();
    }
    // A space just after what fits is the best break.
    if s[cut..].starts_with(' ') {
        last_space = Some(cut);
    }
    let (first, rest) = match last_space {
        Some(i) if i > 0 => (&s[..i], s[i + 1..].trim_start()),
        _ => (&s[..cut], &s[cut..]),
    };
    (first.to_string(), truncate(rest, max))
}

/// A picture over some cells (the font list's sample), sent with the
/// kitty graphics protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// What it shows: a picture with the same key isn't sent again.
    pub key: u64,
    /// The cells it covers.
    pub x: usize,
    pub y: usize,
    pub cols: usize,
    pub rows: usize,
    /// Its pixels: `width` × `height`, 3 bytes (RGB) each.
    pub width: u32,
    pub height: u32,
    pub rgb: Rc<Vec<u8>>,
}

/// The image id the TUI's picture is sent under (one at a time).
const IMAGE_ID: u32 = 7771;

/// Base64 (standard alphabet, padded), for the kitty graphics payload.
fn base64(data: &[u8], out: &mut String) {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
}

impl Image {
    /// The kitty graphics commands that send it and place it at the
    /// cursor, scaled to its cells: raw RGB in 4096-byte chunks, the
    /// cursor left where it is, and no replies (`q=2`).
    fn send(&self, out: &mut String) {
        let mut payload = String::with_capacity(self.rgb.len() * 4 / 3 + 4);
        base64(&self.rgb, &mut payload);
        let chunks: Vec<&str> = payload
            .as_bytes()
            .chunks(4096)
            .map(|c| std::str::from_utf8(c).expect("base64 is ASCII"))
            .collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let more = u8::from(i + 1 < chunks.len());
            if i == 0 {
                let _ = write!(
                    out,
                    "\x1b_Ga=T,f=24,s={},v={},c={},r={},i={IMAGE_ID},C=1,q=2,m={more};{chunk}\x1b\\",
                    self.width, self.height, self.cols, self.rows
                );
            } else {
                let _ = write!(out, "\x1b_Gm={more},q=2;{chunk}\x1b\\");
            }
        }
    }
}

/// One frame: what the screen should show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    cells: Vec<Cell>,
    /// Where the terminal's real cursor goes, if shown.
    pub cursor: Option<(usize, usize)>,
    /// A picture over some of the cells.
    pub image: Option<Image>,
}

impl Frame {
    pub fn new(width: usize, height: usize, base: Style) -> Frame {
        Frame {
            width,
            height,
            cells: vec![Cell::blank(base); width * height],
            cursor: None,
            image: None,
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
    /// The picture on screen: its key and cells.
    shown: Option<(u64, usize, usize, usize, usize)>,
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
            // Clearing the screen also takes pictures off it.
            out.push_str("\x1b[0m\x1b[H\x1b[2J");
            self.shown = None;
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
        let want = frame.image.as_ref().map(|i| (i.key, i.x, i.y, i.cols, i.rows));
        if want != self.shown {
            if self.shown.is_some() {
                let _ = write!(out, "\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\");
            }
            if let Some(image) = &frame.image {
                let _ = write!(out, "\x1b[{};{}H", image.y + 1, image.x + 1);
                image.send(&mut out);
            }
            self.shown = want;
        }
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
    fn long_lines_wrap_onto_a_second() {
        assert_eq!(wrap2("short", 10), ("short".into(), String::new()));
        assert_eq!(
            wrap2("the font the text is drawn with", 12),
            ("the font the".into(), "text is dra…".into())
        );
        assert_eq!(wrap2("one two", 5), ("one".into(), "two".into()));
        // No space to break at: cut where it stops fitting.
        assert_eq!(wrap2("abcdefgh", 5), ("abcde".into(), "fgh".into()));
        // Wide characters count as two.
        assert_eq!(wrap2("字字字 x", 4), ("字字".into(), "字 x".into()));
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

    #[test]
    fn base64_pads_like_the_standard() {
        for (data, want) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
        ] {
            let mut out = String::new();
            base64(data, &mut out);
            assert_eq!(out, want);
        }
    }

    /// A picture is sent once, placed at its cells with the cursor left
    /// alone and no replies; it is deleted when it goes or changes, and
    /// sent again after the screen is cleared.
    #[test]
    fn pictures_are_sent_once_and_taken_off() {
        let mut r = Renderer::default();
        let mut f = Frame::new(10, 4, Style::default());
        f.image = Some(Image {
            key: 1,
            x: 2,
            y: 1,
            cols: 4,
            rows: 2,
            width: 2,
            height: 1,
            rgb: Rc::new(vec![0xff, 0, 0, 0, 0xff, 0]),
        });
        let first = r.render(&f);
        assert!(
            first.contains("\x1b[2;3H\x1b_Ga=T,f=24,s=2,v=1,c=4,r=2,i=7771,C=1,q=2,m=0;/wAAAP8A\x1b\\"),
            "{first:?}"
        );
        assert!(!first.contains("a=d"), "nothing to delete yet");
        let again = r.render(&f);
        assert!(!again.contains("\x1b_G"), "{again:?}");
        f.image.as_mut().unwrap().key = 2;
        let changed = r.render(&f);
        let del = changed.find("\x1b_Ga=d,d=I,i=7771,q=2\x1b\\").expect("the old one deleted");
        assert!(changed.find("a=T").is_some_and(|t| t > del), "{changed:?}");
        r.invalidate();
        let whole = r.render(&f);
        assert!(
            whole.contains("\x1b[2J") && whole.contains("a=T") && !whole.contains("a=d"),
            "{whole:?}"
        );
        f.image = None;
        assert!(r.render(&f).contains("a=d,d=I"));
        // A big one goes in 4096-byte chunks, the last with m=0.
        f.image = Some(Image {
            key: 3,
            x: 0,
            y: 0,
            cols: 10,
            rows: 1,
            width: 2000,
            height: 2,
            rgb: Rc::new(vec![7; 12000]),
        });
        let big = r.render(&f);
        assert_eq!(big.matches("\x1b_G").count(), 4, "16000 bytes of base64");
        assert!(big.contains("m=1;") && big.contains("\x1b_Gm=0,q=2;"), "{}", &big[..80]);
    }
}
