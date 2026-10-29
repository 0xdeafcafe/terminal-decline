//! decline: a GPU-accelerated terminal for macOS.
//!
//! The AppKit parts (`app`, `view`) are thin; the logic they use lives in
//! plain modules that are tested on their own: [`session`] (the program,
//! its PTY and threads), [`keys`], [`mouse`], [`select`], [`layout`], [`frame_policy`],
//! [`config`] and [`pin`].

pub mod app;
mod ascii_layout;
pub mod config;
pub mod control;
pub mod crash;
pub mod frame_policy;
pub mod keybind;
pub mod keys;
pub mod latency;
pub mod layout;
pub mod mouse;
pub mod notify;
pub mod paths;
pub mod pin;
pub mod reload;
pub mod secure_input;
pub mod select;
pub mod session;
#[cfg(test)]
mod testutil;
mod view;
pub mod watch;

pub use paths::Paths;

pub const USAGE: &str = "\
usage: decline [--config FILE] [-e PROGRAM [ARGS...]]
       decline pin NAME [--icon FILE] [--config FILE] [--cwd DIR] [--dir DIR] [--dock] -- PROGRAM [ARGS...]
       decline pin --list | --remove NAME | --update

Runs a terminal window. With -e, runs PROGRAM instead of your login shell.
`decline pin` makes PROGRAM its own macOS app, with its own Dock icon.";
