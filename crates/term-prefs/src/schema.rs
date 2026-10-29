//! The settings schema: one entry per key of `config.toml`, saying where
//! the TUI shows it, how it is edited, and whether a running window takes
//! it on at once.
//!
//! The schema checks the app's `Config` rather than generating it (see
//! docs/design/preferences.md, "The schema"): defaults aren't repeated here,
//! they come from the app ([`crate::Host::defaults`]).

/// The TUI's sections, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Appearance,
    Font,
    Keys,
    Window,
    Pins,
    Advanced,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Appearance,
        Section::Font,
        Section::Keys,
        Section::Window,
        Section::Pins,
        Section::Advanced,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Font => "Font",
            Section::Keys => "Keys",
            Section::Window => "Window",
            Section::Pins => "Pins",
            Section::Advanced => "Advanced",
        }
    }
}

/// What a list holds, for its "add" step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Text,
    /// A font family, picked from all installed fonts or typed.
    Font,
}

/// How a setting is edited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Editor {
    /// On or off (space).
    Toggle,
    /// One of these strings, from a list that filters as you type.
    Choice(&'static [&'static str]),
    /// A theme name, or a light/dark pair of them.
    Theme,
    /// A font family: monospace fonts first, all fonts behind a toggle.
    Font,
    /// A number: ←→ steps by `step` within `min..=max`, or type it.
    Number { min: f64, max: f64, step: f64, integer: bool },
    /// A string; empty leaves it unset.
    Text,
    /// `cell_width`: points (`"1.5"`) or a percentage (`"10%"`).
    Width,
    /// `"#rrggbb"`, shown with a swatch.
    Colour,
    /// The 16 ANSI colours, each a colour editor.
    Palette,
    /// A list of strings: add, remove, reorder.
    List(Item),
    /// A table of names to strings (`env`).
    Map,
    /// The `[keybind]` table.
    Keybinds,
}

/// Whether a running window takes a change on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    /// In every window at once.
    Live,
    /// In windows opened after the change.
    NewWindows,
}

/// One setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    /// The key in `config.toml`: `font_size`, or `colors.foreground` for a
    /// key in a table.
    pub key: &'static str,
    /// The row's label.
    pub label: &'static str,
    pub section: Section,
    pub editor: Editor,
    pub applies: Applies,
    /// One line, shown at the bottom while the row is selected.
    pub description: &'static str,
    /// What an unset value means, for settings without a default value
    /// (`None` in the app's `Config`).
    pub unset: &'static str,
    /// Never shown (`pin_app`).
    pub hidden: bool,
    /// Shown only while editing a pin's own file (`title`).
    pub pin_only: bool,
    /// Can't be reset in a pin's file: a pin with no command opens a login
    /// shell (`command`).
    pub keep_in_pin: bool,
}

impl Setting {
    /// The key split at its dots: `["colors", "foreground"]`.
    pub fn path(&self) -> Vec<String> {
        self.key.split('.').map(str::to_string).collect()
    }
}

const fn setting(key: &'static str, label: &'static str, section: Section, editor: Editor, description: &'static str) -> Setting {
    Setting {
        key,
        label,
        section,
        editor,
        applies: Applies::Live,
        description,
        unset: "",
        hidden: false,
        pin_only: false,
        keep_in_pin: false,
    }
}

const fn new_windows(s: Setting) -> Setting {
    Setting {
        applies: Applies::NewWindows,
        ..s
    }
}

const fn unset(s: Setting, unset: &'static str) -> Setting {
    Setting { unset, ..s }
}

const fn number(min: f64, max: f64, step: f64) -> Editor {
    Editor::Number {
        min,
        max,
        step,
        integer: false,
    }
}

const fn integer(min: f64, max: f64, step: f64) -> Editor {
    Editor::Number {
        min,
        max,
        step,
        integer: true,
    }
}

use Section::*;

