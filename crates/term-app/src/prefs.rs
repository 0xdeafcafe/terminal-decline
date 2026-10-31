//! The app's side of the preferences (`terminal-decline prefs`, the
//! Settings… tab): [`term_prefs::Host`] over the app's own loader, chord
//! parser, fonts, themes and pins, so the TUI says exactly what the app
//! would. See docs/design/preferences.md.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use term_prefs::host::Wake;
use term_prefs::{Action, Checked, Font, Host, Pin, ThemeInfo};

use crate::Paths;
use crate::config::{self, Config};
use crate::keybind::{self, Chord, MenuAction};
use crate::theme::{self, Appearance};

/// The token the Settings tab's requests (`OSC 7771`) must start with,
/// passed to it in [`term_prefs::preview::TOKEN_ENV`]: one for each run
/// of the app, so a program that only writes to the Settings tab's
/// terminal (its editor, a file shown there) can't ask for anything.
pub fn token() -> &'static str {
    use std::hash::BuildHasher;
    static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TOKEN.get_or_init(|| {
        // `RandomState`'s keys come from the system's random source.
        let now = std::time::SystemTime::now();
        let [a, b] = [0u8, 1].map(|i| std::collections::hash_map::RandomState::new().hash_one((i, std::process::id(), now)));
        format!("{a:016x}{b:016x}")
    })
}

/// The real host: the files under `paths`.
pub struct AppHost {
    paths: Paths,
    /// The app's name in the menus (a pin's own name in a pin).
    app_name: String,
    /// The watch the TUI asked for, and whom it wakes.
    watch: Mutex<Option<(crate::watch::Watcher, Arc<Mutex<Wake>>)>>,
}

impl AppHost {
    pub fn new(paths: Paths, app_name: &str) -> AppHost {
        AppHost {
            paths,
            app_name: app_name.into(),
            watch: Mutex::new(None),
        }
    }
}

/// The appearance the system is in, for picking a light/dark theme's
/// colours: windows follow it unless they are given another.
pub(crate) fn system_appearance() -> Appearance {
    let key = objc2_foundation::NSString::from_str("AppleInterfaceStyle");
    let style = objc2_foundation::NSUserDefaults::standardUserDefaults().stringForKey(&key);
    match style {
        Some(s) if s.to_string().eq_ignore_ascii_case("dark") => Appearance::Dark,
        _ => Appearance::Light,
    }
}

/// What the app's loader says about `layers` with layer `edit` read from
/// `text`: its notices, and the default colours the window would get with
/// macOS in `appearance` (the `appearance` setting may override it).
pub fn check(layers: &[PathBuf], themes_dir: &Path, edit: usize, text: &str, appearance: Appearance) -> Checked {
    let refs: Vec<&Path> = layers.iter().map(PathBuf::as_path).collect();
    let (config, notices) = config::load_replacing(&refs, themes_dir, Some((edit, text)));
    let palette = config.palette(config.appearance.resolve(appearance));
    Checked {
        notices,
        foreground: Some(palette.foreground),
        background: Some(palette.background),
        palette: palette.ansi.to_vec(),
    }
}

/// The colours windows show while Settings previews theme `name`: the
/// config `check` loads, made into the one windows get
/// (`Config::previewing`, `[colors]` still on top).
pub fn check_preview(layers: &[PathBuf], themes_dir: &Path, edit: usize, text: &str, name: &str, appearance: Appearance) -> Checked {
    let refs: Vec<&Path> = layers.iter().map(PathBuf::as_path).collect();
    let (config, _) = config::load_replacing(&refs, themes_dir, Some((edit, text)));
    let config = config.previewing(name, themes_dir);
    let palette = config.palette(config.appearance.resolve(appearance));
    Checked {
        notices: Vec::new(),
        foreground: Some(palette.foreground),
        background: Some(palette.background),
        palette: palette.ansi.to_vec(),
    }
}

