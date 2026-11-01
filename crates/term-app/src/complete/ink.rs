//! The command line coloured as it is typed (docs/design/completion.md
//! §5): the line the shell reported, read into words ([`super::lex`]),
//! matched against the cells it is drawn in, as colours for the
//! renderer to paint the default-coloured text with ([`Ink`]). The cells
//! themselves are never changed.

use term_core::{Content, Ink, Terminal, ViewTop, Width};

use super::lex::{self, Kind};

/// The ANSI colour a kind of stretch is drawn in, or `None` to leave it.
/// Commands are coloured by whether they exist ([`line_ink`]'s `known`).
pub fn color_of(kind: Kind) -> Option<u8> {
    match kind {
        Kind::Keyword => Some(2),
        Kind::Flag => Some(4),
        Kind::String => Some(3),
        Kind::Variable => Some(6),
        Kind::Operator | Kind::Redirect => Some(5),
        // Bright black: grey in most themes.
        Kind::Comment => Some(8),
        Kind::Command | Kind::Argument | Kind::Assignment => None,
    }
}

/// A character of the line and the cells it is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    /// Its byte offset in the line.
    byte: usize,
    row: u16,
    col: u16,
    cells: u16,
}

/// Put the command line's colours in `out`. `known` says whether a
/// command runs something (`None` while that isn't known yet): green if
/// it does, red if it doesn't, plain until then. Nothing unless the shell
/// reported the line, the view is at the bottom, and the cells from the
/// prompt's B mark show the line. Only the characters up to the first
/// cell that differs are coloured, so a report that arrived before the
/// shell's redraw colours what it shares with the screen and no more.
pub fn line_ink(t: &Terminal, known: &dyn Fn(&str) -> Option<bool>, out: &mut Vec<Ink>) {
    let Some(report) = t.line_report() else { return };
    if report.too_long || report.text.is_empty() || t.view_top() != ViewTop::Bottom {
        return;
    }
    let top = t.point_at(0, 0);
    if top.col != 0 {
        return;
    }
    let Some(row) = report.at.line.checked_sub(top.line).map(|r| r as usize) else {
        return;
    };
    if row >= t.lines() {
        return;
    }
    let placed = place(t, &report.text, row, report.at.col);
    if placed.is_empty() {
        return;
    }
    let line = lex::read(&report.text, report.cursor);
    // Only the commands on the screen are looked up.
    let shown = placed
        .last()
        .map_or(0, |p| p.byte + report.text[p.byte..].chars().next().map_or(1, char::len_utf8));
    let command_color = |at: usize| {
        let w = line
            .words
            .iter()
            .find(|w| w.role == lex::Role::Command && w.range.contains(&at) && w.range.end <= shown && !w.expands)?;
        Some(if known(&w.text)? { 2 } else { 1 })
    };
    let mut p = 0;
    for span in &line.spans {
        let color = match span.kind {
            Kind::Command => command_color(span.range.start),
            kind => color_of(kind),
        };
        let Some(color) = color else { continue };
        while p < placed.len() && placed[p].byte < span.range.start {
            p += 1;
        }
        while p < placed.len() && placed[p].byte < span.range.end {
            let c = placed[p];
            let cols = c.col..c.col + c.cells;
            match out.last_mut() {
                Some(last) if last.row == c.row && last.cols.end == cols.start && last.color == color => last.cols.end = cols.end,
                _ => out.push(Ink { row: c.row, cols, color }),
            }
            p += 1;
        }
    }
}

