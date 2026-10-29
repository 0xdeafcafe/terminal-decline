//! What the stub and the launcher say when they can't start. Frozen with
//! the stub (FROZEN.md): a pin made today shows these texts for good.

/// Where "Get Terminal Decline" goes (pins.md §11, question 8).
pub const DOWNLOAD_URL: &str = "https://github.com/0xdeafcafe/terminal-decline/releases";

pub const GET_BUTTON: &str = "Get Terminal Decline";
pub const QUIT_BUTTON: &str = "Quit";

/// The alert's title.
pub fn title(name: &str) -> String {
    format!("{name} can't start")
}

/// Why a start failed, from most to least specific: when several apps
/// were tried, the furthest any of them got is what the user is told.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Failure {
    /// No candidate was Terminal Decline with a library.
    NotInstalled,
    /// `dlopen` refused the library.
    LoadFailed { library: String, why: String },
    /// The library failed its signature check.
    CheckFailed { library: String, why: String },
    /// The library has none of the interface versions this pin can use.
    NeedsNewer,
    /// The pin's own Info.plist is missing a key or has the wrong type.
    Damaged(String),
}

impl Failure {
    /// The message, in the alert and on stderr.
    pub fn text(&self, name: &str) -> String {
        match self {
            Failure::Damaged(why) => {
                format!("{name} is damaged ({why}). Run `terminal-decline pin --repair` in a terminal to rebuild it.")
            }
            Failure::NotInstalled => format!(
                "{name} is a Terminal Decline pin, and Terminal Decline isn't installed. Install Terminal Decline in Applications, then open {name} again."
            ),
            Failure::CheckFailed { library, why } => format!(
                "The Terminal Decline library at {library} fails its signature check: {why}. Reinstall Terminal Decline, then open {name} again."
            ),
            Failure::LoadFailed { library, why } => {
                format!(
                    "The Terminal Decline library at {library} can't be loaded: {why}. Reinstall Terminal Decline, then open {name} again."
                )
            }
            Failure::NeedsNewer => format!(
                "{name} needs a newer Terminal Decline. Update it from the Terminal Decline menu, or run `brew upgrade --greedy terminal-decline`."
            ),
        }
    }
}

/// The launcher's message when the library or its entry point is missing.
pub fn launcher_failure(library: &str, why: &str) -> String {
    format!("The Terminal Decline library at {library} {why}. Reinstall Terminal Decline.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_furthest_failure_wins() {
        let check = Failure::CheckFailed {
            library: "l".into(),
            why: "w".into(),
        };
        let load = Failure::LoadFailed {
            library: "l".into(),
            why: "w".into(),
        };
        assert!(Failure::NeedsNewer > check);
        assert!(check > load);
        assert!(load > Failure::NotInstalled);
    }
}