/// Theme `name` as the Settings list shows it: its title, its colours and
/// the theme made as its other half.
pub fn theme_info(themes_dir: &Path, name: &str) -> Option<ThemeInfo> {
    let colors = theme::load(themes_dir, name).ok()?;
    let built_in = theme::BUILT_IN.iter().find(|(n, _)| *n == name);
    let own = std::fs::read_to_string(theme::file_of(themes_dir, name)).ok();
    let text = own.as_deref().or(built_in.map(|(_, t)| *t)).unwrap_or("");
    let defaults = config::Palettes::default().dark;
    let hex = |c: &Option<String>, or: u32| c.as_deref().and_then(config::parse_hex).unwrap_or(or);
    let palette = (0..16)
        .map(|i| match colors.palette.get(i) {
            Some(c) if !c.is_empty() => config::parse_hex(c).unwrap_or(defaults.ansi[i]),
            _ => defaults.ansi[i],
        })
        .collect();
    let partner = theme::PAIRS.iter().find_map(|&(l, d)| match name {
        n if n == l => Some(d.to_string()),
        n if n == d => Some(l.to_string()),
        _ => None,
    });
    Some(ThemeInfo {
        title: theme::title(name, text),
        foreground: hex(&colors.foreground, defaults.foreground),
        background: hex(&colors.background, defaults.background),
        palette,
        partner,
    })
}

/// The theme names: the built-in ones, then the themes folder's.
pub fn themes(themes_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = theme::BUILT_IN.iter().map(|(n, _)| n.to_string()).collect();
    let mut own: Vec<String> = std::fs::read_dir(themes_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter_map(|p| Some(p.file_stem()?.to_str()?.to_string()))
        .filter(|n| !n.starts_with('.') && !names.contains(n))
        .collect();
    own.sort();
    names.extend(own);
    names
}

/// The actions a chord can be bound to, with their menu items.
pub fn actions() -> Vec<Action> {
    MenuAction::ALL
        .iter()
        .map(|a| Action {
            name: a.name().into(),
            label: a.label().into(),
        })
        .collect()
}

/// The menus' shortcuts with no config: canonical chord and action name.
pub fn default_keybinds() -> Vec<(String, String)> {
    keybind::DEFAULTS
        .iter()
        .filter_map(|(c, a)| Some((Chord::parse(c).ok()?.to_string(), a.name().to_string())))
        .collect()
}

/// The installed font families, sorted, with Core Text's monospace flag
/// or equal advances for `iMW0.` (Ayuthaya has those without the flag).
/// Opening each family takes 90–280 ms for all of them.
pub fn fonts() -> Vec<Font> {
    use objc2_core_foundation::{CFString, CGSize};
    use objc2_core_text::{CTFont, CTFontOrientation, CTFontSymbolicTraits};
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return Vec::new();
    };
    let families = objc2_app_kit::NSFontManager::sharedFontManager(mtm).availableFontFamilies();
    let mut out: Vec<Font> = families
        .iter()
        .map(|f| f.to_string())
        .filter(|f| !f.starts_with('.'))
        .map(|family| {
            let font = unsafe { CTFont::with_name(&CFString::from_str(&family), 12.0, std::ptr::null()) };
            let flagged = unsafe { font.symbolic_traits() }.contains(CTFontSymbolicTraits::TraitMonoSpace);
            let monospace = flagged || {
                let mut chars: Vec<u16> = "iMW0.".encode_utf16().collect();
                let mut glyphs = vec![0u16; chars.len()];
                let mut advances = vec![CGSize::new(0.0, 0.0); chars.len()];
                let n = chars.len() as isize;
                let found = unsafe {
                    font.glyphs_for_characters(
                        std::ptr::NonNull::new_unchecked(chars.as_mut_ptr()),
                        std::ptr::NonNull::new_unchecked(glyphs.as_mut_ptr()),
                        n,
                    )
                };
                found && {
                    unsafe {
                        font.advances_for_glyphs(
                            CTFontOrientation::Default,
                            std::ptr::NonNull::new_unchecked(glyphs.as_mut_ptr()),
                            advances.as_mut_ptr(),
                            n,
                        )
                    };
                    advances.iter().all(|a| a.width > 0.0 && (a.width - advances[0].width).abs() < 0.01)
                }
            };
            Font { family, monospace }
        })
        .collect();
    out.sort_by_key(|f| f.family.to_lowercase());
    out.dedup_by(|a, b| a.family == b.family);
    out
}

impl Host for AppHost {
    fn user_config(&self) -> PathBuf {
        self.paths.config_file()
    }

    fn check(&self, layers: &[PathBuf], edit: usize, text: &str) -> Checked {
        check(layers, &self.paths.themes_dir(), edit, text, system_appearance())
    }

