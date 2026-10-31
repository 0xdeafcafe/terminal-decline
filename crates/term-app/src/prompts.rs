//! Shell integration (roadmap E.5): what the prompt marks a shell sends
//! (OSC 133, from the snippets in `shell/`) are used for. Cmd+Up and
//! Cmd+Down scroll the view to the prompt above or below its top line,
//! and Select Command Output selects a command's output.

use term_core::{MarkKind, Point, Terminal, ViewTop, Width};

/// Scroll the view so the nearest prompt above its top line (`up`), or
/// below it, is at the top, as near as history allows (a prompt on the
/// screen below the top can only bring the view to the bottom). False if
/// the view didn't move: no prompt that way.
pub fn jump(t: &mut Terminal, up: bool) -> bool {
    let target = prompts_from_top(t, up).next();
    jump_to(t, target)
}

/// Jump to Previous / Next Failed Command: [`jump`], to the nearest
/// prompt that way whose command exited non-zero.
pub fn jump_to_failed(t: &mut Terminal, up: bool) -> bool {
    let target = prompts_from_top(t, up).find(|p| t.command_failed(*p));
    jump_to(t, target)
}

/// The prompts above the view's top line (`up`), or below it, nearest
/// first.
fn prompts_from_top(t: &Terminal, up: bool) -> impl Iterator<Item = Point> + '_ {
    let top = t.point_at(0, 0);
    let first = if up {
        // The top row may start partway through a line (history laid out
        // at another width): a prompt earlier on that line is above it.
        t.previous_prompt(top.line + 1)
            .filter(|p| *p < top)
            .or_else(|| t.previous_prompt(top.line))
    } else {
        t.next_prompt(top.line)
    };
    std::iter::successors(first, move |p| if up { t.previous_prompt(p.line) } else { t.next_prompt(p.line) })
}

/// Scroll the view so `prompt` is at its top. False if the view didn't
/// move, or there is no prompt.
fn jump_to(t: &mut Terminal, prompt: Option<Point>) -> bool {
    let Some(prompt) = prompt else { return false };
    // The prompt's row at the top of the view; a prompt on the screen
    // brings the view to the bottom.
    let before = t.view_top();
    t.set_view_top(ViewTop::At { point: prompt, offset: 0 });
    t.view_top() != before
}

/// Select a command's output: with the view scrolled back, the command
/// at its top line (where a jump leaves its prompt); at the bottom, the
/// one running, else the last one with output ([`last_output`]). False
/// if there is none.
pub fn select_output(t: &mut Terminal) -> bool {
    if t.view_top() != ViewTop::Bottom {
        return t.select_output(t.point_at(0, 0));
    }
    let Some((a, b)) = last_output(t) else { return false };
    t.set_selection(Some(term_core::Selection::new(a, b)));
    true
}

/// The prompts from the cursor's up, nearest first: the one the cursor
/// is at or after, then those above.
fn prompts_from_cursor(t: &Terminal) -> impl Iterator<Item = Point> + '_ {
    let cursor = t.cursor_point();
    let first = t.previous_prompt(cursor.line + 1).filter(|p| *p <= cursor);
    std::iter::successors(first, move |p| t.previous_prompt(p.line))
}

/// The last command's output, first and last cell, wherever the view
/// is: the command running, else the last one that printed something.
pub fn last_output(t: &Terminal) -> Option<(Point, Point)> {
    // The cursor in a running command's output: that one. At a new
    // prompt: the command before it, further back only if that one
    // printed nothing.
    t.command_output(t.cursor_point())
        .or_else(|| prompts_from_cursor(t).find_map(|p| t.command_output(p)))
}

/// The last command line run (the one running, or the last finished),
/// first and last cell, wherever the view is. A prompt being typed at
/// hasn't run yet, so it is the one before it.
pub fn last_command(t: &Terminal) -> Option<(Point, Point)> {
    prompts_from_cursor(t).find_map(|p| t.command_line(p))
}

