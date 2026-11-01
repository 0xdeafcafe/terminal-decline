//! What the shell reports about the command line being typed (OSC 7701,
//! docs/design/completion.md §3): the line and its cursor on each redraw,
//! and once per prompt what the shell can run.
//!
//! ```text
//! ESC ] 7701 ; line ; <token> ; <keymap> ; <cursor> ; <line> BEL
//! ESC ] 7701 ; env ; <token> ; path=… ; alias=… ; … BEL
//! ```
//!
//! Values are percent-encoded (`%`, `;` and control bytes); lists are
//! newline-separated before encoding. A report without the token the app
//! gave this shell is dropped, so output that merely contains the
//! sequence (a file being `cat`ed) can't forge a command line. Reports
//! don't change what is drawn, so they don't bump the generation.

use super::{Point, Terminal, percent_bytes};

/// zle's keymap when the line was reported (`$KEYMAP`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keymap {
    /// `main`, `emacs` or `viins`: keys type text.
    Insert,
    /// `vicmd`: keys are vi commands.
    ViCommand,
    /// Any other keymap (`isearch`, `menuselect`, a user's own).
    Other,
}

/// The command line as the shell last reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineReport {
    /// The line (`$BUFFER`); empty in a continuation line or when it was
    /// too long to send.
    pub text: String,
    /// The cursor, a byte offset into `text` on a character boundary.
    pub cursor: usize,
    pub keymap: Keymap,
    /// The line was over the shell's limit and not sent.
    pub too_long: bool,
    /// Where the prompt's B mark put the command line, so a report says
    /// which line it belongs to.
    pub at: Point,
}

/// What the shell can run, reported once per prompt when it changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellEnv {
    /// `$PATH`, as the string.
    pub path: String,
    pub aliases: Vec<String>,
    /// Suffix aliases (`alias -s`).
    pub suffix_aliases: Vec<String>,
    /// Global aliases (`alias -g`).
    pub global_aliases: Vec<String>,
    pub functions: Vec<String>,
    pub builtins: Vec<String>,
    pub reserved_words: Vec<String>,
    /// `setopt autocd`: a folder in command position is run as `cd`.
    pub autocd: bool,
    /// `$HISTFILE`.
    pub histfile: Option<String>,
}

/// Hex digits in a token (128 bits).
pub const TOKEN_LEN: usize = 32;

impl Terminal {
    /// The token this shell's reports must carry (32 lower-case hex
    /// digits, as given to the shell in `DECLINE_TOKEN`); `None` drops
    /// every report.
    pub fn set_report_token(&mut self, token: Option<[u8; TOKEN_LEN]>) {
        self.report_token = token;
    }

    /// The token set with [`Terminal::set_report_token`], to carry over
    /// to a terminal that replaces this one.
    pub fn report_token(&self) -> Option<[u8; TOKEN_LEN]> {
        self.report_token
    }

    /// The command line as last reported, while the prompt waits for
    /// input on the primary screen.
    pub fn line_report(&self) -> Option<&LineReport> {
        self.line_report.as_ref().filter(|_| !self.alt_active)
    }

    /// What the shell can run, if it reported it since the last call.
    pub fn take_shell_env(&mut self) -> Option<ShellEnv> {
        self.shell_env.take()
    }

    /// Whether any OSC 7701 sequence was parsed since the last call: the
    /// reader doesn't wake the view for output that was only reports.
    pub fn take_report_seen(&mut self) -> bool {
        std::mem::take(&mut self.report_seen)
    }

    pub(super) fn shell_report(&mut self, kind: &[u8], token: &[u8], rest: &[&[u8]]) {
        self.report_seen = true;
        if self.report_token.as_ref().is_none_or(|t| t[..] != *token) {
            return;
        }
        match kind {
            b"line" => self.line_reported(rest),
            b"env" => self.shell_env = Some(shell_env(rest)),
            _ => {}
        }
    }

