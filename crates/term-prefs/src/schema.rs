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
    Completion,
}

impl Section {
    pub const ALL: [Section; 7] = [
        Section::Appearance,
        Section::Font,
        Section::Keys,
        Section::Window,
        Section::Pins,
        Section::Advanced,
        Section::Completion,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Font => "Font",
            Section::Keys => "Keys",
            Section::Window => "Window",
            Section::Pins => "Pins",
            Section::Completion => "Completion",
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
    /// A font family: every installed family, monospaced ones first, then
    /// the others under a header; the one under the selection is shown
    /// in a sample in the list, and nothing changes until Enter.
    Font,
    /// A face of the configured family (`font_bold`): its own faces by
    /// full name, or unset; any other face's name can be typed. Sampled
    /// like a family.
    Face,
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
    setting(
        "appearance",
        "Appearance",
        Appearance,
        Editor::Choice(&["system", "light", "dark"]),
        "Follow macOS, or always light or always dark: which of the two themes below windows use",
    ),
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
    setting(
        "theme_transition",
        "Theme transition",
        Appearance,
        Editor::Choice(&["fade", "wave", "pulse", "glimmer", "none"]),
        "How a new theme comes in: a fade, a diagonal wave, a pulse, a glimmer, or at once (Reduce Motion makes it a fade)",
    ),
    setting(
        "theme_transition_ms",
        "Transition time",
        Appearance,
        integer(0.0, 2000.0, 50.0),
        "How long a theme transition takes, in milliseconds; 0 is at once",
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
    setting(
        "font_family",
        "Family",
        Font,
        Editor::Font,
        "The font the text is drawn with: monospaced fonts first; the list shows a sample of each, and Enter uses it",
    ),
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
            Editor::Face,
            "The bold face, from the family's faces or any face's full name typed, such as \"Menlo Bold\"",
        ),
        "the family's own",
    ),
    unset(
        setting(
            "font_italic",
            "Italic face",
            Font,
            Editor::Face,
            "The italic face, from the family's faces or any face's full name typed",
        ),
        "the family's own",
    ),
    unset(
        setting(
            "font_bold_italic",
            "Bold italic face",
            Font,
            Editor::Face,
            "The bold italic face, from the family's faces or any face's full name typed",
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
    setting(
        "ambiguous_width",
        "Ambiguous width",
        Font,
        integer(1.0, 2.0, 1.0),
        "Cells taken by East Asian Ambiguous characters (\u{2160}, \u{25cb}, \u{00b1}): 1, or 2 as CJK terminals draw them",
    ),
    setting(
        "font_features_bold",
        "Bold features",
        Font,
        Editor::List(Item::Text),
        "OpenType features for the bold face only, after Features: \"-liga\", \"ss02\"",
    ),
    setting(
        "font_features_italic",
        "Italic features",
        Font,
        Editor::List(Item::Text),
        "OpenType features for the italic face only, after Features: \"ss01\" for a cursive italic",
    ),
    setting(
        "font_features_bold_italic",
        "Bold italic features",
        Font,
        Editor::List(Item::Text),
        "OpenType features for the bold italic face only, after Features",
    ),
    setting(
        "font_variations",
        "Variations",
        Font,
        Editor::List(Item::Text),
        "Variable font axes for the regular face: \"wght=450\" (weight), \"wdth=90\" (width); ignored by fonts without them",
    ),
    setting(
        "font_variations_bold",
        "Bold variations",
        Font,
        Editor::List(Item::Text),
        "Variable font axes for the bold face: \"wght=700\"",
    ),
    setting(
        "font_variations_italic",
        "Italic variations",
        Font,
        Editor::List(Item::Text),
        "Variable font axes for the italic face: \"wght=450\", \"slnt=-10\"",
    ),
    setting(
        "font_variations_bold_italic",
        "Bold italic variations",
        Font,
        Editor::List(Item::Text),
        "Variable font axes for the bold italic face",
    ),
    setting(
        "ligatures",
        "Ligatures",
        Font,
        Editor::Toggle,
        "Join sequences such as -> != === into one glyph where the font has ligatures (off: calt, liga and dlig off)",
    ),
    setting(
        "font_thicken",
        "Thicken",
        Font,
        Editor::Toggle,
        "Draw text a little heavier, as macOS font smoothing used to: easier to read thin fonts on dark backgrounds",
    ),
    setting(
        "font_thicken_strength",
        "Thicken strength",
        Font,
        integer(0.0, 255.0, 15.0),
        "How much Thicken adds, 0 to 255 (255: half a point of stroke around each glyph)",
    ),
    setting(
        "font_synthetic_bold",
        "Synthetic bold",
        Font,
        Editor::Toggle,
        "When the font has no bold face, draw bold by thickening the regular face (off: bold text looks regular)",
    ),
    setting(
        "font_synthetic_italic",
        "Synthetic italic",
        Font,
        Editor::Toggle,
        "When the font has no italic face, draw italic by slanting the regular face (off: italic text stays upright)",
    ),
    setting(
        "adjust_cell_height",
        "Cell height",
        Font,
        Editor::Width,
        "Added to the cell height after Line height: points (\"2\", \"-1\") or a percentage (\"10%\"); the text stays centred",
    ),
    setting(
        "adjust_baseline",
        "Baseline",
        Font,
        Editor::Width,
        "Moves the text up (positive) or down in its cell: points (\"1\") or a percentage of the cell height (\"5%\")",
    ),
    setting(
        "adjust_underline_position",
        "Underline position",
        Font,
        Editor::Width,
        "Moves underlines down (positive) or up: points or a percentage of the cell height",
    ),
    setting(
        "adjust_underline_thickness",
        "Underline thickness",
        Font,
        Editor::Width,
        "Added to the underline's thickness: points (\"0.5\") or a percentage of it (\"50%\")",
    ),
    setting(
        "adjust_strikethrough_position",
        "Strikethrough position",
        Font,
        Editor::Width,
        "Moves the strikethrough down (positive) or up: points or a percentage of the cell height",
    ),
    setting(
        "adjust_strikethrough_thickness",
        "Strikethrough thickness",
        Font,
        Editor::Width,
        "Added to the strikethrough's thickness: points or a percentage of it",
    ),
    setting(
        "adjust_cursor_thickness",
        "Cursor thickness",
        Font,
        Editor::Width,
        "Added to the bar and underline cursor's thickness: points (\"1\") or a percentage of it (\"100%\" doubles it)",
    ),
    setting(
        "symbol_map",
        "Symbol map",
        Font,
        Editor::Map,
        "Code points drawn from a font of their own: \"U+E000-U+F8FF\" = \"Symbols Nerd Font Mono\" for Nerd Font icons",
    ),
    unset(
        setting(
            "emoji_font",
            "Emoji font",
            Font,
            Editor::Font,
            "The font emoji are drawn from, where it has them",
        ),
        "Apple Color Emoji",
    ),
    setting(
        "bold_is_bright",
        "Bold is bright",
        Font,
        Editor::Toggle,
        "Draw bold text in the first 8 ANSI colours in their bright versions, as older terminals did",
    ),
    setting(
        "minimum_contrast",
        "Minimum contrast",
        Font,
        number(1.0, 21.0, 0.5),
        "The least contrast ratio text keeps against its background: 1 is off, 3 keeps dim text legible, 4.5 is WCAG AA, 21 black on white",
    ),
    setting(
        "font_size_step",
        "Size step",
        Font,
        number(0.5, 24.0, 0.5),
        "Points Cmd+= and Cmd+- change a window's font size by (Cmd+0 puts it back)",
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
        "scroll_multiplier",
        "Wheel lines",
        Window,
        number(0.25, 100.0, 0.25),
        "Lines a mouse wheel's click scrolls; a trackpad moves the text by its own pixels",
    ),
    setting(
        "restore_scrollback",
        "Restore scrollback",
        Window,
        Editor::Choice(&["restarts", "always", "never"]),
        "Show each pane's history again after a restart into a new build; always: after quitting too (windows always reopen where they were)",
    ),
    setting(
        "restore_scrollback_limit",
        "Restored lines",
        Window,
        integer(0.0, 1_000_000.0, 1000.0),
        "Most lines of each pane's history kept for that",
    ),
    setting(
        "cursor_style",
        "Cursor",
        Window,
        Editor::Choice(&["block", "bar", "underline"]),
        "The cursor's shape, until a program sets its own",
    ),
    setting(
        "cursor_blink",
        "Blink cursor",
        Window,
        Editor::Toggle,
        "Make the cursor blink, until a program says otherwise",
    ),
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
    setting(
        "title_status",
        "Status in the title",
        Window,
        Editor::Toggle,
        "The title says what the shell does: the command running and for how long, or waiting in which folder",
    ),
    setting(
        "notify_on_finish",
        "Notify when commands finish",
        Window,
        Editor::Toggle,
        "Notify when a long command finishes in a tab or pane you aren't looking at (needs shell integration)",
    ),
    setting(
        "notify_after",
        "Notify after",
        Window,
        integer(0.0, 86_400.0, 5.0),
        "How many seconds a command must run before its finish is notified",
    ),
    setting(
        "quick_terminal_hotkey",
        "Quick terminal hotkey",
        Window,
        Editor::Text,
        "The hotkey that slides the quick terminal in and out from any app (ctrl+`); empty for none",
    ),
    setting(
        "quick_terminal_position",
        "Quick terminal position",
        Window,
        Editor::Choice(&["top", "bottom", "left", "right", "center"]),
        "The edge of the screen the quick terminal slides in from, or the centre",
    ),
    setting(
        "quick_terminal_size",
        "Quick terminal size",
        Window,
        number(0.1, 1.0, 0.05),
        "How much of the screen the quick terminal takes, from its edge",
    ),
    setting(
        "quick_terminal_animation_ms",
        "Quick terminal animation",
        Window,
        integer(0.0, 2000.0, 50.0),
        "Milliseconds the quick terminal takes to slide in or out; 0 for none",
    ),
    setting(
        "quick_terminal_autohide",
        "Hide the quick terminal",
        Window,
        Editor::Toggle,
        "Slide the quick terminal away when another window or app takes the focus",
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
    new_windows(setting(
        "shell_integration",
        "Shell integration",
        Advanced,
        Editor::Toggle,
        "Load prompt marks into zsh, bash and fish, for jumping between prompts and command status (off: start the shell untouched)",
    )),
    setting(
        "highlight_command_line",
        "Colour the command line",
        Completion,
        Editor::Toggle,
        "Colour commands, flags, strings, variables and pipes as you type at zsh's prompt (needs shell integration; new tabs)",
    ),
    setting(
        "command_marks",
        "Mark off commands",
        Completion,
        Editor::Toggle,
        "A faint line above each prompt, and ✓ or ✗ with the time taken at the right of each finished command (needs shell integration)",
    ),
    setting(
        "colour_output",
        "Colour plain output",
        Completion,
        Editor::Toggle,
        "Colour commit hashes, links, numbers and words like error and ok in command output the program left uncoloured",
    ),
    setting(
        "completion",
        "Completion list",
        Completion,
        Editor::Toggle,
        "Our list at zsh's prompt: zsh's matches on Tab, with what each is, and the list as you type when that is on (new tabs)",
    ),
    setting(
        "completion_as_you_type",
        "Open the list as you type",
        Completion,
        Editor::Toggle,
        "The list of files, folders, commands and flags opens by itself after a pause in typing; off, it shows only on Tab",
    ),
    setting(
        "completion_delay",
        "Pause before the list",
        Completion,
        integer(0.0, 2000.0, 50.0),
        "How long typing pauses before the list opens, in milliseconds",
    ),
    setting(
        "completion_rows",
        "List rows",
        Completion,
        integer(3.0, 20.0, 1.0),
        "Choices shown before the list scrolls",
    ),
    setting(
        "completion_ghost_text",
        "Grey text after the cursor",
        Completion,
        Editor::Toggle,
        "The first choice's rest shown grey after the cursor; Right takes it",
    ),
    setting(
        "completion_files",
        "Files and folders",
        Completion,
        Editor::Toggle,
        "Offer files and folders in the list",
    ),
    setting(
        "completion_commands",
        "Commands",
        Completion,
        Editor::Toggle,
        "Offer commands, aliases, functions and builtins in the list",
    ),
    setting(
        "completion_flags",
        "Flags and subcommands",
        Completion,
        Editor::Toggle,
        "Offer a command's flags and subcommands, with what each does, from its man pages",
    ),
    setting(
        "completion_tab_zsh",
        "Tab shows zsh's matches in the list",
        Completion,
        Editor::Toggle,
        "Tab asks zsh, and what it finds shows in the list instead of zsh's own listing (new tabs)",
    ),
    setting(
        "completion_history_order",
        "Most used commands first",
        Completion,
        Editor::Toggle,
        "Order commands in the list by how often you ran them, from zsh's history",
    ),
    setting(
        "completion_enter_puts_in",
        "Return puts in the chosen item",
        Completion,
        Editor::Toggle,
        "Once you move to an item in the list (with Down or the mouse), Return puts it in instead of running the line",
    ),
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
    setting(
        "images",
        "Images",
        Advanced,
        Editor::Toggle,
        "Show images that programs send (kitty graphics, iTerm2, Sixel)",
    ),
    setting(
        "image_memory",
        "Image memory",
        Advanced,
        integer(0.0, 16_384.0, 32.0),
        "MB images may use; 0 is the larger of 32 and two window-sized images",
    ),
    setting(
        "image_file_transfer",
        "Image file transfer",
        Advanced,
        Editor::Toggle,
        "Let programs send images by file path or shared memory, not only inline",
    ),
    setting(
        "update",
        "Updates",
        Advanced,
        Editor::Choice(&["notify", "auto", "off"]),
        "notify: say when there is one; auto: builds from source also update when idle; release builds install on quit",
    ),
    setting(
        "welcome",
        "Welcome line",
        Advanced,
        Editor::Choice(&["full", "update-only", "off"]),
        "The faint line a new tab starts with: version, tabs, memory, a tip and a waiting update (⌘U installs it)",
    ),
    setting(
        "welcome_update_snooze",
        "Update notice snooze",
        Advanced,
        integer(0.0, 720.0, 1.0),
        "Hours the welcome line stops naming an update once it has been shown in 3 tabs; a newer one shows at once",
    ),
    unset(
        setting(
            "editor",
            "Editor",
            Advanced,
            Editor::Text,
            "The program that opens config.toml (Edit config.toml, and e here), with its arguments",
        ),
        "$VISUAL or $EDITOR, else nano, vim or vi",
    ),
    setting(
        "agent_api",
        "Agent API",
        Advanced,
        Editor::Toggle,
        "Let your own programs drive terminals with terminal-decline ctl (the app only, never pins)",
    ),
    setting(
        "agent_api_idle_timeout",
        "Agent API idle timeout",
        Advanced,
        integer(0.0, 10_080.0, 5.0),
        "Minutes a ctl session may go without a request before it is killed; 0 never",
    ),
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
        // (crates/term-config/src/config.rs, `palettes` being computed). The
        // app's own test checks them against `Config` itself.
        assert_eq!(
            top_level_keys(),
            [
                "appearance",
                "theme",
                "theme_transition",
                "theme_transition_ms",
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
                "ambiguous_width",
                "font_features_bold",
                "font_features_italic",
                "font_features_bold_italic",
                "font_variations",
                "font_variations_bold",
                "font_variations_italic",
                "font_variations_bold_italic",
                "ligatures",
                "font_thicken",
                "font_thicken_strength",
                "font_synthetic_bold",
                "font_synthetic_italic",
                "adjust_cell_height",
                "adjust_baseline",
                "adjust_underline_position",
                "adjust_underline_thickness",
                "adjust_strikethrough_position",
                "adjust_strikethrough_thickness",
                "adjust_cursor_thickness",
                "symbol_map",
                "emoji_font",
                "bold_is_bright",
                "minimum_contrast",
                "font_size_step",
                "option_as_meta",
                "keybind",
                "columns",
                "lines",
                "scrollback",
                "scroll_multiplier",
                "restore_scrollback",
                "restore_scrollback_limit",
                "cursor_style",
                "cursor_blink",
                "copy_on_select",
                "confirm_close",
                "title_status",
                "notify_on_finish",
                "notify_after",
                "quick_terminal_hotkey",
                "quick_terminal_position",
                "quick_terminal_size",
                "quick_terminal_animation_ms",
                "quick_terminal_autohide",
                "command",
                "cwd",
                "env",
                "shell_integration",
                "highlight_command_line",
                "command_marks",
                "colour_output",
                "completion",
                "completion_as_you_type",
                "completion_delay",
                "completion_rows",
                "completion_ghost_text",
                "completion_files",
                "completion_commands",
                "completion_flags",
                "completion_tab_zsh",
                "completion_history_order",
                "completion_enter_puts_in",
                "title",
                "images",
                "image_memory",
                "image_file_transfer",
                "update",
                "welcome",
                "welcome_update_snooze",
                "editor",
                "agent_api",
                "agent_api_idle_timeout",
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
        assert_eq!(
            keys,
            [
                "blur",
                "columns",
                "command",
                "cwd",
                "env",
                "lines",
                "scrollback",
                "shell_integration",
                "title"
            ]
        );
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
