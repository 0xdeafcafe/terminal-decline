//! Keyboard input to bytes, following xterm and the kitty keyboard
//! protocol.
//!
//! [`translate`] decides what a key event does: send bytes to the program,
//! go to the input method (text, dead keys, IME), or go to the menus (Cmd
//! shortcuts). The view only builds a [`KeyInput`] from the `NSEvent`.
//! Text that comes back from the input method goes through
//! [`committed_text`], which encodes it as a key event when the program
//! asked for every key as an escape code.

use crate::config::OptionAsMeta;

/// `NSEventModifierFlags` bits used here.
pub mod flags {
    /// Device-dependent bits: which of the left/right modifier keys is
    /// held.
    pub const LEFT_CONTROL: usize = 0x1;
    pub const LEFT_SHIFT: usize = 0x2;
    pub const RIGHT_SHIFT: usize = 0x4;
    pub const LEFT_COMMAND: usize = 0x8;
    pub const RIGHT_COMMAND: usize = 0x10;
    /// Device-dependent bit: the left Option key is held.
    pub const LEFT_OPTION: usize = 0x20;
    /// Device-dependent bit: the right Option key is held.
    pub const RIGHT_OPTION: usize = 0x40;
    pub const RIGHT_CONTROL: usize = 0x2000;
    pub const CAPS_LOCK: usize = 1 << 16;
    pub const SHIFT: usize = 1 << 17;
    pub const CONTROL: usize = 1 << 18;
    pub const OPTION: usize = 1 << 19;
    pub const COMMAND: usize = 1 << 20;
    pub const NUMERIC_PAD: usize = 1 << 21;
}

/// Kitty keyboard protocol flags ("progressive enhancements").
pub mod kitty {
    /// Esc, Alt+key, Ctrl+key and keypad keys as `CSI u`.
    pub const DISAMBIGUATE: u8 = 1;
    /// Repeat and release events.
    pub const EVENT_TYPES: u8 = 2;
    /// The shifted key and the base layout key too.
    pub const ALTERNATE_KEYS: u8 = 4;
    /// Every key as an escape code, text keys and modifier keys included.
    pub const ALL_KEYS: u8 = 8;
    /// With [`ALL_KEYS`]: the text a key types, inside its escape code.
    pub const TEXT: u8 = 16;
}

/// Whether a key went down, auto-repeated, or came up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyEventType {
    #[default]
    Press,
    Repeat,
    Release,
}

impl KeyEventType {
    /// The kitty protocol's number for it.
    fn number(self) -> u8 {
        match self {
            KeyEventType::Press => 1,
            KeyEventType::Repeat => 2,
            KeyEventType::Release => 3,
        }
    }
}

/// A key event, as read from the `NSEvent`: a key press, repeat or
/// release, or a modifier key going down or up (`flagsChanged`, which has
/// no characters).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyInput {
    /// macOS virtual key code.
    pub keycode: u16,
    /// `characters`: the text the key types with all modifiers applied.
    pub chars: String,
    /// `charactersIgnoringModifiers` (Shift still applies).
    pub chars_ignoring_mods: String,
    /// The characters with only Shift applied (not Option), for
    /// Option-as-Meta.
    pub chars_shift_only: String,
    /// The characters with no modifiers at all: the kitty protocol's key
    /// code for text keys. Empty: the lower case of `chars_ignoring_mods`.
    pub chars_unshifted: String,
    /// What the key types on the ASCII-capable layout (see
    /// [`crate::ascii_layout`]), without and with Shift: used for Ctrl and
    /// Meta when the current layout's character isn't ASCII (Ctrl+С on a
    /// Russian layout is Ctrl+C), and as the kitty base layout key.
    pub ascii: Option<char>,
    pub ascii_shifted: Option<char>,
    /// Raw `NSEventModifierFlags` bits, including the left/right device
    /// bits; see [`flags`].
    pub flags: usize,
    /// The key is on the numeric keypad (`NSEventModifierFlagNumericPad`).
    pub numpad: bool,
    /// Press, auto-repeat (`isARepeat`) or release (`keyUp`). Modifier
    /// keys are always given as presses; which way they went is read from
    /// the device bits in `flags`.
    pub event: KeyEventType,
}

/// What the terminal and settings say about how keys are encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    /// DECCKM: arrows send SS3 sequences.
    pub app_cursor: bool,
    /// DECKPAM: the keypad sends SS3 sequences.
    pub app_keypad: bool,
    /// Kitty keyboard protocol flags (0 = legacy); see [`kitty`].
    pub kitty_flags: u8,
    pub option_as_meta: OptionAsMeta,
    /// An input method is composing text; every key goes to it.
    pub composing: bool,
}

/// What to do with a key event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Send these bytes to the program.
    Send(Vec<u8>),
    /// Hand the event to the input system (`interpretKeyEvents`); what it
    /// types goes through [`committed_text`].
    ToTextInput,
    /// A Cmd shortcut: the menus handle it; nothing goes to the program.
    ToMenu,
    /// Nothing to do: a release or modifier key the program didn't ask
    /// for.
    Ignore,
}

/// Decide what a key event does.
pub fn translate(input: &KeyInput, modes: KeyModes) -> KeyAction {
    if let Some(key) = modifier_key(input.keycode) {
        return modifier_event(input, modes, key);
    }
    let release = input.event == KeyEventType::Release;
    if release && modes.kitty_flags & kitty::EVENT_TYPES == 0 {
        return KeyAction::Ignore;
    }
    if input.flags & flags::COMMAND != 0 {
        if release {
            return KeyAction::Ignore;
        }
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
        return if release { KeyAction::Ignore } else { KeyAction::ToTextInput };
    }
    key_action(input, modes)
}

/// The settings that shape one key event's encoding.
#[derive(Clone, Copy)]
struct Enc {
    /// Modifiers as held (Option is Alt).
    mods: Mods,
    /// Option counts as Meta for text keys.
    meta: bool,
    /// Kitty flag 1 or 8: keys go as `CSI u` where the spec says.
    disambiguate: bool,
    /// Kitty flag 8.
    all: bool,
    /// Kitty flag 4.
    alternates: bool,
    /// Kitty flags 8 and 16.
    text: bool,
    /// The event type to report: with kitty flag 2 the event's own, else
    /// every event is a press.
    event: KeyEventType,
}

