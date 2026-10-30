//! Config file: `~/.config/terminal-decline/config.toml`, with a pin's own
//! config (inside its app bundle) layered on top.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use term_font::{CellWidth, FontConfig};
use term_render::Theme;

use crate::keybind::{self, KeybindConfig, Keybinds};
use crate::theme::{self as themes, Appearance, AppearanceMode, ThemeChoice, ThemeTransition};

/// Which Option key acts as Meta (sends ESC + key). Off by default, like
/// Terminal.app and Ghostty, so Option still types @ [ ] { } | \\ ~ on
/// European keyboard layouts.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OptionAsMeta {
    Left,
    Right,
    Both,
    #[default]
    None,
}

/// `update`: what the app does about updates (docs/design/updater.md).
/// `true` and `false`, from before, mean `"notify"` and `"off"`.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    /// Builds from source update themselves once the Mac has been idle a
    /// while, on AC power; release builds install on quit.
    Auto,
    /// Builds from source say an update is there and wait to be told;
    /// release builds install on quit (nothing is interrupted).
    #[default]
    Notify,
    /// No checks.
    Off,
}

impl UpdateMode {
    /// Whether anything is checked at all.
    pub fn on(self) -> bool {
        self != UpdateMode::Off
    }
}

impl<'de> Deserialize<'de> for UpdateMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<UpdateMode, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Text(String),
        }
        let bad = || serde::de::Error::custom("use \"auto\", \"notify\" or \"off\"");
        match Raw::deserialize(d).map_err(|_| bad())? {
            Raw::Bool(true) => Ok(UpdateMode::Notify),
            Raw::Bool(false) => Ok(UpdateMode::Off),
            Raw::Text(t) => match t.as_str() {
                "auto" => Ok(UpdateMode::Auto),
                "notify" => Ok(UpdateMode::Notify),
                "off" => Ok(UpdateMode::Off),
                _ => Err(bad()),
            },
        }
    }
}

/// `[colors]`, and a theme file: only what is set. Unset colours (and
/// palette entries left out or `""`) come from the theme, else the
/// defaults ([`Palette::DEFAULT`]).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    #[serde(deserialize_with = "colour")]
    pub foreground: Option<String>,
    #[serde(deserialize_with = "colour")]
    pub background: Option<String>,
    #[serde(deserialize_with = "colour")]
    pub cursor: Option<String>,
    #[serde(deserialize_with = "colour")]
    pub selection: Option<String>,
    /// The line between panes; unset, the background mixed a quarter of
    /// the way to the foreground.
    #[serde(deserialize_with = "colour")]
    pub split_divider: Option<String>,
    /// The 16 ANSI colours.
    #[serde(deserialize_with = "palette")]
    pub palette: Vec<String>,
}

fn colour<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s = String::deserialize(d)?;
    match parse_hex(&s) {
        Some(_) => Ok(Some(s)),
        None => Err(serde::de::Error::custom(format!("{s:?} is not a colour (use \"#rrggbb\")"))),
    }
}

fn palette<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let list = Vec::<String>::deserialize(d)?;
    if list.len() > 16 {
        return Err(serde::de::Error::custom(format!("{} colours: the palette has 16", list.len())));
    }
    if let Some(bad) = list.iter().find(|s| !s.is_empty() && parse_hex(s).is_none()) {
        return Err(serde::de::Error::custom(format!(
            "{bad:?} is not a colour (use \"#rrggbb\", or \"\" for the theme's)"
        )));
    }
    Ok(list)
}

/// Resolved colours, as 0xRRGGBB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub selection: u32,
    /// Set by a theme or `[colors]`; see [`Palette::divider`].
    pub split_divider: Option<u32>,
    pub ansi: [u32; 16],
}

impl Palette {
    /// Terminal Decline's colours with no theme and no `[colors]`.
    pub const DEFAULT: Palette = Palette {
        foreground: 0xd8dee9,
        background: 0x16181c,
        cursor: 0xd8dee9,
        selection: 0x2e3a4f,
        split_divider: None,
        ansi: [
            0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, //
            0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
        ],
    };

    /// These colours with the ones `colors` sets in their place.
    pub fn with(mut self, colors: &Colors) -> Palette {
        let set = |slot: &mut u32, v: &Option<String>| {
            if let Some(c) = v.as_deref().and_then(parse_hex) {
                *slot = c;
            }
        };
        set(&mut self.foreground, &colors.foreground);
        set(&mut self.background, &colors.background);
        set(&mut self.cursor, &colors.cursor);
        set(&mut self.selection, &colors.selection);
        if let Some(c) = colors.split_divider.as_deref().and_then(parse_hex) {
            self.split_divider = Some(c);
        }
        for (slot, v) in self.ansi.iter_mut().zip(&colors.palette) {
            if let Some(c) = parse_hex(v) {
                *slot = c;
            }
        }
        self
    }
}

impl Palette {
    /// The divider between panes, as 0xRRGGBB: the theme's, else the
    /// background mixed 25% towards the foreground, which shows in any
    /// theme and appearance.
    pub fn divider(&self) -> u32 {
        self.split_divider.unwrap_or_else(|| {
            let channel = |c: u32, shift: u32| (c >> shift) & 0xFF;
            [16, 8, 0].into_iter().fold(0, |out, shift| {
                let (bg, fg) = (channel(self.background, shift), channel(self.foreground, shift));
                out | ((bg * 3 + fg + 2) / 4) << shift
            })
        })
    }
}

impl Default for Palette {
    fn default() -> Palette {
        Palette::DEFAULT
    }
}

/// The colours for each appearance: the theme's (or the defaults) with
/// `[colors]` on top. The same for both unless the theme is a light/dark
/// pair.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Palettes {
    pub light: Palette,
    pub dark: Palette,
}

