//! Themes: `theme = "name"` in the config, or `theme = { light = "x",
//! dark = "y" }` to follow the macOS appearance.
//!
//! A theme is a TOML file of the same keys as `[colors]` (foreground,
//! background, cursor, selection and the 16-colour palette), read from
//! `~/.config/terminal-decline/themes/<name>.toml`, else one of the built-in
//! set embedded here ([`BUILT_IN`]; each file names its authors and licence).
//! Explicit `[colors]` in the config override the theme's.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::config::Colors;

/// The built-in themes, by name, each family's dark ones first.
pub const BUILT_IN: &[(&str, &str)] = &[
    ("slope-dark", include_str!("../themes/slope-dark.toml")),
    ("slope-light", include_str!("../themes/slope-light.toml")),
    ("solarized-dark", include_str!("../themes/solarized-dark.toml")),
    ("solarized-light", include_str!("../themes/solarized-light.toml")),
    ("gruvbox-dark", include_str!("../themes/gruvbox-dark.toml")),
    ("gruvbox-light", include_str!("../themes/gruvbox-light.toml")),
    ("catppuccin-mocha", include_str!("../themes/catppuccin-mocha.toml")),
    ("catppuccin-macchiato", include_str!("../themes/catppuccin-macchiato.toml")),
    ("catppuccin-frappe", include_str!("../themes/catppuccin-frappe.toml")),
    ("catppuccin-latte", include_str!("../themes/catppuccin-latte.toml")),
    ("tokyo-night", include_str!("../themes/tokyo-night.toml")),
    ("tokyo-night-storm", include_str!("../themes/tokyo-night-storm.toml")),
    ("tokyo-night-day", include_str!("../themes/tokyo-night-day.toml")),
    ("rose-pine", include_str!("../themes/rose-pine.toml")),
    ("rose-pine-moon", include_str!("../themes/rose-pine-moon.toml")),
    ("rose-pine-dawn", include_str!("../themes/rose-pine-dawn.toml")),
    ("kanagawa-wave", include_str!("../themes/kanagawa-wave.toml")),
    ("kanagawa-lotus", include_str!("../themes/kanagawa-lotus.toml")),
    ("everforest-dark", include_str!("../themes/everforest-dark.toml")),
    ("everforest-light", include_str!("../themes/everforest-light.toml")),
    ("one-dark", include_str!("../themes/one-dark.toml")),
    ("one-light", include_str!("../themes/one-light.toml")),
    ("github-dark", include_str!("../themes/github-dark.toml")),
    ("github-light", include_str!("../themes/github-light.toml")),
    ("ayu-dark", include_str!("../themes/ayu-dark.toml")),
    ("ayu-mirage", include_str!("../themes/ayu-mirage.toml")),
    ("ayu-light", include_str!("../themes/ayu-light.toml")),
    ("night-owl", include_str!("../themes/night-owl.toml")),
    ("light-owl", include_str!("../themes/light-owl.toml")),
    ("dracula", include_str!("../themes/dracula.toml")),
    ("nord", include_str!("../themes/nord.toml")),
];

/// The built-in light and dark themes made as a pair: the light one, then
/// the dark one.
pub const PAIRS: &[(&str, &str)] = &[
    ("slope-light", "slope-dark"),
    ("solarized-light", "solarized-dark"),
    ("gruvbox-light", "gruvbox-dark"),
    ("catppuccin-latte", "catppuccin-mocha"),
    ("tokyo-night-day", "tokyo-night"),
    ("rose-pine-dawn", "rose-pine"),
    ("kanagawa-lotus", "kanagawa-wave"),
    ("everforest-light", "everforest-dark"),
    ("one-light", "one-dark"),
    ("github-light", "github-dark"),
    ("ayu-light", "ayu-dark"),
    ("light-owl", "night-owl"),
];

/// The theme's name for people, from its file's first line
/// (`# Rosé Pine Dawn, by …`); else `name` itself.
pub fn title(name: &str, text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let Some(rest) = first.strip_prefix("# ") else {
        return name.to_string();
    };
    let end = rest.find([',', ':']).unwrap_or(rest.len());
    let t = rest[..end].trim();
    if t.is_empty() { name.to_string() } else { t.to_string() }
}

/// The macOS appearance a window has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Appearance {
    Light,
    #[default]
    Dark,
}

/// The `theme` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    One(String),
    ByAppearance { light: String, dark: String },
}

