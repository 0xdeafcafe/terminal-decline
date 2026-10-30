//! Keys and text for the agent API, through the app's own key encoding:
//! a key name becomes the [`KeyInput`] the view builds from an `NSEvent`
//! on a US layout, and [`crate::keys::translate`] turns it into bytes,
//! with every mode and kitty keyboard level the program asked for. Alt
//! is Meta.

use crate::config::OptionAsMeta;
use crate::keys::{KeyAction, KeyEventType, KeyInput, KeyModes, committed_text, ctrl_byte, flags, translate};

/// US ANSI layout: key code, character, character with Shift.
const US: &[(u16, char, char)] = &[
    (0x00, 'a', 'A'),
    (0x01, 's', 'S'),
    (0x02, 'd', 'D'),
    (0x03, 'f', 'F'),
    (0x04, 'h', 'H'),
    (0x05, 'g', 'G'),
    (0x06, 'z', 'Z'),
    (0x07, 'x', 'X'),
    (0x08, 'c', 'C'),
    (0x09, 'v', 'V'),
    (0x0B, 'b', 'B'),
    (0x0C, 'q', 'Q'),
    (0x0D, 'w', 'W'),
    (0x0E, 'e', 'E'),
    (0x0F, 'r', 'R'),
    (0x10, 'y', 'Y'),
    (0x11, 't', 'T'),
    (0x12, '1', '!'),
    (0x13, '2', '@'),
    (0x14, '3', '#'),
    (0x15, '4', '$'),
    (0x16, '6', '^'),
    (0x17, '5', '%'),
    (0x18, '=', '+'),
    (0x19, '9', '('),
    (0x1A, '7', '&'),
    (0x1B, '-', '_'),
    (0x1C, '8', '*'),
    (0x1D, '0', ')'),
    (0x1E, ']', '}'),
    (0x1F, 'o', 'O'),
    (0x20, 'u', 'U'),
    (0x21, '[', '{'),
    (0x22, 'i', 'I'),
    (0x23, 'p', 'P'),
    (0x25, 'l', 'L'),
    (0x26, 'j', 'J'),
    (0x27, '\'', '"'),
    (0x28, 'k', 'K'),
    (0x29, ';', ':'),
    (0x2A, '\\', '|'),
    (0x2B, ',', '<'),
    (0x2C, '/', '?'),
    (0x2D, 'n', 'N'),
    (0x2E, 'm', 'M'),
    (0x2F, '.', '>'),
    (0x32, '`', '~'),
    (0x31, ' ', ' '),
];

/// Keys that type no character: names, key code, and the characters
/// macOS gives the event (function keys use its private-use codes).
const NAMED: &[(&[&str], u16, &str)] = &[
    (&["enter", "return", "cr"], 0x24, "\r"),
    (&["tab"], 0x30, "\t"),
    (&["space"], 0x31, " "),
    (&["backspace", "bs"], 0x33, "\x7f"),
    (&["escape", "esc"], 0x35, "\x1b"),
    (&["delete", "del"], 0x75, "\u{F728}"),
    (&["insert", "ins"], 0x72, "\u{F727}"),
    (&["up"], 0x7E, "\u{F700}"),
    (&["down"], 0x7D, "\u{F701}"),
    (&["left"], 0x7B, "\u{F702}"),
    (&["right"], 0x7C, "\u{F703}"),
    (&["home"], 0x73, "\u{F729}"),
    (&["end"], 0x77, "\u{F72B}"),
    (&["pageup", "pgup"], 0x74, "\u{F72C}"),
    (&["pagedown", "pgdn"], 0x79, "\u{F72D}"),
    (&["f1"], 0x7A, "\u{F704}"),
    (&["f2"], 0x78, "\u{F705}"),
    (&["f3"], 0x63, "\u{F706}"),
    (&["f4"], 0x76, "\u{F707}"),
    (&["f5"], 0x60, "\u{F708}"),
    (&["f6"], 0x61, "\u{F709}"),
    (&["f7"], 0x62, "\u{F70A}"),
    (&["f8"], 0x64, "\u{F70B}"),
    (&["f9"], 0x65, "\u{F70C}"),
    (&["f10"], 0x6D, "\u{F70D}"),
    (&["f11"], 0x67, "\u{F70E}"),
    (&["f12"], 0x6F, "\u{F70F}"),
    (&["kp-enter"], 0x4C, "\u{3}"),
];

/// A key with its modifiers, as parsed from `ctrl+shift+a`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySpec {
    keycode: u16,
    /// Characters without and with Shift, for keys that type one.
    chars: Option<(char, char)>,
    /// The event's characters for keys that don't.
    named: &'static str,
    /// `NSEventModifierFlags`, device bits included.
    flags: usize,
    numpad: bool,
}

