//! Shell integration (roadmap E.5): what the prompt marks a shell sends
//! (OSC 133, from the snippets in `shell/`) are used for. Cmd+Up and
//! Cmd+Down scroll the view to the prompt above or below its top line,
//! and Select Command Output selects a command's output.

use term_core::{Point, Terminal};

/// Scroll the view so the nearest prompt above its top line (`up`), or
/// below it, is at the top, as near as history allows (a prompt on the
/// screen below the top can only bring the view to the bottom). False if
/// the view didn't move: no prompt that way.
pub fn jump(t: &mut Terminal, up: bool) -> bool {
    let top = t.point_at(0, 0).line;
    let target = if up { t.previous_prompt(top) } else { t.next_prompt(top) };
    let Some(prompt) = target else { return false };
    let before = t.display_offset();
    // The screen's first line is `top + before`; the view shows the
    // prompt's line at its top when scrolled back by the lines between.
    let screen_top = top + before as u64;
    let want = screen_top.saturating_sub(prompt.line) as isize;
    t.scroll_view(want - before as isize);
    t.display_offset() != before
}

/// Select a command's output: with the view scrolled back, the command
/// at its top line (where a jump leaves its prompt); at the bottom, the
/// one running, else the last one with output. False if there is none.
pub fn select_output(t: &mut Terminal) -> bool {
    if t.display_offset() > 0 {
        return t.select_output(t.point_at(0, 0));
    }
    let Some((row, col)) = t.cursor_in_view() else { return false };
    let cursor = t.point_at(row, col);
    if t.select_output(cursor) {
        return true;
    }
    // The cursor is at a new prompt: the command before it. Further back
    // only if that one printed nothing.
    let mut line = cursor.line;
    while let Some(prompt) = t.previous_prompt(line) {
        if t.select_output(prompt) {
            return true;
        }
        line = prompt.line;
    }
    false
}

/// The output of the command at `p`, for the right-click menu: whether
/// there is one to select.
pub fn has_output(t: &Terminal, p: Point) -> bool {
    t.command_output(p).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::Term;

    /// A prompt, a command and its output as the snippets mark them.
    fn session(t: &mut Term, cmd: &str, output: &str) {
        t.advance(format!("\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07{output}\x1b]133;D;0\x07").as_bytes());
    }

    fn top_text(t: &Term) -> String {
        let dump = t.state.dump();
        dump.lines().next().unwrap_or_default().trim_end_matches(['|', '⏎']).to_string()
    }

    #[test]
    fn jumps_put_prompts_at_the_top() {
        let mut t = Term::new(20, 4, 100);
        session(&mut t, "one", "1\r\n1\r\n1\r\n");
        session(&mut t, "two", "2\r\n2\r\n2\r\n");
        session(&mut t, "three", "3\r\n");
        t.advance(b"\x1b]133;A\x07$ ");
        assert_eq!(t.state.display_offset(), 0);
        assert!(jump(&mut t.state, true));
        assert_eq!(top_text(&t), "$ two");
        assert!(jump(&mut t.state, true));
        assert_eq!(top_text(&t), "$ one");
        assert!(!jump(&mut t.state, true), "no prompt above the first");
        assert!(jump(&mut t.state, false));
        assert_eq!(top_text(&t), "$ two");
        // The next prompt (`three`) is on the screen, below its top: the
        // view goes to the bottom.
        assert!(jump(&mut t.state, false));
        assert_eq!(t.state.display_offset(), 0);
        assert!(!jump(&mut t.state, false), "nothing below the bottom");
        // No marks: nowhere to go.
        let mut plain = Term::new(20, 4, 100);
        plain.advance(b"a\r\nb\r\nc\r\nd\r\ne\r\nf");
        assert!(!jump(&mut plain.state, true));
    }

    #[test]
    fn selects_the_output_at_the_top_or_the_last() {
        let mut t = Term::new(20, 4, 100);
        session(&mut t, "one", "1\r\n11\r\n");
        session(&mut t, "two", "2\r\n");
        session(&mut t, "quiet", "");
        t.advance(b"\x1b]133;A\x07$ ");
        // At the bottom: the last command with output (`quiet` had none).
        assert!(select_output(&mut t.state));
        assert_eq!(t.state.selection_text().as_deref(), Some("2"));
        // After jumping to `one`: its output.
        while top_text(&t) != "$ one" {
            assert!(jump(&mut t.state, true));
        }
        assert!(select_output(&mut t.state));
        assert_eq!(t.state.selection_text().as_deref(), Some("1\n11"));
        // A command still running: its output so far.
        let mut t = Term::new(20, 4, 100);
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07slow\r\n\x1b]133;C\x07so far");
        assert!(select_output(&mut t.state));
        assert_eq!(t.state.selection_text().as_deref(), Some("so far"));
        assert!(has_output(&t.state, t.state.point_at(0, 0)));
        let mut plain = Term::new(20, 4, 100);
        plain.advance(b"text");
        assert!(!select_output(&mut plain.state));
        assert!(!has_output(&plain.state, plain.state.point_at(0, 0)));
    }
}
