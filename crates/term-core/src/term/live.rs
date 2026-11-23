//! A terminal's state handed to another process, which carries on with
//! the same program on the same PTY (docs/design/handoff.md §4).
//!
//! Two parts, both formats that stay readable from one build to the
//! next: the history and primary screen as the blocks
//! [`Terminal::save_history`] writes, here with every row of the screen
//! so it comes back cell for cell; and everything a program may have set
//! (modes, the scroll region, the keyboard flags, the title…) as escape
//! sequences for the new terminal's parser.

use std::fmt::Write as _;

use super::{Charset, CursorShape, Modes, MouseMode, Term, Terminal, default_tabs, replies};
use crate::scrollback::{RestoreError, Restored};

/// What [`Terminal::save_live`] took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    /// History and the primary screen ([`Terminal::save_history`]'s
    /// format).
    pub blocks: Vec<u8>,
    /// Escape sequences that set the rest, for a terminal the blocks
    /// were restored into.
    pub replay: Vec<u8>,
}

impl Terminal {
    /// This terminal's state, for [`Term::restore_live`] in the process
    /// that takes over its program: the newest `max` lines of history,
    /// the primary screen, and what the program set. A full-screen
    /// program's own screen isn't in it: the new terminal starts the
    /// alternate screen empty, and the program is asked to repaint.
    pub fn save_live(&self, max: usize, saved_at: u64) -> Live {
        let (screen, styles) = if self.alt_active {
            (&self.other_screen, &self.other_styles)
        } else {
            (&self.screen, &self.styles)
        };
        let blocks = self
            .scrollback
            .save(screen.iter(), styles.styles(), styles.links().as_slice(), max, saved_at);
        Live {
            blocks,
            replay: self.replay().into_bytes(),
        }
    }

    fn replay(&self) -> String {
        let mut out = String::new();
        let push_flags = |out: &mut String, stack: &[u8]| {
            for flags in stack {
                let _ = write!(out, "\x1b[>{flags}u");
            }
        };
        push_flags(&mut out, &self.kitty_flags[0]);
        if self.alt_active {
            // 1049 saves the cursor it is given: the primary screen's.
            if let Some(saved) = &self.saved_primary {
                let _ = write!(out, "\x1b[{};{}H", saved.cursor.row + 1, saved.cursor.col + 1);
            }
            out.push_str("\x1b[?1049h");
            push_flags(&mut out, &self.kitty_flags[1]);
        }
        if (self.scroll_top, self.scroll_bottom) != (0, self.lines.saturating_sub(1)) {
            let _ = write!(out, "\x1b[{};{}r", self.scroll_top + 1, self.scroll_bottom + 1);
        }
        if self.tabs != default_tabs(self.cols) {
            out.push_str("\x1b[3g");
            for col in self.tabs.iter().enumerate().filter_map(|(c, &stop)| stop.then_some(c)) {
                let _ = write!(out, "\x1b[1;{}H\x1bH", col + 1);
            }
        }
        for (set, charset) in ["(", ")"].iter().zip(&self.charsets) {
            if *charset == Charset::DecGraphics {
                let _ = write!(out, "\x1b{set}0");
            }
        }
        if self.active_charset == 1 {
            out.push('\x0e');
        }
        self.replay_modes(&mut out);
        if (self.cursor_shape, self.cursor_blink) != self.cursor_default {
            let shape = match self.cursor_shape {
                CursorShape::Block => 1,
                CursorShape::Underline => 3,
                CursorShape::Bar => 5,
            };
            let _ = write!(out, "\x1b[{} q", shape + usize::from(!self.cursor_blink));
        }
        if let Some(title) = &self.title {
            let title: String = title.chars().filter(|c| !c.is_control()).collect();
            let _ = write!(out, "\x1b]2;{title}\x07");
        }
        if let Some(cwd) = &self.cwd {
            let host = self.cwd_host.as_deref().unwrap_or("");
            let _ = write!(out, "\x1b]7;file://{}{}\x07", percent_encode(host), percent_encode(cwd));
        }
        let _ = write!(out, "\x1b[{}m", replies::sgr_report(&self.cursor.pen));
        // Last: the scroll region and origin mode move the cursor. In
        // origin mode rows count from the region's top.
        let row = if self.modes.origin {
            self.cursor.row.saturating_sub(self.scroll_top)
        } else {
            self.cursor.row
        };
        let _ = write!(out, "\x1b[{};{}H", row + 1, self.cursor.col + 1);
        out
    }

