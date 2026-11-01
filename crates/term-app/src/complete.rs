//! Completion and colouring at the shell's prompt
//! (docs/design/completion.md): [`lex`] reads the command line into
//! words and says what the cursor is on; [`ink`] colours it.

pub mod ink;
pub mod lex;
