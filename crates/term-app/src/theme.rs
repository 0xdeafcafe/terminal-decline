//! Themes: `theme = "name"` in the config, or `theme = { light = "x",
//! dark = "y" }` to follow the macOS appearance.
//!
//! A theme is a TOML file of the same keys as `[colors]` (foreground,
//! background, cursor, selection and the 16-colour palette), read from
//! `~/.config/terminal-decline/themes/<name>.toml`, else one of the built-in
//! set embedded here ([`BUILT_IN`]; each file names its authors and licence).
//! Explicit `[colors]` in the config override the theme's.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

use crate::config::Colors;

/// The built-in themes, by name.
pub const BUILT_IN: [(&str, &str); 8] = [
    ("slope-dark", include_str!("../themes/slope-dark.toml")),
    ("slope-light", include_str!("../themes/slope-light.toml")),
    ("solarized-dark", include_str!("../themes/solarized-dark.toml")),
    ("solarized-light", include_str!("../themes/solarized-light.toml")),
    ("gruvbox-dark", include_str!("../themes/gruvbox-dark.toml")),
    ("catppuccin-mocha", include_str!("../themes/catppuccin-mocha.toml")),
    ("catppuccin-latte", include_str!("../themes/catppuccin-latte.toml")),
    ("tokyo-night", include_str!("../themes/tokyo-night.toml")),
];

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
            BUILT_IN.map(|(n, _)| n).join(", ")
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
        }
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