    /// Each mode that isn't as a new terminal has it.
    fn replay_modes(&self, out: &mut String) {
        let (m, d) = (&self.modes, Modes::default());
        let private = [
            (1, m.app_cursor, d.app_cursor),
            (6, m.origin, d.origin),
            (7, m.autowrap, d.autowrap),
            (25, m.cursor_visible, d.cursor_visible),
            (80, m.sixel_display, d.sixel_display),
            (1004, m.focus_events, d.focus_events),
            (1006, m.mouse_sgr, d.mouse_sgr),
            (1070, m.sixel_private_colors, d.sixel_private_colors),
            (2004, m.bracketed_paste, d.bracketed_paste),
            (2027, m.grapheme_clusters, d.grapheme_clusters),
            (2031, m.color_scheme_reports, d.color_scheme_reports),
            (8452, m.sixel_cursor_right, d.sixel_cursor_right),
        ];
        for (mode, on, default) in private {
            if on != default {
                let _ = write!(out, "\x1b[?{mode}{}", if on { 'h' } else { 'l' });
            }
        }
        let mouse = match m.mouse {
            MouseMode::Off => None,
            MouseMode::Click => Some(1000),
            MouseMode::Drag => Some(1002),
            MouseMode::Motion => Some(1003),
        };
        if let Some(mode) = mouse {
            let _ = write!(out, "\x1b[?{mode}h");
        }
        if m.app_keypad {
            out.push_str("\x1b=");
        }
        if m.insert {
            out.push_str("\x1b[4h");
        }
        if m.linefeed_newline {
            out.push_str("\x1b[20h");
        }
    }

    /// The blocks of a [`Live`], into this terminal before anything is
    /// written to it: as [`Terminal::restore_history`], but every row
    /// that was on the screen comes back onto it.
    fn restore_live_blocks(&mut self, blocks: &[u8]) -> Result<Restored, RestoreError> {
        if self.alt_active || self.top_line != 0 || !self.scrollback.is_empty() {
            return Err(RestoreError::NotEmpty);
        }
        let r = self.scrollback.restore(blocks)?;
        self.top_line = self.scrollback.total();
        let from = self.scrollback.first_visible();
        self.pull_back(r.screen.min(self.lines));
        self.scrollback.hide_before(from);
        Ok(r)
    }
}

impl Term {
    /// Make this new terminal what `live` was taken from. With blocks
    /// that can't be read (another format), what the program set is
    /// still put back, over an empty screen, and the error returned.
    pub fn restore_live(&mut self, live: &Live) -> Result<Restored, RestoreError> {
        let restored = self.state.restore_live_blocks(&live.blocks);
        self.advance(&live.replay);
        restored
    }
}

