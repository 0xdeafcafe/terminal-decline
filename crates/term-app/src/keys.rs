//! Keyboard input to bytes, following xterm.

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
    use super::*;

    const NONE: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: false,
    };

    #[test]
    fn arrows() {
        assert_eq!(encode_key(Key::Up, NONE, false), b"\x1b[A");
        assert_eq!(encode_key(Key::Up, NONE, true), b"\x1bOA");
        assert_eq!(encode_key(Key::Left, Mods { ctrl: true, ..NONE }, true), b"\x1b[1;5D");
        assert_eq!(encode_key(Key::Right, Mods { alt: true, ..NONE }, false), b"\x1b[1;3C");
    }

    #[test]
    fn function_and_edit_keys() {
        assert_eq!(encode_key(Key::F(1), NONE, false), b"\x1bOP");
        assert_eq!(encode_key(Key::F(5), NONE, false), b"\x1b[15~");
        assert_eq!(encode_key(Key::F(12), Mods { shift: true, ..NONE }, false), b"\x1b[24;2~");
        assert_eq!(encode_key(Key::Delete, NONE, false), b"\x1b[3~");
        assert_eq!(encode_key(Key::Tab, Mods { shift: true, ..NONE }, false), b"\x1b[Z");
        assert_eq!(encode_key(Key::Backspace, Mods { alt: true, ..NONE }, false), b"\x1b\x7f");
    }

    #[test]
    fn ctrl_keys() {
        assert_eq!(ctrl_byte('c'), Some(3));
        assert_eq!(ctrl_byte(' '), Some(0));
        assert_eq!(ctrl_byte('['), Some(0x1b));
    }

    #[test]
    fn kitty() {
        assert_eq!(
            encode_kitty_key(Key::Enter, Mods { shift: true, ..NONE }),
            Some(b"\x1b[13;2u".to_vec())
        );
        assert_eq!(encode_kitty_key(Key::Enter, NONE), None);
        assert_eq!(encode_kitty_key(Key::Escape, NONE), Some(b"\x1b[27u".to_vec()));
        assert_eq!(encode_kitty_char('C', Mods { ctrl: true, ..NONE }), b"\x1b[99;5u");
    }

    #[test]
    fn paste() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb");
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~x[201~y\x1b[201~");
        assert_eq!(paste_bytes("a\x03b\tc\u{9b}", false), b"ab\tc");
    }
}