impl ThemeChoice {
    /// The theme's name for a window with `appearance`.
    pub fn pick(&self, appearance: Appearance) -> &str {
        match (self, appearance) {
            (ThemeChoice::One(name), _) => name,
            (ThemeChoice::ByAppearance { light, .. }, Appearance::Light) => light,
            (ThemeChoice::ByAppearance { dark, .. }, Appearance::Dark) => dark,
        }
    }

    /// Every theme named, each once.
    pub fn names(&self) -> Vec<&str> {
        match self {
            ThemeChoice::One(name) => vec![name],
            ThemeChoice::ByAppearance { light, dark } if light == dark => vec![light],
            ThemeChoice::ByAppearance { light, dark } => vec![light, dark],
        }
    }
}

impl<'de> Deserialize<'de> for ThemeChoice {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<ThemeChoice, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Pair {
            light: String,
            dark: String,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            One(String),
            Pair(Pair),
        }
        let check = |name: String| {
            if valid_name(&name) {
                Ok(name)
            } else {
                Err(serde::de::Error::custom(format!("{name:?} is not a theme name")))
            }
        };
        match Raw::deserialize(d) {
            Ok(Raw::One(name)) => Ok(ThemeChoice::One(check(name)?)),
            Ok(Raw::Pair(p)) => Ok(ThemeChoice::ByAppearance {
                light: check(p.light)?,
                dark: check(p.dark)?,
            }),
            Err(_) => Err(serde::de::Error::custom(
                "use a theme's name (\"solarized-dark\") or { light = \"…\", dark = \"…\" }",
            )),
        }
    }
}

impl Serialize for ThemeChoice {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            ThemeChoice::One(name) => s.serialize_str(name),
            ThemeChoice::ByAppearance { light, dark } => {
                let mut m = s.serialize_map(Some(2))?;
                m.serialize_entry("light", light)?;
                m.serialize_entry("dark", dark)?;
                m.end()
            }
        }
    }
}

/// A name that is a file in the themes directory, nothing outside it.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0'])
}

/// Where a theme of the user's own is: `<themes_dir>/<name>.toml`.
pub fn file_of(themes_dir: &Path, name: &str) -> PathBuf {
    themes_dir.join(format!("{name}.toml"))
}