impl Enc {
    fn new(input: &KeyInput, modes: KeyModes) -> Enc {
        let f = input.flags;
        let (left_alt, right_alt) = (f & flags::LEFT_OPTION != 0, f & flags::RIGHT_OPTION != 0);
        let meta = match modes.option_as_meta {
            OptionAsMeta::Left => left_alt,
            OptionAsMeta::Right => right_alt,
            OptionAsMeta::Both => left_alt || right_alt,
            OptionAsMeta::None => false,
        };
        let k = modes.kitty_flags;
        Enc {
            mods: Mods::of(f),
            meta,
            disambiguate: k & (kitty::DISAMBIGUATE | kitty::ALL_KEYS) != 0,
            all: k & kitty::ALL_KEYS != 0,
            alternates: k & kitty::ALTERNATE_KEYS != 0,
            text: k & kitty::ALL_KEYS != 0 && k & kitty::TEXT != 0,
            event: if k & kitty::EVENT_TYPES != 0 {
                input.event
            } else {
                KeyEventType::Press
            },
        }
    }

    /// Kitty encoding applies: flag 1 or 8, or an event only it can
    /// express.
    fn kitty(self) -> bool {
        self.disambiguate || self.event != KeyEventType::Press
    }
}

/// A key event that isn't a Cmd shortcut or for the input method.
fn key_action(input: &KeyInput, modes: KeyModes) -> KeyAction {
    let e = Enc::new(input, modes);
    let release = e.event == KeyEventType::Release;
    let code = input.keycode;
    let mods = e.mods;
    if e.kitty() {
        if let Some(number) = keypad_number(code) {
            // Keypad digits and operators type text; Enter doesn't.
            let text = printable_text(&input.chars);
            let types_text = !text.is_empty();
            let as_text = types_text && !e.all && !release && !(mods.ctrl || e.meta);
            let legacy_enter = !types_text && !e.disambiguate && !release;
            if !as_text && !legacy_enter {
                let bits = if types_text && !e.all { mods.without_locks() } else { mods.kitty() };
                let text = if e.text && !release { text } else { Vec::new() };
                return KeyAction::Send(Csi::u(number, bits, e.event).text(text).bytes());
            }
        } else if let Some(number) = kitty_only_key(code) {
            return KeyAction::Send(Csi::u(number, mods.kitty(), e.event).bytes());
        } else if let Some(key) = key_from_keycode(code) {
            let legacy_text_key = matches!(key, Key::Enter | Key::Tab | Key::Backspace);
            if legacy_text_key && !e.all {
                // Enter, Tab and Backspace keep their legacy bytes and have
                // no release events, so `reset` can still be typed after a
                // program leaves the mode on.
                if release {
                    return KeyAction::Ignore;
                }
                if e.disambiguate && mods.without_locks() != 0 {
                    return KeyAction::Send(kitty_functional(key, mods.kitty(), e.event));
                }
            } else {
                return KeyAction::Send(kitty_functional(key, mods.kitty(), e.event));
            }
        }
    }
    if let Some(key) = key_from_keycode(code) {
        if input.numpad
            && modes.app_keypad
            && !mods.any()
            && let Some(b) = keypad_app_bytes(code)
        {
            return KeyAction::Send(b.to_vec());
        }
        return KeyAction::Send(encode_key(key, mods, modes.app_cursor));
    }
    let keypad = input.numpad && !(0x7B..=0x7E).contains(&code);
    if keypad
        && modes.app_keypad
        && !mods.any()
        && let Some(b) = keypad_app_bytes(code)
    {
        return KeyAction::Send(b.to_vec());
    }
    let Some(c) = input.chars_ignoring_mods.chars().next() else {
        return if release { KeyAction::Ignore } else { KeyAction::ToTextInput };
    };
    if e.disambiguate || release {
        // Under flag 8 Option is reported as Alt; otherwise only as Meta
        // (without Meta, Option is part of typing text).
        let alt = if e.all { mods.alt } else { e.meta };
        let command = mods.ctrl || e.meta;
        if (e.all || release || command) && !c.is_control() {
            if e.all && !release && !command {
                // The input method may turn it into other text; the view
                // encodes what it types (`committed_text`).
                return KeyAction::ToTextInput;
            }
            if let Some(b) = text_key_csi(input, e, Mods { alt, ..mods }, "") {
                return KeyAction::Send(b);
            }
        }
        if release {
            return KeyAction::Ignore;
        }
    }
    if mods.ctrl
        && let Some(b) = ctrl_byte(ascii_or(c, input, mods.shift))
    {
        let mut out = if e.meta { vec![0x1b] } else { Vec::new() };
        out.push(b);
        return KeyAction::Send(out);
    }
    if e.meta {
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
        return KeyAction::Send(out);
    }
    KeyAction::ToTextInput
}

/// The bytes for `text`, which the input method typed (`insertText`) for
/// `key` (the key event being handled, if any). Plain UTF-8, except with
/// kitty flag 8: if `text` is just what the key types, the key event's
/// escape code (with the text in it under flag 16); text from an input
/// method with no key of its own is a "text event" (key 0) under flag 16,
/// and plain UTF-8 without it, so it isn't lost.
pub fn committed_text(key: Option<&KeyInput>, modes: KeyModes, text: &str) -> Vec<u8> {
    if modes.kitty_flags & kitty::ALL_KEYS == 0 || text.is_empty() {
        return text.as_bytes().to_vec();
    }
    if let Some(input) = key.filter(|k| k.chars == text) {
        let e = Enc::new(input, modes);
        if let Some(b) = text_key_csi(input, e, e.mods, text) {
            return b;
        }
    }
    let cps = printable_text(text);
    if modes.kitty_flags & kitty::TEXT != 0 && !cps.is_empty() {
        return Csi::u(0, 0, KeyEventType::Press).text(cps).bytes();
    }
    text.as_bytes().to_vec()
}