/// Copy Last Command's Output: its text, as copying it selected gives.
pub fn last_output_text(t: &Terminal) -> Option<String> {
    last_output(t).map(|(a, b)| t.text_between(a, b))
}

/// Copy Last Command: its command line, without the blanks after it
/// (where the shell's cursor went on to the next line).
pub fn last_command_text(t: &Terminal) -> Option<String> {
    let (a, b) = last_command(t)?;
    let text = t.text_between(a, b);
    let text = text.trim_end();
    (!text.is_empty()).then(|| text.to_string())
}

/// The output of the command at `p`, for the right-click menu: whether
/// there is one to select.
pub fn has_output(t: &Terminal, p: Point) -> bool {
    t.command_output(p).is_some()
}

/// How far the shell's cursor must move to be on view cell (`line`,
/// `col`), in characters (negative: left), if that cell is on the command
/// line being edited: after the prompt's OSC 133 B mark, on the rows of
/// the line the cursor is on (soft-wrapped rows count as one line), with
/// no command running (a C mark ends the command line). `None` without
/// marks, in the scrollback, in any other place, or on a command line
/// of several lines the shell would move over with Up and Down.
/// Characters are counted as the shell sees them: a wide character or a
/// cluster is one; a click past the end of the text is at its end.
pub fn input_steps(t: &Terminal, line: usize, col: usize) -> Option<isize> {
    if t.view_top() != ViewTop::Bottom {
        return None;
    }
    let (crow, ccol) = t.cursor_in_view()?;
    let cursor = t.point_at(crow, ccol);
    // The latest prompt's marks up to the cursor: is its input open?
    let prompt = t.previous_prompt(cursor.line + 1)?;
    let mut start = None;
    let mut in_prompt = false;
    for l in prompt.line..=cursor.line {
        for m in t.marks_at(l) {
            match m.kind {
                MarkKind::Prompt => (in_prompt, start) = (true, None),
                MarkKind::Input if in_prompt => {
                    start = Some(Point {
                        line: l,
                        col: m.col as usize,
                    })
                }
                MarkKind::Input => {}
                MarkKind::Output | MarkKind::End => (in_prompt, start) = (false, None),
            }
        }
    }
    let start = start?;
    let top = t.point_at(0, 0).line;
    let first = start.line.checked_sub(top)? as usize;
    // The rows of the command line: from the B mark down to where the
    // cursor's line ends; each but the last must wrap onto the next.
    let mut last = crow;
    while last + 1 < t.lines() && t.visible_row(last).wrapped() {
        last += 1;
    }
    if (first..last).any(|r| !t.visible_row(r).wrapped()) {
        return None;
    }
    if line < first || line > last || (line == first && col < start.col) {
        return None;
    }
    // The text ends at the last cell with something in it, or the cursor.
    let cols = t.cols();
    let end = {
        let row = t.visible_row(last);
        let text = row.cells().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1);
        if last == crow { text.max(ccol) } else { text }
    };
    // A click on the right half of a wide character is on the character;
    // on an edge spacer, at the start of the next row.
    let (mut line, mut col) = (line, col.min(cols - 1));
    match t.visible_row(line).cell(col).width() {
        Width::Spacer if col > 0 => col -= 1,
        Width::EdgeSpacer if line < last => (line, col) = (line + 1, 0),
        _ => {}
    }
    if line == last {
        col = col.min(end);
    }
    let count = |a: (usize, usize), b: (usize, usize)| -> isize {
        (a.0..=b.0)
            .map(|r| {
                let (c0, c1) = (if r == a.0 { a.1 } else { 0 }, if r == b.0 { b.1 } else { cols });
                t.visible_row(r)
                    .cells()
                    .skip(c0)
                    .take(c1.saturating_sub(c0))
                    .filter(|c| !c.is_spacer())
                    .count()
            })
            .sum::<usize>() as isize
    };
    let (from, to) = ((crow, ccol), (line, col));
    Some(match from.cmp(&to) {
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Less => count(from, to),
        std::cmp::Ordering::Greater => -count(to, from),
    })
}

