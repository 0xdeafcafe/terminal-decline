//! Where Terminal Decline keeps its files. The real values come from the
//! environment in `main`; tests pass temp dirs.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// `~/.config/terminal-decline`: config.toml, and pins/ with each pin's
    /// settings.
    pub config_dir: PathBuf,
    /// Where pins are created by default: `~/Applications`.
    pub apps_dir: PathBuf,
    /// This binary.
    pub exe: PathBuf,
}

/// The config directory's name, under `~/.config`.
const CONFIG_DIR: &str = "terminal-decline";

/// Its name before the rename to Terminal Decline.
const OLD_CONFIG_DIR: &str = "decline";

/// In the new config directory once [`OldConfig::BothExist`] was reported.
const OLD_IGNORED: &str = ".old-config-ignored";

/// What [`Paths::move_old_config`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OldConfig {
    /// Nothing: no old directory, or it was dealt with before.
    None,
    /// Moved to the new name; the old name links to it.
    Moved,
    /// The old directory was a link: the new one links to the same place.
    Linked,
    /// Both exist, so the old one is ignored (reported this once).
    BothExist,
}

impl OldConfig {
    /// What to tell the user, if anything.
    pub fn notice(self, paths: &Paths) -> Option<String> {
        let old = paths.old_config_dir()?;
        let (old, new) = (crate::crash::tilde(&old), crate::crash::tilde(&paths.config_dir));
        match self {
            OldConfig::None => None,
            OldConfig::Moved | OldConfig::Linked => Some(format!("config: moved {old} to {new} (Decline is now Terminal Decline)")),
            OldConfig::BothExist => Some(format!("config: {old} and {new} both exist; using {new}, {old} is ignored")),
        }
    }
}

/// Another process moved or linked first.
fn is_race(e: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        e.kind(),
        ErrorKind::AlreadyExists | ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty | ErrorKind::IsADirectory | ErrorKind::NotADirectory
    )
}