/// Serialized (unset values left out, `palettes` skipped) for the
/// preferences' defaults and for comparing settings by their schema key.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font_family: String,
    pub font_size: f64,
    /// Fonts tried in order for characters `font_family` lacks, before the
    /// system's own fallback.
    pub font_fallback: Vec<String>,
    /// The bold, italic and bold italic faces by name; unset, the family's
    /// own.
    pub font_bold: Option<String>,
    pub font_italic: Option<String>,
    pub font_bold_italic: Option<String>,
    /// Multiplies the cell height; 1.0 is the font's own line height.
    #[serde(deserialize_with = "line_height")]
    pub line_height: f64,
    /// Widens or narrows cells: `"1.5"` points or `"10%"` of the advance.
    #[serde(deserialize_with = "cell_width", serialize_with = "cell_width_text")]
    pub cell_width: CellWidth,
    /// OpenType features turned on or off, Ghostty's and kitty's syntax:
    /// `["-calt", "-liga", "zero", "ss01=1"]`. Ligatures are on where the
    /// font has them; `["-calt", "-liga"]` turns them off. Entries that
    /// aren't features are a notice each and left out, by [`load`].
    pub font_features: Vec<String>,
    /// Cells East Asian Ambiguous characters take: 1 (the default) or 2,
    /// as CJK terminals draw them (cells design §6.3).
    #[serde(deserialize_with = "ambiguous_width")]
    pub ambiguous_width: u8,
    /// Space around the text, in points.
    pub padding: f64,
    pub scrollback: usize,
    pub option_as_meta: OptionAsMeta,
    /// 0.0–1.0. Below 1 the desktop shows through.
    pub opacity: f64,
    /// 0.0–1.0: how opaque panes without focus are drawn. 1, the default,
    /// doesn't dim them (the hollow cursor marks them).
    #[serde(deserialize_with = "unit")]
    pub unfocused_pane_opacity: f64,
    /// Blur what shows through when `opacity` is below 1.
    pub blur: bool,
    /// Copy a selection to the clipboard as soon as it is made with the
    /// mouse, without Cmd+C.
    pub copy_on_select: bool,
    /// Ask before closing a window or quitting while a program other than
    /// the shell runs in the foreground.
    pub confirm_close: bool,
    pub columns: u16,
    pub lines: u16,
    /// Program and arguments to run instead of the login shell.
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    /// Window title for a pin; also shown in the Window menu.
    pub title: Option<String>,
    /// Kitty graphics, iTerm2 images and Sixel (docs/design/images.md).
    /// On by default; `TD_IMAGES=1` in the environment turns it on for
    /// tests whatever this says.
    pub images: bool,
    /// The memory images may use, in MB (at most 16384); 0, the default,
    /// is the larger of 32 MB and two window-sized images.
    #[serde(deserialize_with = "image_memory")]
    pub image_memory: u32,
    /// Let programs send images by file path or shared memory (kitty's
    /// `t=f`, `t=t` and `t=s`) as well as inline.
    pub image_file_transfer: bool,
    /// Follow macOS (`"system"`), or always `"light"` or `"dark"`: picks
    /// the side of a light/dark theme pair.
    pub appearance: AppearanceMode,
    /// `theme = "name"` or `{ light = "…", dark = "…" }`; see [`crate::theme`].
    pub theme: Option<ThemeChoice>,
    /// How a new theme comes in: `"fade"` (the default), `"wave"`,
    /// `"pulse"`, `"glimmer"` or `"none"`.
    pub theme_transition: ThemeTransition,
    /// How long it takes, in milliseconds (at most
    /// [`THEME_TRANSITION_MS_MAX`]); 0 is at once.
    #[serde(deserialize_with = "transition_ms")]
    pub theme_transition_ms: u32,
    pub colors: Colors,
    /// The theme and `[colors]` resolved, by [`load`].
    #[serde(skip)]
    pub palettes: Palettes,
    /// `[keybind]`: shortcuts remapped, unbound, or sending text; see
    /// [`crate::keybind`].
    pub keybind: KeybindConfig,
    /// The window and tab title says what the focused pane does: the
    /// command running and for how long, or waiting at the prompt in which
    /// folder (needs the shell's OSC 133 marks; see
    /// [`crate::activity::status_title`]).
    pub title_status: bool,
    /// Notify when a command that ran at least `notify_after` seconds
    /// finishes in a tab or pane you aren't looking at (needs the shell's
    /// OSC 133 marks; see [`crate::activity`]).
    pub notify_on_finish: bool,
    /// How long a command must run, in seconds, for `notify_on_finish`.
    #[serde(deserialize_with = "notify_after")]
    pub notify_after: u32,
    /// Updates (docs/design/updater.md): `"notify"` (the default),
    /// `"auto"` or `"off"`. Release builds check once a day and install on
    /// quit unless off; builds from source say when `main` is ahead, and
    /// with `"auto"` update themselves when the Mac is idle. Only Terminal
    /// Decline.app's own process reads it; pins never update themselves.
    pub update: UpdateMode,
    /// Set in a pin's config: the app bundle it belongs to.
    #[allow(dead_code)]
    pub pin_app: Option<String>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            font_family: "Menlo".into(),
            font_size: 13.0,
            font_fallback: Vec::new(),
            font_bold: None,
            font_italic: None,
            font_bold_italic: None,
            line_height: 1.0,
            cell_width: CellWidth::default(),
            font_features: Vec::new(),
            ambiguous_width: 1,
            padding: 6.0,
            scrollback: 10_000,
            option_as_meta: OptionAsMeta::None,
            opacity: 1.0,
            unfocused_pane_opacity: 1.0,
            blur: false,
            copy_on_select: false,
            confirm_close: false,
            columns: 100,
            lines: 30,
            command: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            title: None,
            images: true,
            image_memory: 0,
            image_file_transfer: true,
            appearance: AppearanceMode::System,
            theme: None,
            theme_transition: ThemeTransition::Fade,
            theme_transition_ms: 300,
            colors: Colors::default(),
            palettes: Palettes::default(),
            keybind: KeybindConfig::default(),
            title_status: true,
            notify_on_finish: true,
            notify_after: 10,
            update: UpdateMode::Notify,
            pin_app: None,
        }
    }
}

fn line_height<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(serde::de::Error::custom(format!("{v} is not above 0")))
    }
}

fn ambiguous_width<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let v = i64::deserialize(d)?;
    match v {
        1 | 2 => Ok(v as u8),
        _ => Err(serde::de::Error::custom(format!("{v} is not 1 or 2"))),
    }
}

/// The longest `theme_transition_ms`.
pub const THEME_TRANSITION_MS_MAX: u32 = 2000;

fn transition_ms<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let v = i64::deserialize(d)?;
    match u32::try_from(v) {
        Ok(v) if v <= THEME_TRANSITION_MS_MAX => Ok(v),
        _ => Err(serde::de::Error::custom(format!("{v} is not 0–{THEME_TRANSITION_MS_MAX}"))),
    }
}

/// The most `image_memory` may ask for, in MB.
pub const IMAGE_MEMORY_MAX: u32 = 16_384;

fn image_memory<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let v = i64::deserialize(d)?;
    if (0..=IMAGE_MEMORY_MAX as i64).contains(&v) {
        Ok(v as u32)
    } else {
        Err(serde::de::Error::custom(format!(
            "{v} is not a number of MB from 0 to {IMAGE_MEMORY_MAX} (0 is the default)"
        )))
    }
}

/// The longest `notify_after`: a day.
pub const NOTIFY_AFTER_MAX: u32 = 86_400;

fn notify_after<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let v = i64::deserialize(d)?;
    match u32::try_from(v) {
        Ok(v) if v <= NOTIFY_AFTER_MAX => Ok(v),
        _ => Err(serde::de::Error::custom(format!(
            "{v} is not a number of seconds from 0 to {NOTIFY_AFTER_MAX}"
        ))),
    }
}

fn unit<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(serde::de::Error::custom(format!("{v} is not between 0 and 1")))
    }
}

/// `cell_width` as the config spells it: `"1.5"` or `"10%"`.
fn cell_width_text<S: Serializer>(w: &CellWidth, s: S) -> Result<S::Ok, S::Error> {
    match w {
        CellWidth::Points(p) => s.serialize_str(&p.to_string()),
        CellWidth::Percent(p) => s.serialize_str(&format!("{p}%")),
    }
}

/// `cell_width` as a string (`"1.5"`, `"-1"`, `"10%"`) or a plain number
/// of points.
fn cell_width<'de, D: Deserializer<'de>>(d: D) -> Result<CellWidth, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Text(String),
        Int(i64),
        Float(f64),
    }
    let bad = |what: &dyn std::fmt::Display| {
        serde::de::Error::custom(format!("{what} is not a width: use points (\"1.5\") or a percentage (\"10%\")"))
    };
    match Raw::deserialize(d).map_err(|_| bad(&"this"))? {
        Raw::Text(s) => CellWidth::parse(&s).ok_or_else(|| bad(&format_args!("{s:?}"))),
        Raw::Int(n) => Ok(CellWidth::Points(n as f64)),
        Raw::Float(f) if f.is_finite() => Ok(CellWidth::Points(f)),
        Raw::Float(f) => Err(bad(&f)),
    }
}

