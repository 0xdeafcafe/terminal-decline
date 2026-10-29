//! Keyboard input to bytes, following xterm.
//!
//! [`translate`] decides what a key press does: send bytes to the program,
//! go to the input method (text, dead keys, IME), or go to the menus (Cmd
//! shortcuts). The view only builds a [`KeyInput`] from the `NSEvent`.

use crate::config::OptionAsMeta;

/// `NSEventModifierFlags` bits used here.
pub mod flags {
    /// Device-dependent bit: the left Option key is held.
    pub const LEFT_OPTION: usize = 0x20;
    /// Device-dependent bit: the right Option key is held.
    pub const RIGHT_OPTION: usize = 0x40;
    pub const SHIFT: usize = 1 << 17;
    pub const CONTROL: usize = 1 << 18;
    pub const OPTION: usize = 1 << 19;
    pub const COMMAND: usize = 1 << 20;
    pub const NUMERIC_PAD: usize = 1 << 21;
}

/// A key press, as read from the `NSEvent`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyInput {
    /// macOS virtual key code.
    pub keycode: u16,
    /// `characters`: the text the key types with all modifiers applied.
    pub chars: String,
    /// `charactersIgnoringModifiers`.
    pub chars_ignoring_mods: String,
    /// The characters with only Shift applied (not Option), for
    /// Option-as-Meta.
    pub chars_shift_only: String,
    /// What the key types on the ASCII-capable layout (see
    /// [`crate::ascii_layout`]), without and with Shift: used for Ctrl and
    /// Meta when the current layout's character isn't ASCII (Ctrl+С on a
    /// Russian layout is Ctrl+C).
    pub ascii: Option<char>,
    pub ascii_shifted: Option<char>,
    /// Raw `NSEventModifierFlags` bits, including the left/right Option
    /// device bits; see [`flags`].
    pub flags: usize,
    /// The key is on the numeric keypad (`NSEventModifierFlagNumericPad`).
    pub numpad: bool,
}

/// What the terminal and settings say about how keys are encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    /// DECCKM: arrows send SS3 sequences.
    pub app_cursor: bool,
    /// DECKPAM: the keypad sends SS3 sequences.
    pub app_keypad: bool,
    /// Kitty keyboard protocol flags (0 = legacy).
    pub kitty_flags: u8,
    pub option_as_meta: OptionAsMeta,
    /// An input method is composing text; every key goes to it.
    pub composing: bool,
}

/// What to do with a key press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Send these bytes to the program.
    Send(Vec<u8>),
    /// Hand the event to the input system (`interpretKeyEvents`).
    ToTextInput,
    /// A Cmd shortcut: the menus handle it; nothing goes to the program.
    ToMenu,
}

/// Decide what a key press does.
pub fn translate(input: &KeyInput, modes: KeyModes) -> KeyAction {
    if input.flags & flags::COMMAND != 0 {
        // Cmd shortcuts belong to the menus and are never sent to the
        // program, except the Mac line-editing ones.
        let bytes: &[u8] = match input.keycode {
            0x7B => b"\x01", // Cmd+Left: start of line
            0x7C => b"\x05", // Cmd+Right: end of line
            0x33 => b"\x15", // Cmd+Backspace: delete line
            _ => return KeyAction::ToMenu,
        };
        return KeyAction::Send(bytes.to_vec());
    }
    if modes.composing {
        return KeyAction::ToTextInput;
    }
    match key_bytes(input, modes) {
        Some(b) => KeyAction::Send(b),
        None => KeyAction::ToTextInput,
    }
}

