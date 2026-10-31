//! The quick terminal (Ghostty's quick terminal, iTerm2's hotkey window):
//! a system-wide hotkey (`quick_terminal_hotkey`, default `ctrl+``)
//! slides a terminal window in from an edge of the screen the mouse is
//! on, over full-screen apps, on the active Space; the hotkey again (or,
//! with `quick_terminal_autohide`, the focus moving elsewhere) slides it
//! away. It keeps its session and panes while hidden, and gives back its
//! drawables as any hidden window does.
//!
//! This module is the part with no AppKit: the hotkey as Carbon wants it
//! ([`Hotkey::parse`]), the setting for where it shows ([`Position`]) and
//! the window's frames ([`shown_frame`], [`hidden_frame`]). The hotkey
//! itself is [`crate::hotkey`]; the window, [`crate::app`].

use serde::{Deserialize, Serialize};

use crate::keybind::{Chord, ChordKey, Named};

/// The default hotkey: Control and the key left of 1 on a US keyboard.
pub const DEFAULT_HOTKEY: &str = "ctrl+`";

/// A system-wide hotkey: a key by its place on the keyboard (a macOS
/// virtual key code, as on a US layout) and Carbon's modifier bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub keycode: u32,
    pub modifiers: u32,
}

/// Carbon's modifier bits (`Events.h`).
pub mod carbon {
    pub const CMD: u32 = 1 << 8;
    pub const SHIFT: u32 = 1 << 9;
    pub const OPTION: u32 = 1 << 11;
    pub const CONTROL: u32 = 1 << 12;
}

impl Hotkey {
    /// `quick_terminal_hotkey` as written: a chord in `[keybind]`'s
    /// spelling (`"ctrl+`"`, `"cmd+shift+space"`, `"f12"`), or `""` for
    /// no quick terminal (`None`). A key needs Cmd, Ctrl or Alt, unless
    /// it is F1–F20: a hotkey takes its key from every app.
    pub fn parse(text: &str) -> Result<Option<Hotkey>, String> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        let chord = Chord::parse(text)?;
        let (keycode, shifted) = keycode_of(chord.key).ok_or_else(|| format!("{text:?}: that key can't be a hotkey"))?;
        let function_key = matches!(chord.key, ChordKey::Named(Named::F(_)));
        if !(chord.cmd || chord.ctrl || chord.alt || function_key) {
            return Err(format!(
                "{text:?}: a hotkey needs cmd, ctrl or alt (or an F key), or it takes that key from every app"
            ));
        }
        let modifiers = [
            (chord.cmd, carbon::CMD),
            (chord.ctrl, carbon::CONTROL),
            (chord.alt, carbon::OPTION),
            (chord.shift || shifted, carbon::SHIFT),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |m, (_, bit)| m | bit);
        Ok(Some(Hotkey { keycode, modifiers }))
    }
}

/// A key's virtual key code on a US layout, and whether it is the
/// shifted character of that key (`~` is Shift+`).
fn keycode_of(key: ChordKey) -> Option<(u32, bool)> {
    let named = match key {
        ChordKey::Named(n) => n,
        ChordKey::Char(c) => return char_keycode(c),
    };
    Some((
        match named {
            Named::Enter => 0x24,
            Named::Tab => 0x30,
            Named::Escape => 0x35,
            Named::Backspace => 0x33,
            Named::Delete => 0x75,
            Named::Insert => 0x72,
            Named::Up => 0x7E,
            Named::Down => 0x7D,
            Named::Left => 0x7B,
            Named::Right => 0x7C,
            Named::Home => 0x73,
            Named::End => 0x77,
            Named::PageUp => 0x74,
            Named::PageDown => 0x79,
            Named::F(n) => *[
                0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A,
            ]
            .get(n as usize - 1)?,
        },
        false,
    ))
}

/// US layout: each key's unshifted and shifted characters.
const KEYS: &[(u32, char, char)] = &[
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
    (0x31, ' ', ' '),
    (0x32, '`', '~'),
];

fn char_keycode(c: char) -> Option<(u32, bool)> {
    KEYS.iter().find_map(|&(code, plain, shifted)| match c {
        _ if c == plain => Some((code, false)),
        // Letters are lower case in a chord already.
        _ if c == shifted && !c.is_alphabetic() => Some((code, true)),
        _ => None,
    })
}

/// `quick_terminal_position`: the edge it slides in from, or the centre
/// of the screen (where it fades in).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
    Center,
}

/// A rectangle in AppKit's screen coordinates (origin bottom left, y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// The smallest `quick_terminal_size`, and the default.
pub const MIN_SIZE: f64 = 0.1;
pub const DEFAULT_SIZE: f64 = 0.4;

