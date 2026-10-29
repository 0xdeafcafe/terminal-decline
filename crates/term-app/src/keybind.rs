//! The `[keybind]` config: remap or unbind the app's shortcuts, and make
//! keys send text.
//!
//! ```toml
//! [keybind]
//! "cmd+k" = "clear_scrollback"
//! "cmd+d" = "none"
//! "ctrl+shift+t" = { text = "\u001b[116;6u" }
//! "cmd+enter" = "toggle_full_screen"
//! ```
//!
//! The defaults are the menus' own shortcuts ([`DEFAULTS`]); each entry
//! replaces or removes one chord. The menus are built from the result, so
//! they show the shortcuts that work. A pin's config is layered on top of
//! the user's like any other setting, so it can override single chords.
//!
//! A chord bound to a menu action is the menu's (AppKit matches it before
//! the view sees the key, and the view performs it for chords without
//! Cmd); one bound to text sends that text; an unbound key does what it
//! did with no config: [`crate::keys::translate`] sends it to the program
//! or, for Cmd keys, drops it.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CStr;
use std::fmt;

use serde::{Deserialize, Deserializer};

use crate::keys::{KeyEventType, KeyInput, flags};

/// The app's actions that can be bound: its menu items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MenuAction {
    NewWindow,
    NewTab,
    Close,
    Copy,
    Paste,
    SelectAll,
    ClearScrollback,
    FontBigger,
    FontSmaller,
    FontReset,
    ToggleFullScreen,
    NextTab,
    PreviousTab,
    /// Edit > Find…: a placeholder until the find bar exists (its menu
    /// item stays disabled).
    Find,
}

impl MenuAction {
    pub const ALL: [MenuAction; 14] = [
        MenuAction::NewWindow,
        MenuAction::NewTab,
        MenuAction::Close,
        MenuAction::Copy,
        MenuAction::Paste,
        MenuAction::SelectAll,
        MenuAction::ClearScrollback,
        MenuAction::FontBigger,
        MenuAction::FontSmaller,
        MenuAction::FontReset,
        MenuAction::ToggleFullScreen,
        MenuAction::NextTab,
        MenuAction::PreviousTab,
        MenuAction::Find,
    ];

