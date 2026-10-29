//! The alert a pin or the launcher shows when it can't start: an app
//! started from the Dock has no stderr (docs/design/pins.md §3.5).

use crate::cf::{self, Button};
use crate::message;

/// Set (to anything) in tests, so a failing start doesn't wait for a
/// click.
pub const NO_ALERT_ENV: &str = "TD_PIN_NO_ALERT";

/// Shows "<name> can't start" with `text`, and opens the download page if
/// "Get Terminal Decline" is pressed.
pub fn show(name: &str, text: &str) {
    if std::env::var_os(NO_ALERT_ENV).is_some() {
        return;
    }
    if cf::alert(&message::title(name), text, message::GET_BUTTON, message::QUIT_BUTTON) == Button::Default {
        let _ = std::process::Command::new("/usr/bin/open").arg(message::DOWNLOAD_URL).status();
    }
}