impl Paths {
    /// The real locations: `$XDG_CONFIG_HOME/terminal-decline` (default
    /// `~/.config/terminal-decline`), `~/Applications` and the running
    /// executable.
    pub fn from_env() -> Paths {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let config_base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Paths {
            config_dir: config_base.join(CONFIG_DIR),
            apps_dir: home.join("Applications"),
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("terminal-decline")),
        }
    }

    /// The config directory from before the rename, beside the current one:
    /// `~/.config/decline`.
    pub fn old_config_dir(&self) -> Option<PathBuf> {
        Some(self.config_dir.parent()?.join(OLD_CONFIG_DIR))
    }

    /// The first run after the rename: the old config directory (config.toml,
    /// themes and pins) becomes [`Paths::config_dir`], and the old name a
    /// link to it, so pins made before the rename still find
    /// `~/.config/decline/pins/<id>.toml`. If the old one is itself a link
    /// (dotfiles), the new one links to the same place. If both exist the
    /// new one is used and the two are never merged; that is reported once.
    ///
    /// Safe to run from several processes at once (the app and a pin
    /// starting together): the move is one `rename`, and when another
    /// process got there first it looks again.
    pub fn move_old_config(&self) -> std::io::Result<OldConfig> {
        let Some(old) = self.old_config_dir() else {
            return Ok(OldConfig::None);
        };
        let new = &self.config_dir;
        // The old name links to the new one by its name, beside it.
        let Some(link) = new.file_name() else { return Ok(OldConfig::None) };
        let mut tries = 0;
        loop {
            tries += 1;
            let raced = |e: &std::io::Error| tries < 5 && is_race(e);
            let Ok(old_meta) = old.symlink_metadata() else {
                return Ok(OldConfig::None);
            };
            // Done before: the old name is our link, or both lead to the
            // same place.
            if old_meta.is_symlink() && std::fs::read_link(&old)? == Path::new(link) {
                return Ok(OldConfig::None);
            }
            if let (Ok(a), Ok(b)) = (old.canonicalize(), new.canonicalize())
                && a == b
            {
                return Ok(OldConfig::None);
            }
            if new.symlink_metadata().is_ok() {
                // Seen just before another process moved it: look again.
                use std::os::unix::fs::MetadataExt;
                let unchanged = old.symlink_metadata().is_ok_and(|m| m.ino() == old_meta.ino());
                if !unchanged && tries < 5 {
                    continue;
                }
                return self.both_exist();
            }
            if let Some(base) = new.parent() {
                std::fs::create_dir_all(base)?;
            }
            if old_meta.is_symlink() {
                match std::os::unix::fs::symlink(std::fs::read_link(&old)?, new) {
                    Ok(()) => return Ok(OldConfig::Linked),
                    Err(e) if raced(&e) => continue,
                    Err(e) => return Err(e),
                }
            }
            if !old_meta.is_dir() {
                return Ok(OldConfig::None);
            }
            match std::fs::rename(&old, new) {
                Ok(()) => {}
                Err(e) if raced(&e) => continue,
                Err(e) => return Err(e),
            }
            return match std::os::unix::fs::symlink(link, &old) {
                Ok(()) => Ok(OldConfig::Moved),
                // Something else took the old name meanwhile: the move stands.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(OldConfig::Moved),
                Err(e) => Err(e),
            };
        }
    }

    /// Both directories exist: the new one is used. Said the first time
    /// only, remembered by a file in the new one.
    fn both_exist(&self) -> std::io::Result<OldConfig> {
        let said = self.config_dir.join(OLD_IGNORED);
        if said.exists() || !self.config_dir.is_dir() {
            return Ok(OldConfig::None);
        }
        std::fs::write(&said, "")?;
        Ok(OldConfig::BothExist)
    }

    /// The user's config file.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// The user's own themes: `<name>.toml`.
    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }

    /// Each pin's settings: `<id>.toml`.
    pub fn pins_dir(&self) -> PathBuf {
        self.config_dir.join("pins")
    }

    pub fn pin_config(&self, id: &str) -> PathBuf {
        self.pins_dir().join(format!("{id}.toml"))
    }

    /// The `Contents` directory of the app bundle the executable is in, if
    /// it is in one.
    pub fn contents_dir(&self) -> Option<PathBuf> {
        let exe = self.exe.canonicalize().ok()?;
        contents_dir_of(&exe)
    }
}