    /// The name in the config.
    pub fn name(self) -> &'static str {
        match self {
            MenuAction::NewWindow => "new_window",
            MenuAction::NewTab => "new_tab",
            MenuAction::Close => "close",
            MenuAction::Copy => "copy",
            MenuAction::Paste => "paste",
            MenuAction::SelectAll => "select_all",
            MenuAction::ClearScrollback => "clear_scrollback",
            MenuAction::FontBigger => "font_bigger",
            MenuAction::FontSmaller => "font_smaller",
            MenuAction::FontReset => "font_reset",
            MenuAction::ToggleFullScreen => "toggle_full_screen",
            MenuAction::NextTab => "next_tab",
            MenuAction::PreviousTab => "previous_tab",
            MenuAction::Find => "find",
        }
    }

    /// The menu item's action, sent up the responder chain.
    pub fn selector(self) -> &'static CStr {
        match self {
            MenuAction::NewWindow => c"newWindow:",
            MenuAction::NewTab => c"newTab:",
            MenuAction::Close => c"performClose:",
            MenuAction::Copy => c"copy:",
            MenuAction::Paste => c"paste:",
            MenuAction::SelectAll => c"selectAll:",
            MenuAction::ClearScrollback => c"clearBuffer:",
            MenuAction::FontBigger => c"increaseFontSize:",
            MenuAction::FontSmaller => c"decreaseFontSize:",
            MenuAction::FontReset => c"resetFontSize:",
            MenuAction::ToggleFullScreen => c"toggleFullScreen:",
            MenuAction::NextTab => c"selectNextTab:",
            MenuAction::PreviousTab => c"selectPreviousTab:",
            MenuAction::Find => c"declineFind:",
        }
    }

    fn from_name(name: &str) -> Option<MenuAction> {
        MenuAction::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// The menus' shortcuts with no config.
pub const DEFAULTS: [(&str, MenuAction); 13] = [
    ("cmd+n", MenuAction::NewWindow),
    ("cmd+t", MenuAction::NewTab),
    ("cmd+w", MenuAction::Close),
    ("cmd+c", MenuAction::Copy),
    ("cmd+v", MenuAction::Paste),
    ("cmd+a", MenuAction::SelectAll),
    ("cmd+k", MenuAction::ClearScrollback),
    ("cmd+=", MenuAction::FontBigger),
    ("cmd+-", MenuAction::FontSmaller),
    ("cmd+0", MenuAction::FontReset),
    ("ctrl+cmd+f", MenuAction::ToggleFullScreen),
    ("cmd+}", MenuAction::NextTab),
    ("cmd+{", MenuAction::PreviousTab),
];

/// Keys known by name rather than by the character they type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Named {
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

const NAMES: [(&str, Named); 14] = [
    ("enter", Named::Enter),
    ("tab", Named::Tab),
    ("escape", Named::Escape),
    ("backspace", Named::Backspace),
    ("delete", Named::Delete),
    ("insert", Named::Insert),
    ("up", Named::Up),
    ("down", Named::Down),
    ("left", Named::Left),
    ("right", Named::Right),
    ("home", Named::Home),
    ("end", Named::End),
    ("page_up", Named::PageUp),
    ("page_down", Named::PageDown),
];

impl Named {
    fn parse(s: &str) -> Option<Named> {
        let s = match s {
            "return" => "enter",
            "esc" => "escape",
            "pageup" => "page_up",
            "pagedown" => "page_down",
            s => s,
        };
        if let Some((_, n)) = NAMES.iter().find(|(name, _)| *name == s) {
            return Some(*n);
        }
        let n: u8 = s.strip_prefix('f')?.parse().ok()?;
        (1..=20).contains(&n).then_some(Named::F(n))
    }

    fn of_keycode(code: u16) -> Option<Named> {
        Some(match code {
            0x24 | 0x4C => Named::Enter,
            0x30 => Named::Tab,
            0x35 => Named::Escape,
            0x33 => Named::Backspace,
            0x75 => Named::Delete,
            0x72 => Named::Insert,
            0x7E => Named::Up,
            0x7D => Named::Down,
            0x7B => Named::Left,
            0x7C => Named::Right,
            0x73 => Named::Home,
            0x77 => Named::End,
            0x74 => Named::PageUp,
            0x79 => Named::PageDown,
            0x7A => Named::F(1),
            0x78 => Named::F(2),
            0x63 => Named::F(3),
            0x76 => Named::F(4),
            0x60 => Named::F(5),
            0x61 => Named::F(6),
            0x62 => Named::F(7),
            0x64 => Named::F(8),
            0x65 => Named::F(9),
            0x6D => Named::F(10),
            0x67 => Named::F(11),
            0x6F => Named::F(12),
            0x69 => Named::F(13),
            0x6B => Named::F(14),
            0x71 => Named::F(15),
            0x6A => Named::F(16),
            0x40 => Named::F(17),
            0x4F => Named::F(18),
            0x50 => Named::F(19),
            0x5A => Named::F(20),
            _ => return None,
        })
    }

    /// The menu key equivalent: AppKit's function-key characters.
    fn key_equivalent(self) -> char {
        let private = |n: u32| char::from_u32(n).unwrap_or('\0');
        match self {
            Named::Enter => '\r',
            Named::Tab => '\t',
            Named::Escape => '\u{1b}',
            Named::Backspace => '\u{8}',
            Named::Delete => private(0xF728),
            Named::Insert => private(0xF727),
            Named::Up => private(0xF700),
            Named::Down => private(0xF701),
            Named::Left => private(0xF702),
            Named::Right => private(0xF703),
            Named::Home => private(0xF729),
            Named::End => private(0xF72B),
            Named::PageUp => private(0xF72C),
            Named::PageDown => private(0xF72D),
            Named::F(n) => private(0xF704 + n as u32 - 1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChordKey {
    /// A key by the character it types (letters in lower case).
    Char(char),
    Named(Named),
}

/// Modifiers plus a key, as written in the config (`"ctrl+shift+t"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
    pub key: ChordKey,
}

impl Chord {
    /// Modifiers (`cmd`/`command`/`super`, `ctrl`/`control`,
    /// `alt`/`opt`/`option`, `shift`) and a key joined by `+`: a
    /// character (`k`, `=`, `+`), `space`, `plus`, or a key name (`enter`,
    /// `tab`, `escape`, `backspace`, `delete`, `insert`, `up`, `down`,
    /// `left`, `right`, `home`, `end`, `page_up`, `page_down`, `f1`–`f20`).
    pub fn parse(text: &str) -> Result<Chord, String> {
        let text = text.trim();
        let (mods, key) = if text == "+" {
            ("", "+")
        } else if let Some(m) = text.strip_suffix("++") {
            (m, "+")
        } else {
            text.rsplit_once('+').unwrap_or(("", text))
        };
        let mut chord = Chord {
            ctrl: false,
            alt: false,
            shift: false,
            cmd: false,
            key: ChordKey::Char(' '),
        };
        for m in mods.split('+').filter(|m| !m.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "super" => chord.cmd = true,
                "ctrl" | "control" => chord.ctrl = true,
                "alt" | "opt" | "option" => chord.alt = true,
                "shift" => chord.shift = true,
                _ => return Err(format!("{text:?}: unknown modifier {m:?} (use cmd, ctrl, alt or shift)")),
            }
        }
        let lower = key.to_lowercase();
        let mut chars = key.chars();
        chord.key = match (chars.next(), chars.next()) {
            (Some(c), None) => ChordKey::Char(c.to_lowercase().next().unwrap_or(c)),
            _ if lower == "space" => ChordKey::Char(' '),
            _ if lower == "plus" => ChordKey::Char('+'),
            _ => ChordKey::Named(Named::parse(&lower).ok_or_else(|| format!("{text:?}: unknown key {key:?}"))?),
        };
        Ok(chord)
    }

    /// The menu key equivalent and its modifier mask
    /// (`NSEventModifierFlags` bits).
    pub fn key_equivalent(&self) -> (String, usize) {
        let key = match self.key {
            ChordKey::Char(c) => c,
            ChordKey::Named(n) => n.key_equivalent(),
        };
        let mask = [
            (self.cmd, flags::COMMAND),
            (self.ctrl, flags::CONTROL),
            (self.alt, flags::OPTION),
            (self.shift, flags::SHIFT),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |m, (_, bit)| m | bit);
        (key.to_string(), mask)
    }

    /// The chords a key event matches: its key with the modifiers held,
    /// and with Shift held, also the shifted character without Shift
    /// (`cmd+}` is Cmd+Shift+]). Characters come from the ASCII-capable
    /// layout when known, so bindings work on non-Latin layouts too.
    fn of_event(input: &KeyInput) -> Vec<Chord> {
        let f = input.flags;
        let chord = |key, shift| Chord {
            ctrl: f & flags::CONTROL != 0,
            alt: f & flags::OPTION != 0,
            shift,
            cmd: f & flags::COMMAND != 0,
            key,
        };
        let shift = f & flags::SHIFT != 0;
        if let Some(n) = Named::of_keycode(input.keycode) {
            return vec![chord(ChordKey::Named(n), shift)];
        }
        let first = |s: &str| s.chars().next();
        let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
        let current = first(&input.chars_unshifted).or_else(|| first(&input.chars_ignoring_mods).map(lower));
        let mut out = Vec::new();
        if let Some(c) = input.ascii.or(current) {
            out.push(chord(ChordKey::Char(lower(c)), shift));
        }
        if shift {
            let shifted = input
                .ascii_shifted
                .or_else(|| first(&input.chars_shift_only))
                .or_else(|| first(&input.chars_ignoring_mods));
            if let Some(c) = shifted {
                out.push(chord(ChordKey::Char(c), false));
            }
        }
        out
    }
}

impl fmt::Display for Chord {
    /// The canonical spelling: `ctrl+alt+shift+cmd+key`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.ctrl, "ctrl+"), (self.alt, "alt+"), (self.shift, "shift+"), (self.cmd, "cmd+")] {
            if on {
                f.write_str(name)?;
            }
        }
        match self.key {
            ChordKey::Char(' ') => f.write_str("space"),
            ChordKey::Char('+') => f.write_str("plus"),
            ChordKey::Char(c) => write!(f, "{c}"),
            ChordKey::Named(Named::F(n)) => write!(f, "f{n}"),
            ChordKey::Named(n) => f.write_str(NAMES.iter().find(|(_, m)| *m == n).map_or("?", |(name, _)| name)),
        }
    }
}

/// What a chord is set to in the config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Menu(MenuAction),
    /// `"none"`: the chord does nothing of the app's; the key behaves as
    /// if unbound.
    Unbind,
    /// `{ text = "…" }`: send this text to the program.
    Text(String),
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Action, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Text {
            text: String,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Name(String),
            Text(Text),
        }
        let names = || MenuAction::ALL.map(MenuAction::name).join(", ");
        match Raw::deserialize(d)
            .map_err(|_| serde::de::Error::custom(format!("not an action: use \"none\", {{ text = \"…\" }} or one of {}", names())))?
        {
            Raw::Name(n) if n == "none" => Ok(Action::Unbind),
            Raw::Name(n) => MenuAction::from_name(&n).map(Action::Menu).ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "unknown action {n:?}: use \"none\", {{ text = \"…\" }} or one of {}",
                    names()
                ))
            }),
            Raw::Text(t) => Ok(Action::Text(t.text)),
        }
    }
}