    fn check_preview(&self, layers: &[PathBuf], edit: usize, text: &str, name: &str) -> Checked {
        check_preview(layers, &self.paths.themes_dir(), edit, text, name, system_appearance())
    }

    fn defaults(&self) -> String {
        Config::defaults_toml()
    }

    fn fonts(&self) -> Vec<Font> {
        fonts()
    }

    fn themes(&self) -> Vec<String> {
        themes(&self.paths.themes_dir())
    }

    fn theme_info(&self, name: &str) -> Option<ThemeInfo> {
        theme_info(&self.paths.themes_dir(), name)
    }

    fn parse_chord(&self, text: &str) -> Result<String, String> {
        Chord::parse(text).map(|c| c.to_string())
    }

    fn default_keybinds(&self) -> Vec<(String, String)> {
        default_keybinds()
    }

    fn actions(&self) -> Vec<Action> {
        actions()
    }

    fn fixed_chords(&self) -> Vec<(String, String)> {
        keybind::fixed_chords(&self.app_name)
    }

    fn pins(&self) -> Vec<Pin> {
        pins(&self.paths)
    }

    fn remove_pin(&self, name: &str) -> Result<String, String> {
        remove_pin(&self.paths, name, &crate::running::executable_running)
    }

    fn is_pin_file(&self, path: &Path) -> bool {
        is_pin_file(&self.paths, path)
    }

    fn watch(&self, files: &[PathBuf], wake: Wake) {
        let mut watch = self.watch.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((watcher, current)) = watch.as_ref() {
            *current.lock().unwrap_or_else(|e| e.into_inner()) = wake;
            watcher.set_paths(files.to_vec());
            return;
        }
        let current = Arc::new(Mutex::new(wake));
        let on_change = current.clone();
        match crate::watch::Watcher::start(files.to_vec(), move || {
            let wake = on_change.lock().unwrap_or_else(|e| e.into_inner()).clone();
            wake();
        }) {
            Ok(w) => *watch = Some((w, current)),
            Err(e) => eprintln!("terminal-decline prefs: can't watch the config for changes: {e}"),
        }
    }
}

/// The pins, by their app's name.
pub fn pins(paths: &Paths) -> Vec<Pin> {
    crate::pin::pins(paths)
        .into_iter()
        .map(|(id, app)| {
            let config = paths.pin_config(&id);
            let command = std::fs::read_to_string(&config)
                .ok()
                .and_then(|t| t.parse::<toml::Table>().ok())
                .and_then(|t| t.get("command")?.as_array().cloned())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let icon = app.join("Contents/Resources/icon.icns");
            Pin {
                name: app.file_stem().map_or_else(|| id.clone(), |s| s.to_string_lossy().into_owned()),
                command,
                icon: icon.is_file().then(|| icon.display().to_string()),
                config,
            }
        })
        .collect()
}

/// Removes the pin named `name` unless `running` says its executable is
/// running; says that its Dock tile stays.
pub fn remove_pin(paths: &Paths, name: &str, running: &dyn Fn(&Path) -> bool) -> Result<String, String> {
    let id = crate::pin::slug(name);
    let app = crate::pin::pins(paths)
        .into_iter()
        .find(|(i, _)| *i == id)
        .map(|(_, app)| app)
        .ok_or(format!("no pin named {name}"))?;
    if running(&app.join("Contents/MacOS").join(&id)) {
        return Err(format!("{name} is running: quit it first"));
    }
    crate::pin::remove(name, paths)?;
    Ok(format!("removed {name}; its Dock tile stays until you drag it out of the Dock"))
}

/// Whether `path` is a pin's own settings file.
pub fn is_pin_file(paths: &Paths, path: &Path) -> bool {
    let dir = paths.pins_dir();
    let same = |a: &Path, b: &Path| a == b || a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    path.extension().is_some_and(|e| e == "toml") && !crate::pin::record::is_record(path) && path.parent().is_some_and(|p| same(p, &dir))
}