/// The key presses that move a shell's cursor `steps` characters (negative:
/// left): the arrow keys, in application cursor mode as the program asked.
pub fn arrows(steps: isize, app_cursor: bool) -> Vec<u8> {
    let key: &[u8] = match (steps < 0, app_cursor) {
        (true, false) => b"\x1b[D",
        (true, true) => b"\x1bOD",
        (false, false) => b"\x1b[C",
        (false, true) => b"\x1bOC",
    };
    key.repeat(steps.unsigned_abs())
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
        let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
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
        let mut plain = Term::new(20, 4, 100, term_core::ImagePool::process());
        plain.advance(b"a\r\nb\r\nc\r\nd\r\ne\r\nf");
        assert!(!jump(&mut plain.state, true));
    }

    #[test]
    fn jumps_land_on_prompts_after_a_width_change() {
        // Output lines of 30 cells: two stored lines each at 20 columns,
        // so history read at another width has other rows than lines.
        let long = "x".repeat(30);
        let out = format!("{long}\r\n").repeat(6);
        for cols in [40, 12] {
            let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
            session(&mut t, "one", &out);
            session(&mut t, "two", &out);
            session(&mut t, "three", &out);
            t.advance(b"\x1b]133;A\x07$ ");
            t.state.resize(cols, 4);
            assert!(jump(&mut t.state, true), "at {cols}");
            assert_eq!(top_text(&t), "$ three", "at {cols}");
            assert!(jump(&mut t.state, true));
            assert_eq!(top_text(&t), "$ two", "at {cols}");
            assert!(jump(&mut t.state, true));
            assert_eq!(top_text(&t), "$ one", "at {cols}");
            assert!(!jump(&mut t.state, true), "no prompt above the first");
            assert!(jump(&mut t.state, false));
            assert_eq!(top_text(&t), "$ two", "at {cols}");
            assert!(jump(&mut t.state, false));
            assert_eq!(top_text(&t), "$ three", "at {cols}");
            // The last prompt is on the screen: the view goes to the bottom.
            assert!(jump(&mut t.state, false));
            assert_eq!(t.state.view_top(), ViewTop::Bottom);
            assert!(!jump(&mut t.state, false));
        }
    }

    #[test]
    fn selects_the_output_at_the_top_or_the_last() {
        let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
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
        let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07slow\r\n\x1b]133;C\x07so far");
        assert!(select_output(&mut t.state));
        assert_eq!(t.state.selection_text().as_deref(), Some("so far"));
        assert!(has_output(&t.state, t.state.point_at(0, 0)));
        let mut plain = Term::new(20, 4, 100, term_core::ImagePool::process());
        plain.advance(b"text");
        assert!(!select_output(&mut plain.state));
        assert!(!has_output(&plain.state, plain.state.point_at(0, 0)));
    }

    /// [`session`] with an exit status.
    fn ended(t: &mut Term, cmd: &str, output: &str, status: u32) {
        t.advance(format!("\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07{output}\x1b]133;D;{status}\x07").as_bytes());
    }

    #[test]
    fn jumps_to_failed_commands_only() {
        for cols in [20, 9, 40] {
            let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
            ended(&mut t, "bad1", "a\r\na\r\na\r\n", 1);
            ended(&mut t, "ok", "b\r\nb\r\nb\r\n", 0);
            ended(&mut t, "bad2", "c\r\nc\r\nc\r\n", 2);
            ended(&mut t, "ok2", "d\r\nd\r\nd\r\n", 0);
            t.advance(b"\x1b]133;A\x07$ ");
            t.state.resize(cols, 4);
            assert!(jump_to_failed(&mut t.state, true), "at {cols}");
            assert_eq!(top_text(&t), "$ bad2", "at {cols}");
            assert!(jump_to_failed(&mut t.state, true));
            assert_eq!(top_text(&t), "$ bad1", "at {cols}");
            assert!(!jump_to_failed(&mut t.state, true), "no failed command above the first");
            assert!(jump_to_failed(&mut t.state, false));
            assert_eq!(top_text(&t), "$ bad2", "at {cols}");
            assert!(!jump_to_failed(&mut t.state, false), "none below: the view stays");
            assert_eq!(top_text(&t), "$ bad2");
        }
        // All succeeded: nowhere to go.
        let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
        ended(&mut t, "ok", "1\r\n2\r\n3\r\n4\r\n", 0);
        t.advance(b"\x1b]133;A\x07$ ");
        assert!(!jump_to_failed(&mut t.state, true));
    }

    #[test]
    fn copies_the_last_command_and_its_output() {
        let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
        ended(&mut t, "first", "1\r\n", 0);
        ended(&mut t, "echo two", "2\r\n22\r\n", 0);
        ended(&mut t, "quiet", "", 0);
        // At a new prompt, being typed at: the last command run, and the
        // last output there was (`quiet` printed none).
        t.advance(b"\x1b]133;A\x07$ \x1b]133;B\x07half-typ");
        assert_eq!(last_command_text(&t.state).as_deref(), Some("quiet"));
        assert_eq!(last_output_text(&t.state).as_deref(), Some("2\n22"));
        // Scrolled back to the first: still the last ones, and nothing
        // selected.
        while top_text(&t) != "$ first" {
            assert!(jump(&mut t.state, true));
        }
        assert_eq!(last_command_text(&t.state).as_deref(), Some("quiet"));
        assert_eq!(last_output_text(&t.state).as_deref(), Some("2\n22"));
        assert_eq!(t.state.selection(), None);
        // A command running: it and its output so far.
        t.state.scroll_view_to_bottom();
        t.advance(b"\r\n\x1b]133;C\x07so\r\nfar");
        assert_eq!(last_command_text(&t.state).as_deref(), Some("half-typ"));
        assert_eq!(last_output_text(&t.state).as_deref(), Some("so\nfar"));
        // A command line that wraps, read back at another width.
        let mut t = Term::new(10, 3, 100, term_core::ImagePool::process());
        ended(&mut t, "echo 0123456789abc", "x\r\n", 0);
        t.advance(b"\x1b]133;A\x07$ ");
        // At 7 the space ends a wrapped row, and is copied all the same.
        for cols in [8, 7] {
            t.state.resize(cols, 3);
            assert_eq!(last_command_text(&t.state).as_deref(), Some("echo 0123456789abc"), "at {cols}");
            assert_eq!(last_output_text(&t.state).as_deref(), Some("x"), "at {cols}");
        }
        // No shell integration: nothing to copy.
        let mut plain = Term::new(20, 4, 100, term_core::ImagePool::process());
        plain.advance(b"$ ls\r\na b\r\n$ ");
        assert_eq!(last_command_text(&plain.state), None);
        assert_eq!(last_output_text(&plain.state), None);
    }

    /// A prompt `$ ` with its marks, then `input` typed after it.
    fn typing(t: &mut Term, input: &str) {
        t.advance(format!("\x1b]133;A\x07$ \x1b]133;B\x07{input}").as_bytes());
    }

    #[test]
    fn steps_to_a_cell_of_the_command_line() {
        let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
        typing(&mut t, "echo hello");
        // The cursor is at column 12, after `hello`.
        assert_eq!(input_steps(&t.state, 0, 12), Some(0));
        assert_eq!(input_steps(&t.state, 0, 2), Some(-10), "the first character");
        assert_eq!(input_steps(&t.state, 0, 7), Some(-5));
        assert_eq!(input_steps(&t.state, 0, 19), Some(0), "past the end is the end");
        // The shell's cursor in the middle of the line: right and left.
        t.advance(b"\x1b[8G");
        assert_eq!(input_steps(&t.state, 0, 11), Some(4));
        assert_eq!(input_steps(&t.state, 0, 5), Some(-2));
        assert_eq!(input_steps(&t.state, 0, 19), Some(5));
        // Not on the command line: the prompt, another row.
        assert_eq!(input_steps(&t.state, 0, 1), None);
        assert_eq!(input_steps(&t.state, 1, 0), None);
    }

    #[test]
    fn steps_count_wide_characters_and_clusters_as_one() {
        let mut t = Term::new(20, 4, 100, term_core::ImagePool::process());
        // `a`, a wide character, a flag (a cluster, two cells), `b`.
        typing(&mut t, "a\u{4e16}\u{1f1e9}\u{1f1ea}b");
        let (row, col) = t.state.cursor_in_view().unwrap();
        assert_eq!((row, col), (0, 2 + 1 + 2 + 2 + 1));
        assert_eq!(input_steps(&t.state, 0, 2), Some(-4));
        assert_eq!(input_steps(&t.state, 0, 3), Some(-3), "the wide character's left half");
        assert_eq!(input_steps(&t.state, 0, 4), Some(-3), "and its right half");
        assert_eq!(input_steps(&t.state, 0, 5), Some(-2), "the flag");
        assert_eq!(input_steps(&t.state, 0, 6), Some(-2));
        assert_eq!(input_steps(&t.state, 0, 7), Some(-1), "b");
    }

    #[test]
    fn steps_follow_a_wrapped_command_line() {
        let mut t = Term::new(10, 4, 100, term_core::ImagePool::process());
        typing(&mut t, "0123456789abcdefg");
        // Row 0: `$ 01234567`, row 1: `89abcdefg`, the cursor after it.
        assert_eq!(t.state.cursor_in_view(), Some((1, 9)));
        assert_eq!(input_steps(&t.state, 0, 2), Some(-17));
        assert_eq!(input_steps(&t.state, 1, 3), Some(-6));
        assert_eq!(input_steps(&t.state, 0, 1), None, "the prompt");
        assert_eq!(input_steps(&t.state, 3, 0), None, "below the line");
    }

    #[test]
    fn no_steps_without_an_open_command_line() {
        // No marks.
        let mut plain = Term::new(20, 4, 100, term_core::ImagePool::process());
        plain.advance(b"$ echo hello");
        assert_eq!(input_steps(&plain.state, 0, 4), None);
        // Prompt marked, but before its B mark.
        let mut a = Term::new(20, 4, 100, term_core::ImagePool::process());
        a.advance(b"\x1b]133;A\x07$ ");
        assert_eq!(input_steps(&a.state, 0, 1), None);
        // A command running: its output is not a command line.
        let mut running = Term::new(20, 4, 100, term_core::ImagePool::process());
        typing(&mut running, "sleep 9\r\n\x1b]133;C\x07output");
        assert_eq!(input_steps(&running.state, 0, 4), None);
        assert_eq!(input_steps(&running.state, 1, 2), None);
        // Finished, a new prompt: its input is open again, the old is not.
        let mut done = Term::new(20, 4, 100, term_core::ImagePool::process());
        typing(&mut done, "ls\r\n\x1b]133;C\x07a\r\n\x1b]133;D;0\x07");
        typing(&mut done, "pwd");
        assert_eq!(input_steps(&done.state, 0, 3), None);
        assert_eq!(input_steps(&done.state, 2, 3), Some(-2));
        // Scrolled back: the cursor is not in view.
        let mut back = Term::new(20, 2, 100, term_core::ImagePool::process());
        typing(&mut back, "one\r\ntwo\r\nthree");
        back.state.scroll_view(1);
        assert_eq!(input_steps(&back.state, 1, 3), None);
    }

    #[test]
    fn arrows_follow_the_cursor_mode() {
        assert_eq!(arrows(2, false), b"\x1b[C\x1b[C");
        assert_eq!(arrows(-3, false), b"\x1b[D\x1b[D\x1b[D");
        assert_eq!(arrows(1, true), b"\x1bOC");
        assert_eq!(arrows(-2, true), b"\x1bOD\x1bOD");
        assert!(arrows(0, false).is_empty());
    }
}