    fn line_reported(&mut self, rest: &[&[u8]]) {
        let [keymap, cursor, line] = rest else { return };
        // Only while the prompt waits for input (after B, before C), and
        // not over a program on the alternate screen.
        let Some((line_at, col)) = self.input_at.filter(|_| !self.alt_active) else {
            return;
        };
        let (keymap, too_long) = match *keymap {
            b"main" | b"emacs" | b"viins" => (Keymap::Insert, false),
            b"vicmd" => (Keymap::ViCommand, false),
            b"long" => (Keymap::Other, true),
            _ => (Keymap::Other, false),
        };
        let bytes = percent_bytes(line);
        let cursor = std::str::from_utf8(cursor)
            .ok()
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(bytes.len())
            .min(bytes.len());
        // The cursor counts the shell's bytes; with invalid UTF-8 replaced
        // the offsets move, so measure the prefix the same way.
        let mut cursor = String::from_utf8_lossy(&bytes[..cursor]).len();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        while !text.is_char_boundary(cursor) {
            cursor -= 1;
        }
        self.line_report = Some(LineReport {
            text,
            cursor,
            keymap,
            too_long,
            at: Point { line: line_at, col },
        });
    }
}

fn shell_env(fields: &[&[u8]]) -> ShellEnv {
    let mut env = ShellEnv::default();
    for field in fields {
        let Some(eq) = field.iter().position(|&b| b == b'=') else {
            continue;
        };
        let value = String::from_utf8_lossy(&percent_bytes(&field[eq + 1..])).into_owned();
        let list = || value.split('\n').filter(|s| !s.is_empty()).map(str::to_string).collect();
        match &field[..eq] {
            b"path" => env.path = value.clone(),
            b"alias" => env.aliases = list(),
            b"salias" => env.suffix_aliases = list(),
            b"galias" => env.global_aliases = list(),
            b"func" => env.functions = list(),
            b"builtin" => env.builtins = list(),
            b"resword" => env.reserved_words = list(),
            b"autocd" => env.autocd = value == "1",
            b"histfile" => env.histfile = (!value.is_empty()).then(|| value.clone()),
            _ => {}
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImagePool, Term};

    const TOKEN: &[u8; TOKEN_LEN] = b"0123456789abcdef0123456789abcdef";

    fn bare() -> Term {
        Term::new(40, 5, 100, ImagePool::process())
    }

    fn term() -> Term {
        let mut t = bare();
        t.state.set_report_token(Some(*TOKEN));
        t
    }

    fn feed(t: &mut Term, s: &str) {
        t.advance(s.as_bytes());
    }

    fn prompt(t: &mut Term) {
        feed(t, "\x1b]133;A\x07$ \x1b]133;B\x07");
    }

    fn line(token: &str, keymap: &str, cursor: &str, text: &str) -> String {
        format!("\x1b]7701;line;{token};{keymap};{cursor};{text}\x07")
    }

    #[test]
    fn a_line_at_the_prompt_is_kept_with_its_mark() {
        let mut t = term();
        prompt(&mut t);
        let g = t.state.generation();
        feed(&mut t, &line(std::str::from_utf8(TOKEN).unwrap(), "main", "2", "ls -l"));
        let r = t.state.line_report().unwrap();
        assert_eq!(
            (r.text.as_str(), r.cursor, r.keymap, r.too_long),
            ("ls -l", 2, Keymap::Insert, false)
        );
        assert_eq!(r.at, Point { line: 0, col: 2 });
        // Nothing to draw: the next redraw from the shell brings the frame.
        assert_eq!(t.state.generation(), g);
        assert!(t.state.take_report_seen());
        assert!(!t.state.take_report_seen());
    }

    #[test]
    fn a_wrong_or_missing_token_is_dropped_but_seen() {
        let mut t = term();
        prompt(&mut t);
        feed(&mut t, &line("00000000000000000000000000000000", "main", "1", "x"));
        feed(&mut t, &line("", "main", "1", "x"));
        assert!(t.state.line_report().is_none());
        assert!(t.state.take_report_seen());
        let mut t = bare();
        prompt(&mut t);
        feed(&mut t, &line(std::str::from_utf8(TOKEN).unwrap(), "main", "1", "x"));
        assert!(t.state.line_report().is_none(), "no token set: every report dropped");
    }

    #[test]
    fn only_while_the_prompt_waits_for_input() {
        let tok = std::str::from_utf8(TOKEN).unwrap();
        let mut t = term();
        // Before any B mark.
        feed(&mut t, &line(tok, "main", "1", "x"));
        assert!(t.state.line_report().is_none());
        prompt(&mut t);
        feed(&mut t, &line(tok, "main", "1", "x"));
        assert!(t.state.line_report().is_some());
        // The command runs: the report goes, and later ones are dropped.
        feed(&mut t, "x\r\n\x1b]133;C\x07");
        assert!(t.state.line_report().is_none());
        feed(&mut t, &line(tok, "main", "1", "y"));
        assert!(t.state.line_report().is_none());
        // A new prompt (or Ctrl+C's reprint) drops the old line too.
        prompt(&mut t);
        feed(&mut t, &line(tok, "main", "1", "z"));
        prompt(&mut t);
        assert!(t.state.line_report().is_none());
        // Not over a program on the alternate screen.
        feed(&mut t, "\x1b[?1049h");
        feed(&mut t, &line(tok, "main", "1", "z"));
        assert!(t.state.line_report().is_none());
    }

    #[test]
    fn percent_decoding_and_cursor_in_bytes() {
        let tok = std::str::from_utf8(TOKEN).unwrap();
        let mut t = term();
        prompt(&mut t);
        // `echo "a;b%" é` with the cursor after é: `;` and `%` encoded,
        // é two bytes in UTF-8.
        feed(&mut t, &line(tok, "viins", "15", "echo \"a%3Bb%25\" é"));
        let r = t.state.line_report().unwrap();
        assert_eq!(r.text, "echo \"a;b%\" é");
        assert_eq!(r.cursor, r.text.len());
        // A tab and a newline in the line.
        feed(&mut t, &line(tok, "vicmd", "99", "a%09b%0Ac"));
        let r = t.state.line_report().unwrap();
        assert_eq!((r.text.as_str(), r.cursor, r.keymap), ("a\tb\nc", 5, Keymap::ViCommand));
        // A cursor inside invalid UTF-8 lands on a character boundary.
        feed(&mut t, &line(tok, "main", "2", "a%FFb"));
        let r = t.state.line_report().unwrap();
        assert_eq!(r.text, "a\u{fffd}b");
        assert!(r.text.is_char_boundary(r.cursor));
        // Too long to send.
        feed(&mut t, &line(tok, "long", "0", ""));
        let r = t.state.line_report().unwrap();
        assert!(r.too_long && r.text.is_empty());
        // A malformed report changes nothing.
        feed(&mut t, &format!("\x1b]7701;line;{tok};main;1\x07"));
        assert!(t.state.line_report().unwrap().too_long);
    }

    #[test]
    fn shell_env_lists() {
        let tok = std::str::from_utf8(TOKEN).unwrap();
        let mut t = term();
        feed(
            &mut t,
            &format!(
                "\x1b]7701;env;{tok};path=/bin:/usr/bin;alias=ll%0Agst;salias=;galias=G;func=f%0Ag;builtin=cd%0Aecho;resword=if%0Athen;autocd=1;histfile=/Users/me/.zsh_history;future=1\x07"
            ),
        );
        let env = t.state.take_shell_env().unwrap();
        assert_eq!(env.path, "/bin:/usr/bin");
        assert_eq!(env.aliases, ["ll", "gst"]);
        assert!(env.suffix_aliases.is_empty());
        assert_eq!(env.global_aliases, ["G"]);
        assert_eq!(env.functions, ["f", "g"]);
        assert_eq!(env.builtins, ["cd", "echo"]);
        assert_eq!(env.reserved_words, ["if", "then"]);
        assert!(env.autocd);
        assert_eq!(env.histfile.as_deref(), Some("/Users/me/.zsh_history"));
        assert!(t.state.take_shell_env().is_none());
    }

    #[test]
    fn the_token_survives_a_reset() {
        let tok = std::str::from_utf8(TOKEN).unwrap();
        let mut t = term();
        feed(&mut t, "\x1bc");
        prompt(&mut t);
        feed(&mut t, &line(tok, "main", "1", "x"));
        assert!(t.state.line_report().is_some());
    }
}