impl KeySpec {
    /// Parse `ctrl+c`, `shift+tab`, `alt+f`, `cmd+left`, `F5`, `A` (Shift
    /// and `a`), `+`, `ctrl++`.
    pub fn parse(spec: &str) -> Result<KeySpec, String> {
        let bad = || format!("{spec:?}: not a key (try ctrl+c, enter, shift+tab, alt+left, f5)");
        if spec.is_empty() {
            return Err(bad());
        }
        let (mods, key) = match spec.strip_suffix("++") {
            Some(m) => (m, "+"),
            None if spec == "+" => ("", "+"),
            None => match spec.rsplit_once('+') {
                Some((m, k)) => (m, k),
                None => ("", spec),
            },
        };
        let mut f = 0;
        for m in mods.split('+').filter(|m| !m.is_empty()) {
            f |= match m.to_ascii_lowercase().as_str() {
                "shift" => flags::SHIFT | flags::LEFT_SHIFT,
                "ctrl" | "control" => flags::CONTROL | flags::LEFT_CONTROL,
                "alt" | "opt" | "option" | "meta" => flags::OPTION | flags::LEFT_OPTION,
                "cmd" | "command" | "super" => flags::COMMAND | flags::LEFT_COMMAND,
                _ => return Err(bad()),
            };
        }
        let mut chars = key.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && let Some(mut k) = char_key(c)
        {
            k.flags |= f;
            return Ok(k);
        }
        let lower = key.to_ascii_lowercase();
        let &(_, keycode, named) = NAMED.iter().find(|(names, ..)| names.contains(&lower.as_str())).ok_or_else(bad)?;
        Ok(KeySpec {
            keycode,
            chars: (keycode == 0x31).then_some((' ', ' ')),
            named,
            flags: f,
            numpad: keycode == 0x4C,
        })
    }

    fn shift(&self) -> bool {
        self.flags & flags::SHIFT != 0
    }

    /// The `KeyInput` the view would build for this key going `event`.
    fn input(&self, event: KeyEventType) -> KeyInput {
        let ctrl = self.flags & flags::CONTROL != 0;
        let (chars, ignoring, unshifted, ascii) = match self.chars {
            Some((base, shifted)) => {
                let c = if self.shift() { shifted } else { base };
                let typed = match ctrl.then(|| ctrl_byte(c)).flatten() {
                    Some(b) => (b as char).to_string(),
                    None => c.to_string(),
                };
                (typed, c.to_string(), base.to_string(), Some((base, shifted)))
            }
            None => (self.named.to_string(), self.named.to_string(), String::new(), None),
        };
        KeyInput {
            keycode: self.keycode,
            chars,
            chars_shift_only: ignoring.clone(),
            chars_ignoring_mods: ignoring,
            chars_unshifted: unshifted,
            ascii: ascii.map(|a| a.0),
            ascii_shifted: ascii.map(|a| a.1),
            flags: self.flags,
            numpad: self.numpad,
            event,
        }
    }
}

/// The key that types `c` on a US layout, with Shift if it takes it.
fn char_key(c: char) -> Option<KeySpec> {
    let &(keycode, base, shifted) = US.iter().find(|(_, b, s)| *b == c || *s == c)?;
    let shift = c != base;
    Some(KeySpec {
        keycode,
        chars: Some((base, shifted)),
        named: "",
        flags: if shift { flags::SHIFT | flags::LEFT_SHIFT } else { 0 },
        numpad: false,
    })
}

/// The modes keys are encoded with: the terminal's, with Alt as Meta.
pub fn modes(t: &term_core::Terminal) -> KeyModes {
    let m = t.modes();
    KeyModes {
        app_cursor: m.app_cursor,
        app_keypad: m.app_keypad,
        newline: m.linefeed_newline,
        kitty_flags: t.kitty_keyboard_flags(),
        option_as_meta: OptionAsMeta::Both,
        composing: false,
    }
}

/// The bytes pressing and letting go of `key` sends.
pub fn key_bytes(key: &KeySpec, modes: KeyModes, out: &mut Vec<u8>) -> Result<(), String> {
    for event in [KeyEventType::Press, KeyEventType::Release] {
        let input = key.input(event);
        match translate(&input, modes) {
            KeyAction::Send(b) => out.extend_from_slice(&b),
            KeyAction::ToTextInput if event == KeyEventType::Press => {
                out.extend_from_slice(&committed_text(Some(&input), modes, &input.chars));
            }
            KeyAction::ToMenu => return Err("a Cmd shortcut goes to the app's menus, not the program".into()),
            KeyAction::ToTextInput | KeyAction::Ignore => {}
        }
    }
    Ok(())
}

