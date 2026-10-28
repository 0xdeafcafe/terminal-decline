//! Config file: `~/.config/decline/config.toml`, with a pin's own config
//! (inside its app bundle) layered on top.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use term_render::Theme;

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

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    pub foreground: String,
    pub background: String,
    pub cursor: String,
    pub selection: String,
    /// The 16 ANSI colours.
    pub palette: Vec<String>,
}

impl Default for Colors {
    fn default() -> Colors {
        Colors {
            foreground: "#d8dee9".into(),
            background: "#16181c".into(),
            cursor: "#d8dee9".into(),
            selection: "#2e3a4f".into(),
            palette: [
                "#1d1f21", "#cc6666", "#b5bd68", "#f0c674", "#81a2be", "#b294bb", "#8abeb7", "#c5c8c6", "#666666", "#d54e53", "#b9ca4a",
                "#e7c547", "#7aa6da", "#c397d8", "#70c0b1", "#eaeaea",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font_family: String,
    pub font_size: f64,
    /// Space around the text, in points.
    pub padding: f64,
    pub scrollback: usize,
    pub option_as_meta: OptionAsMeta,
    /// 0.0–1.0. Below 1 the desktop shows through.
    pub opacity: f64,
    /// Blur what shows through when `opacity` is below 1.
    pub blur: bool,
    pub columns: u16,
    pub lines: u16,
    /// Program and arguments to run instead of the login shell.
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    /// Window title for a pin; also shown in the Window menu.
    pub title: Option<String>,
    pub colors: Colors,
    /// Set in a pin's config: the app bundle it belongs to.
    #[allow(dead_code)]
    pub pin_app: Option<String>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            font_family: "Menlo".into(),
            font_size: 13.0,
            padding: 6.0,
            scrollback: 10_000,
            option_as_meta: OptionAsMeta::None,
            opacity: 1.0,
            blur: false,
            columns: 100,
            lines: 30,
            command: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            title: None,
            colors: Colors::default(),
            pin_app: None,
        }
    }
}

pub fn user_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(base.join("decline").join("config.toml"))
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

/// Loads the user config, then each of `layers` on top. Errors are reported
/// and the defaults used, so a typo never stops the terminal from opening.
pub fn load(layers: &[&Path]) -> (Config, Vec<String>) {
    let mut errors = Vec::new();
    let mut table = toml::Table::new();
    for path in user_config_path().iter().map(|p| p.as_path()).chain(layers.iter().copied()) {
        match read_table(path) {
            Ok(t) => merge(&mut table, t),
            Err(e) => errors.push(e),
        }
    }
    let (table, bad) = keep_valid(table);
    errors.extend(bad);
    let config = toml::Value::Table(table).try_into::<Config>().unwrap_or_else(|e| {
        errors.push(format!("config: {e}"));
        Config::default()
    });
    (config, errors)
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
            // Nested tables (colours) are checked key by key too.
            toml::Value::Table(inner) => {
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
    pub fn theme(&self) -> Theme {
        let d = Colors::default();
        let c = &self.colors;
        let pick = |v: &str, fallback: &str| parse_hex(v).or_else(|| parse_hex(fallback)).unwrap_or(0);
        let mut ansi = [0u32; 16];
        for (i, slot) in ansi.iter_mut().enumerate() {
            *slot = pick(c.palette.get(i).map_or("", String::as_str), &d.palette[i]);
        }
        let mut theme = Theme::new(
            ansi,
            pick(&c.foreground, &d.foreground),
            pick(&c.background, &d.background),
            pick(&c.cursor, &d.cursor),
            pick(&c.selection, &d.selection),
        );
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
        assert_eq!(c.colors.foreground, "#ffffff");
        assert_eq!(c.colors.background, "#111111");
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
        assert_eq!(c.colors.background, "#000000");
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
}
