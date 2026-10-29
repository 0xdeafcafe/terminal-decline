//! Secure Keyboard Entry (Terminal Decline menu): while it is on and Terminal
//! Decline is the active app, macOS gives its key events to no other process
//! (key loggers, text expanders, other apps' event taps), as in Terminal.app.
//!
//! macOS has one switch for the whole login session, counted per process
//! (`EnableSecureEventInput` / `DisableSecureEventInput`). Left on while
//! Terminal Decline is in the background, it would also block other apps' hot
//! keys and input tools, so it is held only while Terminal Decline is active,
//! and given back when Terminal Decline quits. The choice is kept in user
//! defaults.

/// The user-defaults key holding the choice.
pub const DEFAULTS_KEY: &str = "SecureKeyboardEntry";

/// The defaults domain (bundle id) from before the rename to Terminal
/// Decline for `id`: the app's or a pin's.
pub fn old_domain(id: &str) -> Option<String> {
    if id == crate::BUNDLE_ID {
        return Some(crate::OLD_BUNDLE_ID.into());
    }
    let pin = id.strip_prefix(crate::pin::ID_PREFIX)?;
    Some(format!("{}{pin}", crate::pin::OLD_ID_PREFIX))
}

/// The choice made before the rename: copied from the old defaults domain
/// of `id` (see [`old_domain`]) when `id`'s own has none. Returns whether
/// it copied.
pub fn carry_over(id: &str) -> bool {
    use objc2_core_foundation::{CFPreferencesAppSynchronize, CFPreferencesCopyAppValue, CFPreferencesSetAppValue, CFString};
    let Some(old) = old_domain(id) else { return false };
    let key = CFString::from_str(DEFAULTS_KEY);
    let (old, new) = (CFString::from_str(&old), CFString::from_str(id));
    if CFPreferencesCopyAppValue(&key, &new).is_some() {
        return false;
    }
    let Some(value) = CFPreferencesCopyAppValue(&key, &old) else {
        return false;
    };
    unsafe { CFPreferencesSetAppValue(&key, Some(&value), &new) };
    CFPreferencesAppSynchronize(&new)
}

/// Whether to hold secure input, from the menu choice and whether the app
/// is active. Each change returns the call to make, if any, so enable and
/// disable calls always pair up.
#[derive(Debug, Default)]
pub struct SecureInput {
    wanted: bool,
    active: bool,
    held: bool,
    quitting: bool,
}

/// A call to make to macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    Enable,
    Disable,
}

impl SecureInput {
    pub fn new(wanted: bool) -> SecureInput {
        SecureInput {
            wanted,
            ..SecureInput::default()
        }
    }

    /// The menu item's checkmark.
    pub fn wanted(&self) -> bool {
        self.wanted
    }

    /// Whether secure input is held now.
    pub fn held(&self) -> bool {
        self.held
    }

    pub fn toggle(&mut self) -> Option<Call> {
        self.wanted = !self.wanted;
        self.update()
    }

    pub fn set_active(&mut self, active: bool) -> Option<Call> {
        self.active = active;
        self.update()
    }

    /// The app is quitting: give it back (the choice itself is kept).
    pub fn quit(&mut self) -> Option<Call> {
        self.quitting = true;
        self.update()
    }

    fn update(&mut self) -> Option<Call> {
        let hold = self.wanted && self.active && !self.quitting;
        if hold == self.held {
            return None;
        }
        self.held = hold;
        Some(if hold { Call::Enable } else { Call::Disable })
    }
}

/// Make `call` (Carbon's HIToolbox, part of every macOS).
pub fn apply(call: Option<Call>) {
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn EnableSecureEventInput() -> i32;
        fn DisableSecureEventInput() -> i32;
    }
    match call {
        Some(Call::Enable) => unsafe {
            EnableSecureEventInput();
        },
        Some(Call::Disable) => unsafe {
            DisableSecureEventInput();
        },
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_domains() {
        assert_eq!(old_domain("red.forbes.terminal-decline").as_deref(), Some("red.forbes.decline"));
        assert_eq!(
            old_domain("red.forbes.terminal-decline.pin.rush").as_deref(),
            Some("red.forbes.decline.pin.rush")
        );
        for other in [
            "red.forbes.decline",
            "red.forbes.terminal-decline.latency",
            "com.apple.Terminal",
            "",
        ] {
            assert_eq!(old_domain(other), None, "{other}");
        }
    }

    #[test]
    fn held_only_while_on_and_active() {
        let mut s = SecureInput::new(false);
        assert_eq!(s.set_active(true), None);
        assert_eq!(s.toggle(), Some(Call::Enable));
        assert!(s.wanted());
        // Into the background and back.
        assert_eq!(s.set_active(false), Some(Call::Disable));
        assert_eq!(s.set_active(false), None);
        assert_eq!(s.set_active(true), Some(Call::Enable));
        assert_eq!(s.set_active(true), None, "never enabled twice");
        assert_eq!(s.toggle(), Some(Call::Disable));
        assert!(!s.wanted());
        assert_eq!(s.set_active(false), None);
        // Turned on in the background: held from the next activation.
        assert_eq!(s.toggle(), None);
        assert_eq!(s.set_active(true), Some(Call::Enable));
    }

    #[test]
    fn saved_choice_applies_at_activation_and_is_given_back_on_quit() {
        let mut s = SecureInput::new(true);
        assert_eq!(s.set_active(true), Some(Call::Enable));
        assert_eq!(s.quit(), Some(Call::Disable));
        assert!(s.wanted(), "the choice is kept for next time");
        assert_eq!(s.set_active(true), None, "nothing after quitting");
        // Quitting while in the background has nothing to give back.
        let mut s = SecureInput::new(true);
        assert_eq!(s.quit(), None);
    }

    #[test]
    fn calls_always_pair_up() {
        // Any sequence of events: enables and disables alternate, starting
        // with an enable, and nothing is held after quitting.
        for seq in 0u32..1 << 10 {
            let mut s = SecureInput::new(seq & 1 != 0);
            let mut held = false;
            for bit in 1..10 {
                let call = match (seq >> bit) & 1 {
                    0 => s.toggle(),
                    _ => s.set_active(bit % 2 == 0),
                };
                match call {
                    Some(Call::Enable) => assert!(!held, "{seq:b}"),
                    Some(Call::Disable) => assert!(held, "{seq:b}"),
                    None => {}
                }
                held = matches!(call, Some(Call::Enable)) || (held && call.is_none());
            }
            match s.quit() {
                Some(Call::Disable) => assert!(held),
                None => assert!(!held, "{seq:b}"),
                Some(Call::Enable) => panic!("enable on quit"),
            }
        }
    }
}