/// Where each character of `text` is drawn, from view row `row`, column
/// `col`, as the shell's line editor draws it (a control character as
/// `^X`, a newline starting the next row), up to the first cell that
/// doesn't match.
fn place(t: &Terminal, text: &str, mut row: usize, mut col: usize) -> Vec<Placed> {
    let (rows, cols) = (t.lines(), t.cols());
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let Some(ch) = text[i..].chars().next() else { break };
        if ch == '\n' {
            row += 1;
            col = 0;
            i += 1;
            continue;
        }
        // Each character the editor draws for this one: `^` and a letter
        // for a control character.
        let shown: [Option<char>; 2] = if ch.is_ascii_control() {
            [Some('^'), Some(((ch as u8) ^ 0x40) as char)]
        } else {
            [Some(ch), None]
        };
        let mut len = ch.len_utf8();
        for want in shown.into_iter().flatten() {
            // Past the row's end, or before a wide character that moved
            // down: on to the next row, if this one wraps onto it.
            loop {
                if row >= rows {
                    return out;
                }
                let r = t.visible_row(row);
                if col < cols && r.cell(col).width() != Width::EdgeSpacer {
                    break;
                }
                if !r.wrapped() {
                    return out;
                }
                row += 1;
                col = 0;
            }
            let r = t.visible_row(row);
            let cell = r.cell(col);
            let cells = if cell.width() == Width::Wide { 2 } else { 1 };
            match cell.content() {
                Content::Char(c) if c == want => {}
                // A cluster: the characters it is made of, all in this cell.
                Content::Cluster(s) if want == ch && text[i..].starts_with(s) => len = s.len(),
                _ => return out,
            }
            out.push(Placed {
                byte: i,
                row: row as u16,
                col: col as u16,
                cells,
            });
            col += cells as usize;
        }
        i += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::{ImagePool, Term};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    /// A terminal `cols` wide with a prompt (`$ `) and `screen` typed after
    /// it, then the shell's report of `line` with the cursor at its end.
    fn term(cols: usize, screen: &str, line: &str) -> Term {
        let mut t = Term::new(cols, 5, 100, ImagePool::process());
        t.state.set_report_token(TOKEN.as_bytes().try_into().ok());
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        t.advance(screen.as_bytes());
        let enc = line.replace('%', "%25").replace(';', "%3B").replace('\t', "%09");
        t.advance(format!("\x1b]7701;line;{TOKEN};main;{};{enc}\x07", line.len()).as_bytes());
        t
    }

    fn ink(t: &Term) -> Vec<(u16, std::ops::Range<u16>, u8)> {
        ink_knowing(t, &|_| None)
    }

    fn ink_knowing(t: &Term, known: &dyn Fn(&str) -> Option<bool>) -> Vec<(u16, std::ops::Range<u16>, u8)> {
        let mut out = Vec::new();
        line_ink(&t.state, known, &mut out);
        out.into_iter().map(|i| (i.row, i.cols, i.color)).collect()
    }

    #[test]
    fn colours_by_kind_in_the_cells_after_the_prompt() {
        let line = "ls -l \"a b\" | wc $X";
        let t = term(80, line, line);
        // `$ ` takes columns 0 and 1.
        assert_eq!(
            ink(&t),
            [
                (0, 5..7, 4),   // -l
                (0, 8..13, 3),  // "a b"
                (0, 14..15, 5), // |
                (0, 19..21, 6), // $X
            ]
        );
    }

    #[test]
    fn commands_green_when_they_run_something_red_when_not() {
        let line = "ls | nope && \"ls\"";
        let t = term(80, line, line);
        let known = |name: &str| match name {
            "ls" => Some(true),
            "nope" => Some(false),
            _ => None,
        };
        assert_eq!(
            ink_knowing(&t, &known),
            [
                (0, 2..4, 2),   // ls
                (0, 5..6, 5),   // |
                (0, 7..11, 1),  // nope
                (0, 12..14, 5), // &&
                (0, 15..19, 3), // "ls": quoted, the string's colour
            ]
        );
        // Typed past what the screen shows: not looked up.
        let t = term(80, "nop", "nope");
        assert_eq!(ink_knowing(&t, &known), []);
    }

    #[test]
    fn a_wrapped_line_and_wide_characters() {
        // 10 columns: `$ echo -n` fills row 0 up to column 9, `x` wraps.
        let line = "echo -nn 漢字 'q'";
        let t = term(10, line, line);
        let got = ink(&t);
        assert!(got.contains(&(0, 7..10, 4)), "{got:?}");
        // 漢字 are wide and plain; 'q' after them on row 1, three cells.
        let q = got.iter().find(|i| i.2 == 3).expect("the string");
        assert_eq!(q.0, 1);
        assert_eq!(q.1.end - q.1.start, 3);
    }

    #[test]
    fn only_what_the_screen_shows() {
        // The report came before the redraw: the screen shows `ls -` only.
        let t = term(80, "ls -", "ls -la | wc");
        assert_eq!(ink(&t), [(0, 5..6, 4)]);
        // Something else entirely on the screen: nothing.
        let t = term(80, "pwd", "ls -l");
        assert_eq!(ink(&t), []);
    }

    #[test]
    fn control_characters_shown_as_caret_letters() {
        let t = term(80, "a ^I \"x\"", "a \t \"x\"");
        assert_eq!(ink(&t), [(0, 7..10, 3)]);
    }

    #[test]
    fn nothing_without_a_report_or_scrolled_back() {
        let mut t = Term::new(80, 5, 100, ImagePool::process());
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07ls -l");
        assert_eq!(ink(&t), []);
        let mut t = term(80, "ls -l", "ls -l");
        for _ in 0..10 {
            t.advance(b"\r\n");
        }
        // The prompt scrolled into history: its line is no longer at the
        // B mark's row on the screen.
        assert_eq!(ink(&t), []);
    }
}
