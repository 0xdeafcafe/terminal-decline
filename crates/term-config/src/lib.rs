//! Terminal Decline's config, with no AppKit: [`config`] (config.toml,
//! its layers and what each setting means), [`theme`] (colour themes,
//! built in and the user's) and [`quick`] (the quick terminal's hotkey,
//! edge and size). The app reads them; `terminal-decline settings`
//! edits the file through term-prefs.

pub mod config;
pub mod quick;
#[cfg(test)]
mod testutil;
pub mod theme;

use term_keys::{keybind, keys};
