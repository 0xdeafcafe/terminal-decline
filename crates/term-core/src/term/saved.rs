//! History saved across a restart, and put back into a new terminal
//! (the app's `restore_scrollback`; [`crate::scrollback::Scrollback::save`]).

use super::Terminal;
use crate::cell::Cell;
use crate::scrollback::{RestoreError, Restored};

impl Terminal {
    /// The newest `max` lines of history and of the primary screen (its
    /// rows down to the last that isn't blank), saved at `saved_at`
    /// (seconds since 1970) for [`Terminal::restore_history`]. None while
    /// the alternate screen is in use: a full-screen program's screen
    /// isn't history, and what it shows (an editor's buffer, a password
    /// manager) isn't saved.
    pub fn save_history(&self, max: usize, saved_at: u64) -> Option<Vec<u8>> {
        if self.alt_active {
            return None;
        }
        let rows = self
            .screen
            .iter()
            .rposition(|r| r.cells.iter().any(|&c| c != Cell::BLANK))
            .map_or(0, |last| last + 1);
        Some(self.scrollback.save(
            self.screen.iter().take(rows),
            self.styles.styles(),
            self.styles.links().as_slice(),
            max,
            saved_at,
        ))
    }

    /// Put back history [`Terminal::save_history`] saved, into this
    /// terminal before anything is written to it: the lines go to
    /// history, and those that were on the screen come back onto it (as
    /// many as fit above one free row), the cursor on the row below them.
    pub fn restore_history(&mut self, bytes: &[u8]) -> Result<Restored, RestoreError> {
        if self.alt_active || self.top_line != 0 || !self.scrollback.is_empty() {
            return Err(RestoreError::NotEmpty);
        }
        let r = self.scrollback.restore(bytes)?;
        self.top_line = self.scrollback.total();
        let from = self.scrollback.first_visible();
        self.pull_back(r.screen.min(self.lines.saturating_sub(1)));
        self.scrollback.hide_before(from);
        Ok(r)
    }
}
