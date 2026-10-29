//! What the preferences need from the app, which term-prefs can't depend
//! on: its loader, its defaults, its chord parser, fonts, themes and pins.
//! The app implements [`Host`]; the tests use [`Fake`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// What the app's loader says about a set of layers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checked {
    /// The notices the app would show for this config, as it words them.
    pub notices: Vec<String>,
    /// The resolved default foreground and background (0xRRGGBB), so the
    /// TUI can pick colours that stay readable on them.
    pub foreground: Option<u32>,
    pub background: Option<u32>,
}

/// An installed font family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Font {
    pub family: String,
    /// Core Text's monospace flag, or equal advances for `iMW0.`.
    pub monospace: bool,
}

/// A pin, as the pin module lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub name: String,
    /// Its command, for showing.
    pub command: String,
    /// Its icon, if it has one (a path or a description).
    pub icon: Option<String>,
    /// The pin's own config file.
    pub config: PathBuf,
}

/// A menu action a shortcut can be bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// The name in the config: `new_tab`.
    pub name: String,
    /// The menu item: `New Tab`.
    pub label: String,
}

/// Called by the host when a watched file changes; safe to call from any
/// thread.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The app's side of the preferences.
pub trait Host {
    /// The user's own config.toml, edited when no `--layer` is given.
    fn user_config(&self) -> PathBuf;

    /// Loads `layers` with the app's own loader, layer `edit` replaced by
    /// `text`, and returns its notices: the TUI shows exactly the app's
    /// messages, and refuses a write that would add one.
    fn check(&self, layers: &[PathBuf], edit: usize, text: &str) -> Checked;

    /// The defaults (`Config::default()`) as a TOML document. Settings with
    /// no default value are left out.
    fn defaults(&self) -> String;

    /// The installed font families. Slow (90–280 ms): asked for when the
    /// picker first opens.
    fn fonts(&self) -> Vec<Font>;

    /// The theme names: built-in and in the themes folder.
    fn themes(&self) -> Vec<String>;

    /// A chord in its canonical spelling (`ctrl+alt+shift+cmd+k`), or why
    /// it isn't one.
    fn parse_chord(&self, text: &str) -> Result<String, String>;

    /// The menus' shortcuts with no config: canonical chord and action
    /// name.
    fn default_keybinds(&self) -> Vec<(String, String)>;

    /// The actions a chord can be bound to.
    fn actions(&self) -> Vec<Action>;

    /// The menus' fixed shortcuts (Quit, Hide, Minimize, Cmd+1…9): chord
    /// and menu item. Binding one takes it from that item.
    fn fixed_chords(&self) -> Vec<(String, String)>;

    /// The pins.
    fn pins(&self) -> Vec<Pin>;

    /// Removes a pin, or says why not (it is running). The message on
    /// success says what is left behind (its Dock tile).
    fn remove_pin(&self, name: &str) -> Result<String, String>;

    /// Whether `path` is a pin's own config file.
    fn is_pin_file(&self, path: &Path) -> bool;

    /// Watches `files` and calls `wake` when one changes. Replaces any
    /// earlier watch. By default nothing is watched; edits still re-read
    /// the file first.
    fn watch(&self, files: &[PathBuf], wake: Wake) {
        let _ = (files, wake);
    }
}

/// A [`Host`] for tests: built-in defaults, two fonts, a few themes and
/// pins, and a loader that checks types against the schema.
#[derive(Default)]
pub struct Fake {
    pub user_config: PathBuf,
    pub fonts: Vec<Font>,
    pub themes: Vec<String>,
    pub pins: RefCell<Vec<Pin>>,
    /// Pins that are running and can't be removed.
    pub running: Vec<String>,
    /// How many times [`Host::fonts`] was asked.
    pub font_scans: RefCell<usize>,
    /// The last watch set up.
    pub watched: Mutex<Option<(Vec<PathBuf>, Wake)>>,
}

impl Fake {
    pub fn new(user_config: PathBuf) -> Fake {
        Fake {
            user_config,
            fonts: ["Menlo", "Monaco", "SF Mono", "Helvetica", "Apple Color Emoji"]
                .into_iter()
                .map(|f| Font {
                    family: f.into(),
                    monospace: matches!(f, "Menlo" | "Monaco" | "SF Mono"),
                })
                .collect(),
            themes: ["slope-dark", "slope-light", "solarized-dark", "tokyo-night"]
                .map(String::from)
                .to_vec(),
            ..Fake::default()
        }
    }

    /// Calls the watch's `wake`, as a file change would.
    pub fn wake(&self) {
        if let Some((_, wake)) = self.watched.lock().unwrap().as_ref() {
            wake();
        }
    }
}

pub const FAKE_DEFAULTS: &str = r#"font_family = "Menlo"
font_size = 13.0
font_fallback = []
line_height = 1.0
cell_width = "0"
font_features = []
padding = 6.0
scrollback = 10000
option_as_meta = "none"
opacity = 1.0
blur = false
copy_on_select = false
confirm_close = false
columns = 100
lines = 30
command = []

