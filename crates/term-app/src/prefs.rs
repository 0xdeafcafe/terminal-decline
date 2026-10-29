//! The app's side of the preferences (`terminal-decline prefs`, the
//! Settings… tab): [`term_prefs::Host`] over the app's own loader, chord
//! parser, fonts, themes and pins, so the TUI says exactly what the app
//! would. See docs/design/preferences.md.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use term_prefs::host::Wake;
use term_prefs::{Action, Checked, Font, Host, Pin};

use crate::Paths;
use crate::config::{self, Config};
use crate::keybind::{self, Chord, MenuAction};
use crate::theme::{self, Appearance};

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
fn system_appearance() -> Appearance {
    let key = objc2_foundation::NSString::from_str("AppleInterfaceStyle");
    let style = objc2_foundation::NSUserDefaults::standardUserDefaults().stringForKey(&key);
    match style {
        Some(s) if s.to_string().eq_ignore_ascii_case("dark") => Appearance::Dark,
        _ => Appearance::Light,
    }
}

/// What the app's loader says about `layers` with layer `edit` read from
/// `text`: its notices, and the default colours the window would get.
pub fn check(layers: &[PathBuf], themes_dir: &Path, edit: usize, text: &str, appearance: Appearance) -> Checked {
    let refs: Vec<&Path> = layers.iter().map(PathBuf::as_path).collect();
    let (config, notices) = config::load_replacing(&refs, themes_dir, Some((edit, text)));
    let palette = config.palette(appearance);
    Checked {
        notices,
        foreground: Some(palette.foreground),
        background: Some(palette.background),
    }
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

    fn defaults(&self) -> String {
        Config::defaults_toml()
    }

    fn fonts(&self) -> Vec<Font> {
        fonts()
    }

    fn themes(&self) -> Vec<String> {
        themes(&self.paths.themes_dir())
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
    path.extension().is_some_and(|e| e == "toml") && path.parent().is_some_and(|p| same(p, &dir))
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
}