/// The `[keybind]` table as read: chord to action.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeybindConfig(pub BTreeMap<Chord, Action>);

impl<'de> Deserialize<'de> for KeybindConfig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<KeybindConfig, D::Error> {
        let raw = BTreeMap::<String, Action>::deserialize(d)?;
        let mut out = BTreeMap::new();
        for (k, v) in raw {
            out.insert(Chord::parse(&k).map_err(serde::de::Error::custom)?, v);
        }
        Ok(KeybindConfig(out))
    }
}

/// Rewrites the keys of a config layer's `[keybind]` table in their
/// canonical spelling, so a later layer's `"command+d"` replaces an
/// earlier `"cmd+d"` when the layers merge. Keys that don't parse are left
/// for the error message.
pub fn canonicalize(table: &mut toml::Table) {
    let Some(toml::Value::Table(binds)) = table.get_mut("keybind") else {
        return;
    };
    let old = std::mem::take(binds);
    for (k, v) in old {
        let key = Chord::parse(&k).map_or(k, |c| c.to_string());
        binds.insert(key, v);
    }
}

/// What a chord does once the defaults and the config are combined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bound {
    Menu(MenuAction),
    Text(String),
}

/// The bindings in effect: the defaults with the config's entries applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keybinds {
    map: BTreeMap<Chord, Bound>,
    /// Chords the config names: the menus' fixed shortcuts (Quit, Hide,
    /// Minimize, Cmd+1…9) give them up.
    configured: BTreeSet<Chord>,
}