/// A text key as `CSI key:shifted:base ; mods:event ; text u`. `None`
/// when the key's character isn't known.
fn text_key_csi(input: &KeyInput, e: Enc, mods: Mods, text: &str) -> Option<Vec<u8>> {
    let key = unshifted(input)?;
    if key.is_control() {
        return None;
    }
    let mut csi = Csi::u(key as u32, if e.all { mods.kitty() } else { mods.without_locks() }, e.event);
    if e.alternates {
        // The shifted key only with Shift; the base layout key only when
        // it differs.
        csi.shifted = mods.shift.then(|| shifted(input)).flatten().filter(|&s| s != key).map(|s| s as u32);
        csi.base = input.ascii.map(lower).filter(|&b| b != key).map(|b| b as u32);
    }
    if e.text && e.event != KeyEventType::Release {
        csi = csi.text(printable_text(text));
    }
    Some(csi.bytes())
}

/// The key with no modifiers on the current layout.
fn unshifted(input: &KeyInput) -> Option<char> {
    input
        .chars_unshifted
        .chars()
        .next()
        .or_else(|| input.chars_ignoring_mods.chars().next().map(lower))
}

/// The key with Shift on the current layout.
fn shifted(input: &KeyInput) -> Option<char> {
    input
        .chars_shift_only
        .chars()
        .next()
        .or_else(|| input.chars_ignoring_mods.chars().next())
}

fn lower(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

/// Code points of `text` other than control characters (C0, DEL, C1),
/// which the kitty protocol never puts in associated text.
fn printable_text(text: &str) -> Vec<u32> {
    text.chars().filter(|c| !c.is_control()).map(|c| c as u32).collect()
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

/// A kitty protocol escape code: `CSI key:shifted:base ; mods:event ; text
/// final`.
struct Csi {
    key: u32,
    shifted: Option<u32>,
    base: Option<u32>,
    /// Modifier bits (the code carries 1 + these).
    mods: u8,
    event: KeyEventType,
    text: Vec<u32>,
    /// `u`, `~`, or a letter (`A`…`S`), whose key number 1 is left out
    /// when there are no other fields.
    fin: u8,
}

impl Csi {
    fn u(key: u32, mods: u8, event: KeyEventType) -> Csi {
        Csi {
            key,
            shifted: None,
            base: None,
            mods,
            event,
            text: Vec::new(),
            fin: b'u',
        }
    }

    fn text(self, text: Vec<u32>) -> Csi {
        Csi { text, ..self }
    }

    fn bytes(&self) -> Vec<u8> {
        use std::fmt::Write;
        let mut mods = String::new();
        if self.event != KeyEventType::Press {
            let _ = write!(mods, "{}:{}", self.mods as u16 + 1, self.event.number());
        } else if self.mods != 0 {
            let _ = write!(mods, "{}", self.mods as u16 + 1);
        }
        let params = !mods.is_empty() || !self.text.is_empty();
        let mut s = String::from("\x1b[");
        if self.fin == b'u' || self.fin == b'~' || params {
            let _ = write!(s, "{}", self.key);
        }
        if self.shifted.is_some() || self.base.is_some() {
            s.push(':');
            if let Some(k) = self.shifted {
                let _ = write!(s, "{k}");
            }
            if let Some(k) = self.base {
                let _ = write!(s, ":{k}");
            }
        }
        if params {
            s.push(';');
            s.push_str(&mods);
        }
        if !self.text.is_empty() {
            s.push(';');
            let cps: Vec<String> = self.text.iter().map(u32::to_string).collect();
            s.push_str(&cps.join(":"));
        }
        s.push(self.fin as char);
        s.into_bytes()
    }
}

/// A functional key in the kitty protocol's table: `CSI 1 ; mods X` for
/// arrows, Home, End, F1, F2 and F4; `CSI n ; mods ~` or `CSI n ; mods u`
/// for the rest.
fn kitty_functional(key: Key, mods: u8, event: KeyEventType) -> Vec<u8> {
    let (key, fin) = match key {
        Key::Up => (1, b'A'),
        Key::Down => (1, b'B'),
        Key::Right => (1, b'C'),
        Key::Left => (1, b'D'),
        Key::End => (1, b'F'),
        Key::Home => (1, b'H'),
        Key::F(1) => (1, b'P'),
        Key::F(2) => (1, b'Q'),
        // CSI R would read as a cursor position report.
        Key::F(3) => (13, b'~'),
        Key::F(4) => (1, b'S'),
        Key::F(n) => (legacy_f_number(n) as u32, b'~'),
        Key::Insert => (2, b'~'),
        Key::Delete => (3, b'~'),
        Key::PageUp => (5, b'~'),
        Key::PageDown => (6, b'~'),
        Key::Escape => (27, b'u'),
        Key::Enter => (13, b'u'),
        Key::Tab => (9, b'u'),
        Key::Backspace => (127, b'u'),
    };
    Csi {
        fin,
        ..Csi::u(key, mods, event)
    }
    .bytes()
}

/// A modifier key and its kitty key code, for `flagsChanged`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModifierKey {
    number: u32,
    /// The device-dependent bit set while this key is held; 0 for Caps
    /// Lock, which is reported as a press each time it toggles.
    held: usize,
}

fn modifier_key(code: u16) -> Option<ModifierKey> {
    let (number, held) = match code {
        0x38 => (57441, flags::LEFT_SHIFT),
        0x3B => (57442, flags::LEFT_CONTROL),
        0x3A => (57443, flags::LEFT_OPTION),
        0x37 => (57444, flags::LEFT_COMMAND),
        0x3C => (57447, flags::RIGHT_SHIFT),
        0x3E => (57448, flags::RIGHT_CONTROL),
        0x3D => (57449, flags::RIGHT_OPTION),
        0x36 => (57450, flags::RIGHT_COMMAND),
        0x39 => (57358, 0),
        _ => return None,
    };
    Some(ModifierKey { number, held })
}

/// A modifier key went down or up: reported only with kitty flag 8 (and
/// its release only with flag 2). The modifier bits are the state after
/// the event, as the spec asks: pressing Left Control sets ctrl, releasing
/// it clears ctrl unless Right Control is still held.
fn modifier_event(input: &KeyInput, modes: KeyModes, key: ModifierKey) -> KeyAction {
    let k = modes.kitty_flags;
    if k & kitty::ALL_KEYS == 0 {
        return KeyAction::Ignore;
    }
    let down = key.held == 0 || input.flags & key.held != 0;
    if !down && k & kitty::EVENT_TYPES == 0 {
        return KeyAction::Ignore;
    }
    let event = if down { KeyEventType::Press } else { KeyEventType::Release };
    KeyAction::Send(Csi::u(key.number, Mods::of(input.flags).kitty(), event).bytes())
}

/// Keypad keys and their kitty key codes (the arrows, which also carry
/// the numeric-pad flag, are the main ones).
fn keypad_number(code: u16) -> Option<u32> {
    Some(match code {
        0x52 => 57399,
        0x53 => 57400,
        0x54 => 57401,
        0x55 => 57402,
        0x56 => 57403,
        0x57 => 57404,
        0x58 => 57405,
        0x59 => 57406,
        0x5B => 57407,
        0x5C => 57408,
        0x41 => 57409, // KP_DECIMAL
        0x4B => 57410, // KP_DIVIDE
        0x43 => 57411, // KP_MULTIPLY
        0x4E => 57412, // KP_SUBTRACT
        0x45 => 57413, // KP_ADD
        0x4C => 57414, // KP_ENTER
        0x51 => 57415, // KP_EQUAL
        _ => return None,
    })
}

/// Keys with no legacy encoding, sent only under the kitty protocol:
/// F13–F20 and the PC keyboard's context-menu key.
fn kitty_only_key(code: u16) -> Option<u32> {
    Some(match code {
        0x69 => 57376,
        0x6B => 57377,
        0x71 => 57378,
        0x6A => 57379,
        0x40 => 57380,
        0x4F => 57381,
        0x50 => 57382,
        0x5A => 57383,
        0x6E => 57363,
        _ => return None,
    })
}

/// Which keys are held down, so a release is sent only for a key whose
/// press went to the program (not one whose press opened a menu or went
/// to an input method).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeysDown([u64; 2]);

impl KeysDown {
    pub fn press(&mut self, code: u16) {
        if let Some(w) = self.0.get_mut(code as usize / 64) {
            *w |= 1 << (code % 64);
        }
    }

    /// Whether `code` was down (it no longer is).
    pub fn release(&mut self, code: u16) -> bool {
        let Some(w) = self.0.get_mut(code as usize / 64) else {
            return false;
        };
        let was = *w & 1 << (code % 64) != 0;
        *w &= !(1 << (code % 64));
        was
    }
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
    /// Cmd (the kitty protocol's super).
    pub sup: bool,
    pub caps_lock: bool,
}

/// The kitty protocol's lock modifier bits (caps lock, num lock).
const LOCKS: u8 = 64 | 128;

impl Mods {
    fn of(f: usize) -> Mods {
        Mods {
            shift: f & flags::SHIFT != 0,
            alt: f & flags::OPTION != 0,
            ctrl: f & flags::CONTROL != 0,
            sup: f & flags::COMMAND != 0,
            caps_lock: f & flags::CAPS_LOCK != 0,
        }
    }

    /// xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }

    /// The kitty protocol's modifier bits: shift 1, alt 2, ctrl 4, super
    /// 8, caps lock 64 (macOS has no num lock).
    fn kitty(self) -> u8 {
        self.shift as u8 | (self.alt as u8) << 1 | (self.ctrl as u8) << 2 | (self.sup as u8) << 3 | (self.caps_lock as u8) << 6
    }

    /// [`Mods::kitty`] without the lock bits, which text keys report only
    /// under flag 8.
    fn without_locks(self) -> u8 {
        self.kitty() & !LOCKS
    }

    /// Shift, Alt or Ctrl is held (what xterm encodes).
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
        Key::F(n) => tilde(legacy_f_number(n)),
        Key::Backspace => return esc_if_alt(if mods.ctrl { b"\x08" } else { b"\x7f" }),
        Key::Tab => {
            return if mods.shift { b"\x1b[Z".to_vec() } else { esc_if_alt(b"\t") };
        }
        Key::Enter => return esc_if_alt(b"\r"),
        Key::Escape => return esc_if_alt(b"\x1b"),
    };
    s.into_bytes()
}