fn read_table(path: &Path, text: Option<&str>) -> Result<toml::Table, String> {
    let read = match text {
        Some(t) => Ok(t.to_string()),
        None => std::fs::read_to_string(path),
    };
    match read {
        Ok(s) => s.parse::<toml::Table>().map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Keys in `over` replace keys in `base`; tables merge.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

/// Loads each of `layers` in order, later ones on top (the user's
/// config.toml first, then a pin's settings, then `--config`). A missing
/// file counts as empty. Errors are reported and the defaults used, so a
/// typo never stops the terminal from opening. Themes are looked for in
/// `themes_dir`, then among the built-in ones.
pub fn load(layers: &[&Path], themes_dir: &Path) -> (Config, Vec<String>) {
    load_replacing(layers, themes_dir, None)
}

/// [`load`], with layer `i` read from `text` instead of its file when
/// `replace` is `Some((i, text))`: what the preferences check an edit with
/// before writing it.
pub fn load_replacing(layers: &[&Path], themes_dir: &Path, replace: Option<(usize, &str)>) -> (Config, Vec<String>) {
    let mut errors = Vec::new();
    let mut table = toml::Table::new();
    for (i, &path) in layers.iter().enumerate() {
        let text = replace.filter(|(r, _)| *r == i).map(|(_, t)| t);
        match read_table(path, text) {
            Ok(mut t) => {
                // So a pin's "command+d" replaces the user's "cmd+d".
                keybind::canonicalize(&mut t);
                merge(&mut table, t)
            }
            Err(e) => errors.push(e),
        }
    }
    let (table, bad) = keep_valid(table);
    errors.extend(bad);
    let mut config = toml::Value::Table(table).try_into::<Config>().unwrap_or_else(|e| {
        errors.push(format!("config: {e}"));
        Config::default()
    });
    check_features(&mut config, &mut errors);
    config.palettes = resolve(config.theme.as_ref(), &config.colors, themes_dir, &mut errors);
    (config, errors)
}

/// Leaves out the `font_features` entries that aren't features, with a
/// notice for each, so one typo costs only that entry.
fn check_features(config: &mut Config, errors: &mut Vec<String>) {
    config.font_features.retain(|entry| {
        let ok = term_font::Tag::parse_feature(entry).is_some();
        if !ok {
            errors.push(format!(
                "config: font_features: {entry:?} is not a feature (use \"tag\", \"-tag\" or \"tag=n\", \
                 with a tag of four letters such as \"liga\"); ignored"
            ));
        }
        ok
    });
}

/// The colours for each appearance: theme, then `[colors]`. A theme that
/// can't be loaded is reported and the defaults used.
fn resolve(theme: Option<&ThemeChoice>, colors: &Colors, themes_dir: &Path, errors: &mut Vec<String>) -> Palettes {
    let mut base = |name: &str| match themes::load(themes_dir, name) {
        Ok(c) => Palette::DEFAULT.with(&c),
        Err(e) => {
            errors.push(format!("config: theme: {e}; ignored"));
            Palette::DEFAULT
        }
    };
    let (light, dark) = match theme {
        None => (Palette::DEFAULT, Palette::DEFAULT),
        Some(choice) => {
            let light = base(choice.pick(Appearance::Light));
            let dark_name = choice.pick(Appearance::Dark);
            let dark = if dark_name == choice.pick(Appearance::Light) {
                light
            } else {
                base(dark_name)
            };
            (light, dark)
        }
    };
    Palettes {
        light: light.with(colors),
        dark: dark.with(colors),
    }
}

/// Where the config comes from, kept so it can be read again when a file
/// changes.
#[derive(Clone, Debug, Default)]
pub struct Source {
    /// The user's config.toml, then a pin's settings, then `--config`.
    pub layers: Vec<PathBuf>,
    /// The user's own themes: `~/.config/terminal-decline/themes`.
    pub themes_dir: PathBuf,
    /// `-e PROGRAM ARGS...`, which replaces `command` whatever the files say.
    pub command: Option<Vec<String>>,
    /// The layer Settings… and Edit config.toml edit: the user's file, or
    /// in a pin the pin's own (an index into `layers`).
    pub edit: usize,
}

impl Source {
    /// The config and its errors, as [`load`] gives them.
    pub fn load(&self) -> (Config, Vec<String>) {
        let refs: Vec<&Path> = self.layers.iter().map(PathBuf::as_path).collect();
        let (mut config, errors) = load(&refs, &self.themes_dir);
        if let Some(cmd) = self.command.as_ref().filter(|c| !c.is_empty()) {
            config.command = cmd.clone();
        }
        (config, errors)
    }

    /// The file Settings… and Edit config.toml edit.
    pub fn edited(&self) -> Option<&Path> {
        self.layers.get(self.edit).map(PathBuf::as_path)
    }

    /// `terminal-decline prefs` arguments for editing this config with
    /// the app there to record chords.
    pub fn prefs_args(&self) -> Vec<String> {
        let mut args = vec!["prefs".to_string(), "--layer".to_string()];
        args.extend(self.layers.iter().map(|l| l.display().to_string()));
        args.extend(["--edit".to_string(), self.edit.to_string(), "--in-app".to_string()]);
        args
    }

    /// The files whose changes reload the config: its layers and the
    /// theme files it names (watched even before they exist).
    pub fn watched(&self, config: &Config) -> Vec<PathBuf> {
        let mut files = self.layers.clone();
        if let Some(choice) = &config.theme {
            files.extend(choice.names().into_iter().map(|n| themes::file_of(&self.themes_dir, n)));
        }
        files
    }
}

/// Settings that a running window can't take on: they apply to windows
/// opened after a reload. The rest apply to every window at once (see
/// DESIGN.md, Config). Which ones is the schema's (`term_prefs::schema`):
/// `blur` is a view set up with the window, term-core can't change a
/// terminal's `scrollback` limit, and the program, its folder and its
/// environment are already running.
pub fn new_windows_only(old: &Config, new: &Config) -> Vec<&'static str> {
    let (old, new) = (old.as_table(), new.as_table());
    term_prefs::schema::new_windows_keys()
        .into_iter()
        .filter(|key| setting_of(&old, key) != setting_of(&new, key))
        .collect()
}

/// The value at a schema key (`columns`, `colors.background`), if set.
fn setting_of<'a>(table: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let mut parts = key.split('.');
    let mut value = table.get(parts.next()?)?;
    for part in parts {
        value = value.as_table()?.get(part)?;
    }
    Some(value)
}

impl Config {
    /// As a TOML table, unset values left out.
    pub fn as_table(&self) -> toml::Table {
        toml::Table::try_from(self).unwrap_or_default()
    }

    /// `Config::default()` as a TOML document: the preferences' defaults.
    pub fn defaults_toml() -> String {
        toml::to_string(&Config::default()).unwrap_or_default()
    }
}

/// The notice for settings that changed but apply to new windows only.
pub fn new_windows_notice(changed: &[&str]) -> Option<String> {
    match changed {
        [] => None,
        [one] => Some(format!("config: {one} changed: it applies to new windows")),
        many => Some(format!("config: {} changed: they apply to new windows", many.join(", "))),
    }
}

fn accepts(table: toml::Table) -> Result<(), String> {
    toml::Value::Table(table)
        .try_into::<Config>()
        .map(|_| ())
        .map_err(|e| e.message().to_string())
}

/// Drops settings that are unknown or of the wrong type, one by one, so a
/// typo costs only that line rather than the whole file.
fn keep_valid(table: toml::Table) -> (toml::Table, Vec<String>) {
    let mut good = toml::Table::new();
    let mut errors = Vec::new();
    for (key, value) in table {
        match value {
            // Nested tables (colours) are checked key by key too; `theme`'s
            // light/dark pair is one setting.
            toml::Value::Table(inner) if key != "theme" => {
                let mut kept = toml::Table::new();
                for (k, v) in inner {
                    let probe =
                        toml::Table::from_iter([(key.clone(), toml::Value::Table(toml::Table::from_iter([(k.clone(), v.clone())])))]);
                    match accepts(probe) {
                        Ok(()) => {
                            kept.insert(k, v);
                        }
                        Err(e) => errors.push(format!("config: {key}.{k}: {e}; ignored")),
                    }
                }
                if accepts(toml::Table::from_iter([(key.clone(), toml::Value::Table(kept.clone()))])).is_ok() {
                    good.insert(key, toml::Value::Table(kept));
                } else {
                    errors.push(format!("config: {key}: not a table of settings; ignored"));
                }
            }
            v => match accepts(toml::Table::from_iter([(key.clone(), v.clone())])) {
                Ok(()) => {
                    good.insert(key, v);
                }
                Err(e) => errors.push(format!("config: {key}: {e}; ignored")),
            },
        }
    }
    (good, errors)
}

pub fn parse_hex(s: &str) -> Option<u32> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    u32::from_str_radix(s, 16).ok()
}

