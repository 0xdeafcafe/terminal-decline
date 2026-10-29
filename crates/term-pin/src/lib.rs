//! A Dock pin's stub and the app's launcher (docs/design/pins.md §2.2, §3).
//!
//! A pin is a small bundle that never changes after it is made, so its
//! code signature, and with it every permission macOS gave it, stays the
//! same across Terminal Decline upgrades. Its executable is this crate's
//! stub: it finds the installed Terminal Decline.app, checks the library's
//! signature against a requirement written into the pin, loads the
//! library and calls it through the frozen interface in [`abi`].
//!
//! Only `libc` and hand-written FFI to CoreFoundation and Security: what
//! the stub links strongly is frozen with it.

pub mod abi;
// P-1 uses the rest.
#[allow(dead_code)]
pub mod cf;
pub mod sec;