/// Every key of `config.toml`, in the order the TUI shows them.
pub const SETTINGS: &[Setting] = &[
    // Appearance
    unset(
        setting(
            "theme",
            "Theme",
            Appearance,
            Editor::Theme,
            "A built-in theme or one in the themes folder; a light/dark pair follows the macOS appearance",
        ),
        "none (the built-in colours)",
    ),
    unset(
        setting(
            "colors.foreground",
            "Foreground",
            Appearance,
            Editor::Colour,
            "The text colour, over the theme's",
        ),
        "from the theme",
    ),
    unset(
        setting(
            "colors.background",
            "Background",
            Appearance,
            Editor::Colour,
            "The background colour, over the theme's",
        ),
        "from the theme",
    ),
    unset(
        setting(
            "colors.cursor",
            "Cursor",
            Appearance,
            Editor::Colour,
            "The cursor colour, over the theme's",
        ),
        "from the theme",
    ),
    unset(
        setting(
            "colors.selection",
            "Selection",
            Appearance,
            Editor::Colour,
            "The selection colour, over the theme's",
        ),
        "from the theme",
    ),
    unset(
        setting(
            "colors.split_divider",
            "Split divider",
            Appearance,
            Editor::Colour,
            "The line between panes, over the theme's",
        ),
        "the background mixed a quarter of the way to the foreground",
    ),
    setting(
        "colors.palette",
        "Palette",
        Appearance,
        Editor::Palette,
        "The 16 ANSI colours, over the theme's; empty ones come from the theme",
    ),
    setting(
        "opacity",
        "Opacity",
        Appearance,
        number(0.0, 1.0, 0.05),
        "Below 1 the desktop shows through (this tab stays opaque)",
    ),
    setting(
        "unfocused_pane_opacity",
        "Unfocused panes",
        Appearance,
        number(0.0, 1.0, 0.05),
        "How opaque panes without focus are drawn; 1 doesn't dim them",
    ),
    new_windows(setting(
        "blur",
        "Blur",
        Appearance,
        Editor::Toggle,
        "Blur what shows through when opacity is below 1",
    )),
    setting(
        "padding",
        "Padding",
        Appearance,
        number(0.0, 100.0, 1.0),
        "Space around the text, in points",
    ),
    // Font
    setting("font_family", "Family", Font, Editor::Font, "The font the text is drawn with"),
    setting(
        "font_size",
        "Size",
        Font,
        number(6.0, 72.0, 1.0),
        "The font size in points (Cmd+= and Cmd+- change it per window)",
    ),
    setting(
        "font_fallback",
        "Fallback",
        Font,
        Editor::List(Item::Font),
        "Fonts tried in order for characters the family lacks, before the system's own fallback",
    ),
    unset(
        setting(
            "font_bold",
            "Bold face",
            Font,
            Editor::Text,
            "The bold face by name, such as \"Menlo Bold\"",
        ),
        "the family's own",
    ),
    unset(
        setting("font_italic", "Italic face", Font, Editor::Text, "The italic face by name"),
        "the family's own",
    ),
    unset(
        setting(
            "font_bold_italic",
            "Bold italic face",
            Font,
            Editor::Text,
            "The bold italic face by name",
        ),
        "the family's own",
    ),
    setting(
        "line_height",
        "Line height",
        Font,
        number(0.5, 3.0, 0.05),
        "Multiplies the cell height; 1 is the font's own line height",
    ),
    setting(
        "cell_width",
        "Cell width",
        Font,
        Editor::Width,
        "Widens or narrows cells: points (\"1.5\", \"-1\") or a percentage of the advance (\"10%\")",
    ),
    setting(
        "font_features",
        "Features",
        Font,
        Editor::List(Item::Text),
        "OpenType features on or off: \"zero\", \"ss01=1\", \"-liga\"; [\"-calt\", \"-liga\"] turns ligatures off",
    ),
    // Keys
    setting(
        "option_as_meta",
        "Option as Meta",
        Keys,
        Editor::Choice(&["none", "left", "right", "both"]),
        "Which Option key sends Esc + key instead of typing characters such as @ [ ] { }",
    ),
    setting(
        "keybind",
        "Shortcuts",
        Keys,
        Editor::Keybinds,
        "The menus' shortcuts, remapped, unbound or sending text",
    ),
    // Window
    new_windows(setting(
        "columns",
        "Columns",
        Window,
        integer(10.0, 1000.0, 1.0),
        "A new window's width in cells",
    )),
    new_windows(setting(
        "lines",
        "Lines",
        Window,
        integer(5.0, 500.0, 1.0),
        "A new window's height in cells",
    )),
    new_windows(setting(
        "scrollback",
        "Scrollback",
        Window,
        integer(0.0, 1_000_000.0, 1000.0),
        "Lines kept above the screen",
    )),
    setting(
        "copy_on_select",
        "Copy on select",
        Window,
        Editor::Toggle,
        "Copy a selection to the clipboard as soon as it is made with the mouse",
    ),
    setting(
        "confirm_close",
        "Confirm close",
        Window,
        Editor::Toggle,
        "Ask before closing a window or quitting while a program other than the shell runs",
    ),
    // Advanced
    new_windows(Setting {
        keep_in_pin: true,
        ..unset(
            setting(
                "command",
                "Command",
                Advanced,
                Editor::List(Item::Text),
                "Program and arguments to run instead of the login shell",
            ),
            "the login shell",
        )
    }),
    new_windows(unset(
        setting("cwd", "Folder", Advanced, Editor::Text, "The folder new windows start in"),
        "your home folder",
    )),
    new_windows(setting(
        "env",
        "Environment",
        Advanced,
        Editor::Map,
        "Variables set for the programs started",
    )),
    new_windows(Setting {
        pin_only: true,
        ..unset(
            setting(
                "title",
                "Title",
                Advanced,
                Editor::Text,
                "The pin's window title; also shown in the Window menu",
            ),
            "the program's own",
        )
    }),
    Setting {
        hidden: true,
        ..setting(
            "pin_app",
            "Pin app",
            Advanced,
            Editor::Text,
            "The app bundle a pin's file belongs to",
        )
    },
];