impl Config {
    /// The font settings at `size` points (the window's current size,
    /// which Cmd+= and Cmd+- change).
    pub fn font_config(&self, size: f64) -> FontConfig {
        FontConfig {
            fallback: self.font_fallback.clone(),
            bold: self.font_bold.clone(),
            italic: self.font_italic.clone(),
            bold_italic: self.font_bold_italic.clone(),
            line_height: self.line_height,
            cell_width: self.cell_width,
            features: self.font_features.iter().filter_map(|f| term_font::Tag::parse_feature(f)).collect(),
            ..FontConfig::new(&self.font_family, size)
        }
    }

    /// Whether the config names fonts besides the family (fallbacks or
    /// faces), the only ones `Fonts::missing` can list.
    pub fn names_fonts(&self) -> bool {
        !self.font_fallback.is_empty() || self.font_bold.is_some() || self.font_italic.is_some() || self.font_bold_italic.is_some()
    }

    /// A notice for each font named in the config that isn't installed
    /// (`missing`, from `Fonts::missing`), naming the setting it came from.
    pub fn missing_font_notices(&self, missing: &[String]) -> Vec<String> {
        missing
            .iter()
            .map(|name| {
                let key = [
                    ("font_bold", &self.font_bold),
                    ("font_italic", &self.font_italic),
                    ("font_bold_italic", &self.font_bold_italic),
                ]
                .into_iter()
                .find(|(_, v)| v.as_deref() == Some(name.as_str()))
                .map_or("font_fallback", |(k, _)| k);
                format!("config: {key}: no font named {name:?} is installed; ignored")
            })
            .collect()
    }

    /// The shortcuts in effect: the menus' own with `[keybind]` applied.
    pub fn keybinds(&self) -> Keybinds {
        Keybinds::new(&self.keybind)
    }

    /// The colours for a window with `appearance`.
    pub fn palette(&self, appearance: Appearance) -> Palette {
        match appearance {
            Appearance::Light => self.palettes.light,
            Appearance::Dark => self.palettes.dark,
        }
    }

    /// This config with theme `name` for light and dark alike ("" for
    /// none), `[colors]` still on top: what windows show while Settings
    /// previews a theme. A theme that doesn't load gives the defaults.
    pub fn previewing(&self, name: &str, themes_dir: &Path) -> Config {
        let mut c = self.clone();
        c.theme = (!name.is_empty()).then(|| ThemeChoice::One(name.to_string()));
        c.palettes = resolve(c.theme.as_ref(), &c.colors, themes_dir, &mut Vec::new());
        c
    }

    /// The renderer's theme for a window with `appearance`.
    pub fn theme(&self, appearance: Appearance) -> Theme {
        let p = self.palette(appearance);
        let mut theme = Theme::new(p.ansi, p.foreground, p.background, p.cursor, p.selection);
        theme.background_opacity = (self.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        theme
    }
}

/// A theme colour (0xAABBGGRR, as the renderer stores it) as 0xRRGGBB.
pub fn rgb_of(c: u32) -> u32 {
    (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
}

/// Foreground, background and cursor colours as 0xRRGGBB, for answering
/// colour queries (OSC 10, 11 and 12).
pub fn query_colors(theme: &Theme) -> [u32; 3] {
    [theme.foreground, theme.background, theme.cursor].map(rgb_of)
}

/// The theme's 256 colours as 0xRRGGBB: the palette programs read (OSC 4)
/// and go back to (OSC 104).
pub fn query_palette(theme: &Theme) -> [u32; 256] {
    theme.palette.map(rgb_of)
}

/// The theme's colours as the terminal's defaults: what colour queries
/// answer and resets go back to.
/// Programs are told whether the colours are dark or light (DSR 996, mode
/// 2031) from the theme's background, so a light theme in dark mode is
/// reported light. Returns whether a report waits in the terminal's
/// replies, to be sent to the program.
pub fn set_term_colors(t: &mut term_core::Terminal, theme: &Theme) -> bool {
    t.set_default_colors(query_colors(theme));
    t.set_default_palette(&query_palette(theme));
    t.set_dark(is_dark(rgb_of(theme.background)))
}

/// Whether a background (0xRRGGBB) is dark: its relative luminance under
/// a half.
pub fn is_dark(rgb: u32) -> bool {
    let c = |s: u32| ((rgb >> s) & 0xff) as f64 / 255.0;
    0.2126 * c(16) + 0.7152 * c(8) + 0.0722 * c(0) < 0.5
}

/// What a pane draws with: `base` with the colours its program set (OSC
/// 4, 10–12) on top, or `None` while they are all the theme's.
pub fn program_theme(base: &Theme, t: &term_core::Terminal) -> Option<Theme> {
    use term_core::DynamicColor;
    let palette = t.palette();
    let dynamic = [DynamicColor::Foreground, DynamicColor::Background, DynamicColor::Cursor].map(|c| t.dynamic_color(c));
    if *palette == query_palette(base) && dynamic == query_colors(base) {
        return None;
    }
    let mut theme = base.clone();
    theme.palette = palette.map(term_render::rgb);
    let [fg, bg, cursor] = dynamic;
    [theme.foreground, theme.cursor] = [fg, cursor].map(term_render::rgb);
    // The search hits and the text under the cursor follow the
    // program's background, so they stay readable on it.
    if term_render::rgb(bg) != base.background {
        theme.set_background(bg);
    }
    Some(theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_merge() {
        let mut base: toml::Table = "font_size = 13\n[colors]\nforeground = '#ffffff'\nbackground = '#000000'"
            .parse()
            .unwrap();
        let over: toml::Table = "font_size = 16\n[colors]\nbackground = '#111111'".parse().unwrap();
        merge(&mut base, over);
        let c: Config = toml::Value::Table(base).try_into().unwrap();
        assert_eq!(c.font_size, 16.0);
        assert_eq!(c.colors.foreground.as_deref(), Some("#ffffff"));
        assert_eq!(c.colors.background.as_deref(), Some("#111111"));
    }

    #[test]
    fn bad_settings_dropped_one_by_one() {
        let t: toml::Table = "font_size = 15\nfont_sise = 14\npadding = 'wide'\n[colors]\nbackground = '#000000'\nbackgrund = '#ffffff'"
            .parse()
            .unwrap();
        let (good, errors) = keep_valid(t);
        let c: Config = toml::Value::Table(good).try_into().unwrap();
        assert_eq!(c.font_size, 15.0);
        assert_eq!(c.padding, Config::default().padding);
        assert_eq!(c.colors.background.as_deref(), Some("#000000"));
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("font_sise")));
        assert!(errors.iter().any(|e| e.contains("colors.backgrund")));
        assert!(errors.iter().any(|e| e.starts_with("config: padding")));
    }

    #[test]
    fn colors_not_a_table_is_ignored() {
        let t: toml::Table = "font_size = 16\ncolors = 3".parse().unwrap();
        let (good, errors) = keep_valid(t);
        let c: Config = toml::Value::Table(good).try_into().unwrap();
        assert_eq!(c.font_size, 16.0);
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    use crate::testutil::TempDir;

    #[test]
    fn missing_files_give_defaults() {
        let dir = TempDir::new("cfg-missing");
        let (c, errors) = load(&[&dir.join("config.toml"), &dir.join("pin.toml")], Path::new("/nonexistent"));
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(c.font_size, Config::default().font_size);
        assert_eq!(c.columns, 100);
        let (c, errors) = load(&[], Path::new("/nonexistent"));
        assert!(errors.is_empty());
        assert_eq!(c.font_family, "Menlo");
    }

    #[test]
    fn files_layer_in_order() {
        let dir = TempDir::new("cfg-layers");
        let user = dir.join("config.toml");
        let pin = dir.join("pin.toml");
        std::fs::write(
            &user,
            "font_size = 14\npadding = 2\n[colors]\nforeground = '#010203'\nbackground = '#040506'\n",
        )
        .unwrap();
        std::fs::write(&pin, "font_size = 18\ncommand = ['htop']\n[colors]\nbackground = '#0a0b0c'\n").unwrap();
        let (c, errors) = load(&[&user, &pin], Path::new("/nonexistent"));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.font_size, 18.0);
        assert_eq!(c.padding, 2.0);
        assert_eq!(c.command, ["htop"]);
        assert_eq!(c.colors.foreground.as_deref(), Some("#010203"));
        assert_eq!(c.colors.background.as_deref(), Some("#0a0b0c"));
    }

    #[test]
    fn unreadable_or_unknown_settings_report_errors() {
        let dir = TempDir::new("cfg-errors");
        let broken = dir.join("broken.toml");
        std::fs::write(&broken, "font_size = = 3").unwrap();
        let typo = dir.join("typo.toml");
        std::fs::write(&typo, "font_sise = 14\nlines = 40\n").unwrap();
        let (c, errors) = load(&[&broken, &typo], Path::new("/nonexistent"));
        assert_eq!(c.lines, 40);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].starts_with(&broken.display().to_string()), "{errors:?}");
        assert!(errors[1].contains("font_sise"), "{errors:?}");
        // A directory can't be read as a file.
        let (_, errors) = load(&[dir.path()], Path::new("/nonexistent"));
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    #[test]
    fn option_as_meta_values() {
        for (text, want) in [
            ("left", Some(OptionAsMeta::Left)),
            ("right", Some(OptionAsMeta::Right)),
            ("both", Some(OptionAsMeta::Both)),
            ("none", Some(OptionAsMeta::None)),
            ("Left", None),
            ("yes", None),
        ] {
            let t: toml::Table = format!("option_as_meta = '{text}'").parse().unwrap();
            let (good, errors) = keep_valid(t);
            let c: Config = toml::Value::Table(good).try_into().unwrap();
            match want {
                Some(w) => assert_eq!((c.option_as_meta, errors.len()), (w, 0), "{text}"),
                None => assert_eq!((c.option_as_meta, errors.len()), (OptionAsMeta::None, 1), "{text}"),
            }
        }
    }

