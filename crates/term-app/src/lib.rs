//! Terminal Decline: a GPU-accelerated terminal for macOS.
//!
//! The AppKit parts (`app`, `view`) are thin; the logic they use lives in
//! plain modules that are tested on their own: [`session`] (the program, its
//! PTY and threads), [`keys`], [`mouse`], [`select`], [`layout`],
//! [`frame_policy`], [`config`] and [`pin`].

pub mod app;
mod ascii_layout;
pub mod config;
pub mod control;
pub mod crash;
pub mod desktop;
pub mod frame_policy;
pub mod import;
pub mod keybind;
pub mod keys;
pub mod latency;
pub mod layout;
pub mod mouse;
pub mod notify;
pub mod paths;
pub mod pin;
pub mod reload;
pub mod running;
pub mod secure_input;
pub mod select;
pub mod session;
#[cfg(test)]
mod testutil;
pub mod theme;
mod view;
pub mod watch;

pub use paths::Paths;

/// The product's name as people see it, in menus, Services and questions.
/// The one place it is spelled in the code; the bundle's own name is set
/// in `scripts/bundle.sh`.
pub const PRODUCT_NAME: &str = "Terminal Decline";

/// The app's bundle id, also its defaults domain and os_log subsystem.
pub const BUNDLE_ID: &str = "red.forbes.terminal-decline";

/// The bundle id before the rename to Terminal Decline.
pub const OLD_BUNDLE_ID: &str = "red.forbes.decline";

pub const USAGE: &str = "\
usage: terminal-decline [--config FILE] [-e PROGRAM [ARGS...]]
       terminal-decline pin NAME [--icon FILE] [--config FILE] [--cwd DIR] [--dir DIR] [--dock] -- PROGRAM [ARGS...]
       terminal-decline pin --list | --remove NAME | --update
       terminal-decline import ghostty [PATH] [--write [--force]]
       terminal-decline import iterm FILE.itermcolors [--name NAME] [--write [--force]]

Runs a terminal window. With -e, runs PROGRAM instead of your login shell.
`terminal-decline pin` makes PROGRAM its own macOS app, with its own Dock icon.";