/// The setting with this key.
pub fn get(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// The top-level keys of `config.toml` (`colors` once).
pub fn top_level_keys() -> Vec<&'static str> {
    let mut keys: Vec<&str> = Vec::new();
    for s in SETTINGS {
        let top = s.key.split('.').next().unwrap_or(s.key);
        if !keys.contains(&top) {
            keys.push(top);
        }
    }
    keys
}

/// The keys of `[colors]`.
pub fn colors_keys() -> Vec<&'static str> {
    SETTINGS.iter().filter_map(|s| s.key.strip_prefix("colors.")).collect()
}

/// The settings a running window can't take on, for the app's reload
/// (`config::new_windows_only`).
pub fn new_windows_keys() -> Vec<&'static str> {
    SETTINGS
        .iter()
        .filter(|s| s.applies == Applies::NewWindows)
        .map(|s| s.key)
        .collect()
}

/// The names of the 16 ANSI colours.
pub const PALETTE_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright black",
    "bright red",
    "bright green",
    "bright yellow",
    "bright blue",
    "bright magenta",
    "bright cyan",
    "bright white",
];

/// `"#rrggbb"` (or `"rrggbb"`) as 0xRRGGBB, as the app reads colours.
pub fn parse_hex(s: &str) -> Option<u32> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(s, 16).ok()
}

/// Whether `s` is a cell width the app accepts: a number of points, or a
/// percentage.
pub fn valid_width(s: &str) -> bool {
    let s = s.trim();
    let num = s.strip_suffix('%').unwrap_or(s).trim();
    num.parse::<f64>().is_ok_and(f64::is_finite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_sections_hold_settings() {
        let mut keys: Vec<&str> = SETTINGS.iter().map(|s| s.key).collect();
        keys.sort_unstable();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "a key is listed twice");
        for section in Section::ALL {
            if section == Section::Pins {
                continue;
            }
            assert!(
                SETTINGS.iter().any(|s| s.section == section && !s.hidden),
                "{section:?} has no settings"
            );
        }
    }

    #[test]
    fn config_keys() {
        // The fields of the app's `Config` and `Colors` today
        // (crates/term-app/src/config.rs, `palettes` being computed). The
        // app's own test checks them against `Config` itself.
        assert_eq!(
            top_level_keys(),
            [
                "theme",
                "colors",
                "opacity",
                "unfocused_pane_opacity",
                "blur",
                "padding",
                "font_family",
                "font_size",
                "font_fallback",
                "font_bold",
                "font_italic",
                "font_bold_italic",
                "line_height",
                "cell_width",
                "font_features",
                "option_as_meta",
                "keybind",
                "columns",
                "lines",
                "scrollback",
                "copy_on_select",
                "confirm_close",
                "command",
                "cwd",
                "env",
                "title",
                "pin_app",
            ]
        );
        assert_eq!(
            colors_keys(),
            ["foreground", "background", "cursor", "selection", "split_divider", "palette"]
        );
    }

    #[test]
    fn new_windows_flags_match_the_reload() {
        // `config::new_windows_only` in the app.
        let mut keys = new_windows_keys();
        keys.sort_unstable();
        assert_eq!(keys, ["blur", "columns", "command", "cwd", "env", "lines", "scrollback", "title"]);
    }

    #[test]
    fn ranges_are_sane() {
        for s in SETTINGS {
            if let Editor::Number { min, max, step, .. } = s.editor {
                assert!(min < max && step > 0.0 && step <= max - min, "{}", s.key);
            }
        }
        // The ones that keep the prefs tab readable.
        assert!(matches!(
            get("font_size").unwrap().editor,
            Editor::Number { min: 6.0, max: 72.0, .. }
        ));
        assert!(matches!(
            get("line_height").unwrap().editor,
            Editor::Number { min: 0.5, max: 3.0, .. }
        ));
    }

    #[test]
    fn hex_and_widths() {
        assert_eq!(parse_hex("#ff0080"), Some(0xff0080));
        assert_eq!(parse_hex("ff0080"), Some(0xff0080));
        assert_eq!(parse_hex("#ff008"), None);
        assert_eq!(parse_hex("#+f0080"), None);
        assert!(valid_width("1.5") && valid_width("-1") && valid_width("10%") && valid_width("-5 %"));
        assert!(!valid_width("wide") && !valid_width("%") && !valid_width(""));
    }
}