/// Where the window sits while shown, on a screen whose usable area
/// (`visibleFrame`: without the menu bar and the Dock) is `screen`: along
/// the whole of its edge, `size` (a fraction) of the screen deep; in the
/// centre, `size` of each side.
pub fn shown_frame(position: Position, screen: Rect, size: f64) -> Rect {
    let size = size.clamp(MIN_SIZE, 1.0);
    let (w, h) = match position {
        Position::Top | Position::Bottom => (screen.w, screen.h * size),
        Position::Left | Position::Right => (screen.w * size, screen.h),
        Position::Center => (screen.w * size, screen.h * size),
    };
    let (w, h) = (w.round(), h.round());
    let (x, y) = match position {
        Position::Top => (screen.x, screen.y + screen.h - h),
        Position::Bottom => (screen.x, screen.y),
        Position::Left => (screen.x, screen.y),
        Position::Right => (screen.x + screen.w - w, screen.y),
        Position::Center => ((screen.x + (screen.w - w) / 2.0).round(), (screen.y + (screen.h - h) / 2.0).round()),
    };
    Rect { x, y, w, h }
}

/// Where it slides from and back to: `shown` moved just past the edge of
/// `screen` it sits on. In the centre it stays put (it fades instead).
pub fn hidden_frame(position: Position, screen: Rect, shown: Rect) -> Rect {
    let (x, y) = match position {
        Position::Top => (shown.x, screen.y + screen.h),
        Position::Bottom => (shown.x, screen.y - shown.h),
        Position::Left => (screen.x - shown.w, shown.y),
        Position::Right => (screen.x + screen.w, shown.y),
        Position::Center => (shown.x, shown.y),
    };
    Rect { x, y, ..shown }
}

/// The screen the mouse is on, among `screens` (their whole frames), by
/// index; else the first (the main screen).
pub fn screen_under(screens: &[Rect], x: f64, y: f64) -> usize {
    screens.iter().position(|s| s.contains(x, y)).unwrap_or(0)
}

/// The longest `quick_terminal_animation_ms`.
pub const MAX_ANIMATION_MS: u32 = 2000;

