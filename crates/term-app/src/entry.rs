//! Starting Terminal Decline: [`run`] reads the arguments and starts the
//! app, a pin or a subcommand, in one of three roles (docs/design/pins.md
//! §2.5, §3.6).
//!
//! - [`Role::App`]: started by the app's launcher
//!   (`Contents/MacOS/terminal-decline`), which loads this library from
//!   `Contents/Frameworks/libterminal_decline.dylib`.
//! - [`Role::Pin`]: started by a pin's stub, which found the app and
//!   loaded its library. Only `--restore` and `-psn_` are accepted, so
//!   nothing can run another command with the pin's permissions.
//! - [`Role::Bare`]: the static `terminal-decline` binary (`cargo run`,
//!   `cargo install`, tests). It tells a pin from the app by its own file
//!   name, as clone pins (a copy of this binary) need.

use std::path::{Path, PathBuf};

use crate::{Paths, USAGE, app, config, crash, pin};

/// What a pin's stub passed in (`struct td_pin_info_v1`), as Rust values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinInfo {
    /// The pin's id (`TDPinId`): its settings are `pins/<id>.toml`.
    pub id: String,
    /// The pin's `.app`.
    pub bundle: PathBuf,
    /// The Terminal Decline.app the library was loaded from.
    pub app_bundle: PathBuf,
    /// The pin's `CFBundleShortVersionString`.
    pub stub_version: String,
}

/// How this process was started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// The app, through its launcher.
    App,
    /// A pin, through its stub.
    Pin(PinInfo),
    /// The static binary.
    Bare,
}

impl Role {
    /// The Terminal Decline.app this process's code came from, if any.
    /// For a bare binary, only when it is the app's own executable (a
    /// bundle built before the launcher, not a clone pin).
    pub fn app_bundle(&self, exe: &Path) -> Option<PathBuf> {
        match self {
            Role::Pin(info) => Some(info.app_bundle.clone()),
            Role::App => crate::paths::bundle_of(exe),
            Role::Bare => {
                let exe = exe.canonicalize().ok()?;
                let name = exe.file_name()?.to_str()?;
                if pin::OWN_NAMES.contains(&name) {
                    crate::paths::bundle_of(&exe)
                } else {
                    None
                }
            }
        }
    }
}

/// Run Terminal Decline with `args` (without the program name). Returns
/// the exit status; once windows are open it runs until the app quits.
pub fn run(role: Role, args: Vec<String>) -> i32 {
    let paths = Paths::from_env_as(&role);
    crate::paths::set_current(paths.clone());
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    crash::install(crash::logs_dir(&home.unwrap_or_else(std::env::temp_dir)), true);
    crash::set_role(&role);

    // First run after the rename: ~/.config/decline →
    // ~/.config/terminal-decline, before anything reads either.
    let moved = match paths.move_old_config() {
        Ok(done) => done.notice(&paths),
        Err(e) => Some(format!(
            "config: couldn't move the old config to {}: {e}",
            paths.config_dir.display()
        )),
    };

    if let Role::Pin(info) = &role {
        if let Err(e) = pin_arguments(&args) {
            eprintln!("{}: {e}", info.id);
            return 2;
        }
    } else if let Some(code) = subcommand(&args, &paths, moved.as_deref()) {
        return code;
    }

    let mut layers: Vec<PathBuf> = vec![paths.config_file()];
    // Running as a pin: its settings, on top of the user's config, and
    // the file Settings… edits.
    let mut edit = 0;
    let own_pin = match &role {
        Role::Pin(info) => Some(paths.pin_config(&info.id)),
        Role::App => None,
        Role::Bare => pin::own_pin_config(&paths),
    };
    if let Some(p) = own_pin {
        layers.push(p);
        edit = 1;
    }
    let mut command = None;
    let mut restore = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "-V" | "--version" => {
                println!("terminal-decline {}", env!("CARGO_PKG_VERSION"));
                return 0;
            }
            "--config" => {
                i += 1;
                match args.get(i) {
                    Some(p) => layers.push(PathBuf::from(p)),
                    None => return die("--config needs a file"),
                }
            }
            // A dogfood build restarting into a newer one.
            "--restore" => {
                i += 1;
                if let Some(p) = args.get(i) {
                    restore = crate::reload::take(Path::new(p));
                }
            }
            "-e" | "--command" => {
                command = Some(args[i + 1..].to_vec());
                break;
            }
            // Finder passes this when launching an app by double-click.
            a if a.starts_with("-psn_") => {}
            a => return die(&format!("unknown argument {a:?}\n{USAGE}")),
        }
        i += 1;
    }

    let from = config::Source {
        layers,
        themes_dir: paths.themes_dir(),
        command,
        edit,
    };
    let (config, mut errors) = from.load();
    errors.splice(0..0, moved);
    // Clone pins from before stubs: said in the app's first window
    // (pins.md §5.4), never rebuilt without being asked.
    if role == Role::App {
        errors.extend(pin::migration_notice(&paths));
    }
    for e in &errors {
        eprintln!("terminal-decline: {e}");
    }
    let name = app_name(&role, &paths);
    // Shown in each window too: an app started from the Dock has no stderr.
    app::run(config, errors, &name, restore, from, role);
    0
}