[env]

[colors]
palette = []

[keybind]
"#;

const DEFAULT_KEYS: [(&str, &str); 13] = [
    ("cmd+n", "new_window"),
    ("cmd+t", "new_tab"),
    ("cmd+w", "close"),
    ("cmd+c", "copy"),
    ("cmd+v", "paste"),
    ("cmd+a", "select_all"),
    ("cmd+k", "clear_scrollback"),
    ("cmd+=", "font_bigger"),
    ("cmd+-", "font_smaller"),
    ("cmd+0", "font_reset"),
    ("ctrl+cmd+f", "toggle_full_screen"),
    ("cmd+}", "next_tab"),
    ("cmd+{", "previous_tab"),
];

impl Host for Fake {
    fn user_config(&self) -> PathBuf {
        self.user_config.clone()
    }

    fn check(&self, layers: &[PathBuf], edit: usize, text: &str) -> Checked {
        let mut notices = Vec::new();
        let mut background = None;
        for (i, path) in layers.iter().enumerate() {
            let owned;
            let text = if i == edit {
                text
            } else {
                owned = std::fs::read_to_string(path).unwrap_or_default();
                &owned
            };
            match text.trim_start_matches('\u{feff}').parse::<toml_edit::DocumentMut>() {
                Ok(doc) => {
                    fake_check(doc.as_table(), "", self, &mut notices);
                    if let Some(bg) = doc
                        .get("colors")
                        .and_then(|c| c.get("background"))
                        .and_then(|v| v.as_str())
                        .and_then(crate::schema::parse_hex)
                    {
                        background = Some(bg);
                    }
                }
                Err(e) => notices.push(format!("{}: {e}", path.display())),
            }
        }
        Checked {
            notices,
            foreground: Some(0xd8dee9),
            background: Some(background.unwrap_or(0x16181c)),
        }
    }

    fn defaults(&self) -> String {
        FAKE_DEFAULTS.into()
    }

    fn fonts(&self) -> Vec<Font> {
        *self.font_scans.borrow_mut() += 1;
        self.fonts.clone()
    }

    fn themes(&self) -> Vec<String> {
        self.themes.clone()
    }

    fn parse_chord(&self, text: &str) -> Result<String, String> {
        fake_chord(text)
    }

    fn default_keybinds(&self) -> Vec<(String, String)> {
        DEFAULT_KEYS.iter().map(|(c, a)| (c.to_string(), a.to_string())).collect()
    }

    fn actions(&self) -> Vec<Action> {
        DEFAULT_KEYS
            .iter()
            .map(|(_, a)| Action {
                name: a.to_string(),
                label: title_case(a),
            })
            .chain([Action {
                name: "find".into(),
                label: "Find".into(),
            }])
            .collect()
    }

    fn fixed_chords(&self) -> Vec<(String, String)> {
        vec![
            ("cmd+q".into(), "Quit".into()),
            ("cmd+h".into(), "Hide".into()),
            ("cmd+m".into(), "Minimize".into()),
        ]
    }

    fn pins(&self) -> Vec<Pin> {
        self.pins.borrow().clone()
    }

    fn remove_pin(&self, name: &str) -> Result<String, String> {
        if self.running.iter().any(|r| r == name) {
            return Err(format!("{name} is running: quit it first"));
        }
        self.pins.borrow_mut().retain(|p| p.name != name);
        Ok(format!("removed {name}; its Dock tile stays until you remove it"))
    }

    fn is_pin_file(&self, path: &Path) -> bool {
        self.pins.borrow().iter().any(|p| p.config == path)
    }

    fn watch(&self, files: &[PathBuf], wake: Wake) {
        *self.watched.lock().unwrap() = Some((files.to_vec(), wake));
    }
}

fn title_case(name: &str) -> String {
    name.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The app's chord parser in miniature (`keybind::Chord::parse` and its
/// `Display`), for the fake.
pub fn fake_chord(text: &str) -> Result<String, String> {
    let text = text.trim();
    let (mods, key) = if text == "+" {
        ("", "+")
    } else if let Some(m) = text.strip_suffix("++") {
        (m, "+")
    } else {
        text.rsplit_once('+').unwrap_or(("", text))
    };
    let (mut ctrl, mut alt, mut shift, mut cmd) = (false, false, false, false);
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        match m.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" => cmd = true,
            "ctrl" | "control" => ctrl = true,
            "alt" | "opt" | "option" => alt = true,
            "shift" => shift = true,
            _ => return Err(format!("{text:?}: unknown modifier {m:?} (use cmd, ctrl, alt or shift)")),
        }
    }
    let lower = key.to_lowercase();
    let key = match key.chars().count() {
        1 if key == " " => "space".to_string(),
        1 if key == "+" => "plus".to_string(),
        1 => lower,
        _ if lower == "space" || lower == "plus" => lower,
        _ if [
            "enter",
            "tab",
            "escape",
            "backspace",
            "delete",
            "up",
            "down",
            "left",
            "right",
            "home",
            "end",
        ]
        .contains(&lower.as_str())
            || lower
                .strip_prefix('f')
                .is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=20).contains(&n))) =>
        {
            lower
        }
        _ => return Err(format!("{text:?}: unknown key {key:?}")),
    };
    let mut out = String::new();
    for (on, name) in [(ctrl, "ctrl+"), (alt, "alt+"), (shift, "shift+"), (cmd, "cmd+")] {
        if on {
            out.push_str(name);
        }
    }
    out.push_str(&key);
    Ok(out)
}