/// Bytes for keys that bypass text input: special keys, Ctrl combos and
/// Option-as-Meta. `None`: the key types text.
fn key_bytes(input: &KeyInput, modes: KeyModes) -> Option<Vec<u8>> {
    let f = input.flags;
    // Device-dependent bits for which Option key is held.
    let (left_alt, right_alt) = (f & flags::LEFT_OPTION != 0, f & flags::RIGHT_OPTION != 0);
    let meta = match modes.option_as_meta {
        OptionAsMeta::Left => left_alt,
        OptionAsMeta::Right => right_alt,
        OptionAsMeta::Both => left_alt || right_alt,
        OptionAsMeta::None => false,
    };
    let mods = Mods {
        shift: f & flags::SHIFT != 0,
        alt: f & flags::OPTION != 0,
        ctrl: f & flags::CONTROL != 0,
    };
    let code = input.keycode;
    let kitty = modes.kitty_flags & 1 != 0;
    let keypad = input.numpad && !(0x7B..=0x7E).contains(&code);
    if keypad
        && modes.app_keypad
        && !mods.any()
        && let Some(b) = keypad_app_bytes(code)
    {
        return Some(b.to_vec());
    }
    if let Some(key) = key_from_keycode(code) {
        if kitty && let Some(b) = encode_kitty_key(key, mods) {
            return Some(b);
        }
        return Some(encode_key(key, mods, modes.app_cursor));
    }
    let c = input.chars_ignoring_mods.chars().next()?;
    if kitty && (mods.ctrl || meta) && !c.is_control() {
        let mods = Mods { alt: meta, ..mods };
        return Some(encode_kitty_char(c, mods));
    }
    if mods.ctrl
        && let Some(b) = ctrl_byte(ascii_or(c, input, mods.shift))
    {
        let mut out = if meta { vec![0x1b] } else { Vec::new() };
        out.push(b);
        return Some(out);
    }
    if meta {
        // The character with Shift applied but not Option.
        let text = if input.chars_shift_only.is_empty() {
            &input.chars_ignoring_mods
        } else {
            &input.chars_shift_only
        };
        let mut out = vec![0x1b];
        match text.chars().next() {
            Some(t) if !t.is_ascii() => {
                let mut buf = [0; 4];
                out.extend_from_slice(ascii_or(t, input, mods.shift).encode_utf8(&mut buf).as_bytes());
            }
            _ => out.extend_from_slice(text.as_bytes()),
        }
        return Some(out);
    }
    None
}

/// `c` if it is ASCII, else what the key types on the ASCII-capable layout
/// (with Shift if `shift`), for Ctrl and Meta on non-Latin layouts.
fn ascii_or(c: char, input: &KeyInput, shift: bool) -> char {
    if c.is_ascii() {
        return c;
    }
    let ascii = if shift { input.ascii_shifted.or(input.ascii) } else { input.ascii };
    ascii.unwrap_or(c)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Backspace,
    Tab,
    Enter,
    Escape,
    F(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Mods {
    /// xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }

    pub fn any(self) -> bool {
        self.shift || self.alt || self.ctrl
    }
}

/// macOS virtual key codes for keys that don't produce text.
pub fn key_from_keycode(code: u16) -> Option<Key> {
    Some(match code {
        0x7E => Key::Up,
        0x7D => Key::Down,
        0x7B => Key::Left,
        0x7C => Key::Right,
        0x73 => Key::Home,
        0x77 => Key::End,
        0x74 => Key::PageUp,
        0x79 => Key::PageDown,
        0x72 => Key::Insert,
        0x75 => Key::Delete,
        0x33 => Key::Backspace,
        0x30 => Key::Tab,
        0x24 | 0x4C => Key::Enter,
        0x35 => Key::Escape,
        0x7A => Key::F(1),
        0x78 => Key::F(2),
        0x63 => Key::F(3),
        0x76 => Key::F(4),
        0x60 => Key::F(5),
        0x61 => Key::F(6),
        0x62 => Key::F(7),
        0x64 => Key::F(8),
        0x65 => Key::F(9),
        0x6D => Key::F(10),
        0x67 => Key::F(11),
        0x6F => Key::F(12),
        _ => return None,
    })
}

pub fn encode_key(key: Key, mods: Mods, app_cursor: bool) -> Vec<u8> {
    let m = mods.param();
    let csi_letter = |letter: char| {
        if mods.any() {
            format!("\x1b[1;{m}{letter}")
        } else if app_cursor {
            format!("\x1bO{letter}")
        } else {
            format!("\x1b[{letter}")
        }
    };
    let tilde = |n: u8| {
        if mods.any() {
            format!("\x1b[{n};{m}~")
        } else {
            format!("\x1b[{n}~")
        }
    };
    let esc_if_alt = |b: &[u8]| {
        let mut v = if mods.alt { vec![0x1b] } else { Vec::new() };
        v.extend_from_slice(b);
        v
    };
    let s = match key {
        Key::Up => csi_letter('A'),
        Key::Down => csi_letter('B'),
        Key::Right => csi_letter('C'),
        Key::Left => csi_letter('D'),
        Key::Home => csi_letter('H'),
        Key::End => csi_letter('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => {
            let letter = (b'P' + n - 1) as char;
            if mods.any() {
                format!("\x1b[1;{m}{letter}")
            } else {
                format!("\x1bO{letter}")
            }
        }
        Key::F(n) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                _ => 24,
            };
            tilde(code)
        }
        Key::Backspace => return esc_if_alt(if mods.ctrl { b"\x08" } else { b"\x7f" }),
        Key::Tab => {
            return if mods.shift { b"\x1b[Z".to_vec() } else { esc_if_alt(b"\t") };
        }
        Key::Enter => return esc_if_alt(b"\r"),
        Key::Escape => return esc_if_alt(b"\x1b"),
    };
    s.into_bytes()
}

