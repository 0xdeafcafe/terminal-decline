//! Updates (docs/design/updater.md). The decisions, updates from source
//! and what is told about them are term-update's, re-exported here under
//! their old names; [`sparkle`] (Sparkle.framework, its driver and the
//! alerts) and the [`Host`] that posts term-update's notifications and
//! answers for the windows stay with the app.

pub use term_update::*;

pub mod sparkle;

use crate::notify::{self, Button, Note, Target};

/// term-update's side of the app: notifications, the programs running in
/// the windows, and whether the restart waits for the app to sit unused.
struct App;

impl Host for App {
    fn show(&self, n: Notice) {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return };
        let target = match n.open {
            Some(open) => Target::Open(open),
            None => Target::App,
        };
        let mut note = Note::new(n.title, n.body, target);
        note.key = n.key;
        note.group = n.group;
        if n.update_button {
            note.button = Some(Button::Update);
        }
        notify::show(mtm, note);
    }

    fn programs_running(&self) -> bool {
        !crate::app::running_programs().is_empty()
    }

    fn set_update_asked(&self, asked: bool) {
        crate::app::set_update_asked(asked);
    }
}

/// Give term-update the app's side; before anything there starts.
pub fn install_host() {
    set_host(&App);
}

impl From<&crate::entry::Role> for Role {
    fn from(role: &crate::entry::Role) -> Role {
        match role {
            crate::entry::Role::App => Role::App,
            crate::entry::Role::Pin(_) => Role::Pin,
            crate::entry::Role::Bare => Role::Bare,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// term-update spells these too.
    #[test]
    fn the_same_names_as_the_app() {
        assert_eq!(super::TEST_CONTROL_ENV, crate::control::ENV);
        let home = Path::new("/Users/x");
        assert_eq!(super::logs_dir(home), crate::crash::logs_dir(home));
    }
}
