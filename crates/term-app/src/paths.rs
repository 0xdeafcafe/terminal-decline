//! Where decline keeps its files. The real values come from the environment
//! in `main`; tests pass temp dirs.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// `~/.config/decline`: config.toml, and pins/ with each pin's settings.
    pub config_dir: PathBuf,
    /// Where pins are created by default: `~/Applications`.
    pub apps_dir: PathBuf,
    /// This binary.
    pub exe: PathBuf,
}

impl Paths {
    /// The real locations: `$XDG_CONFIG_HOME/decline` (default
    /// `~/.config/decline`), `~/Applications` and the running executable.
    pub fn from_env() -> Paths {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let config_base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Paths {
            config_dir: config_base.join("decline"),
            apps_dir: home.join("Applications"),
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("decline")),
        }
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