/// Application keypad mode (DECKPAM): keypad keys send SS3 sequences so
/// programs can tell them from the main keyboard.
pub fn keypad_app_bytes(code: u16) -> Option<&'static [u8]> {
    Some(match code {
        0x52 => b"\x1bOp",
        0x53 => b"\x1bOq",
        0x54 => b"\x1bOr",
        0x55 => b"\x1bOs",
        0x56 => b"\x1bOt",
        0x57 => b"\x1bOu",
        0x58 => b"\x1bOv",
        0x59 => b"\x1bOw",
        0x5B => b"\x1bOx",
        0x5C => b"\x1bOy",
        0x41 => b"\x1bOn",
        0x43 => b"\x1bOj",
        0x45 => b"\x1bOk",
        0x4B => b"\x1bOo",
        0x4E => b"\x1bOm",
        0x51 => b"\x1bOX",
        0x4C => b"\x1bOM",
        _ => return None,
    })
}

/// Kitty keyboard protocol, "disambiguate" level (flag 1): keys whose legacy
/// bytes are ambiguous are sent as `CSI code ; mods u`. This is what lets
/// programs see Shift+Enter, Ctrl+Enter, Ctrl+Tab, and Esc apart from Alt.
pub fn encode_kitty_key(key: Key, mods: Mods) -> Option<Vec<u8>> {
    let code = match key {
        Key::Escape => 27,
        Key::Enter => 13,
        Key::Tab => 9,
        Key::Backspace => 127,
        _ => return None,
    };
    if key != Key::Escape && !mods.any() {
        return None;
    }
    Some(
        if mods.any() {
            format!("\x1b[{code};{}u", mods.param())
        } else {
            format!("\x1b[{code}u")
        }
        .into_bytes(),
    )
}

/// Kitty protocol for Ctrl/Alt with a character key.
pub fn encode_kitty_char(c: char, mods: Mods) -> Vec<u8> {
    let c = c.to_lowercase().next().unwrap_or(c);
    format!("\x1b[{};{}u", c as u32, mods.param()).into_bytes()
}

/// Ctrl plus a printable key, as a control byte.
pub fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | ' ' | '2' | '`' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' | '~' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