fn contents_dir_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents").then(|| contents.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn paths(dir: &TempDir) -> Paths {
        Paths {
            config_dir: dir.join("config/terminal-decline"),
            apps_dir: dir.join("Applications"),
            exe: dir.join("terminal-decline"),
        }
    }

    fn read(p: impl AsRef<Path>) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    /// An old config dir with a config, a theme and a pin's settings.
    fn old_config(dir: &TempDir) -> PathBuf {
        let old = dir.join("config/decline");
        std::fs::create_dir_all(old.join("themes")).unwrap();
        std::fs::create_dir_all(old.join("pins")).unwrap();
        std::fs::write(old.join("config.toml"), "font_size = 15\n").unwrap();
        std::fs::write(old.join("themes/mine.toml"), "").unwrap();
        std::fs::write(old.join("pins/rush.toml"), "command = ['rush']\n").unwrap();
        old
    }

    #[test]
    fn old_only_moves_and_the_old_name_links_to_it() {
        let dir = TempDir::new("move-config");
        let p = paths(&dir);
        let old = old_config(&dir);
        assert_eq!(p.old_config_dir(), Some(old.clone()));

        assert_eq!(p.move_old_config().unwrap(), OldConfig::Moved);
        assert!(!p.config_dir.is_symlink() && p.config_dir.is_dir());
        assert_eq!(read(p.config_file()), "font_size = 15\n");
        assert!(p.themes_dir().join("mine.toml").is_file());
        assert!(p.pin_config("rush").is_file());
        assert_eq!(std::fs::read_link(&old).unwrap(), Path::new("terminal-decline"));
        // Pins made before the rename still find their settings.
        assert_eq!(read(old.join("pins/rush.toml")), "command = ['rush']\n");
        let notice = OldConfig::Moved.notice(&p).unwrap();
        assert!(
            notice.starts_with("config: moved ") && notice.contains("terminal-decline"),
            "{notice}"
        );

        // Done: later runs change nothing, even with the new one gone.
        assert_eq!(p.move_old_config().unwrap(), OldConfig::None);
        std::fs::remove_dir_all(&p.config_dir).unwrap();
        assert_eq!(p.move_old_config().unwrap(), OldConfig::None);
        assert!(p.config_dir.symlink_metadata().is_err());
        assert_eq!(std::fs::read_link(&old).unwrap(), Path::new("terminal-decline"));
    }

    #[test]
    fn new_only_or_neither_is_left_alone() {
        let dir = TempDir::new("new-config");
        let p = paths(&dir);
        std::fs::create_dir_all(&p.config_dir).unwrap();
        std::fs::write(p.config_file(), "new\n").unwrap();
        assert_eq!(p.move_old_config().unwrap(), OldConfig::None);
        assert_eq!(read(p.config_file()), "new\n");
        assert!(dir.join("config/decline").symlink_metadata().is_err());
        assert_eq!(OldConfig::None.notice(&p), None);

        let dir = TempDir::new("no-config");
        let p = paths(&dir);
        assert_eq!(p.move_old_config().unwrap(), OldConfig::None);
        assert!(!dir.join("config").exists());
    }

    #[test]
    fn both_use_the_new_one_say_so_once_and_never_merge() {
        let dir = TempDir::new("both-config");
        let p = paths(&dir);
        let old = old_config(&dir);
        std::fs::create_dir_all(&p.config_dir).unwrap();
        std::fs::write(p.config_file(), "new\n").unwrap();

        assert_eq!(p.move_old_config().unwrap(), OldConfig::BothExist);
        let notice = OldConfig::BothExist.notice(&p).unwrap();
        assert!(notice.contains("both exist"), "{notice}");
        assert_eq!(p.move_old_config().unwrap(), OldConfig::None, "said once");
        assert_eq!(read(p.config_file()), "new\n");
        assert_eq!(read(old.join("config.toml")), "font_size = 15\n");
        assert!(!p.themes_dir().exists() && !p.pins_dir().exists(), "nothing merged");
        assert!(!old.is_symlink());
    }

    /// A config kept elsewhere and linked in (dotfiles): the new name links
    /// to the same place.
    #[test]
    fn old_as_a_link_gives_a_new_link_to_the_same_place() {
        let dir = TempDir::new("link-config");
        let p = paths(&dir);
        let real = dir.join("dotfiles/decline");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("config.toml"), "x = 1\n").unwrap();
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let old = dir.join("config/decline");
        std::os::unix::fs::symlink(&real, &old).unwrap();

        assert_eq!(p.move_old_config().unwrap(), OldConfig::Linked);
        assert_eq!(std::fs::read_link(&p.config_dir).unwrap(), real);
        assert_eq!(std::fs::read_link(&old).unwrap(), real);
        assert_eq!(read(p.config_file()), "x = 1\n");
        // Both lead to the same place now: nothing more to do or say.
        assert_eq!(p.move_old_config().unwrap(), OldConfig::None);
        assert!(!real.join(OLD_IGNORED).exists());
    }

    /// The app and pins starting at once: one moves, the others see it
    /// done, none fails.
    #[test]
    fn racing_processes_move_it_once() {
        for round in 0..20 {
            let dir = TempDir::new(&format!("race-config-{round}"));
            let p = paths(&dir);
            let old = old_config(&dir);
            let barrier = std::sync::Barrier::new(8);
            let results: Vec<OldConfig> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..8)
                    .map(|_| {
                        s.spawn(|| {
                            barrier.wait();
                            p.move_old_config().unwrap()
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            assert_eq!(results.iter().filter(|r| **r == OldConfig::Moved).count(), 1, "{results:?}");
            assert!(
                results.iter().all(|r| matches!(r, OldConfig::Moved | OldConfig::None)),
                "{results:?}"
            );
            assert_eq!(read(p.config_file()), "font_size = 15\n");
            assert_eq!(std::fs::read_link(&old).unwrap(), Path::new("terminal-decline"));
        }
    }
}
