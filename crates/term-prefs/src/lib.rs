//! The preferences: a terminal program Terminal Decline runs in a tab of
//! its own (`terminal-decline prefs`, Cmd+,). No platform code: it edits
//! `config.toml` and gets what it needs from the app through [`Host`].
//! See docs/design/preferences.md.
//!
//! - [`schema`]: every setting, its section, editor and whether running
//!   windows take it on.
//! - [`file`]: safe edits to a config file with `toml_edit`.
//! - [`layers`]: the values shown, and where each comes from.

pub mod file;
pub mod host;
pub mod layers;
pub mod schema;

pub use host::{Action, Checked, Fake, Font, Host, Pin};