/// Text for the program when pasting: newlines become carriage returns, and
/// with bracketed paste the text is wrapped so the program knows it's pasted.
/// Escape characters are removed so pasted text can't end the bracket early.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let cleaned = text.replace("\r\n", "\r").replace('\n', "\r");
    // Control characters other than tab and newline could run commands or
    // end the bracket early; drop them (C1 controls too).
    let safe: String = cleaned.chars().filter(|&c| c == '\t' || c == '\r' || !(c.is_control())).collect();
    let mut out = Vec::with_capacity(safe.len() + 12);
    if bracketed {
        out.extend_from_slice(b"\x1b[200~");
        out.extend_from_slice(safe.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
    } else {
        out.extend_from_slice(safe.as_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    //! Tables of key presses through `translate`, the same path the view
    //! uses. Expected bytes are written out, not computed with the code
    //! under test.

    use super::flags::*;
    use super::*;

    fn key(keycode: u16, flags: usize) -> KeyInput {
        KeyInput {
            keycode,
            flags,
            ..KeyInput::default()
        }
    }

    fn text(c: &str, flags: usize) -> KeyInput {
        KeyInput {
            keycode: 0,
            chars: c.into(),
            chars_ignoring_mods: c.into(),
            chars_shift_only: c.into(),
            flags,
            ..KeyInput::default()
        }
    }

    fn modes(app_cursor: bool) -> KeyModes {
        KeyModes {
            app_cursor,
            ..KeyModes::default()
        }
    }

    fn send(b: &[u8]) -> KeyAction {
        KeyAction::Send(b.to_vec())
    }

    fn show(a: &KeyAction) -> String {
        match a {
            KeyAction::Send(b) => format!("Send({:?})", String::from_utf8_lossy(b)),
            other => format!("{other:?}"),
        }
    }

    /// Modifier combinations and xterm's parameter for each.
    const MODS: [(&str, usize, u8); 8] = [
        ("none", 0, 1),
        ("shift", SHIFT, 2),
        ("alt", OPTION | LEFT_OPTION, 3),
        ("shift+alt", SHIFT | OPTION | LEFT_OPTION, 4),
        ("ctrl", CONTROL, 5),
        ("shift+ctrl", SHIFT | CONTROL, 6),
        ("alt+ctrl", OPTION | RIGHT_OPTION | CONTROL, 7),
        ("shift+alt+ctrl", SHIFT | OPTION | LEFT_OPTION | CONTROL, 8),
    ];

    #[test]
    fn special_keys_every_modifier_both_cursor_modes() {
        // (name, keycode, final letter): CSI/SS3 letter keys.
        let letters = [
            ("up", 0x7E, 'A'),
            ("down", 0x7D, 'B'),
            ("right", 0x7C, 'C'),
            ("left", 0x7B, 'D'),
            ("home", 0x73, 'H'),
            ("end", 0x77, 'F'),
        ];
        // (name, keycode, number): CSI n ~ keys.
        let tildes = [
            ("insert", 0x72, 2),
            ("delete", 0x75, 3),
            ("page up", 0x74, 5),
            ("page down", 0x79, 6),
        ];
        let mut rows = 0;
        for app_cursor in [false, true] {
            for (mod_name, flags, m) in MODS {
                for (name, code, letter) in letters {
                    let want = match (m, app_cursor) {
                        (1, false) => format!("\x1b[{letter}"),
                        (1, true) => format!("\x1bO{letter}"),
                        _ => format!("\x1b[1;{m}{letter}"),
                    };
                    let got = translate(&key(code, flags), modes(app_cursor));
                    assert_eq!(got, send(want.as_bytes()), "{mod_name}+{name} app_cursor={app_cursor}");
                    // The arrows also arrive with the numeric-pad flag set.
                    let numpad = KeyInput {
                        numpad: true,
                        ..key(code, flags)
                    };
                    let keypad_modes = KeyModes {
                        app_keypad: true,
                        ..modes(app_cursor)
                    };
                    assert_eq!(translate(&numpad, keypad_modes), got, "numpad {mod_name}+{name}");
                    rows += 1;
                }
                for (name, code, n) in tildes {
                    let want = if m == 1 { format!("\x1b[{n}~") } else { format!("\x1b[{n};{m}~") };
                    assert_eq!(
                        translate(&key(code, flags), modes(app_cursor)),
                        send(want.as_bytes()),
                        "{mod_name}+{name} app_cursor={app_cursor}"
                    );
                    rows += 1;
                }
            }
        }
        assert_eq!(rows, 2 * 8 * 10);
    }

    #[test]
    fn editing_keys() {
        let alt = OPTION | LEFT_OPTION;
        let rows: &[(&str, u16, usize, &[u8])] = &[
            ("backspace", 0x33, 0, b"\x7f"),
            ("shift+backspace", 0x33, SHIFT, b"\x7f"),
            ("alt+backspace", 0x33, alt, b"\x1b\x7f"),
            ("ctrl+backspace", 0x33, CONTROL, b"\x08"),
            ("ctrl+alt+backspace", 0x33, CONTROL | alt, b"\x1b\x08"),
            ("tab", 0x30, 0, b"\t"),
            ("shift+tab", 0x30, SHIFT, b"\x1b[Z"),
            ("alt+tab", 0x30, alt, b"\x1b\t"),
            ("ctrl+tab", 0x30, CONTROL, b"\t"),
            ("enter", 0x24, 0, b"\r"),
            ("keypad enter", 0x4C, 0, b"\r"),
            ("shift+enter", 0x24, SHIFT, b"\r"),
            ("alt+enter", 0x24, alt, b"\x1b\r"),
            ("escape", 0x35, 0, b"\x1b"),
            ("alt+escape", 0x35, alt, b"\x1b\x1b"),
        ];
        for &(name, code, flags, want) in rows {
            for app_cursor in [false, true] {
                assert_eq!(translate(&key(code, flags), modes(app_cursor)), send(want), "{name}");
            }
        }
    }

    #[test]
    fn function_keys() {
        // (keycode, plain, with shift)
        let rows: [(u16, &[u8], &[u8]); 12] = [
            (0x7A, b"\x1bOP", b"\x1b[1;2P"),
            (0x78, b"\x1bOQ", b"\x1b[1;2Q"),
            (0x63, b"\x1bOR", b"\x1b[1;2R"),
            (0x76, b"\x1bOS", b"\x1b[1;2S"),
            (0x60, b"\x1b[15~", b"\x1b[15;2~"),
            (0x61, b"\x1b[17~", b"\x1b[17;2~"),
            (0x62, b"\x1b[18~", b"\x1b[18;2~"),
            (0x64, b"\x1b[19~", b"\x1b[19;2~"),
            (0x65, b"\x1b[20~", b"\x1b[20;2~"),
            (0x6D, b"\x1b[21~", b"\x1b[21;2~"),
            (0x67, b"\x1b[23~", b"\x1b[23;2~"),
            (0x6F, b"\x1b[24~", b"\x1b[24;2~"),
        ];
        for (i, (code, plain, shifted)) in rows.into_iter().enumerate() {
            for app_cursor in [false, true] {
                assert_eq!(translate(&key(code, 0), modes(app_cursor)), send(plain), "F{}", i + 1);
                assert_eq!(translate(&key(code, SHIFT), modes(app_cursor)), send(shifted), "shift+F{}", i + 1);
            }
        }
        assert_eq!(translate(&key(0x6F, CONTROL), modes(false)), send(b"\x1b[24;5~"));
        assert_eq!(translate(&key(0x7A, CONTROL | SHIFT), modes(false)), send(b"\x1b[1;6P"));
    }

    #[test]
    fn keypad_in_app_mode() {
        let rows: [(u16, &str, &[u8]); 17] = [
            (0x52, "0", b"\x1bOp"),
            (0x53, "1", b"\x1bOq"),
            (0x54, "2", b"\x1bOr"),
            (0x55, "3", b"\x1bOs"),
            (0x56, "4", b"\x1bOt"),
            (0x57, "5", b"\x1bOu"),
            (0x58, "6", b"\x1bOv"),
            (0x59, "7", b"\x1bOw"),
            (0x5B, "8", b"\x1bOx"),
            (0x5C, "9", b"\x1bOy"),
            (0x41, ".", b"\x1bOn"),
            (0x43, "*", b"\x1bOj"),
            (0x45, "+", b"\x1bOk"),
            (0x4B, "/", b"\x1bOo"),
            (0x4E, "-", b"\x1bOm"),
            (0x51, "=", b"\x1bOX"),
            (0x4C, "\r", b"\x1bOM"),
        ];
        let app = KeyModes {
            app_keypad: true,
            ..KeyModes::default()
        };
        for (code, c, want) in rows {
            let input = KeyInput {
                keycode: code,
                numpad: true,
                ..text(c, 0)
            };
            assert_eq!(translate(&input, app), send(want), "keypad {c:?}");
            // Not in app mode: the key types its character (Enter sends CR).
            let normal = translate(&input, KeyModes::default());
            if code == 0x4C {
                assert_eq!(normal, send(b"\r"));
            } else {
                assert_eq!(normal, KeyAction::ToTextInput, "keypad {c:?}");
            }
            // With a modifier, app mode doesn't apply.
            let shifted = KeyInput {
                flags: SHIFT,
                ..input.clone()
            };
            if code != 0x4C {
                assert_eq!(translate(&shifted, app), KeyAction::ToTextInput, "shift+keypad {c:?}");
            }
        }
        // The same keys on the main keyboard are never keypad keys.
        assert_eq!(translate(&text("1", 0), app), KeyAction::ToTextInput);
    }

    #[test]
    fn ctrl_table_for_all_ascii() {
        let mut want = std::collections::HashMap::new();
        for c in 'a'..='z' {
            want.insert(c, c as u8 - b'a' + 1);
            want.insert(c.to_ascii_uppercase(), c as u8 - b'a' + 1);
        }
        for (chars, byte) in [
            ("@ 2`", 0x00),
            ("[3", 0x1b),
            ("\\4", 0x1c),
            ("]5", 0x1d),
            ("^6~", 0x1e),
            ("_-/7", 0x1f),
            ("8?", 0x7f),
        ] {
            for c in chars.chars() {
                want.insert(c, byte);
            }
        }
        for b in 0x20u8..0x7f {
            let c = b as char;
            let got = translate(&text(&c.to_string(), CONTROL), KeyModes::default());
            let expected = match want.get(&c) {
                Some(&byte) => send(&[byte]),
                None => KeyAction::ToTextInput,
            };
            assert_eq!(got, expected, "ctrl+{c:?}");
        }
        // Ctrl with Option-as-Meta: ESC first.
        let meta = KeyModes {
            option_as_meta: OptionAsMeta::Both,
            ..KeyModes::default()
        };
        assert_eq!(translate(&text("c", CONTROL | OPTION | LEFT_OPTION), meta), send(b"\x1b\x03"));
    }

    #[test]
    fn option_as_meta_sides() {
        let left = OPTION | LEFT_OPTION;
        let right = OPTION | RIGHT_OPTION;
        let press = |flags: usize, shift_only: &str| KeyInput {
            keycode: 0,
            chars: "å".into(),
            chars_ignoring_mods: "a".into(),
            chars_shift_only: shift_only.into(),
            flags,
            ..KeyInput::default()
        };
        // (setting, left Option is Meta, right Option is Meta)
        for (setting, l, r) in [
            (OptionAsMeta::Left, true, false),
            (OptionAsMeta::Right, false, true),
            (OptionAsMeta::Both, true, true),
            (OptionAsMeta::None, false, false),
        ] {
            let m = KeyModes {
                option_as_meta: setting,
                ..KeyModes::default()
            };
            for (side, flags, is_meta) in [("left", left, l), ("right", right, r), ("both", left | right, l || r)] {
                let want = |b: &[u8]| if is_meta { send(b) } else { KeyAction::ToTextInput };
                assert_eq!(translate(&press(flags, "a"), m), want(b"\x1ba"), "{setting:?} {side}");
                assert_eq!(translate(&press(flags | SHIFT, "A"), m), want(b"\x1bA"), "{setting:?} shift+{side}");
            }
        }
        // Meta with no shift-only characters known: the plain ones.
        let m = KeyModes {
            option_as_meta: OptionAsMeta::Left,
            ..KeyModes::default()
        };
        assert_eq!(translate(&press(left, ""), m), send(b"\x1ba"));
        // Special keys with Option are Alt whatever the setting.
        assert_eq!(translate(&key(0x7B, right), m), send(b"\x1b[1;3D"));
    }

    /// A key on a non-Latin layout: `here` is what the current layout
    /// types, `ascii` what the ASCII-capable layout does.
    fn non_latin(here: &str, shifted: &str, ascii: char, ascii_shifted: char, flags: usize) -> KeyInput {
        let shift = flags & SHIFT != 0;
        KeyInput {
            keycode: 8,
            chars: if shift { shifted.into() } else { here.into() },
            chars_ignoring_mods: if shift { shifted.into() } else { here.into() },
            chars_shift_only: if shift { shifted.into() } else { here.into() },
            ascii: Some(ascii),
            ascii_shifted: Some(ascii_shifted),
            flags,
            ..KeyInput::default()
        }
    }

    #[test]
    fn ctrl_and_meta_on_non_latin_layouts() {
        let meta = KeyModes {
            option_as_meta: OptionAsMeta::Both,
            ..KeyModes::default()
        };
        let alt = OPTION | LEFT_OPTION;
        let rows: &[(&str, KeyInput, KeyModes, KeyAction)] = &[
            // Russian: the C key types с / С.
            (
                "ru ctrl+с",
                non_latin("с", "С", 'c', 'C', CONTROL),
                KeyModes::default(),
                send(b"\x03"),
            ),
            (
                "ru ctrl+shift+с",
                non_latin("с", "С", 'c', 'C', CONTROL | SHIFT),
                KeyModes::default(),
                send(b"\x03"),
            ),
            ("ru meta+с", non_latin("с", "С", 'c', 'C', alt), meta, send(b"\x1bc")),
            ("ru meta+shift+с", non_latin("с", "С", 'c', 'C', alt | SHIFT), meta, send(b"\x1bC")),
            (
                "ru ctrl+meta+с",
                non_latin("с", "С", 'c', 'C', CONTROL | alt),
                meta,
                send(b"\x1b\x03"),
            ),
            // Greek: the C key types ψ.
            (
                "el ctrl+ψ",
                non_latin("ψ", "Ψ", 'c', 'C', CONTROL),
                KeyModes::default(),
                send(b"\x03"),
            ),
            ("el meta+ψ", non_latin("ψ", "Ψ", 'c', 'C', alt), meta, send(b"\x1bc")),
            // The [ key on the Russian layout types х: Ctrl+[ is ESC.
            (
                "ru ctrl+х",
                non_latin("х", "Х", '[', '{', CONTROL),
                KeyModes::default(),
                send(b"\x1b"),
            ),
            // Without Ctrl or Meta the layout's own text is typed.
            ("ru с", non_latin("с", "С", 'c', 'C', 0), meta, KeyAction::ToTextInput),
            (
                "ru option+с",
                non_latin("с", "С", 'c', 'C', alt),
                KeyModes::default(),
                KeyAction::ToTextInput,
            ),
        ];
        for (name, input, m, want) in rows {
            assert_eq!(show(&translate(input, *m)), show(want), "{name}");
        }
        // An ASCII character on the current layout wins (Dvorak, AZERTY).
        let dvorak = KeyInput {
            ascii: Some('i'),
            ..text("j", CONTROL)
        };
        assert_eq!(translate(&dvorak, KeyModes::default()), send(b"\x0a"));
        // No ASCII-capable layout known: the character as it is.
        let unknown = KeyInput {
            ascii: None,
            ascii_shifted: None,
            ..non_latin("с", "С", 'c', 'C', CONTROL)
        };
        assert_eq!(translate(&unknown, KeyModes::default()), KeyAction::ToTextInput);
        let unknown_meta = KeyInput { flags: alt, ..unknown };
        assert_eq!(translate(&unknown_meta, meta), send("\x1bс".as_bytes()));
    }

    #[test]
    fn kitty_disambiguate() {
        let kitty = KeyModes {
            kitty_flags: 1,
            ..KeyModes::default()
        };
        let meta = KeyModes {
            option_as_meta: OptionAsMeta::Left,
            ..kitty
        };
        let alt = OPTION | LEFT_OPTION;
        let rows: &[(&str, KeyInput, KeyModes, &[u8])] = &[
            ("enter", key(0x24, 0), kitty, b"\r"),
            ("shift+enter", key(0x24, SHIFT), kitty, b"\x1b[13;2u"),
            ("ctrl+enter", key(0x24, CONTROL), kitty, b"\x1b[13;5u"),
            ("alt+enter", key(0x24, alt), kitty, b"\x1b[13;3u"),
            ("tab", key(0x30, 0), kitty, b"\t"),
            ("shift+tab", key(0x30, SHIFT), kitty, b"\x1b[9;2u"),
            ("ctrl+tab", key(0x30, CONTROL), kitty, b"\x1b[9;5u"),
            ("ctrl+shift+tab", key(0x30, CONTROL | SHIFT), kitty, b"\x1b[9;6u"),
            ("backspace", key(0x33, 0), kitty, b"\x7f"),
            ("alt+backspace", key(0x33, alt), kitty, b"\x1b[127;3u"),
            ("ctrl+backspace", key(0x33, CONTROL), kitty, b"\x1b[127;5u"),
            ("escape", key(0x35, 0), kitty, b"\x1b[27u"),
            ("shift+escape", key(0x35, SHIFT), kitty, b"\x1b[27;2u"),
            ("up", key(0x7E, 0), kitty, b"\x1b[A"),
            ("ctrl+up", key(0x7E, CONTROL), kitty, b"\x1b[1;5A"),
            ("ctrl+c", text("c", CONTROL), kitty, b"\x1b[99;5u"),
            ("ctrl+shift+c", text("C", CONTROL | SHIFT), kitty, b"\x1b[99;6u"),
            ("ctrl+[", text("[", CONTROL), kitty, b"\x1b[91;5u"),
            ("meta+a", text("a", alt), meta, b"\x1b[97;3u"),
            ("ctrl+meta+a", text("a", alt | CONTROL), meta, b"\x1b[97;7u"),
        ];
        for (name, input, m, want) in rows {
            assert_eq!(show(&translate(input, *m)), show(&send(want)), "{name}");
        }
        // Option that isn't Meta types text, kitty or not.
        assert_eq!(translate(&text("a", alt), kitty), KeyAction::ToTextInput);
        assert_eq!(translate(&text("a", 0), kitty), KeyAction::ToTextInput);
    }

    #[test]
    fn cmd_keys() {
        let rows: &[(&str, KeyInput, KeyAction)] = &[
            ("cmd+left", key(0x7B, COMMAND), send(b"\x01")),
            ("cmd+right", key(0x7C, COMMAND), send(b"\x05")),
            ("cmd+backspace", key(0x33, COMMAND), send(b"\x15")),
            ("cmd+shift+left", key(0x7B, COMMAND | SHIFT), send(b"\x01")),
            ("cmd+up", key(0x7E, COMMAND), KeyAction::ToMenu),
            ("cmd+c", text("c", COMMAND), KeyAction::ToMenu),
            ("cmd+v", text("v", COMMAND), KeyAction::ToMenu),
            ("cmd+q", text("q", COMMAND), KeyAction::ToMenu),
            ("cmd+=", text("=", COMMAND), KeyAction::ToMenu),
            ("cmd+ctrl+f", text("f", COMMAND | CONTROL), KeyAction::ToMenu),
            ("cmd+option+h", text("h", COMMAND | OPTION | LEFT_OPTION), KeyAction::ToMenu),
            ("cmd+enter", key(0x24, COMMAND), KeyAction::ToMenu),
            ("cmd+f1", key(0x7A, COMMAND), KeyAction::ToMenu),
        ];
        let all = KeyModes {
            app_cursor: true,
            app_keypad: true,
            kitty_flags: 1,
            option_as_meta: OptionAsMeta::Both,
            composing: false,
        };
        for (name, input, want) in rows {
            for m in [KeyModes::default(), all, KeyModes { composing: true, ..all }] {
                assert_eq!(&translate(input, m), want, "{name} {m:?}");
            }
        }
    }

    #[test]
    fn composing_goes_to_the_input_method() {
        let m = KeyModes {
            composing: true,
            kitty_flags: 1,
            option_as_meta: OptionAsMeta::Both,
            ..KeyModes::default()
        };
        for input in [
            key(0x24, 0),
            key(0x35, 0),
            key(0x33, 0),
            key(0x7B, 0),
            key(0x7A, 0),
            text("c", CONTROL),
            text("a", OPTION | LEFT_OPTION),
            text("a", 0),
        ] {
            assert_eq!(translate(&input, m), KeyAction::ToTextInput, "{input:?}");
        }
        // Plain text always goes to the input system.
        assert_eq!(translate(&text("a", 0), KeyModes::default()), KeyAction::ToTextInput);
        assert_eq!(translate(&text("", CONTROL), KeyModes::default()), KeyAction::ToTextInput);
    }

    #[test]
    fn paste_filtering() {
        let rows: &[(&str, bool, &[u8])] = &[
            ("a\nb", false, b"a\rb"),
            ("a\r\nb", false, b"a\rb"),
            ("a\rb\n", false, b"a\rb\r"),
            ("tab\tkept", false, b"tab\tkept"),
            ("héllo ✨", false, "héllo ✨".as_bytes()),
            ("a\x00b\x03c\x07d\x08e\x1bf\x7fg", false, b"abcdefg"),
            ("c1\u{80}\u{85}\u{9b}\u{9f}end", false, b"c1end"),
            ("a\nb", true, b"\x1b[200~a\rb\x1b[201~"),
            ("", true, b"\x1b[200~\x1b[201~"),
            // A pasted end-of-paste marker can't end the bracket early.
            ("x\x1b[201~y", true, b"\x1b[200~x[201~y\x1b[201~"),
            ("x\u{9b}201~rm -rf ~\n", true, b"\x1b[200~x201~rm -rf ~\r\x1b[201~"),
            ("x\x1b[201~y", false, b"x[201~y"),
        ];
        for &(input, bracketed, want) in rows {
            assert_eq!(
                String::from_utf8_lossy(&paste_bytes(input, bracketed)),
                String::from_utf8_lossy(want),
                "{input:?} bracketed={bracketed}"
            );
        }
        // No control character but tab and CR survives, whatever is pasted.
        let all: String = (0u32..0x300).filter_map(char::from_u32).collect();
        let out = paste_bytes(&all, true);
        let inner = &out[6..out.len() - 6];
        assert!(
            !inner.iter().any(|&b| b < 0x20 && b != b'\t' && b != b'\r' || b == 0x7f),
            "{inner:?}"
        );
        assert!(!String::from_utf8_lossy(inner).chars().any(|c| ('\u{80}'..='\u{9f}').contains(&c)));
    }
}
