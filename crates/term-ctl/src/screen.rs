//! What `screen` reads: the rows as text, and with `--json` each cell's
//! text, resolved colours and attributes, the cursor, title, alternate
//! screen and modes. The text of a row is as the test control's dump
//! writes it ([`term_core::Terminal::dump`]), without its markers.

use serde_json::{Value, json};
use term_core::{Attrs, ColorKind, CursorShape, DynamicColor, MouseMode, RowRef, Terminal, Underline, Width};

/// One row's text: its cells up to the last one that isn't blank, wide
/// characters once, spaces at the end dropped.
pub fn row_text(row: &RowRef) -> String {
    let len = row.cells().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
    let mut text = String::new();
    for c in row.cells().take(len).filter(|c| !c.is_spacer()) {
        c.push_text(&mut text);
    }
    text.truncate(text.trim_end_matches(' ').len());
    text
}

/// Call `f` with up to `scrollback` history rows (oldest first), then
/// every screen row. The view is put back where it was.
pub fn each_row(t: &mut Terminal, scrollback: usize, mut f: impl FnMut(&RowRef, bool)) {
    let top = t.view_top();
    t.scroll_view_to_bottom();
    if scrollback > 0 {
        t.scroll_view(scrollback.min(isize::MAX as usize) as isize);
        let mut back = t.display_offset();
        while back > 0 {
            t.scroll_view_to_bottom();
            t.scroll_view(back as isize);
            let n = back.min(t.lines());
            for line in 0..n {
                f(&t.visible_row(line), true);
            }
            back -= n;
        }
        t.scroll_view_to_bottom();
    }
    for line in 0..t.lines() {
        f(&t.visible_row(line), false);
    }
    t.set_view_top(top);
}

/// The screen's rows as text (history first, `scrollback` at most).
pub fn lines(t: &mut Terminal, scrollback: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(t.lines());
    each_row(t, scrollback, |row, _| out.push(row_text(row)));
    out
}

fn hex(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xFF_FFFF)
}

fn shape(s: CursorShape) -> &'static str {
    match s {
        CursorShape::Block => "block",
        CursorShape::Underline => "underline",
        CursorShape::Bar => "bar",
    }
}

fn mouse(m: MouseMode) -> &'static str {
    match m {
        MouseMode::Off => "off",
        MouseMode::Click => "click",
        MouseMode::Drag => "drag",
        MouseMode::Motion => "motion",
        #[allow(unreachable_patterns)]
        _ => "other",
    }
}