/// `terminal-decline prefs ARGS`: the preferences in this terminal.
pub fn run(args: &[String], paths: &Paths) -> i32 {
    let name = crate::pin::own_app_name(paths).unwrap_or_else(|| crate::PRODUCT_NAME.into());
    term_prefs::run(args, &AppHost::new(paths.clone(), &name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn check_reads_the_edited_layer_from_the_text() {
        let dir = TempDir::new("prefs-check");
        let user = dir.join("config.toml");
        let pin = dir.join("pin.toml");
        std::fs::write(&user, "font_sise = 3\n[colors]\nforeground = '#010203'\n").unwrap();
        let layers = [user.clone(), pin];
        // The pin's text, not its (missing) file; the user's file as it is.
        let c = check(
            &layers,
            &dir.join("themes"),
            1,
            "[colors]\nbackground = '#0a0b0c'\n",
            Appearance::Dark,
        );
        assert_eq!(c.foreground, Some(0x010203));
        assert_eq!(c.background, Some(0x0a0b0c));
        assert_eq!(c.notices.len(), 1, "{:?}", c.notices);
        assert!(c.notices[0].contains("font_sise"), "{:?}", c.notices);
        let bad = check(&layers, &dir.join("themes"), 1, "columns = 'wide'\n", Appearance::Dark);
        assert_eq!(bad.notices.len(), 2, "{:?}", bad.notices);
        let unparsable = check(&layers, &dir.join("themes"), 0, "font_size = \n", Appearance::Dark);
        assert!(
            unparsable.notices[0].starts_with(&user.display().to_string()),
            "{:?}",
            unparsable.notices
        );
    }

    #[test]
    fn themes_are_built_in_then_the_folders() {
        let dir = TempDir::new("prefs-themes");
        std::fs::write(dir.join("zebra.toml"), "").unwrap();
        std::fs::write(dir.join("aardvark.toml"), "").unwrap();
        std::fs::write(dir.join("slope-dark.toml"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        let t = themes(dir.path());
        assert_eq!(t.len(), theme::BUILT_IN.len() + 2, "{t:?}");
        assert_eq!(t[0], "slope-dark");
        assert_eq!(&t[t.len() - 2..], ["aardvark", "zebra"]);
    }

    #[test]
    fn actions_and_defaults_are_the_menus() {
        let a = actions();
        assert_eq!(a.len(), MenuAction::ALL.len());
        assert!(a.iter().any(|a| a.name == "settings" && a.label == "Settings…"));
        assert!(a.iter().any(|a| a.name == "new_tab" && a.label == "New Tab"));
        // The update actions can be bound (Settings lists them), with no
        // chord by default.
        for (name, label) in [
            ("check_for_updates", "Check for Updates…"),
            ("update_to_latest", "Update to Latest"),
            ("restart_to_update", "Restart to Update"),
        ] {
            assert!(a.iter().any(|a| a.name == name && a.label == label), "{name}");
            assert!(!default_keybinds().iter().any(|(_, n)| n == name), "{name} is unbound");
        }
        let d = default_keybinds();
        assert!(d.contains(&("cmd+,".into(), "settings".into())));
        assert!(d.contains(&("shift+cmd+d".into(), "split_down".into())));
        // Canonical spellings, which the TUI finds entries by.
        for (c, _) in &d {
            assert_eq!(Chord::parse(c).unwrap().to_string(), *c);
        }
        let fixed = keybind::fixed_chords("Terminal Decline");
        assert!(fixed.contains(&("cmd+q".into(), "Quit Terminal Decline".into())));
        assert!(fixed.contains(&("cmd+9".into(), "Show Tab 9".into())));
        for (c, _) in &fixed {
            assert_eq!(Chord::parse(c).unwrap().to_string(), *c);
        }
    }

    #[test]
    fn pins_are_listed_and_kept_while_running() {
        let dir = TempDir::new("prefs-pins");
        let paths = Paths {
            config_dir: dir.join("config"),
            apps_dir: dir.join("Applications"),
            exe: dir.join("terminal-decline"),
            app_bundle: None,
        };
        let app = paths.apps_dir.join("Rush.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::write(app.join("Contents/MacOS/rush"), "").unwrap();
        std::fs::write(app.join("Contents/Resources/icon.icns"), "").unwrap();
        std::fs::create_dir_all(paths.pins_dir()).unwrap();
        let cfg = paths.pin_config("rush");
        std::fs::write(&cfg, format!("command = ['/bin/rush', '-v']\npin_app = '{}'\n", app.display())).unwrap();
        let p = pins(&paths);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].name.as_str(), p[0].command.as_str()), ("Rush", "/bin/rush -v"));
        assert!(p[0].icon.as_deref().is_some_and(|i| i.ends_with("icon.icns")));
        assert!(is_pin_file(&paths, &cfg));
        assert!(!is_pin_file(&paths, &paths.config_file()));
        let err = remove_pin(&paths, "Rush", &|exe| exe.ends_with("Rush.app/Contents/MacOS/rush")).unwrap_err();
        assert!(err.contains("running"), "{err}");
        assert!(app.exists() && cfg.exists());
        let ok = remove_pin(&paths, "Rush", &|_| false).unwrap();
        assert!(ok.contains("Dock tile stays"), "{ok}");
        assert!(!app.exists() && !cfg.exists());
        assert!(remove_pin(&paths, "Rush", &|_| false).is_err());
    }

    /// The colours the Settings tab is drawn on follow the `appearance`
    /// setting over macOS's, as the windows do.
    #[test]
    fn check_and_theme_info_follow_the_appearance() {
        let dir = TempDir::new("prefs-appearance");
        let user = dir.join("config.toml");
        let layers = [user];
        let pair = "theme = { light = 'rose-pine-dawn', dark = 'rose-pine' }\n";
        let bg = |text: &str, system| check(&layers, &dir.join("themes"), 0, text, system).background;
        assert_eq!(bg(pair, Appearance::Dark), Some(0x191724));
        assert_eq!(bg(pair, Appearance::Light), Some(0xfaf4ed));
        let light = format!("appearance = 'light'\n{pair}");
        assert_eq!(bg(&light, Appearance::Dark), Some(0xfaf4ed));
        let dark = format!("appearance = 'dark'\n{pair}");
        assert_eq!(bg(&dark, Appearance::Light), Some(0x191724));
        let c = check(&layers, &dir.join("themes"), 0, &dark, Appearance::Light);
        assert_eq!(c.palette.len(), 16);
        assert!(c.notices.is_empty(), "{:?}", c.notices);

        let info = theme_info(&dir.join("themes"), "rose-pine-dawn").unwrap();
        assert_eq!(info.title, "Rosé Pine Dawn");
        assert_eq!(info.background, 0xfaf4ed);
        assert_eq!(info.partner.as_deref(), Some("rose-pine"));
        assert!(!info.is_dark());
        assert_eq!(info.palette.len(), 16);
        assert!(theme_info(&dir.join("themes"), "nope").is_none());
        assert_eq!(theme_info(&dir.join("themes"), "dracula").unwrap().partner, None);
    }

    /// One token for the app's run, hard to guess, and what Settings
    /// signs with it is heard; the same request unsigned isn't.
    #[test]
    fn settings_requests_need_the_token() {
        let t = token();
        assert_eq!(t, token(), "the same for the whole run");
        assert!(t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit()), "{t}");
        let signed = term_prefs::preview::sign(&term_prefs::Preview::Cancel.osc(), Some(t));
        let what = signed.strip_prefix("\x1b]7771;").unwrap().strip_suffix("\x1b\\").unwrap();
        assert_eq!(term_prefs::preview::verify(what, t), Some("theme-preview;cancel"));
        assert_eq!(term_prefs::preview::verify("theme-preview;cancel", t), None);
    }

    /// The Settings tab picks its colours from what windows show while it
    /// previews a theme: `[colors]` stay on top of the previewed theme.
    #[test]
    fn preview_colours_keep_the_colors_overrides() {
        let dir = TempDir::new("prefs-preview");
        let layers = [dir.join("config.toml")];
        let themes = dir.join("themes");
        let plain = check_preview(&layers, &themes, 0, "theme = 'rose-pine'\n", "rose-pine-dawn", Appearance::Dark);
        assert_eq!(plain.background, Some(0xfaf4ed), "the previewed theme, for dark too");
        let black = "theme = 'rose-pine'\n[colors]\nforeground = '#000000'\nbackground = '#000000'\n";
        let c = check_preview(&layers, &themes, 0, black, "rose-pine-dawn", Appearance::Light);
        assert_eq!((c.foreground, c.background), (Some(0), Some(0)));
        assert_eq!(c.palette, plain.palette, "the theme's ANSI colours");
    }
}
