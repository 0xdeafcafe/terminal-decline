//! The frozen interface between a pin's stub and the library, mirroring
//! `abi/terminal_decline_pin.h` (docs/design/pins.md §3.5).
//!
//! Each version is its own struct and its own function; none is ever
//! changed or removed. A stub of version N looks up `td_pin_main_vN`, then
//! N − 1, … down to 1 ([`entry_names`]).

use std::ffi::{CStr, c_char, c_int};

/// `TD_PIN_ABI_V1`.
pub const ABI_V1: u32 = 1;

/// The newest interface version this stub knows.
pub const ABI_LATEST: u32 = ABI_V1;

/// `struct td_pin_info_v1`. All pointers stay valid for the life of the
/// process; paths are bytes, not necessarily UTF-8.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PinInfoV1 {
    /// `size_of::<PinInfoV1>()`: a check, not a version.
    pub size: u32,
    /// [`ABI_V1`].
    pub abi: u32,
    pub pin_id: *const c_char,
    pub pin_bundle: *const c_char,
    pub app_bundle: *const c_char,
    pub stub_version: *const c_char,
    pub argc: c_int,
    pub argv: *const *const c_char,
}

// The layout the header gives on arm64 and x86_64 (LP64); a test compiles
// the header and checks the same numbers.
const _: () = {
    assert!(size_of::<PinInfoV1>() == 56);
    assert!(std::mem::offset_of!(PinInfoV1, size) == 0);
    assert!(std::mem::offset_of!(PinInfoV1, abi) == 4);
    assert!(std::mem::offset_of!(PinInfoV1, pin_id) == 8);
    assert!(std::mem::offset_of!(PinInfoV1, pin_bundle) == 16);
    assert!(std::mem::offset_of!(PinInfoV1, app_bundle) == 24);
    assert!(std::mem::offset_of!(PinInfoV1, stub_version) == 32);
    assert!(std::mem::offset_of!(PinInfoV1, argc) == 40);
    assert!(std::mem::offset_of!(PinInfoV1, argv) == 48);
};

/// `int td_pin_main_v1(const struct td_pin_info_v1 *)`.
pub type PinMainV1 = unsafe extern "C" fn(*const PinInfoV1) -> c_int;

/// `int td_app_main_v1(int argc, const char *const *argv)`: the app's own
/// launcher, which ships with the library, so this one is not frozen.
pub type AppMainV1 = unsafe extern "C" fn(c_int, *const *const c_char) -> c_int;

/// The library's file name inside `Contents/Frameworks`.
pub const LIBRARY: &str = "libterminal_decline.dylib";

/// The library's signing identifier, part of every requirement a pin
/// carries (§3.4).
pub const LIBRARY_ID: &str = "red.forbes.terminal-decline.lib";

/// The launcher's entry point.
pub const APP_MAIN_V1: &CStr = c"td_app_main_v1";

/// `td_pin_main_v<version>`, NUL-terminated, for `dlsym`.
pub fn entry_name(version: u32) -> Vec<u8> {
    format!("td_pin_main_v{version}\0").into_bytes()
}

/// The entry points a stub that knows `latest` tries, in order: its own
/// version first, then each older one down to 1.
pub fn entry_names(latest: u32) -> impl Iterator<Item = (u32, Vec<u8>)> {
    (1..=latest).rev().map(|v| (v, entry_name(v)))
}

/// The requirement a dev pin (made by an ad-hoc signed app) writes: any
/// library with our identifier. Guards against mistakes, not attacks.
pub fn dev_requirement() -> String {
    format!("identifier \"{LIBRARY_ID}\"")
}

/// The requirement a release pin writes: a library signed by Apple's chain
/// for `team`, with our identifier.
pub fn team_requirement(team: &str) -> String {
    format!("anchor apple generic and certificate leaf[subject.OU] = \"{team}\" and identifier \"{LIBRARY_ID}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_count_down_to_one() {
        let names: Vec<_> = entry_names(3).map(|(v, n)| (v, String::from_utf8(n).unwrap())).collect();
        assert_eq!(
            names,
            [
                (3, "td_pin_main_v3\0".to_string()),
                (2, "td_pin_main_v2\0".to_string()),
                (1, "td_pin_main_v1\0".to_string())
            ]
        );
    }

    #[test]
    fn requirements() {
        assert_eq!(dev_requirement(), r#"identifier "red.forbes.terminal-decline.lib""#);
        assert_eq!(
            team_requirement("6Z49PF6642"),
            r#"anchor apple generic and certificate leaf[subject.OU] = "6Z49PF6642" and identifier "red.forbes.terminal-decline.lib""#
        );
    }
}