/// `pin`, `prefs`, `import` and `ctl`: their exit status, or `None` to start the
/// app.
fn subcommand(args: &[String], paths: &Paths, moved: Option<&str>) -> Option<i32> {
    let first = args.first().map(String::as_str);
    if matches!(first, Some("pin" | "import" | "prefs" | "settings" | "ctl"))
        && let Some(m) = moved
    {
        eprintln!("terminal-decline: {m}");
    }
    match first {
        Some("pin") => Some(match pin::run(&args[1..], paths, &mut pin::system_runner) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("terminal-decline pin: {e}");
                1
            }
        }),
        // Settings… (Cmd+,), or run by hand: the preferences in this terminal,
        // as `prefs` or by its name, `settings`. By hand nothing is sent to
        // the terminal: no app is assumed to be there (`--in-app` says so).
        Some("prefs" | "settings") => Some(crate::prefs::run(&args[1..], paths)),
        // The agent API (docs/design/agent-api.md).
        Some("ctl") => Some(crate::ctl::run(&args[1..])),
        Some("import") => Some(match crate::import::run(&args[1..], paths) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("terminal-decline import: {e}");
                1
            }
        }),
        _ => None,
    }
}

/// In the pin role only `--restore FILE` and Finder's `-psn_…` are
/// accepted: `open -b <pin> --args -e …` must not run another program,
/// or another config, with the pin's permissions (pins.md §1, §3.6).
pub fn pin_arguments(args: &[String]) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--restore" if i + 1 < args.len() => i += 1,
            a if a.starts_with("-psn_") => {}
            a => {
                return Err(format!(
                    "a pin takes no arguments ({a:?} refused); its command and settings are in its pins/<id>.toml"
                ));
            }
        }
        i += 1;
    }
    Ok(())
}

/// The name in the menus: the pin's, or the app bundle's, or the product's.
fn app_name(role: &Role, paths: &Paths) -> String {
    let stem = |b: &Path| b.file_stem().map(|s| s.to_string_lossy().into_owned());
    match role {
        Role::Pin(info) => stem(&info.bundle),
        Role::App => paths.app_bundle.as_deref().and_then(stem),
        Role::Bare => pin::own_app_name(paths),
    }
    .unwrap_or_else(|| crate::PRODUCT_NAME.into())
}

fn die(msg: &str) -> i32 {
    eprintln!("terminal-decline: {msg}");
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pins_accept_only_restore_and_psn() {
        assert!(pin_arguments(&[]).is_ok());
        assert!(pin_arguments(&args(&["-psn_0_12345"])).is_ok());
        assert!(pin_arguments(&args(&["--restore", "/tmp/r.json", "-psn_0_1"])).is_ok());
        for refused in [
            &["-e", "/bin/sh"][..],
            &["--command", "x"],
            &["--config", "/tmp/c.toml"],
            &["pin", "--list"],
            &["prefs"],
            &["settings"],
            &["import", "ghostty"],
            &["-h"],
            &["--restore"],
            &["--restore", "/tmp/r.json", "-e", "sh"],
        ] {
            let e = pin_arguments(&args(refused)).unwrap_err();
            assert!(e.contains("refused"), "{refused:?}: {e}");
        }
    }

    #[test]
    fn settings_is_prefs_by_another_name() {
        let paths = Paths::from_env();
        // Both reach the preferences, which refuse a bad argument (2)
        // before touching the terminal; anything else starts the app.
        for name in ["prefs", "settings"] {
            assert_eq!(subcommand(&args(&[name, "--bogus"]), &paths, None), Some(2), "{name}");
            assert_eq!(subcommand(&args(&[name, "--help"]), &paths, None), Some(2), "{name}");
        }
        assert_eq!(subcommand(&args(&["nonsense"]), &paths, None), None);
        assert_eq!(subcommand(&args(&[]), &paths, None), None);
    }

    #[test]
    fn app_bundle_by_role() {
        let dir = crate::testutil::TempDir::new("role-bundle");
        let app = dir.join("Terminal Decline.app");
        let macos = app.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let launcher = macos.join("terminal-decline");
        std::fs::write(&launcher, "").unwrap();
        let clone = macos.join("rush");
        std::fs::write(&clone, "").unwrap();
        let app = app.canonicalize().unwrap();

        assert_eq!(Role::App.app_bundle(&launcher), Some(app.clone()));
        assert_eq!(Role::Bare.app_bundle(&launcher), Some(app.clone()));
        // A clone pin's binary is not the app.
        assert_eq!(Role::Bare.app_bundle(&clone), None);
        assert_eq!(Role::Bare.app_bundle(&dir.join("terminal-decline")), None);
        let pin = Role::Pin(PinInfo {
            id: "rush".into(),
            bundle: dir.join("Rush.app"),
            app_bundle: PathBuf::from("/Applications/Terminal Decline.app"),
            stub_version: "1".into(),
        });
        assert_eq!(pin.app_bundle(&clone), Some(PathBuf::from("/Applications/Terminal Decline.app")));
    }
}
