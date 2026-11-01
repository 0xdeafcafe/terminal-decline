//! Completion and colouring at the shell's prompt
//! (docs/design/completion.md): [`lex`] reads the command line into
//! words and says what the cursor is on; [`ink`] colours it;
//! [`sources`] reads what can be offered, off the main thread.

use std::sync::Arc;

pub mod ink;
pub mod lex;
pub mod sources;

/// What a pane keeps for completion and colouring.
#[derive(Default)]
pub struct PaneState {
    /// What its shell can run, as last reported (OSC 7701 `env`).
    pub shell_env: Option<Arc<term_core::ShellEnv>>,
}
