//! Terminal Decline: a GPU-accelerated terminal for macOS.
//!
//! The AppKit parts (`app`, `view`) are thin; the logic they use lives in
//! plain modules that are tested on their own: [`session`] (the program, its
//! PTY and threads), [`keys`], [`mouse`], [`select`], [`layout`],
//! [`frame_policy`], [`panes`], [`config`] and [`pin`].

pub mod app;
mod arrange_view;
mod background;
pub mod control;
pub mod crash;
pub mod ctl;
pub mod desktop;
mod divider_view;
pub mod entry;
mod file_menu;
mod find_bar;
pub mod frame_policy;
pub mod hotkey;
pub mod idle_filters;
pub mod images;
pub mod latency;
pub mod layout;
pub mod links;
pub mod memory;
pub mod mouse;
pub mod notify;
mod pane_strip;
mod pane_view;
pub mod paths;
pub mod pin;
pub mod prefs;
pub mod scroll;
pub mod search;
pub mod secure_input;
pub mod select;
pub mod session;
mod size_labels;
#[cfg(test)]
mod testutil;
pub mod update;
mod view;
pub mod watch;
pub mod welcome;

// Moved to crates of their own, under their old names here.
pub use term_activity::{activity, prompts, running};
pub use term_complete as complete;
pub use term_config::{config, quick, theme};
pub use term_import as import;
use term_keys::ascii_layout;
pub use term_keys::{keybind, keys};
pub use term_restore::{panes, reload, saved_history};

pub use entry::{PinInfo, Role, run};
pub use paths::Paths;

/// The product's name as people see it (spelled in term-activity, whose
/// titles end with it).
pub use term_activity::PRODUCT_NAME;

/// The app's bundle id, also its defaults domain and os_log subsystem
/// (spelled in term-activity, with the name).
pub use term_activity::BUNDLE_ID;

/// The bundle id before the rename to Terminal Decline.
pub const OLD_BUNDLE_ID: &str = "red.forbes.decline";

pub const USAGE: &str = "\
usage: terminal-decline [--config FILE] [-e PROGRAM [ARGS...]]
       terminal-decline pin NAME [--icon FILE] [--config FILE] [--cwd DIR] [--dir DIR] [--dock] [-- PROGRAM [ARGS...]]
       terminal-decline pin --list [--json] | --edit NAME | --remove NAME
       terminal-decline pin --repair [NAME] [--yes] | --update [--yes]
       terminal-decline import ghostty [PATH] [--write [--force]]
       terminal-decline import iterm FILE.itermcolors [--name NAME] [--write [--force]]
       terminal-decline settings [--layer FILE]... [--edit N]   (also `prefs`)
       terminal-decline ctl COMMAND [ARGS...]   (drive terminals from agents; ctl --help)

Runs a terminal window. With -e, runs PROGRAM instead of your login shell.
`terminal-decline pin` makes PROGRAM (NAME if none is given) its own macOS app,
with its own Dock icon: FILE, else the program's own (its app bundle's, or
NAME.png, NAME.icns, icon.png or icon.icns beside it), else one drawn for it.
In the app's shells, `decline` runs this binary.";