/// How long the slide takes, in seconds: none with Reduce Motion on (it
/// then appears at once, as macOS's own windows do), under the test
/// control, or with `quick_terminal_animation_ms = 0`.
pub fn animation_seconds(ms: u32, reduce_motion: bool, background: bool) -> f64 {
    match reduce_motion || background {
        true => 0.0,
        false => ms.min(MAX_ANIMATION_MS) as f64 / 1000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(keycode: u32, modifiers: u32) -> Option<Hotkey> {
        Some(Hotkey { keycode, modifiers })
    }

    #[test]
    fn hotkeys_parse_to_carbon_codes() {
        use carbon::*;
        assert_eq!(Hotkey::parse(DEFAULT_HOTKEY), Ok(hk(0x32, CONTROL)));
        assert_eq!(Hotkey::parse("ctrl+`"), Ok(hk(0x32, CONTROL)));
        assert_eq!(Hotkey::parse(""), Ok(None));
        assert_eq!(Hotkey::parse("  "), Ok(None));
        assert_eq!(Hotkey::parse("cmd+shift+space"), Ok(hk(0x31, CMD | SHIFT)));
        assert_eq!(Hotkey::parse("Command+Option+T"), Ok(hk(0x11, CMD | OPTION)));
        assert_eq!(Hotkey::parse("alt+enter"), Ok(hk(0x24, OPTION)));
        assert_eq!(Hotkey::parse("ctrl+alt+cmd+shift+k"), Ok(hk(0x28, CMD | CONTROL | OPTION | SHIFT)));
        // F keys alone are fine; F12 is 0x6F.
        assert_eq!(Hotkey::parse("f12"), Ok(hk(0x6F, 0)));
        assert_eq!(Hotkey::parse("shift+f1"), Ok(hk(0x7A, SHIFT)));
        assert_eq!(Hotkey::parse("f20"), Ok(hk(0x5A, 0)));
        // A shifted character is its key with Shift.
        assert_eq!(Hotkey::parse("ctrl+~"), Ok(hk(0x32, CONTROL | SHIFT)));
        assert_eq!(Hotkey::parse("cmd+plus"), Ok(hk(0x18, CMD | SHIFT)));
        assert_eq!(Hotkey::parse("cmd+="), Ok(hk(0x18, CMD)));
        assert_eq!(Hotkey::parse("ctrl+1"), Ok(hk(0x12, CONTROL)));
        assert_eq!(Hotkey::parse("ctrl+up"), Ok(hk(0x7E, CONTROL)));
    }

    #[test]
    fn bad_hotkeys_say_why() {
        for (text, says) in [
            ("k", "needs cmd, ctrl or alt"),
            ("shift+k", "needs cmd, ctrl or alt"),
            ("space", "needs cmd, ctrl or alt"),
            ("hyper+k", "unknown modifier"),
            ("ctrl+nope", "unknown key"),
            ("ctrl+é", "can't be a hotkey"),
            ("ctrl+f21", "unknown key"),
        ] {
            let e = Hotkey::parse(text).unwrap_err();
            assert!(e.contains(says), "{text}: {e}");
        }
    }

    #[test]
    fn every_key_of_the_table_has_one_code() {
        let mut codes: Vec<u32> = KEYS.iter().map(|k| k.0).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), KEYS.len());
        for &(code, plain, _) in KEYS {
            assert_eq!(char_keycode(plain), Some((code, false)), "{plain:?}");
        }
    }

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1512.0,
        h: 944.0,
    };

    #[test]
    fn frames_per_position() {
        // The top, 40% deep, under the menu bar: the usable area ends
        // below it, at 944.
        let top = shown_frame(Position::Top, SCREEN, 0.4);
        assert_eq!(
            top,
            Rect {
                x: 0.0,
                y: 566.0,
                w: 1512.0,
                h: 378.0
            }
        );
        assert_eq!(hidden_frame(Position::Top, SCREEN, top), Rect { y: 944.0, ..top });
        let bottom = shown_frame(Position::Bottom, SCREEN, 0.5);
        assert_eq!(
            bottom,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1512.0,
                h: 472.0
            }
        );
        assert_eq!(hidden_frame(Position::Bottom, SCREEN, bottom), Rect { y: -472.0, ..bottom });
        let left = shown_frame(Position::Left, SCREEN, 0.25);
        assert_eq!(
            left,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 378.0,
                h: 944.0
            }
        );
        assert_eq!(hidden_frame(Position::Left, SCREEN, left), Rect { x: -378.0, ..left });
        let right = shown_frame(Position::Right, SCREEN, 0.25);
        assert_eq!(
            right,
            Rect {
                x: 1134.0,
                y: 0.0,
                w: 378.0,
                h: 944.0
            }
        );
        assert_eq!(hidden_frame(Position::Right, SCREEN, right), Rect { x: 1512.0, ..right });
        let center = shown_frame(Position::Center, SCREEN, 0.5);
        assert_eq!(
            center,
            Rect {
                x: 378.0,
                y: 236.0,
                w: 756.0,
                h: 472.0
            }
        );
        assert_eq!(hidden_frame(Position::Center, SCREEN, center), center);
    }

    #[test]
    fn frames_on_a_second_screen() {
        // A screen left of the main one and higher, with a Dock at its
        // bottom (its usable area starts 70 pt up).
        let screen = Rect {
            x: -1920.0,
            y: 200.0 + 70.0,
            w: 1920.0,
            h: 1080.0 - 70.0 - 25.0,
        };
        let top = shown_frame(Position::Top, screen, 0.4);
        assert_eq!(
            top,
            Rect {
                x: -1920.0,
                y: 270.0 + 985.0 - 394.0,
                w: 1920.0,
                h: 394.0
            }
        );
        // Slides from just above the usable area's top.
        assert_eq!(hidden_frame(Position::Top, screen, top).y, 1255.0);
        let right = shown_frame(Position::Right, screen, 0.3);
        assert_eq!(right.x + right.w, 0.0, "{right:?}");
        assert_eq!(hidden_frame(Position::Right, screen, right).x, 0.0);
        let bottom = shown_frame(Position::Bottom, screen, 0.3);
        assert_eq!(bottom.y, 270.0);
        assert_eq!(hidden_frame(Position::Bottom, screen, bottom).y, 270.0 - bottom.h);
    }

    #[test]
    fn sizes_are_clamped() {
        assert_eq!(shown_frame(Position::Top, SCREEN, 5.0), SCREEN);
        assert_eq!(shown_frame(Position::Top, SCREEN, 0.0).h, (944.0 * MIN_SIZE).round());
        assert_eq!(shown_frame(Position::Center, SCREEN, 1.0), SCREEN);
    }

    #[test]
    fn the_screen_under_the_mouse() {
        let screens = [
            SCREEN,
            Rect {
                x: 1512.0,
                y: -100.0,
                w: 2560.0,
                h: 1440.0,
            },
        ];
        assert_eq!(screen_under(&screens, 10.0, 10.0), 0);
        assert_eq!(screen_under(&screens, 2000.0, 1300.0), 1);
        // Between screens (the mouse is always on one; rounding): the main one.
        assert_eq!(screen_under(&screens, -5.0, 0.0), 0);
        assert_eq!(screen_under(&[], 0.0, 0.0), 0);
    }

    #[test]
    fn animation_respects_reduce_motion() {
        assert_eq!(animation_seconds(150, false, false), 0.15);
        assert_eq!(animation_seconds(150, true, false), 0.0);
        assert_eq!(animation_seconds(150, false, true), 0.0);
        assert_eq!(animation_seconds(0, false, false), 0.0);
        assert_eq!(animation_seconds(60_000, false, false), 2.0);
    }
}