/// `CSI n ~` numbers of F5–F12.
fn legacy_f_number(n: u8) -> u8 {
    match n {
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        _ => 24,
    }
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
#[allow(clippy::type_complexity)] // tables of rows
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

    // Kitty keyboard protocol, beyond flag 1. The expected bytes are the
    // spec's (https://sw.kovidgoyal.net/kitty/keyboard-protocol/), written
    // out by hand.

    fn kitty_modes(flags: u8) -> KeyModes {
        KeyModes {
            kitty_flags: flags,
            ..KeyModes::default()
        }
    }

    fn with_event(input: KeyInput, event: KeyEventType) -> KeyInput {
        KeyInput { event, ..input }
    }

    fn check(rows: &[(&str, KeyInput, KeyModes, KeyAction)]) {
        for (name, input, m, want) in rows {
            assert_eq!(show(&translate(input, *m)), show(want), "{name}");
        }
    }

    #[test]
    fn kitty_modifier_encoding() {
        // 1 + shift 1 + alt 2 + ctrl 4 + super 8 + caps lock 64, on Escape
        // (flag 1), which reports every modifier.
        let alt = OPTION | LEFT_OPTION;
        let rows: [(usize, &[u8]); 10] = [
            (0, b"\x1b[27u"),
            (SHIFT, b"\x1b[27;2u"),
            (alt, b"\x1b[27;3u"),
            (SHIFT | alt, b"\x1b[27;4u"),
            (CONTROL, b"\x1b[27;5u"),
            (SHIFT | CONTROL, b"\x1b[27;6u"),
            (alt | CONTROL, b"\x1b[27;7u"),
            (SHIFT | alt | CONTROL, b"\x1b[27;8u"),
            (CAPS_LOCK, b"\x1b[27;65u"),
            (CAPS_LOCK | CONTROL | SHIFT, b"\x1b[27;70u"),
        ];
        for (flags, want) in rows {
            assert_eq!(
                show(&translate(&key(0x35, flags), kitty_modes(1))),
                show(&send(want)),
                "escape {flags:#x}"
            );
        }
        // Super is Cmd, seen on the Cmd keys themselves (flag 8).
        assert_eq!(
            translate(&key(0x37, COMMAND | LEFT_COMMAND), kitty_modes(8)),
            send(b"\x1b[57444;9u")
        );
        assert_eq!(
            translate(&key(0x36, COMMAND | RIGHT_COMMAND | SHIFT), kitty_modes(8)),
            send(b"\x1b[57450;10u")
        );
        // Text keys under flag 1 leave the lock modifiers out; flag 8
        // reports them.
        assert_eq!(translate(&text("a", CONTROL | CAPS_LOCK), kitty_modes(1)), send(b"\x1b[97;5u"));
        assert_eq!(translate(&text("a", CONTROL | CAPS_LOCK), kitty_modes(8)), send(b"\x1b[97;69u"));
        // Enter, Tab and Backspace stay legacy under flag 1 with only a
        // lock modifier.
        assert_eq!(translate(&key(0x24, CAPS_LOCK), kitty_modes(1)), send(b"\r"));
        assert_eq!(translate(&key(0x24, CAPS_LOCK | SHIFT), kitty_modes(1)), send(b"\x1b[13;66u"));
    }

    #[test]
    fn kitty_functional_key_codes() {
        // (name, keycode, numpad, no modifiers, with ctrl): the spec's
        // functional key table under flag 1.
        let rows: &[(&str, u16, bool, &[u8], &[u8])] = &[
            ("escape", 0x35, false, b"\x1b[27u", b"\x1b[27;5u"),
            ("enter", 0x24, false, b"\r", b"\x1b[13;5u"),
            ("tab", 0x30, false, b"\t", b"\x1b[9;5u"),
            ("backspace", 0x33, false, b"\x7f", b"\x1b[127;5u"),
            ("insert", 0x72, false, b"\x1b[2~", b"\x1b[2;5~"),
            ("delete", 0x75, false, b"\x1b[3~", b"\x1b[3;5~"),
            ("left", 0x7B, true, b"\x1b[D", b"\x1b[1;5D"),
            ("right", 0x7C, true, b"\x1b[C", b"\x1b[1;5C"),
            ("up", 0x7E, true, b"\x1b[A", b"\x1b[1;5A"),
            ("down", 0x7D, true, b"\x1b[B", b"\x1b[1;5B"),
            ("page up", 0x74, false, b"\x1b[5~", b"\x1b[5;5~"),
            ("page down", 0x79, false, b"\x1b[6~", b"\x1b[6;5~"),
            ("home", 0x73, false, b"\x1b[H", b"\x1b[1;5H"),
            ("end", 0x77, false, b"\x1b[F", b"\x1b[1;5F"),
            ("f1", 0x7A, false, b"\x1b[P", b"\x1b[1;5P"),
            ("f2", 0x78, false, b"\x1b[Q", b"\x1b[1;5Q"),
            ("f3", 0x63, false, b"\x1b[13~", b"\x1b[13;5~"),
            ("f4", 0x76, false, b"\x1b[S", b"\x1b[1;5S"),
            ("f5", 0x60, false, b"\x1b[15~", b"\x1b[15;5~"),
            ("f6", 0x61, false, b"\x1b[17~", b"\x1b[17;5~"),
            ("f7", 0x62, false, b"\x1b[18~", b"\x1b[18;5~"),
            ("f8", 0x64, false, b"\x1b[19~", b"\x1b[19;5~"),
            ("f9", 0x65, false, b"\x1b[20~", b"\x1b[20;5~"),
            ("f10", 0x6D, false, b"\x1b[21~", b"\x1b[21;5~"),
            ("f11", 0x67, false, b"\x1b[23~", b"\x1b[23;5~"),
            ("f12", 0x6F, false, b"\x1b[24~", b"\x1b[24;5~"),
            ("f13", 0x69, false, b"\x1b[57376u", b"\x1b[57376;5u"),
            ("f14", 0x6B, false, b"\x1b[57377u", b"\x1b[57377;5u"),
            ("f15", 0x71, false, b"\x1b[57378u", b"\x1b[57378;5u"),
            ("f16", 0x6A, false, b"\x1b[57379u", b"\x1b[57379;5u"),
            ("f17", 0x40, false, b"\x1b[57380u", b"\x1b[57380;5u"),
            ("f18", 0x4F, false, b"\x1b[57381u", b"\x1b[57381;5u"),
            ("f19", 0x50, false, b"\x1b[57382u", b"\x1b[57382;5u"),
            ("f20", 0x5A, false, b"\x1b[57383u", b"\x1b[57383;5u"),
            ("menu", 0x6E, false, b"\x1b[57363u", b"\x1b[57363;5u"),
            ("kp enter", 0x4C, true, b"\x1b[57414u", b"\x1b[57414;5u"),
        ];
        for &(name, code, numpad, plain, ctrl) in rows {
            // Cursor key mode doesn't change the kitty forms.
            for app_cursor in [false, true] {
                let m = KeyModes {
                    app_cursor,
                    app_keypad: true,
                    ..kitty_modes(1)
                };
                let input = |flags| KeyInput {
                    numpad,
                    ..key(code, flags)
                };
                assert_eq!(show(&translate(&input(0), m)), show(&send(plain)), "{name}");
                assert_eq!(show(&translate(&input(CONTROL), m)), show(&send(ctrl)), "ctrl+{name}");
            }
        }
        // Without the protocol, F13 and up keep going to the input system.
        assert_eq!(translate(&key(0x69, 0), KeyModes::default()), KeyAction::ToTextInput);
        // Keypad keys that type text: text under flag 1, their own codes
        // with Ctrl or under flag 8 (with the text under flag 16).
        let keypad: [(u16, &str, u32); 16] = [
            (0x52, "0", 57399),
            (0x53, "1", 57400),
            (0x54, "2", 57401),
            (0x55, "3", 57402),
            (0x56, "4", 57403),
            (0x57, "5", 57404),
            (0x58, "6", 57405),
            (0x59, "7", 57406),
            (0x5B, "8", 57407),
            (0x5C, "9", 57408),
            (0x41, ".", 57409),
            (0x4B, "/", 57410),
            (0x43, "*", 57411),
            (0x4E, "-", 57412),
            (0x45, "+", 57413),
            (0x51, "=", 57415),
        ];
        for (code, c, n) in keypad {
            let input = |flags| KeyInput {
                keycode: code,
                numpad: true,
                ..text(c, flags)
            };
            let cp = c.chars().next().unwrap() as u32;
            assert_eq!(translate(&input(0), kitty_modes(1)), KeyAction::ToTextInput, "kp {c}");
            assert_eq!(
                show(&translate(&input(CONTROL), kitty_modes(1))),
                show(&send(format!("\x1b[{n};5u").as_bytes())),
                "ctrl+kp {c}"
            );
            assert_eq!(
                show(&translate(&input(0), kitty_modes(8))),
                show(&send(format!("\x1b[{n}u").as_bytes())),
                "kp {c} flag 8"
            );
            assert_eq!(
                show(&translate(&input(0), kitty_modes(8 | 16))),
                show(&send(format!("\x1b[{n};;{cp}u").as_bytes())),
                "kp {c} flags 8+16"
            );
        }
    }

    #[test]
    fn kitty_event_types() {
        use KeyEventType::*;
        let flags_1_2 = kitty_modes(1 | 2);
        let rows: &[(&str, KeyInput, KeyModes, KeyAction)] = &[
            ("up", key(0x7E, 0), flags_1_2, send(b"\x1b[A")),
            ("up repeat", with_event(key(0x7E, 0), Repeat), flags_1_2, send(b"\x1b[1;1:2A")),
            ("up release", with_event(key(0x7E, 0), Release), flags_1_2, send(b"\x1b[1;1:3A")),
            (
                "ctrl+up release",
                with_event(key(0x7E, CONTROL), Release),
                flags_1_2,
                send(b"\x1b[1;5:3A"),
            ),
            ("delete release", with_event(key(0x75, 0), Release), flags_1_2, send(b"\x1b[3;1:3~")),
            ("escape repeat", with_event(key(0x35, 0), Repeat), flags_1_2, send(b"\x1b[27;1:2u")),
            (
                "escape release",
                with_event(key(0x35, 0), Release),
                flags_1_2,
                send(b"\x1b[27;1:3u"),
            ),
            // Text keys: the press and repeat type text; the release is an
            // escape code.
            ("a", text("a", 0), flags_1_2, KeyAction::ToTextInput),
            ("a repeat", with_event(text("a", 0), Repeat), flags_1_2, KeyAction::ToTextInput),
            ("a release", with_event(text("a", 0), Release), flags_1_2, send(b"\x1b[97;1:3u")),
            (
                "shift+a release",
                with_event(text("A", SHIFT), Release),
                flags_1_2,
                send(b"\x1b[97;2:3u"),
            ),
            ("ctrl+c", text("c", CONTROL), flags_1_2, send(b"\x1b[99;5u")),
            (
                "ctrl+c repeat",
                with_event(text("c", CONTROL), Repeat),
                flags_1_2,
                send(b"\x1b[99;5:2u"),
            ),
            (
                "ctrl+c release",
                with_event(text("c", CONTROL), Release),
                flags_1_2,
                send(b"\x1b[99;5:3u"),
            ),
            // Enter, Tab and Backspace: no release events without flag 8.
            ("enter release", with_event(key(0x24, 0), Release), flags_1_2, KeyAction::Ignore),
            ("tab release", with_event(key(0x30, 0), Release), flags_1_2, KeyAction::Ignore),
            ("backspace release", with_event(key(0x33, 0), Release), flags_1_2, KeyAction::Ignore),
            ("enter repeat", with_event(key(0x24, 0), Repeat), flags_1_2, send(b"\r")),
            (
                "enter release flag 8",
                with_event(key(0x24, 0), Release),
                kitty_modes(2 | 8),
                send(b"\x1b[13;1:3u"),
            ),
            (
                "tab release flag 8",
                with_event(key(0x30, 0), Release),
                kitty_modes(2 | 8),
                send(b"\x1b[9;1:3u"),
            ),
            // Without flag 2: no releases, and a repeat is a press.
            (
                "up release flag 1",
                with_event(key(0x7E, 0), Release),
                kitty_modes(1),
                KeyAction::Ignore,
            ),
            (
                "a release flag 1",
                with_event(text("a", 0), Release),
                kitty_modes(1),
                KeyAction::Ignore,
            ),
            (
                "a release legacy",
                with_event(text("a", 0), Release),
                KeyModes::default(),
                KeyAction::Ignore,
            ),
            (
                "escape repeat flag 1",
                with_event(key(0x35, 0), Repeat),
                kitty_modes(1),
                send(b"\x1b[27u"),
            ),
            (
                "up repeat legacy",
                with_event(key(0x7E, 0), Repeat),
                KeyModes::default(),
                send(b"\x1b[A"),
            ),
            // Flag 2 alone: presses stay legacy, releases are escape codes.
            ("escape flag 2", key(0x35, 0), kitty_modes(2), send(b"\x1b")),
            (
                "escape release flag 2",
                with_event(key(0x35, 0), Release),
                kitty_modes(2),
                send(b"\x1b[27;1:3u"),
            ),
            // Cmd keys and keys for the input method have no releases.
            (
                "cmd+c release",
                with_event(text("c", COMMAND), Release),
                flags_1_2,
                KeyAction::Ignore,
            ),
            (
                "composing release",
                with_event(text("a", 0), Release),
                KeyModes {
                    composing: true,
                    ..flags_1_2
                },
                KeyAction::Ignore,
            ),
        ];
        check(rows);
    }

    #[test]
    fn kitty_modifier_keys() {
        // (name, keycode, held bit, kitty code)
        let keys = [
            ("left shift", 0x38, SHIFT | LEFT_SHIFT, 57441, 2),
            ("left control", 0x3B, CONTROL | LEFT_CONTROL, 57442, 5),
            ("left alt", 0x3A, OPTION | LEFT_OPTION, 57443, 3),
            ("left super", 0x37, COMMAND | LEFT_COMMAND, 57444, 9),
            ("right shift", 0x3C, SHIFT | RIGHT_SHIFT, 57447, 2),
            ("right control", 0x3E, CONTROL | RIGHT_CONTROL, 57448, 5),
            ("right alt", 0x3D, OPTION | RIGHT_OPTION, 57449, 3),
            ("right super", 0x36, COMMAND | RIGHT_COMMAND, 57450, 9),
        ];
        for (name, code, held, n, m) in keys {
            // Pressed: the flags include it; released: they don't.
            let down = key(code, held);
            let up = key(code, 0);
            assert_eq!(
                show(&translate(&down, kitty_modes(8))),
                show(&send(format!("\x1b[{n};{m}u").as_bytes())),
                "{name}"
            );
            assert_eq!(
                show(&translate(&up, kitty_modes(8 | 2))),
                show(&send(format!("\x1b[{n};1:3u").as_bytes())),
                "{name} up"
            );
            assert_eq!(translate(&up, kitty_modes(8)), KeyAction::Ignore, "{name} up without flag 2");
            for m in [KeyModes::default(), kitty_modes(1), kitty_modes(1 | 2 | 4 | 16)] {
                assert_eq!(translate(&down, m), KeyAction::Ignore, "{name} {m:?}");
            }
        }
        // Left Control up while Right Control is held: ctrl stays set.
        assert_eq!(
            translate(&key(0x3B, CONTROL | RIGHT_CONTROL), kitty_modes(8 | 2)),
            send(b"\x1b[57442;5:3u")
        );
        // Caps Lock reports a press each time it toggles.
        assert_eq!(translate(&key(0x39, CAPS_LOCK), kitty_modes(8 | 2)), send(b"\x1b[57358;65u"));
        assert_eq!(translate(&key(0x39, 0), kitty_modes(8 | 2)), send(b"\x1b[57358u"));
    }

    #[test]
    fn kitty_alternate_keys() {
        let alternates = kitty_modes(1 | 4);
        let ctrl_plus = KeyInput {
            chars_unshifted: "=".into(),
            ..text("+", CONTROL | SHIFT)
        };
        let rows: &[(&str, KeyInput, KeyModes, KeyAction)] = &[
            ("ctrl+a", text("a", CONTROL), alternates, send(b"\x1b[97;5u")),
            // The shifted key only with Shift.
            ("ctrl+shift+a", text("A", CONTROL | SHIFT), alternates, send(b"\x1b[97:65;6u")),
            ("ctrl+shift+= is ctrl+plus", ctrl_plus, alternates, send(b"\x1b[61:43;6u")),
            // The base layout key only when it differs: Ctrl+С on a
            // Russian layout is Ctrl+C on the standard one.
            (
                "ru ctrl+с",
                non_latin("с", "С", 'c', 'C', CONTROL),
                alternates,
                send("\x1b[1089::99;5u".as_bytes()),
            ),
            (
                "ru ctrl+shift+с",
                non_latin("с", "С", 'c', 'C', CONTROL | SHIFT),
                alternates,
                send("\x1b[1089:1057:99;6u".as_bytes()),
            ),
            (
                "ru ctrl+с flag 1",
                non_latin("с", "С", 'c', 'C', CONTROL),
                kitty_modes(1),
                send("\x1b[1089;5u".as_bytes()),
            ),
            (
                "us base is the key",
                KeyInput {
                    ascii: Some('c'),
                    ..text("c", CONTROL)
                },
                alternates,
                send(b"\x1b[99;5u"),
            ),
            // Functional keys have no alternates.
            ("shift+up", key(0x7E, SHIFT), alternates, send(b"\x1b[1;2A")),
            ("shift+escape", key(0x35, SHIFT), alternates, send(b"\x1b[27;2u")),
            // Releases carry them too.
            (
                "ctrl+shift+a release",
                with_event(text("A", CONTROL | SHIFT), KeyEventType::Release),
                kitty_modes(1 | 2 | 4),
                send(b"\x1b[97:65;6:3u"),
            ),
            // Plain text stays text: flag 4 changes only escape codes.
            ("shift+a", text("A", SHIFT), alternates, KeyAction::ToTextInput),
        ];
        check(rows);
    }

    #[test]
    fn kitty_all_keys_and_associated_text() {
        let all = kitty_modes(8);
        let all_text = kitty_modes(8 | 16);
        let alt = OPTION | LEFT_OPTION;
        // Text keys go to the input method; what it types is encoded.
        assert_eq!(translate(&text("a", 0), all), KeyAction::ToTextInput);
        assert_eq!(translate(&text("A", SHIFT), all_text), KeyAction::ToTextInput);
        let option_a = KeyInput {
            chars: "å".into(),
            chars_ignoring_mods: "a".into(),
            chars_shift_only: "a".into(),
            flags: alt,
            ..KeyInput::default()
        };
        let a_repeat = with_event(text("a", 0), KeyEventType::Repeat);
        let rows: &[(&str, Option<&KeyInput>, KeyModes, &str, &[u8])] = &[
            ("a", Some(&text("a", 0)), all, "a", b"\x1b[97u"),
            ("shift+a", Some(&text("A", SHIFT)), all, "A", b"\x1b[97;2u"),
            // The spec's example: shift+a -> CSI 97 ; 2 ; 65 u.
            ("shift+a text", Some(&text("A", SHIFT)), all_text, "A", b"\x1b[97;2;65u"),
            ("a text", Some(&text("a", 0)), all_text, "a", b"\x1b[97;;97u"),
            ("space", Some(&text(" ", 0)), all, " ", b"\x1b[32u"),
            ("caps lock a", Some(&text("A", CAPS_LOCK)), all_text, "A", b"\x1b[97;65;65u"),
            ("option+a", Some(&option_a), all_text, "å", b"\x1b[97;3;229u"),
            (
                "shift+a alternates",
                Some(&text("A", SHIFT)),
                kitty_modes(8 | 4 | 16),
                "A",
                b"\x1b[97:65;2;65u",
            ),
            ("a repeat", Some(&a_repeat), kitty_modes(8 | 2 | 16), "a", b"\x1b[97;1:2;97u"),
            ("a repeat without flag 2", Some(&a_repeat), all, "a", b"\x1b[97u"),
            // Text the key didn't type itself (an input method's): a
            // text event, key 0, as in the spec's alt+a -> CSI 0 ; ; 229 u.
            ("ime", None, all_text, "å", b"\x1b[0;;229u"),
            ("ime two", None, all_text, "日本", b"\x1b[0;;26085:26412u"),
            ("other text than the key", Some(&text("e", 0)), all_text, "é", b"\x1b[0;;233u"),
            // Without flag 16 it has no other way to arrive: plain text.
            ("ime without 16", None, all, "é", "é".as_bytes()),
            // Without flag 8, text is text.
            ("legacy", Some(&text("a", 0)), KeyModes::default(), "a", b"a"),
            ("flag 1", Some(&text("A", SHIFT)), kitty_modes(1), "A", b"A"),
            ("flag 16 alone", Some(&text("a", 0)), kitty_modes(16), "a", b"a"),
        ];
        for &(name, input, m, typed, want) in rows {
            assert_eq!(
                String::from_utf8_lossy(&committed_text(input, m, typed)),
                String::from_utf8_lossy(want),
                "{name}"
            );
        }
        // Keys that don't go to the input method are encoded at once.
        let direct: &[(&str, KeyInput, KeyModes, KeyAction)] = &[
            ("enter", key(0x24, 0), all, send(b"\x1b[13u")),
            ("tab", key(0x30, 0), all, send(b"\x1b[9u")),
            ("backspace", key(0x33, 0), all, send(b"\x1b[127u")),
            ("shift+enter", key(0x24, SHIFT), all, send(b"\x1b[13;2u")),
            ("escape", key(0x35, 0), all, send(b"\x1b[27u")),
            ("up", key(0x7E, 0), all, send(b"\x1b[A")),
            // Control codes are never associated text.
            (
                "ctrl+a",
                KeyInput {
                    chars: "\x01".into(),
                    ..text("a", CONTROL)
                },
                all_text,
                send(b"\x1b[97;5u"),
            ),
            // Option is reported as Alt under flag 8, Meta or not.
            ("ctrl+option+a", text("a", CONTROL | alt), all, send(b"\x1b[97;7u")),
            (
                "a release",
                with_event(text("a", 0), KeyEventType::Release),
                kitty_modes(8 | 2 | 16),
                send(b"\x1b[97;1:3u"),
            ),
            // An input method composing still gets every key.
            (
                "composing",
                key(0x24, 0),
                KeyModes { composing: true, ..all },
                KeyAction::ToTextInput,
            ),
        ];
        check(direct);
    }

    /// The spec's legacy text key table for `i`, `3` and `;` (with Option
    /// as Meta), and the same keys under flag 1. (Its Ctrl+Shift column is
    /// `CSI u` in kitty's legacy mode; decline keeps xterm's bytes there.)
    #[test]
    fn kitty_spec_text_key_examples() {
        let meta = KeyModes {
            option_as_meta: OptionAsMeta::Both,
            ..KeyModes::default()
        };
        let alt = OPTION | LEFT_OPTION;
        let press = |plain: &str, shifted: &str, flags: usize| KeyInput {
            chars_unshifted: plain.into(),
            ..text(if flags & SHIFT != 0 { shifted } else { plain }, flags)
        };
        // (key, shifted, alt, ctrl, shift+alt, alt+ctrl)
        let legacy: [(&str, &str, &[u8], KeyAction, &[u8], &[u8]); 3] = [
            ("i", "I", b"\x1bi", send(b"\t"), b"\x1bI", b"\x1b\t"),
            ("3", "#", b"\x1b3", send(b"\x1b"), b"\x1b#", b"\x1b\x1b"),
            (";", ":", b"\x1b;", KeyAction::ToTextInput, b"\x1b:", b"\x1b;"),
        ];
        for (k, s, a, c, sa, ac) in legacy {
            assert_eq!(translate(&press(k, s, 0), meta), KeyAction::ToTextInput, "{k}");
            assert_eq!(translate(&press(k, s, SHIFT), meta), KeyAction::ToTextInput, "shift+{k}");
            assert_eq!(translate(&press(k, s, alt), meta), send(a), "alt+{k}");
            assert_eq!(translate(&press(k, s, CONTROL), meta), c, "ctrl+{k}");
            assert_eq!(translate(&press(k, s, SHIFT | alt), meta), send(sa), "shift+alt+{k}");
            assert_eq!(translate(&press(k, s, alt | CONTROL), meta), send(ac), "alt+ctrl+{k}");
        }
        let flag1 = KeyModes { kitty_flags: 1, ..meta };
        for (k, s, n) in [("i", "I", 105), ("3", "#", 51), (";", ":", 59)] {
            let want = |m: u8| send(format!("\x1b[{n};{m}u").as_bytes());
            assert_eq!(translate(&press(k, s, alt), flag1), want(3), "alt+{k}");
            assert_eq!(translate(&press(k, s, CONTROL), flag1), want(5), "ctrl+{k}");
            assert_eq!(translate(&press(k, s, SHIFT | alt), flag1), want(4), "shift+alt+{k}");
            assert_eq!(translate(&press(k, s, alt | CONTROL), flag1), want(7), "alt+ctrl+{k}");
            assert_eq!(translate(&press(k, s, CONTROL | SHIFT), flag1), want(6), "ctrl+shift+{k}");
            assert_eq!(translate(&press(k, s, SHIFT), flag1), KeyAction::ToTextInput, "shift+{k}");
        }
    }

    #[test]
    fn keys_down_for_releases() {
        let mut down = KeysDown::default();
        assert!(!down.release(0x24));
        down.press(0x24);
        down.press(0x7E);
        down.press(200);
        assert!(down.release(0x24));
        assert!(!down.release(0x24), "released once");
        assert!(down.release(0x7E));
        assert!(!down.release(200), "out of range keys are never down");
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
