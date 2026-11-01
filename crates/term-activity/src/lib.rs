//! What each pane's shell is doing, as plain data and decisions with no
//! AppKit: [`activity`] (OSC 133 marks to a command running or just
//! finished, the status title, whether to notify), [`prompts`] (jumping
//! between prompts, selecting a command's output) and [`running`] (the
//! program in the foreground, for asking before a close and for saving
//! its command line for a restart).

pub mod activity;
pub mod prompts;
pub mod running;

/// The product's name as people see it, in menus, Services, questions and
/// the titles [`activity::status_title`] makes. The one place it is
/// spelled in the code; the bundle's own name is set in
/// `scripts/bundle.sh`.
pub const PRODUCT_NAME: &str = "Terminal Decline";

/// The app's bundle id, also its defaults domain and os_log subsystem.
/// Spelled here with the name, for the crates that need it too (the
/// updater runs only in this bundle).
pub const BUNDLE_ID: &str = "red.forbes.terminal-decline";