/// What the view does with a key before [`crate::keys::translate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision<'a> {
    /// Send this text to the program.
    Send(&'a str),
    /// Perform the menu action (a chord without Cmd, which the menus
    /// don't match).
    Perform(MenuAction),
}

impl Default for Keybinds {
    fn default() -> Keybinds {
        Keybinds::new(&KeybindConfig::default())
    }
}

impl Keybinds {
    pub fn new(config: &KeybindConfig) -> Keybinds {
        let mut map: BTreeMap<Chord, Bound> = DEFAULTS
            .iter()
            .map(|(c, a)| (Chord::parse(c).expect("default chords parse"), Bound::Menu(*a)))
            .collect();
        for (chord, action) in &config.0 {
            match action {
                Action::Unbind => map.remove(chord),
                Action::Menu(a) => map.insert(*chord, Bound::Menu(*a)),
                Action::Text(t) => map.insert(*chord, Bound::Text(t.clone())),
            };
        }
        Keybinds {
            map,
            configured: config.0.keys().copied().collect(),
        }
    }

    /// What `chord` is bound to.
    pub fn get(&self, chord: &Chord) -> Option<&Bound> {
        self.map.get(chord)
    }

    /// The binding a key event matches, if any.
    pub fn resolve(&self, input: &KeyInput) -> Option<&Bound> {
        Chord::of_event(input).iter().find_map(|c| self.map.get(c))
    }