/// The whole reply for `screen`: `cells` only with `json`.
pub fn screen(t: &mut Terminal, scrollback: usize, json: bool) -> Value {
    let m = *t.modes();
    let cursor = t.cursor();
    let (row, col) = (cursor.row(), cursor.col());
    let mut lines_out = Vec::new();
    let mut wrapped = Vec::new();
    let mut cells = Vec::new();
    let mut history = 0;
    let palette = *t.palette();
    let fg = t.dynamic_color(DynamicColor::Foreground);
    let bg = t.dynamic_color(DynamicColor::Background);
    let resolve = |kind: ColorKind, default: u32| match kind {
        ColorKind::Default => default,
        ColorKind::Indexed(i) => palette[i as usize],
        ColorKind::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
    };
    each_row(t, scrollback, |r, from_history| {
        history += from_history as usize;
        lines_out.push(row_text(r));
        wrapped.push(r.wrapped());
        if !json {
            return;
        }
        let mut row_cells = Vec::with_capacity(r.len());
        for (x, c) in r.cells().enumerate() {
            if c.is_spacer() {
                continue;
            }
            let s = c.style();
            let (mut f, mut b) = (resolve(s.fg.kind(), fg), resolve(s.bg.kind(), bg));
            if s.attrs.contains(Attrs::INVERSE) {
                std::mem::swap(&mut f, &mut b);
            }
            let mut text = String::new();
            c.push_text(&mut text);
            let mut cell = json!({ "x": x, "t": text, "fg": hex(f), "bg": hex(b) });
            let o = cell.as_object_mut().expect("an object");
            if c.width() == Width::Wide {
                o.insert("wide".into(), true.into());
            }
            for (name, a) in [
                ("bold", Attrs::BOLD),
                ("dim", Attrs::DIM),
                ("italic", Attrs::ITALIC),
                ("inverse", Attrs::INVERSE),
                ("hidden", Attrs::HIDDEN),
                ("strike", Attrs::STRIKE),
                ("overline", Attrs::OVERLINE),
                ("blink", Attrs::BLINK),
            ] {
                if s.attrs.contains(a) {
                    o.insert(name.into(), true.into());
                }
            }
            let underline = match s.underline {
                Underline::None => None,
                Underline::Single => Some("single"),
                Underline::Double => Some("double"),
                Underline::Curly => Some("curly"),
                Underline::Dotted => Some("dotted"),
                Underline::Dashed => Some("dashed"),
            };
            if let Some(u) = underline {
                o.insert("underline".into(), u.into());
                if s.underline_color.kind() != ColorKind::Default {
                    o.insert("underline_color".into(), hex(resolve(s.underline_color.kind(), f)).into());
                }
            }
            if let Some(link) = r.link(x) {
                o.insert("link".into(), link.uri.clone().into());
            }
            row_cells.push(cell);
        }
        cells.push(Value::Array(row_cells));
    });
    let mut out = json!({
        "ok": true,
        "cols": t.cols(),
        "rows": t.lines(),
        "title": t.title(),
        "cwd": t.cwd(),
        "alt_screen": t.alt_screen_active(),
        "cursor": {
            "row": row,
            "col": col,
            "visible": m.cursor_visible,
            "shape": shape(t.cursor_shape()),
            "blink": t.cursor_blink(),
        },
        "modes": {
            "app_cursor": m.app_cursor,
            "app_keypad": m.app_keypad,
            "bracketed_paste": m.bracketed_paste,
            "focus_events": m.focus_events,
            "mouse": mouse(m.mouse),
            "mouse_sgr": m.mouse_sgr,
            "synchronized": m.synchronized,
            "kitty_keyboard": t.kitty_keyboard_flags(),
        },
        "colors": { "foreground": hex(fg), "background": hex(bg) },
        "history_lines": history,
        "lines": lines_out,
        "wrapped": wrapped,
    });
    if json {
        out["cells"] = Value::Array(cells);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::Term;

    #[test]
    fn reads_text_cells_and_history() {
        let mut t = Term::new(10, 3, 100, term_core::ImagePool::process());
        t.state.set_default_colors([0xffffff, 0x000000, 0xffffff]);
        t.advance(b"one\r\ntwo\r\nthree\r\nfour\r\n\x1b[1;31mred\x1b[0m \x1b[7mi\x1b[0m\x1b]0;hello\x07\x1b[?2004h");
        let mut st = t.state;
        assert_eq!(lines(&mut st, 0), ["three", "four", "red i"]);
        assert_eq!(lines(&mut st, 10), ["one", "two", "three", "four", "red i"]);
        let v = screen(&mut st, 1, true);
        assert_eq!(v["lines"], json!(["two", "three", "four", "red i"]));
        assert_eq!(v["history_lines"], 1);
        assert_eq!(v["title"], "hello");
        assert_eq!(v["modes"]["bracketed_paste"], true);
        assert_eq!(v["cursor"]["row"], 2);
        assert_eq!(v["cursor"]["col"], 5);
        let red = &v["cells"][3][0];
        assert_eq!(red["t"], "r");
        assert_eq!(red["bold"], true);
        assert_eq!(red["fg"], hex(st.palette()[1]));
        let inverse = &v["cells"][3][4];
        assert_eq!((inverse["fg"].as_str(), inverse["bg"].as_str()), (Some("#000000"), Some("#ffffff")));
        // The view is where it was.
        assert_eq!(st.display_offset(), 0);
        assert!(screen(&mut st, 0, false).get("cells").is_none());
    }
}