/// The bytes typing `text` sends, key by key: newlines as Enter, tabs as
/// Tab, ESC as Escape; text no US key types as the input method types it.
pub fn text_bytes(text: &str, modes: KeyModes, out: &mut Vec<u8>) {
    let named = |name: &str| KeySpec::parse(name).expect("a known key");
    let mut buf = [0u8; 4];
    for c in text.chars() {
        let key = match c {
            '\n' | '\r' => Some(named("enter")),
            '\t' => Some(named("tab")),
            '\x1b' => Some(named("escape")),
            '\x7f' => Some(named("backspace")),
            c if c.is_control() => {
                out.push(c as u8);
                continue;
            }
            c => char_key(c),
        };
        match key {
            Some(k) => {
                // A text key never goes to the menus.
                let _ = key_bytes(&k, modes, out);
            }
            None => out.extend_from_slice(&committed_text(None, modes, c.encode_utf8(&mut buf))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::kitty;

    fn key(spec: &str, modes: KeyModes) -> Vec<u8> {
        let mut out = Vec::new();
        key_bytes(&KeySpec::parse(spec).unwrap(), modes, &mut out).unwrap();
        out
    }

    fn legacy() -> KeyModes {
        KeyModes {
            option_as_meta: OptionAsMeta::Both,
            ..KeyModes::default()
        }
    }

    #[test]
    fn legacy_keys() {
        let m = legacy();
        for (spec, want) in [
            ("a", &b"a"[..]),
            ("A", b"A"),
            ("shift+a", b"A"),
            ("ctrl+c", b"\x03"),
            ("ctrl+C", b"\x03"),
            ("ctrl+[", b"\x1b"),
            ("alt+f", b"\x1bf"),
            ("alt+shift+f", b"\x1bF"),
            ("enter", b"\r"),
            ("Return", b"\r"),
            ("tab", b"\t"),
            ("shift+tab", b"\x1b[Z"),
            ("esc", b"\x1b"),
            ("backspace", b"\x7f"),
            ("space", b" "),
            ("up", b"\x1b[A"),
            ("shift+up", b"\x1b[1;2A"),
            ("ctrl+left", b"\x1b[1;5D"),
            ("f1", b"\x1bOP"),
            ("F5", b"\x1b[15~"),
            ("pagedown", b"\x1b[6~"),
            ("delete", b"\x1b[3~"),
            ("+", b"+"),
            ("ctrl++", b"+"),
            ("cmd+left", b"\x01"),
        ] {
            assert_eq!(key(spec, m), want, "{spec}");
        }
        let app = KeyModes { app_cursor: true, ..m };
        assert_eq!(key("up", app), b"\x1bOA");
        let lnm = KeyModes { newline: true, ..m };
        assert_eq!(key("enter", lnm), b"\r\n");
    }

    #[test]
    fn kitty_levels() {
        let k = |flags| KeyModes {
            kitty_flags: flags,
            ..legacy()
        };
        // Disambiguate: Esc and Ctrl keys as CSI u; text stays text.
        assert_eq!(key("esc", k(kitty::DISAMBIGUATE)), b"\x1b[27u");
        assert_eq!(key("ctrl+c", k(kitty::DISAMBIGUATE)), b"\x1b[99;5u");
        assert_eq!(key("a", k(kitty::DISAMBIGUATE)), b"a");
        // Event types: a release follows the press.
        assert_eq!(key("esc", k(kitty::DISAMBIGUATE | kitty::EVENT_TYPES)), b"\x1b[27u\x1b[27;1:3u");
        // All keys: text keys too.
        assert_eq!(key("a", k(kitty::DISAMBIGUATE | kitty::ALL_KEYS)), b"\x1b[97u");
        let mut out = Vec::new();
        text_bytes("ab", k(kitty::ALL_KEYS), &mut out);
        assert_eq!(out, b"\x1b[97u\x1b[98u");
    }

    #[test]
    fn text_is_typed() {
        let mut out = Vec::new();
        text_bytes("ls -la\n", legacy(), &mut out);
        assert_eq!(out, b"ls -la\r");
        out.clear();
        text_bytes("é ✓\t\x03", legacy(), &mut out);
        assert_eq!(out, "é ✓\t\x03".as_bytes());
    }

    #[test]
    fn bad_keys() {
        for spec in ["", "hyper+a", "ctrl+", "nope", "ctrl+nope", "f13"] {
            assert!(KeySpec::parse(spec).is_err(), "{spec}");
        }
        let mut out = Vec::new();
        assert!(key_bytes(&KeySpec::parse("cmd+q").unwrap(), legacy(), &mut out).is_err());
    }
}
