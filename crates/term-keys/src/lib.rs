//! Keyboard input for Terminal Decline, with no AppKit: [`keys`] turns a
//! key press into bytes, input method or menu; [`keybind`] is the
//! `[keybind]` config; [`ascii_layout`] reads the ASCII-capable keyboard
//! layout (Carbon's Text Input Sources) for Ctrl and Meta on non-Latin
//! layouts. The view builds the inputs from its `NSEvent`s.

pub mod ascii_layout;
pub mod keybind;
pub mod keys;
