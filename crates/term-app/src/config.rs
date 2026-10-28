//! Config file: `~/.config/decline/config.toml`, with a pin's own config
//! (inside its app bundle) layered on top.

use std::collections::BTreeMap;
use std::path::Path;

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
/// typo never stops the terminal from opening.
pub fn load(layers: &[&Path]) -> (Config, Vec<String>) {
    let mut errors = Vec::new();
    let mut table = toml::Table::new();
    for &path in layers {
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

    use crate::testutil::TempDir;

    #[test]
    fn missing_files_give_defaults() {
        let dir = TempDir::new("cfg-missing");
        let (c, errors) = load(&[&dir.join("config.toml"), &dir.join("pin.toml")]);
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(c.font_size, Config::default().font_size);
        assert_eq!(c.columns, 100);
        let (c, errors) = load(&[]);
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
        let (c, errors) = load(&[&user, &pin]);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.font_size, 18.0);
        assert_eq!(c.padding, 2.0);
        assert_eq!(c.command, ["htop"]);
        assert_eq!(c.colors.foreground, "#010203");
        assert_eq!(c.colors.background, "#0a0b0c");
    }

    #[test]
    fn unreadable_or_unknown_settings_report_errors() {
        let dir = TempDir::new("cfg-errors");
        let broken = dir.join("broken.toml");
        std::fs::write(&broken, "font_size = = 3").unwrap();
        let typo = dir.join("typo.toml");
        std::fs::write(&typo, "font_sise = 14\nlines = 40\n").unwrap();
        let (c, errors) = load(&[&broken, &typo]);
        assert_eq!(c.lines, 40);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].starts_with(&broken.display().to_string()), "{errors:?}");
        assert!(errors[1].contains("font_sise"), "{errors:?}");
        // A directory can't be read as a file.
        let (_, errors) = load(&[dir.path()]);
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

    /// Bad colours fall back one by one; the query colours (OSC 10/11/12)
    /// come back as 0xRRGGBB.
    #[test]
    fn theme_and_query_colours() {
        let mut c = Config::default();
        c.colors.foreground = "#112233".into();
        c.colors.background = "nonsense".into();
        c.colors.cursor = "#a1b2c3".into();
        c.colors.palette = vec!["#ff0000".into(), "bad".into()];
        let theme = c.theme();
        assert_eq!(query_colors(&theme), [0x112233, 0x16181c, 0xa1b2c3]);
        assert_eq!(rgb_of(theme.palette[0]), 0xff0000);
        assert_eq!(rgb_of(theme.palette[1]), 0xcc6666, "falls back to the default");
        assert_eq!(rgb_of(theme.palette[15]), 0xeaeaea, "missing entries use the defaults");
        assert_eq!(rgb_of(theme.selection), 0x2e3a4f);
        assert_eq!(query_colors(&Config::default().theme()), [0xd8dee9, 0x16181c, 0xd8dee9]);
        for (r, g, b) in [(0x12, 0x34, 0x56), (0xff, 0, 0), (0, 0, 0xff)] {
            assert_eq!(rgb_of(term_render::rgb(r << 16 | g << 8 | b)), r << 16 | g << 8 | b);
        }
    }

    #[test]
    fn opacity_clamps() {
        for (opacity, want) in [(1.0, 255), (0.5, 128), (0.0, 0), (-1.0, 0), (2.0, 255)] {
            let c = Config {
                opacity,
                ..Config::default()
            };
            assert_eq!(c.theme().background_opacity, want, "{opacity}");
        }
    }
}
