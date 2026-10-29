//! Config file: `~/.config/terminal-decline/config.toml`, with a pin's own
//! config (inside its app bundle) layered on top.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};
use term_font::{CellWidth, FontConfig};
use term_render::Theme;

use crate::keybind::{self, KeybindConfig, Keybinds};
use crate::theme::{self as themes, Appearance, ThemeChoice};

/// Which Option key acts as Meta (sends ESC + key). Off by default, like
/// Terminal.app and Ghostty, so Option still types @ [ ] { } | \\ ~ on
/// European keyboard layouts.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OptionAsMeta {
    Left,
    Right,
    Both,
    #[default]
    None,
}

/// `[colors]`, and a theme file: only what is set. Unset colours (and
/// palette entries left out or `""`) come from the theme, else the
/// defaults ([`Palette::DEFAULT`]).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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
    pub ansi: [u32; 16],
}

impl Palette {
    /// Terminal Decline's colours with no theme and no `[colors]`.
    pub const DEFAULT: Palette = Palette {
        foreground: 0xd8dee9,
        background: 0x16181c,
        cursor: 0xd8dee9,
        selection: 0x2e3a4f,
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
        for (slot, v) in self.ansi.iter_mut().zip(&colors.palette) {
            if let Some(c) = parse_hex(v) {
                *slot = c;
            }
        }
        self
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
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
    #[serde(deserialize_with = "cell_width")]
    pub cell_width: CellWidth,
    /// Space around the text, in points.
    pub padding: f64,
    pub scrollback: usize,
    pub option_as_meta: OptionAsMeta,
    /// 0.0–1.0. Below 1 the desktop shows through.
    pub opacity: f64,
    /// Blur what shows through when `opacity` is below 1.
    pub blur: bool,
    /// Copy a selection to the clipboard as soon as it is made with the
    /// mouse, without Cmd+C.
    pub copy_on_select: bool,
    pub columns: u16,
    pub lines: u16,
    /// Program and arguments to run instead of the login shell.
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    /// Window title for a pin; also shown in the Window menu.
    pub title: Option<String>,
    /// `theme = "name"` or `{ light = "…", dark = "…" }`; see [`crate::theme`].
    pub theme: Option<ThemeChoice>,
    pub colors: Colors,
    /// The theme and `[colors]` resolved, by [`load`].
    #[serde(skip)]
    pub palettes: Palettes,
    /// `[keybind]`: shortcuts remapped, unbound, or sending text; see
    /// [`crate::keybind`].
    pub keybind: KeybindConfig,
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
            padding: 6.0,
            scrollback: 10_000,
            option_as_meta: OptionAsMeta::None,
            opacity: 1.0,
            blur: false,
            copy_on_select: false,
            columns: 100,
            lines: 30,
            command: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            title: None,
            theme: None,
            colors: Colors::default(),
            palettes: Palettes::default(),
            keybind: KeybindConfig::default(),
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

fn read_table(path: &Path) -> Result<toml::Table, String> {
    match std::fs::read_to_string(path) {
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
    let mut errors = Vec::new();
    let mut table = toml::Table::new();
    for &path in layers {
        match read_table(path) {
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
    config.palettes = resolve(config.theme.as_ref(), &config.colors, themes_dir, &mut errors);
    (config, errors)
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
/// DESIGN.md, Config).
pub fn new_windows_only(old: &Config, new: &Config) -> Vec<&'static str> {
    let mut changed = Vec::new();
    let mut check = |name, differs: bool| {
        if differs {
            changed.push(name);
        }
    };
    check("command", old.command != new.command);
    check("cwd", old.cwd != new.cwd);
    check("env", old.env != new.env);
    check("columns", old.columns != new.columns);
    check("lines", old.lines != new.lines);
    // term-core has no way to change a terminal's scrollback limit.
    check("scrollback", old.scrollback != new.scrollback);
    // The blur is a view behind the terminal's, set up with the window.
    check("blur", old.blur != new.blur);
    check("title", old.title != new.title);
    changed
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
            ..FontConfig::new(&self.font_family, size)
        }
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
        assert_eq!(changed, ["command", "columns", "scrollback", "blur"]);
        assert_eq!(
            new_windows_notice(&changed).unwrap(),
            "config: command, columns, scrollback, blur changed: they apply to new windows"
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
    fn opacity_clamps() {
        for (opacity, want) in [(1.0, 255), (0.5, 128), (0.0, 0), (-1.0, 0), (2.0, 255)] {
            let c = Config {
                opacity,
                ..Config::default()
            };
            assert_eq!(c.theme(Appearance::Dark).background_opacity, want, "{opacity}");
        }
    }
}