    #[test]
    fn hex_colours() {
        for (text, want) in [
            ("#112233", Some(0x112233)),
            ("112233", Some(0x112233)),
            ("#ABCdef", Some(0xabcdef)),
            ("#123", None),
            ("#1122334", None),
            ("#gg0000", None),
            ("", None),
        ] {
            assert_eq!(parse_hex(text), want, "{text:?}");
        }
    }

    /// Bad colours are reported one by one and the rest kept; the query
    /// colours (OSC 10/11/12) come back as 0xRRGGBB.
    #[test]
    fn theme_and_query_colours() {
        let (c, errors) =
            parse("[colors]\nforeground = '#112233'\nbackground = 'nonsense'\ncursor = '#a1b2c3'\npalette = ['#ff0000', '', '#00ff00']");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].starts_with("config: colors.background: \"nonsense\" is not a colour"),
            "{errors:?}"
        );
        let mut c = c;
        c.palettes = resolve(None, &c.colors, Path::new("/nonexistent"), &mut Vec::new());
        let theme = c.theme(Appearance::Dark);
        assert_eq!(query_colors(&theme), [0x112233, 0x16181c, 0xa1b2c3]);
        assert_eq!(rgb_of(theme.palette[0]), 0xff0000);
        assert_eq!(rgb_of(theme.palette[1]), 0xcc6666, "\"\" keeps the default");
        assert_eq!(rgb_of(theme.palette[2]), 0x00ff00);
        assert_eq!(rgb_of(theme.palette[15]), 0xeaeaea, "missing entries use the defaults");
        assert_eq!(rgb_of(theme.selection), 0x2e3a4f);
        assert_eq!(
            query_colors(&Config::default().theme(Appearance::Light)),
            [0xd8dee9, 0x16181c, 0xd8dee9]
        );
        for (r, g, b) in [(0x12, 0x34, 0x56), (0xff, 0, 0), (0, 0, 0xff)] {
            assert_eq!(rgb_of(term_render::rgb(r << 16 | g << 8 | b)), r << 16 | g << 8 | b);
        }
        let (_, errors) = parse("[colors]\npalette = ['#ff0000', 'red']");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("\"red\" is not a colour"), "{errors:?}");
    }

    /// `theme` loads a built-in theme or the user's file of that name;
    /// `[colors]` override it; a pair follows the appearance; a missing
    /// theme is a notice.
    #[test]
    fn themes_load_and_colors_override_them() {
        let dir = TempDir::new("cfg-theme");
        let themes = dir.join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        let file = dir.join("config.toml");
        let with = |text: &str| {
            std::fs::write(&file, text).unwrap();
            load(&[&file], &themes)
        };
        let (c, errors) = with("theme = 'solarized-dark'\n");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.palette(Appearance::Light), c.palette(Appearance::Dark));
        let p = c.palette(Appearance::Dark);
        assert_eq!((p.foreground, p.background, p.ansi[1]), (0x839496, 0x002b36, 0xdc322f));
        let (c, errors) = with("theme = 'solarized-dark'\n[colors]\nbackground = '#000000'\npalette = ['', '#ff0000']\n");
        assert!(errors.is_empty(), "{errors:?}");
        let p = c.palette(Appearance::Dark);
        assert_eq!((p.foreground, p.background), (0x839496, 0x000000), "[colors] wins");
        assert_eq!((p.ansi[0], p.ansi[1], p.ansi[2]), (0x073642, 0xff0000, 0x859900));
        std::fs::write(themes.join("mine.toml"), "background = '#123456'\n").unwrap();
        let (c, errors) = with("theme = { light = 'mine', dark = 'tokyo-night' }\n");
        assert!(errors.is_empty(), "{errors:?}");
        let (light, dark) = (c.palette(Appearance::Light), c.palette(Appearance::Dark));
        assert_eq!(light.background, 0x123456);
        assert_eq!(light.foreground, Palette::DEFAULT.foreground, "unset in the theme");
        assert_eq!(dark.background, 0x1a1b26);
        assert_eq!(c.theme(Appearance::Light).background, term_render::rgb(0x123456));
        let (c, errors) = with("theme = 'no-such-theme'\nfont_size = 15\n");
        assert_eq!(c.font_size, 15.0);
        assert_eq!(c.palettes, Palettes::default());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].starts_with("config: theme: no theme named \"no-such-theme\""),
            "{errors:?}"
        );
        let (_, errors) = with("theme = { light = 'mine' }\n");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("config: theme: "), "{errors:?}");
        // The theme files named are watched too.
        let from = Source {
            layers: vec![file.clone()],
            themes_dir: themes.clone(),
            command: None,
            edit: 0,
        };
        let (c, _) = with("theme = { light = 'mine', dark = 'tokyo-night' }\n");
        assert_eq!(
            from.watched(&c),
            [file.clone(), themes.join("mine.toml"), themes.join("tokyo-night.toml")]
        );
    }

    fn parse(text: &str) -> (Config, Vec<String>) {
        let (good, errors) = keep_valid(text.parse().unwrap());
        (toml::Value::Table(good).try_into().unwrap(), errors)
    }

    #[test]
    fn font_settings_default_to_the_fonts_own() {
        let c = Config::default();
        assert_eq!(c.font_config(13.0), FontConfig::new("Menlo", 13.0));
        assert_eq!(c.font_config(20.0).size, 20.0);
    }

    #[test]
    fn font_settings_are_read() {
        let (c, errors) = parse(
            "font_family = 'JetBrains Mono'\nfont_fallback = ['Symbols Nerd Font Mono', 'Apple Color Emoji']\n\
             font_bold = 'JetBrains Mono ExtraBold'\nfont_italic = 'Menlo Italic'\nfont_bold_italic = 'Menlo Bold Italic'\n\
             line_height = 1.2\ncell_width = '10%'",
        );
        assert!(errors.is_empty(), "{errors:?}");
        let fc = c.font_config(15.0);
        assert_eq!(fc.family, "JetBrains Mono");
        assert_eq!(fc.size, 15.0);
        assert_eq!(fc.fallback, ["Symbols Nerd Font Mono", "Apple Color Emoji"]);
        assert_eq!(fc.bold.as_deref(), Some("JetBrains Mono ExtraBold"));
        assert_eq!(fc.italic.as_deref(), Some("Menlo Italic"));
        assert_eq!(fc.bold_italic.as_deref(), Some("Menlo Bold Italic"));
        assert_eq!(fc.line_height, 1.2);
        assert_eq!(fc.cell_width, CellWidth::Percent(10.0));
    }

    #[test]
    fn font_features_are_read_and_bad_entries_noticed() {
        let t = |s| term_font::Tag::new(s).unwrap();
        let (mut c, errors) = parse("font_features = ['-calt', '+liga', 'zero', 'ss01=1', 'cv05 = 2', 'toolong', '', 'x=y']");
        assert!(errors.is_empty(), "{errors:?}");
        let mut errors = Vec::new();
        check_features(&mut c, &mut errors);
        assert_eq!(c.font_features, ["-calt", "+liga", "zero", "ss01=1", "cv05 = 2"]);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(
            errors[0].starts_with("config: font_features: \"toolong\" is not a feature"),
            "{errors:?}"
        );
        assert!(errors.iter().all(|e| e.ends_with("; ignored")), "{errors:?}");
        assert_eq!(
            c.font_config(13.0).features,
            [(t("calt"), 0), (t("liga"), 1), (t("zero"), 1), (t("ss01"), 1), (t("cv05"), 2)]
        );
        // Not a list: the whole setting goes, as for any other.
        let (c, errors) = parse("font_features = 'liga'");
        assert_eq!(c.font_features, Vec::<String>::new());
        assert!(errors[0].starts_with("config: font_features: "), "{errors:?}");
    }

    #[test]
    fn font_features_load_from_a_file() {
        let dir = TempDir::new("cfg-features");
        let user = dir.join("config.toml");
        std::fs::write(&user, "font_features = ['-calt', 'nope!', '-liga']\n").unwrap();
        let (c, errors) = load(&[&user], Path::new("/nonexistent"));
        assert_eq!(c.font_features, ["-calt", "-liga"]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("\"nope!\""), "{errors:?}");
    }

    #[test]
    fn ambiguous_width_is_1_or_2() {
        assert_eq!(Config::default().ambiguous_width, 1);
        let (c, errors) = parse("ambiguous_width = 2");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.ambiguous_width, 2);
        for bad in ["0", "3", "1.5", "'2'"] {
            let (c, errors) = parse(&format!("ambiguous_width = {bad}"));
            assert_eq!(c.ambiguous_width, 1, "{bad}");
            assert!(errors[0].starts_with("config: ambiguous_width: "), "{bad}: {errors:?}");
        }
    }

    #[test]
    fn cell_width_values() {
        for (text, want) in [
            ("'0'", Some(CellWidth::Points(0.0))),
            ("'1.5'", Some(CellWidth::Points(1.5))),
            ("'-1'", Some(CellWidth::Points(-1.0))),
            ("'10%'", Some(CellWidth::Percent(10.0))),
            ("2", Some(CellWidth::Points(2.0))),
            ("0.5", Some(CellWidth::Points(0.5))),
            ("'wide'", None),
            ("'10px'", None),
            ("true", None),
        ] {
            let (c, errors) = parse(&format!("cell_width = {text}\nline_height = 2"));
            assert_eq!(c.line_height, 2.0, "{text}");
            match want {
                Some(w) => assert_eq!((c.cell_width, errors.len()), (w, 0), "{text}: {errors:?}"),
                None => {
                    assert_eq!(c.cell_width, CellWidth::default(), "{text}");
                    assert_eq!(errors.len(), 1, "{text}: {errors:?}");
                    assert!(errors[0].starts_with("config: cell_width: "), "{errors:?}");
                    assert!(errors[0].contains("not a width"), "{errors:?}");
                }
            }
        }
    }

    #[test]
    fn line_height_must_be_above_zero() {
        for text in ["0", "-1.5", "'tall'", "nan"] {
            let (c, errors) = parse(&format!("line_height = {text}"));
            assert_eq!(c.line_height, 1.0, "{text}");
            assert_eq!(errors.len(), 1, "{text}: {errors:?}");
            assert!(errors[0].starts_with("config: line_height: "), "{errors:?}");
        }
    }

    #[test]
    fn missing_fonts_name_their_setting() {
        let c = Config {
            font_fallback: vec!["Nope Mono".into()],
            font_bold: Some("Nope Bold".into()),
            font_bold_italic: Some("Nope Bold Italic".into()),
            ..Config::default()
        };
        let notices = c.missing_font_notices(&["Nope Bold".into(), "Nope Bold Italic".into(), "Nope Mono".into()]);
        assert_eq!(
            notices,
            [
                "config: font_bold: no font named \"Nope Bold\" is installed; ignored",
                "config: font_bold_italic: no font named \"Nope Bold Italic\" is installed; ignored",
                "config: font_fallback: no font named \"Nope Mono\" is installed; ignored",
            ]
        );
        assert!(c.missing_font_notices(&[]).is_empty());
    }

    #[test]
    fn copy_on_select_is_off_unless_set() {
        assert!(!Config::default().copy_on_select);
        assert!(parse("copy_on_select = true").0.copy_on_select);
        assert!(!parse("copy_on_select = false").0.copy_on_select);
        let (c, errors) = parse("copy_on_select = 'yes'");
        assert!(!c.copy_on_select);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("config: copy_on_select: "), "{errors:?}");
    }

    #[test]
    fn update_is_on_unless_turned_off() {
        assert_eq!(Config::default().update, UpdateMode::Notify);
        assert_eq!(parse("update = false").0.update, UpdateMode::Off);
        assert_eq!(parse("update = true").0.update, UpdateMode::Notify);
        assert_eq!(parse("update = 'auto'").0.update, UpdateMode::Auto);
        assert_eq!(parse("update = 'off'").0.update, UpdateMode::Off);
        let (c, errors) = parse("update = 'no'");
        assert_eq!(c.update, UpdateMode::Notify);
        assert!(errors[0].starts_with("config: update: "), "{errors:?}");
    }

    #[test]
    fn finish_notifications_are_on_after_ten_seconds() {
        let c = Config::default();
        assert!(c.title_status);
        assert!(!parse("title_status = false").0.title_status);
        assert!(c.notify_on_finish);
        assert_eq!(c.notify_after, 10);
        let (c, errors) = parse("notify_on_finish = false\nnotify_after = 30");
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!c.notify_on_finish);
        assert_eq!(c.notify_after, 30);
        for bad in ["notify_after = -1", "notify_after = 86401", "notify_after = 'ten'"] {
            let (c, errors) = parse(bad);
            assert_eq!(c.notify_after, 10, "{bad}");
            assert!(errors[0].starts_with("config: notify_after: "), "{errors:?}");
        }
    }

    #[test]
    fn confirm_close_is_off_unless_set() {
        assert!(!Config::default().confirm_close);
        assert!(parse("confirm_close = true").0.confirm_close);
        let (c, errors) = parse("confirm_close = 1");
        assert!(!c.confirm_close);
        assert!(errors[0].starts_with("config: confirm_close: "), "{errors:?}");
    }

    #[test]
    fn keybind_entries_are_checked_one_by_one() {
        let (c, errors) = parse(
            "[keybind]\n\"cmd+k\" = \"none\"\n\"cmd+j\" = \"explode\"\n\"hyper+x\" = \"copy\"\n\
             \"ctrl+shift+t\" = { text = \"\\u001b[116;6u\" }\n",
        );
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("config: keybind.cmd+j: unknown action \"explode\"")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("config: keybind.hyper+x: ") && e.contains("unknown modifier")),
            "{errors:?}"
        );
        let b = c.keybinds();
        assert_eq!(b.get(&keybind::Chord::parse("cmd+k").unwrap()), None);
        assert_eq!(
            b.get(&keybind::Chord::parse("ctrl+shift+t").unwrap()),
            Some(&keybind::Bound::Text("\x1b[116;6u".into()))
        );
        let (_, errors) = parse("keybind = 'cmd+k'");
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    /// A pin's `[keybind]` is layered on the user's chord by chord, however
    /// either spells it.
    #[test]
    fn pin_keybinds_override_the_users() {
        use keybind::{Bound, Chord, MenuAction};
        let dir = TempDir::new("cfg-keybind");
        let user = dir.join("config.toml");
        let pin = dir.join("pin.toml");
        std::fs::write(
            &user,
            "[keybind]\n\"cmd+d\" = { text = \"user\" }\n\"cmd+l\" = \"clear_scrollback\"\n\"shift+cmd+t\" = \"new_window\"\n",
        )
        .unwrap();
        std::fs::write(
            &pin,
            "[keybind]\n\"command+D\" = \"none\"\n\"cmd+shift+t\" = { text = \"pin\" }\n\"cmd+w\" = \"none\"\n",
        )
        .unwrap();
        let (c, errors) = load(&[&user, &pin], Path::new("/nonexistent"));
        assert!(errors.is_empty(), "{errors:?}");
        let b = c.keybinds();
        let get = |s: &str| b.get(&Chord::parse(s).unwrap()).cloned();
        assert_eq!(get("cmd+d"), None, "the pin unbinds the user's text");
        assert_eq!(get("cmd+shift+t"), Some(Bound::Text("pin".into())));
        assert_eq!(get("cmd+l"), Some(Bound::Menu(MenuAction::ClearScrollback)), "the user's own stays");
        assert_eq!(get("cmd+w"), None, "the pin unbinds a default");
        assert_eq!(get("cmd+k"), Some(Bound::Menu(MenuAction::ClearScrollback)));
        // The user's config alone.
        let (c, _) = load(&[&user], Path::new("/nonexistent"));
        assert_eq!(c.keybinds().get(&Chord::parse("cmd+d").unwrap()), Some(&Bound::Text("user".into())));
    }

    #[test]
    fn some_settings_wait_for_new_windows() {
        let old = Config::default();
        assert!(new_windows_only(&old, &old.clone()).is_empty());
        let (live, _) = parse(
            "font_size = 20\npadding = 1\nopacity = 0.5\noption_as_meta = 'left'\ncopy_on_select = true\n\
             [colors]\nbackground = '#000000'\n[keybind]\n\"cmd+k\" = \"none\"",
        );
        assert!(new_windows_only(&old, &live).is_empty());
        let (later, _) = parse("columns = 80\ncommand = ['htop']\nscrollback = 5\nblur = true");
        let changed = new_windows_only(&old, &later);
        assert_eq!(changed, ["blur", "columns", "scrollback", "command"]);
        assert_eq!(
            new_windows_notice(&changed).unwrap(),
            "config: blur, columns, scrollback, command changed: they apply to new windows"
        );
        assert_eq!(
            new_windows_notice(&["lines"]).unwrap(),
            "config: lines changed: it applies to new windows"
        );
        assert_eq!(new_windows_notice(&[]), None);
    }

    #[test]
    fn source_keeps_the_command_line_program() {
        let dir = TempDir::new("cfg-source");
        let file = dir.join("config.toml");
        std::fs::write(&file, "command = ['vim']\nfont_size = 15\n").unwrap();
        let mut source = Source {
            layers: vec![file.clone()],
            command: None,
            ..Source::default()
        };
        assert_eq!(source.load().0.command, ["vim"]);
        source.command = Some(vec!["htop".into()]);
        let (c, errors) = source.load();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!((c.command.as_slice(), c.font_size), (["htop".to_string()].as_slice(), 15.0));
        assert_eq!(source.watched(&c), [file]);
    }

    #[test]
    fn settings_edit_the_pins_file_over_yours() {
        let source = Source {
            layers: vec!["/u/config.toml".into(), "/u/pins/rush.toml".into(), "/tmp/extra.toml".into()],
            edit: 1,
            ..Source::default()
        };
        assert_eq!(source.edited(), Some(Path::new("/u/pins/rush.toml")));
        assert_eq!(
            source.prefs_args(),
            [
                "prefs",
                "--layer",
                "/u/config.toml",
                "/u/pins/rush.toml",
                "/tmp/extra.toml",
                "--edit",
                "1",
                "--in-app"
            ]
        );
        // What term-prefs reads them as.
        let args = term_prefs::Args::parse(&source.prefs_args()[1..]).unwrap();
        assert_eq!((args.layers.len(), args.edit, args.in_app), (3, 1, true));
    }

    #[test]
    fn pane_settings() {
        let (c, errors) = parse("unfocused_pane_opacity = 0.7\n[colors]\nsplit_divider = '#123456'\n");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.unfocused_pane_opacity, 0.7);
        assert_eq!(Palette::DEFAULT.with(&c.colors).divider(), 0x123456);
        let (c, errors) = parse("unfocused_pane_opacity = 1.5\n[colors]\nsplit_divider = 'grey'\n");
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(c.unfocused_pane_opacity, 1.0, "not dimmed by default");
        // Unset: the background a quarter of the way to the foreground.
        let p = Palette {
            background: 0x000000,
            foreground: 0xffffff,
            ..Palette::DEFAULT
        };
        assert_eq!(p.divider(), 0x404040);
        let p = Palette {
            background: 0xfdf6e3,
            foreground: 0x657b83,
            ..Palette::DEFAULT
        };
        assert_eq!(p.divider(), 0xd7d7cb);
        // A theme can set it.
        let t = crate::theme::parse("background = '#000000'\nsplit_divider = '#ff0000'\n").unwrap();
        assert_eq!(Palette::DEFAULT.with(&t).divider(), 0xff0000);
    }

    /// The field names serde passes to `deserialize_struct`, which are the
    /// keys `Config` and `Colors` accept (skipped fields left out).
    fn serde_fields<'de, T: Deserialize<'de>>() -> Vec<&'static str> {
        struct Recorder(Vec<&'static str>);
        #[derive(Debug)]
        struct Stop;
        impl std::fmt::Display for Stop {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("stop")
            }
        }
        impl std::error::Error for Stop {}
        impl serde::de::Error for Stop {
            fn custom<M: std::fmt::Display>(_: M) -> Stop {
                Stop
            }
        }
        impl<'de> Deserializer<'de> for &mut Recorder {
            type Error = Stop;
            fn deserialize_any<V: serde::de::Visitor<'de>>(self, _: V) -> Result<V::Value, Stop> {
                Err(Stop)
            }
            fn deserialize_struct<V: serde::de::Visitor<'de>>(
                self,
                _: &'static str,
                fields: &'static [&'static str],
                _: V,
            ) -> Result<V::Value, Stop> {
                self.0.extend_from_slice(fields);
                Err(Stop)
            }
            serde::forward_to_deserialize_any! {
                bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
                bytes byte_buf option unit unit_struct newtype_struct seq tuple
                tuple_struct map enum identifier ignored_any
            }
        }
        let mut r = Recorder(Vec::new());
        let _ = T::deserialize(&mut r);
        r.0
    }

    #[test]
    fn the_schema_has_every_key_of_config() {
        use term_prefs::schema;
        let mut config = serde_fields::<Config>();
        let mut keys = schema::top_level_keys();
        config.sort_unstable();
        keys.sort_unstable();
        assert_eq!(keys, config, "term_prefs::schema::SETTINGS and Config's fields");
        let mut colors = serde_fields::<Colors>();
        let mut keys = schema::colors_keys();
        colors.sort_unstable();
        keys.sort_unstable();
        assert_eq!(keys, colors, "the schema's colors.* and Colors' fields");
    }

    #[test]
    fn each_default_loads_alone_with_no_errors() {
        let defaults = Config::defaults_toml();
        let table: toml::Table = defaults.parse().unwrap();
        // Everything with a default value is there; unset ones aren't.
        for key in [
            "font_family",
            "font_size",
            "cell_width",
            "keybind",
            "colors",
            "option_as_meta",
            "env",
        ] {
            assert!(table.contains_key(key), "{key} missing from:\n{defaults}");
        }
        for key in ["theme", "cwd", "title", "pin_app", "font_bold"] {
            assert!(!table.contains_key(key), "{key} has no default:\n{defaults}");
        }
        assert_eq!(table["cell_width"].as_str(), Some("0"));
        let (all, errors) = parse(&defaults);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(all, Config::default());
        for (key, value) in &table {
            let one = toml::to_string(&toml::Table::from_iter([(key.clone(), value.clone())])).unwrap();
            let (c, errors) = parse(&one);
            assert!(errors.is_empty(), "{one}: {errors:?}");
            assert_eq!(c, Config::default(), "{one}");
        }
        // And the TUI knows each one.
        for key in table.keys() {
            assert!(term_prefs::schema::get(key).is_some() || key == "colors", "{key}");
        }
    }

    #[test]
    fn serialized_settings_read_back() {
        let text = "theme = { light = 'slope-light', dark = 'slope-dark' }\ncell_width = '10%'\n\
                    [keybind]\n'cmd+d' = 'none'\n'ctrl+t' = { text = 'x' }\n'command+K' = 'new_tab'\n";
        let (c, errors) = parse(text);
        assert!(errors.is_empty(), "{errors:?}");
        let (again, errors) = parse(&toml::to_string(&c).unwrap());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            again,
            Config {
                palettes: c.palettes,
                ..c.clone()
            }
        );
        let t = c.as_table();
        assert_eq!(t["keybind"]["cmd+k"].as_str(), Some("new_tab"));
        assert_eq!(t["cell_width"].as_str(), Some("10%"));
    }

    #[test]
    fn each_new_windows_setting_is_noticed() {
        let old = Config::default();
        for (key, text) in [
            ("command", "command = ['htop']"),
            ("cwd", "cwd = '/tmp'"),
            ("env", "env = { A = 'b' }"),
            ("columns", "columns = 80"),
            ("lines", "lines = 20"),
            ("scrollback", "scrollback = 5"),
            ("blur", "blur = true"),
            ("title", "title = 'x'"),
        ] {
            let (new, errors) = parse(text);
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(new_windows_only(&old, &new), [key], "{text}");
        }
        let mut keys = term_prefs::schema::new_windows_keys();
        keys.sort_unstable();
        assert_eq!(keys, ["blur", "columns", "command", "cwd", "env", "lines", "scrollback", "title"]);
    }

    #[test]
    fn image_settings_default_read_and_report_bad_values() {
        let (c, errors) = parse("");
        assert!(errors.is_empty(), "{errors:?}");
        assert!(c.images && c.image_memory == 0 && c.image_file_transfer);
        let all = "images = false\nimage_memory = 256\nimage_file_transfer = false";
        let (c, errors) = parse(all);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!c.images && c.image_memory == 256 && !c.image_file_transfer);
        for text in [
            "image_memory = -1",
            "image_memory = 99999",
            "image_memory = 'big'",
            "images = 'yes'",
            "image_file_transfer = 1",
        ] {
            let (c, errors) = parse(text);
            assert_eq!(errors.len(), 1, "{text}: {errors:?}");
            assert!(
                errors[0].starts_with("config: image") && errors[0].ends_with("; ignored"),
                "{errors:?}"
            );
            assert_eq!((c.images, c.image_memory, c.image_file_transfer), (true, 0, true), "{text}");
        }
        // All three apply to running windows.
        let (new, _) = parse(all);
        assert!(new_windows_only(&Config::default(), &new).is_empty());
    }

    #[test]
    fn a_layer_can_be_read_from_text() {
        let dir = TempDir::new("cfg-replace");
        let user = dir.join("config.toml");
        std::fs::write(&user, "font_size = 14\n").unwrap();
        let (c, errors) = load_replacing(&[&user], Path::new("/nonexistent"), Some((0, "font_size = 18\n")));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.font_size, 18.0);
        let (c, _) = load_replacing(&[&user], Path::new("/nonexistent"), Some((1, "font_size = 18\n")));
        assert_eq!(c.font_size, 14.0);
    }

    #[test]
    fn opacity_clamps() {
        for (opacity, want) in [(1.0, 255), (0.5, 128), (0.0, 0), (-1.0, 0), (2.0, 255)] {
            let c = Config {
                opacity,
                ..Config::default()
            };
            assert_eq!(c.theme(Appearance::Dark).background_opacity, want, "{opacity}");
        }
    }

    /// Settings' preview: the theme for light and dark alike, `[colors]`
    /// still on top, and nothing else changed.
    #[test]
    fn a_previewed_theme_replaces_the_configs() {
        let none = Path::new("/nonexistent");
        let (c, errors) = parse("theme = { light = 'slope-light', dark = 'slope-dark' }\nfont_size = 15\n[colors]\ncursor = '#ff0000'\n");
        assert!(errors.is_empty(), "{errors:?}");
        let p = c.previewing("rose-pine-dawn", none);
        let dawn = Palette::DEFAULT.with(&themes::load(none, "rose-pine-dawn").unwrap());
        assert_eq!(p.palette(Appearance::Dark).background, dawn.background);
        assert_eq!(p.palette(Appearance::Light), p.palette(Appearance::Dark));
        assert_eq!(p.palette(Appearance::Dark).cursor, 0xff0000, "[colors] stay on top");
        assert_eq!(p.font_size, 15.0);
        let p = c.previewing("", none);
        assert_eq!(p.palette(Appearance::Light).background, Palette::DEFAULT.background);
    }

    #[test]
    fn programs_colours_are_drawn_over_the_theme() {
        let theme = Config::default().theme(Appearance::Dark);
        let mut t = term_core::Term::new(10, 2, 0, term_core::ImagePool::process());
        set_term_colors(&mut t.state, &theme);
        assert!(t.state.take_colors_changed());
        // The theme's palette is what programs read.
        assert_eq!(t.state.palette()[1], rgb_of(theme.palette[1]));
        assert_eq!(t.state.palette()[200], rgb_of(theme.palette[200]));
        assert_eq!(program_theme(&theme, &t.state), None);
        // OSC 11 and OSC 4: drawn with the program's colours, the rest
        // the theme's.
        t.advance(b"\x1b]11;#102030\x1b\\\x1b]4;1;rgb:ff/00/00\x1b\\");
        assert!(t.state.take_colors_changed());
        let drawn = program_theme(&theme, &t.state).expect("program colours");
        assert_eq!(rgb_of(drawn.background), 0x102030);
        assert_eq!(rgb_of(drawn.palette[1]), 0xff0000);
        assert_eq!(drawn.palette[2], theme.palette[2]);
        assert_eq!(drawn.foreground, theme.foreground);
        assert_eq!(drawn.selection, theme.selection);
        assert_ne!(drawn.search_hit, theme.search_hit, "hits follow the program's background");
        // A new theme (a config reload, the appearance switching): the
        // program's colours stay on top of it.
        let mut other = theme.clone();
        other.palette[2] = term_render::rgb(0x00ff00);
        other.set_background(0x000000);
        set_term_colors(&mut t.state, &other);
        assert!(t.state.take_colors_changed());
        let drawn = program_theme(&other, &t.state).expect("program colours");
        assert_eq!(rgb_of(drawn.background), 0x102030);
        assert_eq!(rgb_of(drawn.palette[1]), 0xff0000);
        assert_eq!(rgb_of(drawn.palette[2]), 0x00ff00);
        // Resets go back to the theme.
        t.advance(b"\x1b]111\x1b\\\x1b]104\x1b\\");
        assert!(t.state.take_colors_changed());
        assert_eq!(program_theme(&other, &t.state), None);
    }
}