/// `text` for a `file://` URL: all but letters, digits and `/._~-`
/// percent-encoded, byte by byte.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"/._~-".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImagePool;

    fn term(cols: usize, lines: usize) -> Term {
        Term::new(cols, lines, 1000, ImagePool::process())
    }

    /// What a program can see or rely on, as text.
    fn seen(t: &Terminal) -> String {
        format!(
            "{}\n{:?} flags={} region={:?} title={:?} cwd={:?}",
            t.dump(),
            t.modes,
            t.kitty_keyboard_flags(),
            (t.scroll_top, t.scroll_bottom),
            t.title,
            t.cwd,
        )
    }

    fn handed_over(old: &Term) -> Term {
        let live = old.state.save_live(1000, 7);
        let mut new = term(old.state.cols, old.state.lines);
        let restored = new.restore_live(&live).expect("restored");
        assert_eq!(restored.saved_at, 7);
        new
    }

    #[test]
    fn a_shell_at_its_prompt_comes_back_cell_for_cell() {
        let mut old = term(20, 5);
        // More lines than the screen, colour, a wrapped line, a prompt
        // on the last row with the cursor after it.
        old.advance(b"one\r\ntwo\r\n\x1b[31mred\x1b[0m\r\nabcdefghijklmnopqrstuvwxyz\r\nfive\r\nsix\r\n\x1b]133;A\x07$ \x1b]133;B\x07ls");
        old.advance(b"\x1b]2;my title\x07\x1b]7;file://host/tmp/a%20b\x07\x1b[?2004h");
        let new = handed_over(&old);
        assert_eq!(seen(&new.state), seen(&old.state));
        assert_eq!(new.state.scrollback_len(), old.state.scrollback_len());
        assert_eq!(new.state.cwd_host(), Some("host"));
        // The full screen: its last row is the prompt, not a free row.
        assert!(new.state.dump().contains("$ ls|\ncursor=(4,4)"), "{}", new.state.dump());
        // And what is typed next lands where it would have.
        let (mut old, mut new) = (old, new);
        for t in [&mut old, &mut new] {
            t.advance(b" -l\r\nout\r\n");
        }
        assert_eq!(seen(&new.state), seen(&old.state));
    }

    #[test]
    fn what_a_program_set_is_set_again() {
        let mut old = term(30, 6);
        old.advance(b"text\r\n\x1b[?1h\x1b=\x1b[?7l\x1b[?25l\x1b[?1004h\x1b[?1002h\x1b[?1006h\x1b[?2027l\x1b[?2031h");
        old.advance(b"\x1b[4h\x1b[20h\x1b[>5u\x1b[>1u\x1b[2;5r\x1b[3g\x1b[1;4H\x1bH\x1b[1;11H\x1bH\x1b(0\x1b[5 q\x1b[1;38;5;9m\x1b[3;7H");
        let mut new = handed_over(&old);
        assert_eq!(seen(&new.state), seen(&old.state));
        assert_eq!(new.state.tabs, old.state.tabs);
        assert_eq!(new.state.charsets, old.state.charsets);
        assert_eq!(
            (new.state.cursor_shape, new.state.cursor_blink),
            (old.state.cursor_shape, old.state.cursor_blink)
        );
        assert_eq!(new.state.cursor.pen, old.state.cursor.pen);
        // Both keyboard flags, in order: popping one gives the other.
        for t in [&mut old, &mut new] {
            t.advance(b"\x1b[<1u");
        }
        assert_eq!((new.state.kitty_keyboard_flags(), old.state.kitty_keyboard_flags()), (5, 5));
    }

    #[test]
    fn origin_mode_puts_the_cursor_on_its_row() {
        let mut old = term(20, 8);
        old.advance(b"\x1b[3;6r\x1b[?6h\x1b[2;4H");
        assert_eq!((old.state.cursor.row, old.state.cursor.col), (3, 3));
        let new = handed_over(&old);
        assert_eq!(seen(&new.state), seen(&old.state));
    }

    #[test]
    fn a_full_screen_program_gets_an_empty_alternate_screen_and_its_modes() {
        let mut old = term(20, 5);
        old.advance(b"before\r\n$ vim\r\n\x1b[>1u\x1b[?1049h\x1b[?1h\x1b[?1000h\x1b[>3u\x1b[2;3Heditor text\x1b[4;2H");
        let mut new = handed_over(&old);
        assert!(new.state.alt_active);
        assert_eq!((new.state.cursor.row, new.state.cursor.col), (3, 1));
        assert_eq!(new.state.modes, old.state.modes);
        assert_eq!(new.state.kitty_keyboard_flags(), 3);
        // Its cells aren't carried: it repaints.
        assert!(!new.state.dump().contains("editor"), "{}", new.state.dump());
        // Leaving it shows the shell's screen as it was, the cursor back
        // where the program started, with the primary screen's flags.
        for t in [&mut old, &mut new] {
            t.advance(b"\x1b[?1049l");
        }
        assert_eq!(seen(&new.state), seen(&old.state));
        assert_eq!(new.state.kitty_keyboard_flags(), 1);
    }

    #[test]
    fn blocks_from_another_format_still_leave_the_modes() {
        let mut old = term(20, 5);
        old.advance(b"text\x1b[?2004h\x1b[?1049h");
        let mut live = old.state.save_live(1000, 7);
        live.blocks = b"not blocks".to_vec();
        let mut new = term(20, 5);
        assert!(new.restore_live(&live).is_err());
        assert!(new.state.alt_active && new.state.modes.bracketed_paste);
    }
}