/// Type checks against the schema, worded like the app's loader.
fn fake_check(table: &dyn toml_edit::TableLike, prefix: &str, host: &Fake, notices: &mut Vec<String>) {
    use crate::schema::{self, Editor};
    for (key, item) in table.iter() {
        let full = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        if full == "colors" {
            match item.as_table_like() {
                Some(t) => fake_check(t, "colors", host, notices),
                None => notices.push("config: colors: not a table of settings; ignored".into()),
            }
            continue;
        }
        let Some(setting) = schema::get(&full) else {
            notices.push(format!("config: {full}: unknown field `{key}`; ignored"));
            continue;
        };
        let value = item.as_value();
        let bad = |what: &str| format!("config: {full}: {what}; ignored");
        let ok = match setting.editor {
            Editor::Toggle => value.is_some_and(|v| v.is_bool()),
            Editor::Number { integer, .. } => value.is_some_and(|v| v.is_integer() || (!integer && v.is_float())),
            Editor::Choice(choices) => value.and_then(|v| v.as_str()).is_some_and(|s| choices.contains(&s)),
            Editor::Text | Editor::Font => value.is_some_and(|v| v.is_str()),
            Editor::Width => value.is_some_and(|v| v.is_integer() || v.is_float() || v.as_str().is_some_and(schema::valid_width)),
            Editor::Colour => value.and_then(|v| v.as_str()).and_then(schema::parse_hex).is_some(),
            Editor::Palette => value.and_then(|v| v.as_array()).is_some_and(|a| {
                a.len() <= 16
                    && a.iter()
                        .all(|v| v.as_str().is_some_and(|s| s.is_empty() || schema::parse_hex(s).is_some()))
            }),
            Editor::List(_) => value.and_then(|v| v.as_array()).is_some_and(|a| a.iter().all(|v| v.is_str())),
            Editor::Theme => match item.as_table_like() {
                Some(t) => {
                    t.iter().all(|(k, v)| (k == "light" || k == "dark") && v.as_str().is_some())
                        && t.get("light").is_some()
                        && t.get("dark").is_some()
                }
                None => value.and_then(|v| v.as_str()).is_some(),
            },
            Editor::Map => item.as_table_like().is_some_and(|t| t.iter().all(|(_, v)| v.as_str().is_some())),
            Editor::Keybinds => match item.as_table_like() {
                Some(t) => {
                    let mut ok = true;
                    for (chord, v) in t.iter() {
                        if let Err(e) = fake_chord(chord) {
                            notices.push(bad(&e));
                            ok = false;
                        }
                        let action_ok = match v.as_str() {
                            Some(name) => name == "none" || host.actions().iter().any(|a| a.name == name),
                            None => v
                                .as_table_like()
                                .is_some_and(|t| t.len() == 1 && t.get("text").and_then(|t| t.as_str()).is_some()),
                        };
                        if !action_ok {
                            notices.push(bad(&format!("{chord}: not an action")));
                            ok = false;
                        }
                    }
                    if !ok {
                        continue;
                    }
                    true
                }
                None => false,
            },
        };
        if !ok {
            notices.push(bad("not a valid value"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_chords_spell_like_the_app() {
        assert_eq!(fake_chord("command+K").unwrap(), "cmd+k");
        assert_eq!(fake_chord("cmd+shift+ctrl+t").unwrap(), "ctrl+shift+cmd+t");
        assert_eq!(fake_chord("cmd++").unwrap(), "cmd+plus");
        assert_eq!(fake_chord("Option+Enter").unwrap(), "alt+enter");
        assert!(fake_chord("hyper+k").is_err());
        assert!(fake_chord("cmd+banana").is_err());
    }

    #[test]
    fn fake_loader_checks_types() {
        let fake = Fake::new(PathBuf::from("/nonexistent/config.toml"));
        let layers = [fake.user_config()];
        let ok = "font_size = 14\ntheme = { light = 'a', dark = 'b' }\n[colors]\npalette = ['', '#ff0000']\n[keybind]\n'cmd+d' = 'none'\n";
        assert_eq!(fake.check(&layers, 0, ok).notices, Vec::<String>::new());
        let bad = "font_size = 'big'\nfont_sise = 3\n[colors]\nforeground = 'red'\n[keybind]\n'hyper+k' = 'new_tab'\n";
        assert_eq!(fake.check(&layers, 0, bad).notices.len(), 4, "{:?}", fake.check(&layers, 0, bad));
    }
}