/// The colours of theme `name`: the user's file if there is one, else the
/// built-in theme of that name.
pub fn load(themes_dir: &Path, name: &str) -> Result<Colors, String> {
    let path = file_of(themes_dir, name);
    match std::fs::read_to_string(&path) {
        Ok(text) => return parse(&text).map_err(|e| format!("theme {name:?}: {}: {e}", path.display())),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("theme {name:?}: {}: {e}", path.display())),
        Err(_) => {}
    }
    match BUILT_IN.iter().find(|(n, _)| *n == name) {
        Some((_, text)) => parse(text).map_err(|e| format!("built-in theme {name:?}: {e}")),
        None => Err(format!(
            "no theme named {name:?}: not in {} nor built in ({})",
            themes_dir.display(),
            BUILT_IN.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// A theme file's colours.
pub fn parse(text: &str) -> Result<Colors, String> {
    toml::from_str::<Colors>(text).map_err(|e| e.message().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn every_built_in_theme_is_complete() {
        for (name, text) in BUILT_IN {
            let c = parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(c.foreground.is_some() && c.background.is_some(), "{name}");
            assert!(c.cursor.is_some() && c.selection.is_some(), "{name}");
            assert_eq!(c.palette.len(), 16, "{name}");
            assert!(text.lines().next().unwrap().starts_with("# "), "{name}: names its source");
            assert!(text.contains("licence") || text.contains("License"), "{name}: states its licence");
            for c in c
                .palette
                .iter()
                .chain([&c.foreground, &c.background, &c.cursor, &c.selection].into_iter().flatten())
            {
                assert!(crate::config::parse_hex(c).is_some(), "{name}: {c:?}");
            }
            assert_ne!(c.foreground, c.background, "{name}");
            assert_ne!(title(name, text), *name, "{name}: has a title");
        }
        let mut names: Vec<_> = BUILT_IN.iter().map(|(n, _)| *n).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), BUILT_IN.len(), "each name once");
        assert!(BUILT_IN.len() >= 30, "{}", BUILT_IN.len());
    }

    /// Each theme loads through [`load`] and is as light or dark as its
    /// name says; each pair is one light and one dark theme.
    #[test]
    fn every_built_in_theme_loads_and_pairs_are_light_and_dark() {
        let dir = TempDir::new("themes-all");
        let luma = |c: &Colors| {
            let rgb = crate::config::parse_hex(c.background.as_deref().unwrap()).unwrap();
            let ch = |s: u32| ((rgb >> s) & 0xff) as f64 / 255.0;
            0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
        };
        let light_words = ["light", "latte", "day", "dawn", "lotus"];
        for (name, _) in BUILT_IN {
            let c = load(dir.path(), name).unwrap_or_else(|e| panic!("{e}"));
            let light = light_words.iter().any(|w| name.contains(w));
            assert_eq!(luma(&c) > 0.5, light, "{name}: background {:?}", c.background);
        }
        for (light, dark) in PAIRS {
            assert!(luma(&load(dir.path(), light).unwrap()) > 0.5, "{light}");
            assert!(luma(&load(dir.path(), dark).unwrap()) < 0.5, "{dark}");
        }
        assert_eq!(
            title("rose-pine-dawn", BUILT_IN.iter().find(|t| t.0 == "rose-pine-dawn").unwrap().1),
            "Rosé Pine Dawn"
        );
        assert_eq!(title("slope-dark", BUILT_IN[0].1), "Slope Dark");
        assert_eq!(title("mine", "foreground = '#000000'"), "mine");
    }

    #[test]
    fn user_files_come_before_built_in_ones() {
        let dir = TempDir::new("themes");
        let built_in = load(dir.path(), "solarized-dark").unwrap();
        assert_eq!(built_in.background.as_deref(), Some("#002b36"));
        std::fs::write(dir.join("solarized-dark.toml"), "background = '#010101'\n").unwrap();
        let mine = load(dir.path(), "solarized-dark").unwrap();
        assert_eq!(mine.background.as_deref(), Some("#010101"));
        assert_eq!(mine.foreground, None, "unset in the file");
        std::fs::write(dir.join("own.toml"), "foreground = '#222222'\npalette = ['#ff0000']\n").unwrap();
        let own = load(dir.path(), "own").unwrap();
        assert_eq!(own.palette, ["#ff0000"]);
    }

    #[test]
    fn bad_or_missing_themes_say_why() {
        let dir = TempDir::new("themes-bad");
        let e = load(dir.path(), "nope").unwrap_err();
        assert!(e.starts_with("no theme named \"nope\": not in "), "{e}");
        assert!(e.contains("slope-dark, slope-light"), "{e}");
        std::fs::write(dir.join("typo.toml"), "backgrund = '#000000'\n").unwrap();
        let e = load(dir.path(), "typo").unwrap_err();
        assert!(e.contains("typo.toml") && e.contains("unknown field `backgrund`"), "{e}");
        std::fs::write(dir.join("badhex.toml"), "background = 'black'\n").unwrap();
        let e = load(dir.path(), "badhex").unwrap_err();
        assert!(e.contains("not a colour"), "{e}");
    }

    #[test]
    fn light_and_dark_pick_by_appearance() {
        let pair = ThemeChoice::ByAppearance {
            light: "slope-light".into(),
            dark: "slope-dark".into(),
        };
        assert_eq!(pair.pick(Appearance::Light), "slope-light");
        assert_eq!(pair.pick(Appearance::Dark), "slope-dark");
        assert_eq!(pair.names(), ["slope-light", "slope-dark"]);
        let one = ThemeChoice::One("gruvbox-dark".into());
        assert_eq!(one.pick(Appearance::Light), "gruvbox-dark");
        assert_eq!(one.pick(Appearance::Dark), "gruvbox-dark");
        let same = ThemeChoice::ByAppearance {
            light: "x".into(),
            dark: "x".into(),
        };
        assert_eq!(same.names(), ["x"]);
    }

    #[test]
    fn theme_setting_forms() {
        #[derive(Deserialize, Debug)]
        struct T {
            theme: ThemeChoice,
        }
        let t: T = toml::from_str("theme = 'tokyo-night'").unwrap();
        assert_eq!(t.theme, ThemeChoice::One("tokyo-night".into()));
        let t: T = toml::from_str("theme = { light = 'a', dark = 'b' }").unwrap();
        assert_eq!(
            t.theme,
            ThemeChoice::ByAppearance {
                light: "a".into(),
                dark: "b".into()
            }
        );
        for bad in [
            "theme = 3",
            "theme = { light = 'a' }",
            "theme = '../x'",
            "theme = ''",
            "theme = { light = 'a', dark = 'b', x = 'c' }",
        ] {
            assert!(toml::from_str::<T>(bad).is_err(), "{bad}");
        }
    }
}