    /// What the view does with a key press before translating it. `None`:
    /// translate it as usual (unbound, or a Cmd chord the menus own).
    pub fn decide(&self, input: &KeyInput) -> Option<Decision<'_>> {
        if input.event == KeyEventType::Release {
            return None;
        }
        match self.resolve(input)? {
            Bound::Text(t) => Some(Decision::Send(t)),
            Bound::Menu(a) if input.flags & flags::COMMAND == 0 => Some(Decision::Perform(*a)),
            Bound::Menu(_) => None,
        }
    }

    /// The chords bound to `action`, its default first if it still has
    /// it: the first is shown in the menu, the rest work hidden.
    pub fn chords(&self, action: MenuAction) -> Vec<Chord> {
        let default = DEFAULTS.iter().find(|(_, a)| *a == action).and_then(|(c, _)| Chord::parse(c).ok());
        let mut out: Vec<Chord> = self
            .map
            .iter()
            .filter(|(_, b)| **b == Bound::Menu(action))
            .map(|(c, _)| *c)
            .collect();
        if let Some(d) = default
            && let Some(i) = out.iter().position(|c| *c == d)
        {
            out.remove(i);
            out.insert(0, d);
        }
        out
    }

    /// Whether a fixed menu shortcut (`chord`, as `"cmd+q"`) keeps its
    /// key: not if the config binds that chord to anything.
    pub fn keeps_fixed(&self, chord: &str) -> bool {
        Chord::parse(chord).is_ok_and(|c| !self.configured.contains(&c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::flags::*;
    use crate::keys::{KeyAction, KeyModes, translate};

    fn chord(s: &str) -> Chord {
        Chord::parse(s).unwrap()
    }

    fn config(toml_text: &str) -> KeybindConfig {
        #[derive(Deserialize)]
        struct T {
            keybind: KeybindConfig,
        }
        toml::from_str::<T>(toml_text).unwrap().keybind
    }

    fn key(c: &str, flags: usize) -> KeyInput {
        KeyInput {
            chars: c.into(),
            chars_ignoring_mods: c.into(),
            chars_shift_only: c.into(),
            flags,
            ..KeyInput::default()
        }
    }

    #[test]
    fn chords_parse_and_print() {
        for (text, canonical) in [
            ("cmd+k", "cmd+k"),
            ("command+K", "cmd+k"),
            ("shift+cmd+t", "shift+cmd+t"),
            ("cmd+ctrl+alt+shift+x", "ctrl+alt+shift+cmd+x"),
            ("super+opt+option+a", "alt+cmd+a"),
            ("control+enter", "ctrl+enter"),
            ("cmd+return", "cmd+enter"),
            ("ctrl+esc", "ctrl+escape"),
            ("cmd+=", "cmd+="),
            ("cmd++", "cmd+plus"),
            ("cmd+plus", "cmd+plus"),
            ("+", "plus"),
            ("ctrl+space", "ctrl+space"),
            ("cmd+}", "cmd+}"),
            ("f5", "f5"),
            ("shift+F20", "shift+f20"),
            ("alt+PageUp", "alt+page_up"),
            ("cmd+page_down", "cmd+page_down"),
            ("ctrl+shift+left", "ctrl+shift+left"),
            ("cmd+с", "cmd+с"),
        ] {
            let c = Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(c.to_string(), canonical, "{text}");
            assert_eq!(Chord::parse(canonical), Ok(c), "{canonical} round trip");
        }
        for (text, why) in [
            ("hyper+k", "unknown modifier"),
            ("cmd+kk", "unknown key"),
            ("cmd+f21", "unknown key"),
            ("cmd+", "unknown key"),
            ("", "unknown key"),
            ("cmd+f0", "unknown key"),
        ] {
            let e = Chord::parse(text).unwrap_err();
            assert!(e.contains(why), "{text:?}: {e}");
        }
    }

    #[test]
    fn actions_parse() {
        let c = config(
            "[keybind]\n\"cmd+k\" = \"clear_scrollback\"\n\"cmd+d\" = \"none\"\n\
             \"ctrl+shift+t\" = { text = \"\\u001b[116;6u\" }\n\"cmd+enter\" = \"toggle_full_screen\"\n",
        );
        assert_eq!(c.0.len(), 4);
        assert_eq!(c.0[&chord("cmd+k")], Action::Menu(MenuAction::ClearScrollback));
        assert_eq!(c.0[&chord("cmd+d")], Action::Unbind);
        assert_eq!(c.0[&chord("ctrl+shift+t")], Action::Text("\x1b[116;6u".into()));
        assert_eq!(c.0[&chord("cmd+enter")], Action::Menu(MenuAction::ToggleFullScreen));
        for a in MenuAction::ALL {
            let c = config(&format!("[keybind]\n\"cmd+x\" = \"{}\"", a.name()));
            assert_eq!(c.0[&chord("cmd+x")], Action::Menu(a), "{}", a.name());
        }
        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct T {
            keybind: KeybindConfig,
        }
        for (bad, why) in [
            ("\"cmd+x\" = \"explode\"", "unknown action \"explode\""),
            ("\"cmd+x\" = 3", "not an action"),
            ("\"cmd+x\" = { txt = \"a\" }", "not an action"),
            ("\"hyper+x\" = \"copy\"", "unknown modifier"),
        ] {
            let e = toml::from_str::<T>(&format!("[keybind]\n{bad}")).unwrap_err();
            assert!(e.message().contains(why), "{bad}: {}", e.message());
        }
    }

    #[test]
    fn defaults_are_the_menus_shortcuts() {
        let b = Keybinds::default();
        for (c, a) in DEFAULTS {
            assert_eq!(b.get(&chord(c)), Some(&Bound::Menu(a)), "{c}");
            assert_eq!(b.chords(a), [chord(c)], "{c}");
        }
        assert!(b.chords(MenuAction::Find).is_empty());
        assert_eq!(chord("ctrl+cmd+f").key_equivalent(), ("f".into(), COMMAND | CONTROL));
        assert_eq!(chord("cmd+}").key_equivalent(), ("}".into(), COMMAND));
        assert_eq!(chord("cmd+enter").key_equivalent(), ("\r".into(), COMMAND));
        assert_eq!(chord("shift+cmd+up").key_equivalent(), ("\u{F700}".into(), COMMAND | SHIFT));
        assert_eq!(chord("alt+f12").key_equivalent(), ("\u{F70F}".into(), OPTION));
    }

    #[test]
    fn config_remaps_unbinds_and_adds() {
        let b = Keybinds::new(&config(
            "[keybind]\n\"cmd+k\" = \"none\"\n\"cmd+l\" = \"clear_scrollback\"\n\"cmd+enter\" = \"toggle_full_screen\"\n\
             \"cmd+t\" = { text = \"hi\" }\n\"ctrl+shift+n\" = \"new_tab\"\n\"cmd+q\" = \"none\"\n",
        ));
        // Clear Scrollback moved to Cmd+L.
        assert_eq!(b.chords(MenuAction::ClearScrollback), [chord("cmd+l")]);
        assert_eq!(b.get(&chord("cmd+k")), None);
        // Full screen keeps its default, shown first, and gains Cmd+Enter.
        assert_eq!(b.chords(MenuAction::ToggleFullScreen), [chord("ctrl+cmd+f"), chord("cmd+enter")]);
        // Cmd+T sends text, so New Tab has only the new chord.
        assert_eq!(b.get(&chord("cmd+t")), Some(&Bound::Text("hi".into())));
        assert_eq!(b.chords(MenuAction::NewTab), [chord("ctrl+shift+n")]);
        // Fixed shortcuts give up chords the config names.
        assert!(!b.keeps_fixed("cmd+q"));
        assert!(b.keeps_fixed("cmd+h"));
        assert!(b.keeps_fixed("alt+cmd+h"));
        assert!(Keybinds::default().keeps_fixed("cmd+q"));
    }

    #[test]
    fn key_events_resolve_to_bindings() {
        let b = Keybinds::new(&config(
            "[keybind]\n\"ctrl+shift+t\" = { text = \"\\u001b[116;6u\" }\n\"cmd+enter\" = \"toggle_full_screen\"\n\
             \"ctrl+alt+n\" = \"new_window\"\n\"cmd+shift+]\" = \"new_tab\"\n\"f5\" = { text = \"make\\r\" }\n",
        ));
        let enter = |flags| KeyInput {
            keycode: 0x24,
            flags,
            ..KeyInput::default()
        };
        let f5 = KeyInput {
            keycode: 0x60,
            ..KeyInput::default()
        };
        let russian_t = KeyInput {
            ascii: Some('t'),
            ascii_shifted: Some('T'),
            ..key("Е", CONTROL | SHIFT)
        };
        let rows: &[(&str, KeyInput, Option<Decision>)] = &[
            ("ctrl+shift+t", key("T", CONTROL | SHIFT), Some(Decision::Send("\x1b[116;6u"))),
            ("ctrl+shift+t on a russian layout", russian_t, Some(Decision::Send("\x1b[116;6u"))),
            ("f5", f5.clone(), Some(Decision::Send("make\r"))),
            (
                "ctrl+alt+n",
                key("n", CONTROL | OPTION | LEFT_OPTION),
                Some(Decision::Perform(MenuAction::NewWindow)),
            ),
            // Cmd chords bound to menu actions are the menus' to match.
            ("cmd+enter", enter(COMMAND), None),
            ("cmd+c", key("c", COMMAND), None),
            // Shifted characters: Cmd+Shift+] is both cmd+shift+] and cmd+}.
            (
                "cmd+shift+] (as typed: })",
                KeyInput {
                    chars_unshifted: "]".into(),
                    ..key("}", COMMAND | SHIFT)
                },
                None,
            ),
            // Unbound keys.
            ("ctrl+t", key("t", CONTROL), None),
            ("enter", enter(0), None),
            ("cmd+d", key("d", COMMAND), None),
            // Releases never trigger bindings.
            (
                "f5 release",
                KeyInput {
                    event: KeyEventType::Release,
                    ..f5
                },
                None,
            ),
        ];
        for (name, input, want) in rows {
            assert_eq!(&b.decide(input), want, "{name}");
        }
        let shifted_bracket = KeyInput {
            chars_unshifted: "]".into(),
            ..key("}", COMMAND | SHIFT)
        };
        assert_eq!(b.resolve(&shifted_bracket), Some(&Bound::Menu(MenuAction::NewTab)));
        assert_eq!(
            b.resolve(&key("}", COMMAND | SHIFT)),
            Some(&Bound::Menu(MenuAction::NextTab)),
            "cmd+}} by its character"
        );
    }

    /// An unbound Cmd key does what it did with no config: line editing
    /// keys go to the program, the rest do nothing.
    #[test]
    fn unbound_cmd_keys_fall_back_to_translate() {
        let b = Keybinds::new(&config(
            "[keybind]\n\"cmd+d\" = \"none\"\n\"cmd+left\" = \"none\"\n\"cmd+k\" = \"none\"\n",
        ));
        let left = KeyInput {
            keycode: 0x7B,
            flags: COMMAND,
            ..KeyInput::default()
        };
        for (name, input, want) in [
            ("cmd+d", key("d", COMMAND), KeyAction::ToMenu),
            ("cmd+k", key("k", COMMAND), KeyAction::ToMenu),
            ("cmd+x", key("x", COMMAND), KeyAction::ToMenu),
            ("cmd+left", left, KeyAction::Send(b"\x01".to_vec())),
        ] {
            assert_eq!(b.decide(&input), None, "{name}");
            assert_eq!(translate(&input, KeyModes::default()), want, "{name}");
        }
        // A text binding wins over the line-editing default.
        let b = Keybinds::new(&config("[keybind]\n\"cmd+left\" = { text = \"\\u001bb\" }\n"));
        let left = KeyInput {
            keycode: 0x7B,
            flags: COMMAND,
            ..KeyInput::default()
        };
        assert_eq!(b.decide(&left), Some(Decision::Send("\x1bb")));
    }

    #[test]
    fn canonical_keys_let_later_layers_replace_chords() {
        let mut t: toml::Table = "[keybind]\n\"command+D\" = \"none\"\n\"shift+cmd+t\" = \"new_tab\"\n\"bad+x\" = \"copy\"\n"
            .parse()
            .unwrap();
        canonicalize(&mut t);
        let keys: Vec<&String> = t["keybind"].as_table().unwrap().keys().collect();
        assert_eq!(keys, ["bad+x", "cmd+d", "shift+cmd+t"]);
        let mut no_binds: toml::Table = "font_size = 3".parse().unwrap();
        canonicalize(&mut no_binds);
        assert_eq!(no_binds.len(), 1);
    }
}
