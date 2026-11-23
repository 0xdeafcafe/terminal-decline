//! Esc at the prompt clears the line being typed (`escape_clears_line`),
//! asking first when it is long (`confirm_clear_line`).
//!
//! Only where the app knows a line is being edited and Esc means nothing
//! else: zsh with the shell integration reports its line
//! ([`term_core::LineReport`]), in emacs mode (vi's insert mode needs
//! Esc), with the shell itself in the terminal's foreground (not fzf
//! started from a key binding) and the completion list closed (Esc
//! closes that). The app then types [`CLEAR`] to `decline.zsh`'s widget,
//! which empties the line as a new one, so Up starts from the newest
//! command again.

use term_core::{Keymap, LineReport};

/// What the app types for `decline.zsh` to clear the line.
pub const CLEAR: &[u8] = b"\x1b[7701~!clear\x07";

/// What Esc does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Esc {
    /// Goes to the program, as any key.
    Pass,
    /// Clears the line.
    Clear,
    /// Asks, then clears the line: it is long.
    Ask,
}

/// What a pane has when Esc is pressed.
#[derive(Clone, Copy, Debug)]
pub struct At<'a> {
    /// The line the shell last reported, if it is at its prompt.
    pub line: Option<&'a LineReport>,
    /// The completion list is showing.
    pub list_showing: bool,
    /// The shell is the terminal's foreground program.
    pub shell_in_foreground: bool,
    /// `escape_clears_line`.
    pub on: bool,
    /// `confirm_clear_line`.
    pub confirm_over: u32,
}

/// What Esc, with no modifier, does in a pane.
pub fn decide(at: At) -> Esc {
    let Some(line) = at.line else { return Esc::Pass };
    if !at.on || at.list_showing || !at.shell_in_foreground || line.keymap != Keymap::Insert {
        return Esc::Pass;
    }
    // A line too long to report has no text here.
    if line.too_long {
        return if at.confirm_over == 0 { Esc::Clear } else { Esc::Ask };
    }
    // Nothing to clear: Esc is the shell's (a prefix for Meta keys).
    if line.text.is_empty() {
        return Esc::Pass;
    }
    let long = line.text.contains('\n') || line.text.chars().count() > at.confirm_over as usize;
    if at.confirm_over != 0 && long { Esc::Ask } else { Esc::Clear }
}

/// The question for a long line: its title and text.
pub fn question(line: &LineReport) -> (String, String) {
    let what = if line.too_long {
        "The line is very long.".to_string()
    } else {
        let shown: String = line.text.chars().take(120).map(|c| if c.is_control() { ' ' } else { c }).collect();
        let more = if line.text.chars().count() > 120 { "…" } else { "" };
        format!("{shown}{more}")
    };
    ("Clear the command line?".into(), what)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, keymap: Keymap) -> LineReport {
        LineReport {
            text: text.into(),
            cursor: text.len(),
            keymap,
            too_long: false,
            at: term_core::Point { line: 0, col: 0 },
        }
    }

    fn at(line: Option<&LineReport>) -> At<'_> {
        At {
            line,
            list_showing: false,
            shell_in_foreground: true,
            on: true,
            confirm_over: 80,
        }
    }

    #[test]
    fn esc_clears_a_line_being_typed() {
        let l = line("git status", Keymap::Insert);
        assert_eq!(decide(at(Some(&l))), Esc::Clear);
        // Nothing typed, or no prompt: the shell's or the program's.
        assert_eq!(decide(at(Some(&line("", Keymap::Insert)))), Esc::Pass);
        assert_eq!(decide(at(None)), Esc::Pass);
    }

    #[test]
    fn esc_is_left_alone_where_it_means_something_else() {
        let l = line("git status", Keymap::Insert);
        for keymap in [Keymap::ViInsert, Keymap::ViCommand, Keymap::Other] {
            assert_eq!(decide(at(Some(&line("git status", keymap)))), Esc::Pass, "{keymap:?}");
        }
        assert_eq!(
            decide(At {
                list_showing: true,
                ..at(Some(&l))
            }),
            Esc::Pass
        );
        // fzf from a key binding: the line report is still the shell's.
        assert_eq!(
            decide(At {
                shell_in_foreground: false,
                ..at(Some(&l))
            }),
            Esc::Pass
        );
        assert_eq!(decide(At { on: false, ..at(Some(&l)) }), Esc::Pass);
    }

    #[test]
    fn a_long_line_is_asked_about_unless_told_not_to() {
        let exactly = "x".repeat(80);
        let over = "é".repeat(81);
        assert_eq!(decide(at(Some(&line(&exactly, Keymap::Insert)))), Esc::Clear);
        assert_eq!(decide(at(Some(&line(&over, Keymap::Insert)))), Esc::Ask);
        // Several lines are long however short.
        assert_eq!(decide(at(Some(&line("a\nb", Keymap::Insert)))), Esc::Ask);
        let too_long = LineReport {
            too_long: true,
            ..line("", Keymap::Insert)
        };
        assert_eq!(decide(at(Some(&too_long))), Esc::Ask);
        // Don't ask again: 0.
        for l in [&line(&over, Keymap::Insert), &line("a\nb", Keymap::Insert), &too_long] {
            assert_eq!(
                decide(At {
                    confirm_over: 0,
                    ..at(Some(l))
                }),
                Esc::Clear
            );
        }
    }

    #[test]
    fn the_question_shows_the_start_of_the_line() {
        let (title, text) = question(&line("echo a\tb", Keymap::Insert));
        assert_eq!((title.as_str(), text.as_str()), ("Clear the command line?", "echo a b"));
        let (_, text) = question(&line(&"y".repeat(200), Keymap::Insert));
        assert_eq!(text.chars().count(), 121);
        assert!(text.ends_with('…'));
    }
}
