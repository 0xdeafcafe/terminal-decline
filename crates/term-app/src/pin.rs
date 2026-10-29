//! Dock pins: make any program its own macOS app.
//!
//! A pin is a small app bundle holding an APFS clone of this binary (no extra
//! disk space), its own bundle id (so macOS gives it its own Dock icon and
//! Cmd+Tab entry) and an optional icon. What it runs, and any config setting
//! it overrides, live in `~/.config/terminal-decline/pins/<id>.toml`, outside
//! the bundle, so editing them doesn't break the bundle's code signature. The
//! binary knows it's a pin from its own file name, which is the pin's id.
//!
//! Every location comes in through [`Paths`] and every other program (cp,
//! xattr, codesign, lsregister, sips, iconutil, defaults, killall) runs
//! through a [`Runner`], so tests use temp dirs and record the commands.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::paths::Paths;

pub const ID_PREFIX: &str = "red.forbes.terminal-decline.pin.";

/// Pins' bundle ids before the rename to Terminal Decline.
pub const OLD_ID_PREFIX: &str = "red.forbes.decline.pin.";

/// The executable's own names: `terminal-decline`, and the `decline` alias
/// the cask links (and builds from before the rename). Never pin ids.
const OWN_NAMES: [&str; 2] = ["terminal-decline", "decline"];

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// Runs a command to completion. The real one is [`system_runner`].
pub type Runner<'a> = dyn FnMut(&mut Command) -> io::Result<ExitStatus> + 'a;

pub fn system_runner(cmd: &mut Command) -> io::Result<ExitStatus> {
    cmd.status()
}

fn succeeds(run: &mut Runner, cmd: &mut Command) -> bool {
    run(cmd).map(|s| s.success()).unwrap_or(false)
}

/// When running as a pin: its config file.
pub fn own_pin_config(paths: &Paths) -> Option<PathBuf> {
    paths.contents_dir()?;
    // Through a link (the cask's `decline`): the name of what it leads to.
    let exe = paths.exe.canonicalize().ok()?;
    let id = exe.file_name()?.to_str()?;
    if OWN_NAMES.contains(&id) {
        return None;
    }
    let p = paths.pin_config(id);
    p.exists().then_some(p)
}

/// The app's display name when running from a bundle.
pub fn own_app_name(paths: &Paths) -> Option<String> {
    let app = paths.contents_dir()?.parent()?.to_path_buf();
    Some(app.file_stem()?.to_string_lossy().into_owned())
}

/// The pin's id: lower-case ASCII letters and digits, other runs of
/// characters as `-`, no `-` at either end.
pub fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    s.trim_matches('-').to_string()
}

/// A pin name becomes `<name>.app` in a directory, so it must not be able to
/// name a path elsewhere.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.contains('/') || name.contains("..") || name.contains('\0') {
        return Err(format!("{name:?}: a pin name can't contain '/' or '..'"));
    }
    if slug(name).is_empty() {
        return Err("give the pin a name with a letter or digit in it, e.g. `terminal-decline pin Rush -- ./rush`".into());
    }
    if OWN_NAMES.contains(&slug(name).as_str()) {
        return Err(format!("{name:?} is Terminal Decline's own name; pick another for the pin"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub name: String,
    pub icon: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub dir: PathBuf,
    pub dock: bool,
    pub command: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Help,
    List,
    Update,
    Remove(String),
    Create(Options),
}

/// `terminal-decline pin …` arguments (after `pin`). `apps_dir` is the
/// default for `--dir`.
pub fn parse_args(args: &[String], apps_dir: &Path) -> Result<Request, String> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => return Ok(Request::Help),
        Some("--list") => return Ok(Request::List),
        Some("--update") => return Ok(Request::Update),
        Some("--remove") => {
            let name = args.get(1).ok_or("--remove needs a name")?;
            if let Some(extra) = args.get(2) {
                return Err(format!("unexpected {extra:?} after --remove {name}"));
            }
            check_name(name)?;
            return Ok(Request::Remove(name.clone()));
        }
        _ => {}
    }
    let mut o = Options {
        name: String::new(),
        icon: None,
        config: None,
        cwd: None,
        dir: apps_dir.to_path_buf(),
        dock: false,
        command: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut value = || {
            i += 1;
            args.get(i).cloned().ok_or(format!("{a} needs a value"))
        };
        match a {
            "--icon" => o.icon = Some(value()?.into()),
            "--config" => o.config = Some(value()?.into()),
            "--cwd" => o.cwd = Some(value()?.into()),
            "--dir" => o.dir = value()?.into(),
            "--dock" => o.dock = true,
            "--" => {
                o.command = args[i + 1..].to_vec();
                break;
            }
            a if a.starts_with('-') => return Err(format!("unknown option {a}")),
            a if o.name.is_empty() => o.name = a.to_string(),
            a => return Err(format!("unexpected {a:?}; put the program after --")),
        }
        i += 1;
    }
    if o.name.is_empty() {
        return Err("give the pin a name, e.g. `terminal-decline pin Rush -- ./rush`".into());
    }
    check_name(&o.name)?;
    if o.command.is_empty() {
        return Err("give the program to run after --".into());
    }
    Ok(Request::Create(o))
}

pub fn run(args: &[String], paths: &Paths, run: &mut Runner) -> Result<(), String> {
    match parse_args(args, &paths.apps_dir)? {
        Request::Help => println!("{}", crate::USAGE),
        Request::List => {
            for (app, cmd) in list(paths) {
                println!("{}\t{cmd}", app.display());
            }
        }
        Request::Update => update(paths, run)?,
        Request::Remove(name) => {
            let app = remove(&name, paths)?;
            println!("removed {}", app.display());
        }
        Request::Create(o) => {
            let made = create(o, paths, run)?;
            println!("created {}", made.app.display());
            println!("settings: {}", made.config.display());
            if made.dock {
                println!("added to the Dock");
            }
        }
    }
    Ok(())
}

/// Whether `app` is a pin bundle Terminal Decline made, so it is safe to
/// delete: a `.app` inside the apps dir with `Contents/MacOS/<id>` in it. The
/// pin's settings file names the bundle and can be edited, so this is checked
/// rather than trusted.
pub fn is_pin_bundle(app: &Path, id: &str, apps_dir: &Path) -> bool {
    let (Ok(app), Ok(apps)) = (app.canonicalize(), apps_dir.canonicalize()) else {
        return false;
    };
    app.extension().is_some_and(|e| e == "app") && app.starts_with(&apps) && app != apps && app.join("Contents/MacOS").join(id).is_file()
}

/// Delete a pin's bundle and settings. Deletes nothing unless the bundle
/// passes [`is_pin_bundle`].
pub fn remove(name: &str, paths: &Paths) -> Result<PathBuf, String> {
    check_name(name)?;
    let id = slug(name);
    let app = pins(paths)
        .into_iter()
        .find(|(i, _)| *i == id)
        .map(|(_, app)| app)
        .ok_or(format!("no pin named {name}"))?;
    if !is_pin_bundle(&app, &id, &paths.apps_dir) {
        return Err(format!(
            "{} is not a Terminal Decline pin in {}; nothing removed (delete it and {} by hand)",
            app.display(),
            paths.apps_dir.display(),
            paths.pin_config(&id).display()
        ));
    }
    let real = app.canonicalize().map_err(|e| e.to_string())?;
    std::fs::remove_dir_all(&real).map_err(|e| format!("{}: {e}", real.display()))?;
    let _ = std::fs::remove_file(paths.pin_config(&id));
    Ok(app)
}

/// Absolute path for a program given as a path (./rush); bare names (htop)
/// are left for PATH lookup at launch.
fn resolve_program(p: &str) -> Result<String, String> {
    if p.contains('/') {
        let abs = std::fs::canonicalize(p).map_err(|e| format!("{p}: {e}"))?;
        return Ok(abs.to_string_lossy().into_owned());
    }
    Ok(p.to_string())
}

/// The pin's settings: the given config file, plus what to run (`command`
/// absolute for a path), `title`, `pin_app` and a `cwd` defaulting to the
/// program's directory.
pub fn pin_table(o: &Options, app: &Path) -> Result<toml::Table, String> {
    let mut command = o.command.clone();
    command[0] = resolve_program(&command[0])?;
    let mut table = match &o.config {
        Some(p) => std::fs::read_to_string(p)
            .map_err(|e| format!("{}: {e}", p.display()))?
            .parse::<toml::Table>()
            .map_err(|e| format!("{}: {e}", p.display()))?,
        None => toml::Table::new(),
    };
    let cwd = match &o.cwd {
        Some(c) => Some(std::fs::canonicalize(c).map_err(|e| format!("{}: {e}", c.display()))?),
        // A program given by path usually expects to run from where it lives.
        None if command[0].starts_with('/') => Path::new(&command[0]).parent().map(Path::to_path_buf),
        None => None,
    };
    table.insert(
        "command".into(),
        toml::Value::Array(command.into_iter().map(toml::Value::String).collect()),
    );
    table.entry("title").or_insert_with(|| toml::Value::String(o.name.clone()));
    table.insert("pin_app".into(), toml::Value::String(app.to_string_lossy().into_owned()));
    if let Some(cwd) = cwd {
        table
            .entry("cwd")
            .or_insert_with(|| toml::Value::String(cwd.to_string_lossy().into_owned()));
    }
    Ok(table)
}

/// What [`create`] made.
#[derive(Debug)]
pub struct Created {
    pub app: PathBuf,
    pub config: PathBuf,
    pub dock: bool,
}

pub fn create(o: Options, paths: &Paths, run: &mut Runner) -> Result<Created, String> {
    check_name(&o.name)?;
    let slug = slug(&o.name);
    let app = o.dir.join(format!("{}.app", o.name));
    let config_path = paths.pin_config(&slug);
    // Two pins with the same id would share a Dock identity.
    if let Some((_, other)) = pins(paths).into_iter().find(|(id, other)| *id == slug && *other != app) {
        return Err(format!(
            "{} already uses the id {slug:?}; pick another name or remove it first",
            other.display()
        ));
    }
    let table = pin_table(&o, &app)?;
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos).map_err(|e| format!("{}: {e}", macos.display()))?;
    std::fs::create_dir_all(&resources).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(paths.pins_dir()).map_err(|e| e.to_string())?;

    let pin_toml = format!(
        "# Made by `terminal-decline pin`. Any config.toml setting can go here.\n{}",
        toml::to_string_pretty(&table).map_err(|e| e.to_string())?
    );
    std::fs::write(&config_path, pin_toml).map_err(|e| e.to_string())?;

    let has_icon = match &o.icon {
        Some(icon) => {
            make_icns(icon, &resources.join("icon.icns"), run)?;
            true
        }
        None => false,
    };
    std::fs::write(contents.join("Info.plist"), info_plist(&o.name, &slug, has_icon)).map_err(|e| e.to_string())?;
    install_binary(&paths.exe, &macos.join(&slug), run)?;
    finish_bundle(&app, &slug, run)?;
    if o.dock {
        add_to_dock(&app, run)?;
    }
    Ok(Created {
        app,
        config: config_path,
        dock: o.dock,
    })
}

/// Clone (APFS copy-on-write) `exe` into the bundle. Written to a new file
/// and renamed into place: overwriting a signed binary that may be running
/// gets it killed on Apple Silicon.
fn install_binary(exe: &Path, dest: &Path, run: &mut Runner) -> Result<(), String> {
    let exe = exe.canonicalize().map_err(|e| format!("{}: {e}", exe.display()))?;
    let tmp = dest.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    let cloned = succeeds(run, Command::new("/bin/cp").arg("-c").arg(&exe).arg(&tmp)) && tmp.is_file();
    if !cloned {
        // Different volume (no clone possible): plain copy.
        std::fs::copy(&exe, &tmp).map_err(|e| format!("copying binary: {e}"))?;
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("installing binary: {e}"))?;
    Ok(())
}

/// Drop the download quarantine copied from Terminal Decline itself, sign the
/// bundle with its own identifier (sealing its Info.plist), and register it
/// with LaunchServices. Signing must work; registering is only worth a
/// warning.
fn finish_bundle(app: &Path, id: &str, run: &mut Runner) -> Result<(), String> {
    let _ = run(Command::new("/usr/bin/xattr").args(["-dr", "com.apple.quarantine"]).arg(app));
    let identifier = format!("{ID_PREFIX}{id}");
    let signed = succeeds(
        run,
        Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--identifier", &identifier])
            .arg(app)
            .stderr(Stdio::null()),
    );
    if !signed {
        return Err(format!("codesign failed for {}", app.display()));
    }
    if !succeeds(run, Command::new(LSREGISTER).arg("-f").arg(app)) {
        eprintln!("warning: LaunchServices registration failed; the Dock may not pick up the icon until you open the app once");
    }
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn info_plist(name: &str, slug: &str, has_icon: bool) -> String {
    let name = xml_escape(name);
    let icon = if has_icon {
        "\t<key>CFBundleIconFile</key>\n\t<string>icon</string>\n"
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>{slug}</string>
	<key>CFBundleIdentifier</key>
	<string>{ID_PREFIX}{slug}</string>
	<key>CFBundleName</key>
	<string>{name}</string>
	<key>CFBundleDisplayName</key>
	<string>{name}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
{icon}	<key>LSMinimumSystemVersion</key>
	<string>14.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
</dict>
</plist>
"#,
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// Any image macOS can read → .icns, via `sips` and `iconutil`.
fn make_icns(src: &Path, dest: &Path, run: &mut Runner) -> Result<(), String> {
    if src.extension().is_some_and(|e| e == "icns") {
        std::fs::copy(src, dest).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let set = dest.with_extension("iconset");
    std::fs::create_dir_all(&set).map_err(|e| e.to_string())?;
    for size in [16, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let px = (size * scale).to_string();
            let out = set.join(format!("icon_{size}x{size}{suffix}.png"));
            let ok = succeeds(
                run,
                Command::new("/usr/bin/sips")
                    .args(["-s", "format", "png", "-z", &px, &px])
                    .arg(src)
                    .arg("--out")
                    .arg(&out)
                    .stdout(Stdio::null()),
            );
            if !ok {
                let _ = std::fs::remove_dir_all(&set);
                return Err(format!("could not read image {}", src.display()));
            }
        }
    }
    let ok = succeeds(
        run,
        Command::new("/usr/bin/iconutil").args(["-c", "icns"]).arg(&set).arg("-o").arg(dest),
    );
    let _ = std::fs::remove_dir_all(&set);
    if ok { Ok(()) } else { Err("iconutil failed".into()) }
}

/// A `file://` URL for a directory, percent-encoding everything but
/// unreserved characters and `/`.
pub fn file_url(dir: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut url = String::from("file://");
    for &b in dir.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    if !url.ends_with('/') {
        url.push('/');
    }
    url
}

/// The Dock's `persistent-apps` entry for `app`.
pub fn dock_tile(app: &Path) -> String {
    format!(
        "<dict><key>tile-data</key><dict><key>file-data</key><dict><key>_CFURLString</key><string>{}</string><key>_CFURLStringType</key><integer>15</integer></dict></dict></dict>",
        xml_escape(&file_url(app))
    )
}

fn add_to_dock(app: &Path, run: &mut Runner) -> Result<(), String> {
    let tile = dock_tile(app);
    if !succeeds(
        run,
        Command::new("/usr/bin/defaults").args(["write", "com.apple.dock", "persistent-apps", "-array-add", &tile]),
    ) {
        return Err("could not update the Dock settings".into());
    }
    let _ = run(Command::new("/usr/bin/killall").arg("Dock"));
    Ok(())
}

/// Every pin: (id, app path), from the pins' settings files, sorted.
pub fn pins(paths: &Paths) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(paths.pins_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter_map(|p| {
            let id = p.file_stem()?.to_str()?.to_string();
            let t = std::fs::read_to_string(&p).ok()?.parse::<toml::Table>().ok()?;
            Some((id, PathBuf::from(t.get("pin_app")?.as_str()?)))
        })
        .collect();
    out.sort();
    out
}

/// Every pin's app and command, for `--list`.
pub fn list(paths: &Paths) -> Vec<(PathBuf, String)> {
    pins(paths)
        .into_iter()
        .map(|(id, app)| {
            let cmd = std::fs::read_to_string(paths.pin_config(&id))
                .ok()
                .and_then(|c| c.parse::<toml::Table>().ok())
                .and_then(|t| t.get("command").cloned())
                .map(|c| c.to_string())
                .unwrap_or_default();
            (app, cmd)
        })
        .collect()
}

/// After upgrading Terminal Decline, refresh the binary inside every pin,
/// and its Info.plist, so pins made before the rename (bundle ids
/// `red.forbes.decline.pin.*`) get the current [`ID_PREFIX`].
pub fn update(paths: &Paths, run: &mut Runner) -> Result<(), String> {
    for (id, app) in pins(paths) {
        let bin = app.join("Contents/MacOS").join(&id);
        if !bin.is_file() {
            eprintln!("skipping {}: not found", app.display());
            continue;
        }
        // The bundle is `<name>.app`; its icon, if any, stays as it is.
        let name = app.file_stem().map_or_else(|| id.clone(), |s| s.to_string_lossy().into_owned());
        let has_icon = app.join("Contents/Resources/icon.icns").is_file();
        let plist = app.join("Contents/Info.plist");
        let renamed = std::fs::read_to_string(&plist).is_ok_and(|p| p.contains(OLD_ID_PREFIX));
        std::fs::write(&plist, info_plist(&name, &id, has_icon)).map_err(|e| format!("{}: {e}", plist.display()))?;
        install_binary(&paths.exe, &bin, run)?;
        finish_bundle(&app, &id, run)?;
        if renamed {
            // Its Secure Keyboard Entry choice, kept under the old id.
            crate::secure_input::carry_over(&format!("{ID_PREFIX}{id}"));
        }
        println!("updated {}", app.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::process::ExitStatusExt;

    use super::*;
    use crate::testutil::TempDir;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// A temp home with a config dir, an apps dir and a small fake binary.
    struct Env {
        dir: TempDir,
        paths: Paths,
    }

    impl Env {
        fn new(name: &str) -> Env {
            let dir = TempDir::new(name);
            let apps = dir.join("Applications");
            std::fs::create_dir_all(&apps).unwrap();
            let exe = dir.join("terminal-decline");
            std::fs::write(&exe, b"not really a binary").unwrap();
            let paths = Paths {
                config_dir: dir.join("config/terminal-decline"),
                apps_dir: apps,
                exe,
            };
            Env { dir, paths }
        }

        fn options(&self, name: &str, command: &[&str]) -> Options {
            Options {
                name: name.into(),
                icon: None,
                config: None,
                cwd: None,
                dir: self.paths.apps_dir.clone(),
                dock: false,
                command: s(command),
            }
        }
    }

    /// Records every command; exit codes by program file name (default 0).
    /// `cp` fails so the binary is copied with std::fs instead.
    struct Recorder {
        calls: Vec<Vec<String>>,
        codes: HashMap<&'static str, i32>,
    }

    impl Recorder {
        fn new() -> Recorder {
            Recorder {
                calls: Vec::new(),
                codes: HashMap::from([("cp", 1)]),
            }
        }

        fn failing(mut self, program: &'static str) -> Recorder {
            self.codes.insert(program, 1);
            self
        }

        fn runner(&mut self) -> impl FnMut(&mut Command) -> io::Result<ExitStatus> + '_ {
            move |cmd: &mut Command| {
                let program = Path::new(cmd.get_program()).file_name().unwrap().to_string_lossy().into_owned();
                let mut call = vec![program.clone()];
                call.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
                self.calls.push(call);
                let code = self.codes.get(program.as_str()).copied().unwrap_or(0);
                Ok(ExitStatus::from_raw(code << 8))
            }
        }

        fn programs(&self) -> Vec<&str> {
            self.calls.iter().map(|c| c[0].as_str()).collect()
        }
    }

    #[test]
    fn slugs() {
        for (name, want) in [
            ("Rush", "rush"),
            ("My Tool!", "my-tool"),
            ("  spaced  ", "spaced"),
            ("a.b_c", "a-b-c"),
            ("Ünïcode", "n-code"),
            ("123", "123"),
            ("!!!", ""),
        ] {
            assert_eq!(slug(name), want, "{name:?}");
        }
    }

    #[test]
    fn names_that_name_other_paths_are_rejected() {
        for bad in ["../evil", "a/b", "/abs", "..", "x..y", "!!!", "", "Terminal Decline", "decline"] {
            assert!(check_name(bad).is_err(), "{bad:?} accepted");
        }
        for good in ["Rush", "My Tool", "a.b", "émoji ✨"] {
            assert_eq!(check_name(good), Ok(()), "{good:?}");
        }
    }

    #[test]
    fn argument_parsing() {
        let apps = Path::new("/apps");
        let err = |args: &[&str]| parse_args(&s(args), apps).unwrap_err();
        assert!(err(&["--remove"]).contains("needs a name"));
        assert!(err(&["--remove", "a", "b"]).contains("unexpected"));
        assert!(err(&["--remove", "../x"]).contains("'/'"));
        assert!(err(&["Rush"]).contains("after --"));
        assert!(err(&["Rush", "--"]).contains("after --"));
        assert!(err(&["--", "./rush"]).contains("name"));
        assert!(err(&["--bogus", "Rush", "--", "x"]).contains("unknown option --bogus"));
        assert!(err(&["Rush", "extra", "--", "x"]).contains("put the program after --"));
        assert!(err(&["Rush", "--icon"]).contains("--icon needs a value"));
        assert!(err(&["../Rush", "--", "x"]).contains("'/'"));
        assert!(err(&["a/b", "--", "x"]).contains("'/'"));
        assert!(err(&["!!!", "--", "x"]).contains("letter or digit"));

        assert_eq!(parse_args(&[], apps), Ok(Request::Help));
        assert_eq!(parse_args(&s(&["--help"]), apps), Ok(Request::Help));
        assert_eq!(parse_args(&s(&["--list"]), apps), Ok(Request::List));
        assert_eq!(parse_args(&s(&["--update"]), apps), Ok(Request::Update));
        assert_eq!(parse_args(&s(&["--remove", "Rush"]), apps), Ok(Request::Remove("Rush".into())));
        assert_eq!(
            parse_args(
                &s(&[
                    "My Tool", "--dock", "--icon", "i.png", "--config", "c.toml", "--cwd", "/w", "--dir", "/d", "--", "./rush", "--x", "--"
                ]),
                apps
            ),
            Ok(Request::Create(Options {
                name: "My Tool".into(),
                icon: Some("i.png".into()),
                config: Some("c.toml".into()),
                cwd: Some("/w".into()),
                dir: "/d".into(),
                dock: true,
                command: s(&["./rush", "--x", "--"]),
            }))
        );
        // --dir defaults to the apps dir.
        match parse_args(&s(&["Rush", "--", "rush"]), apps) {
            Ok(Request::Create(o)) => assert_eq!(o.dir, apps),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pin_config_content() {
        let env = Env::new("table");
        let app = env.paths.apps_dir.join("Rush.app");
        let here = std::env::current_dir().unwrap().canonicalize().unwrap();

        // A relative path becomes absolute; cwd defaults to its directory.
        let t = pin_table(&env.options("Rush", &["./Cargo.toml", "-v"]), &app).unwrap();
        let cmd: Vec<&str> = t["command"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(cmd, [here.join("Cargo.toml").to_str().unwrap(), "-v"]);
        assert_eq!(t["cwd"].as_str(), here.to_str());
        assert_eq!(t["title"].as_str(), Some("Rush"));
        assert_eq!(t["pin_app"].as_str(), app.to_str());

        // A bare name is left for PATH lookup, with no cwd.
        let t = pin_table(&env.options("Top", &["htop"]), &app).unwrap();
        assert_eq!(t["command"].as_array().unwrap()[0].as_str(), Some("htop"));
        assert!(!t.contains_key("cwd"));

        // A missing program is an error.
        assert!(pin_table(&env.options("X", &["./no-such-program"]), &app).is_err());

        // --cwd wins; settings from --config are kept, and its title too.
        let cfg = env.dir.join("extra.toml");
        std::fs::write(&cfg, "title = 'Custom'\nfont_size = 15\n").unwrap();
        let mut o = env.options("Rush", &["htop"]);
        o.config = Some(cfg);
        o.cwd = Some(env.dir.path().to_path_buf());
        let t = pin_table(&o, &app).unwrap();
        assert_eq!(t["title"].as_str(), Some("Custom"));
        assert_eq!(t["font_size"].as_integer(), Some(15));
        assert_eq!(t["cwd"].as_str(), env.dir.path().to_str());

        // The table loads as a config.
        let path = env.dir.join("pin.toml");
        std::fs::write(&path, toml::to_string(&t).unwrap()).unwrap();
        let (c, errors) = crate::config::load(&[&path], std::path::Path::new("/nonexistent"));
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(c.command, ["htop"]);
        assert_eq!(c.pin_app.as_deref(), app.to_str());
    }

    #[test]
    fn plist_escapes_name_and_has_keys() {
        let p = info_plist("A & <B> \"q\"", "a-b", true);
        assert!(p.contains("<string>A &amp; &lt;B&gt; &quot;q&quot;</string>"), "{p}");
        for key in [
            "<key>CFBundleExecutable</key>\n\t<string>a-b</string>",
            "<key>CFBundleIdentifier</key>\n\t<string>red.forbes.terminal-decline.pin.a-b</string>",
            "<key>CFBundlePackageType</key>\n\t<string>APPL</string>",
            "<key>CFBundleIconFile</key>",
            "<key>NSHighResolutionCapable</key>",
        ] {
            assert!(p.contains(key), "missing {key}");
        }
        assert!(!info_plist("A", "a", false).contains("CFBundleIconFile"));
    }

    #[test]
    fn own_pin_detection() {
        let env = Env::new("own");
        let bundle = env.paths.apps_dir.join("Rush.app/Contents/MacOS");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("rush"), b"").unwrap();
        std::fs::write(bundle.join("terminal-decline"), b"").unwrap();
        std::fs::create_dir_all(env.paths.pins_dir()).unwrap();
        std::fs::write(env.paths.pin_config("rush"), "").unwrap();
        let with_exe = |exe: PathBuf| Paths { exe, ..env.paths.clone() };

        let pin = with_exe(bundle.join("rush"));
        assert_eq!(own_pin_config(&pin), Some(env.paths.pin_config("rush")));
        assert_eq!(own_app_name(&pin).as_deref(), Some("Rush"));
        // Terminal Decline itself inside a bundle (Terminal Decline.app) is
        // not a pin.
        assert_eq!(own_pin_config(&with_exe(bundle.join("terminal-decline"))), None);
        // Nor through the cask's `decline` link, even with a settings file
        // of that name, nor as a build from before the rename.
        std::fs::write(env.paths.pin_config("decline"), "").unwrap();
        std::fs::write(env.paths.pin_config("terminal-decline"), "").unwrap();
        let link = env.dir.join("decline");
        std::os::unix::fs::symlink(bundle.join("terminal-decline"), &link).unwrap();
        assert_eq!(own_pin_config(&with_exe(link)), None);
        std::fs::write(bundle.join("decline"), b"").unwrap();
        assert_eq!(own_pin_config(&with_exe(bundle.join("decline"))), None);
        // A link to a pin's binary is that pin.
        let pin_link = env.dir.join("rush-link");
        std::os::unix::fs::symlink(bundle.join("rush"), &pin_link).unwrap();
        assert_eq!(own_pin_config(&with_exe(pin_link)), Some(env.paths.pin_config("rush")));
        // Outside a bundle: neither a pin nor an app name.
        let plain = with_exe(env.paths.exe.clone());
        assert_eq!(own_pin_config(&plain), None);
        assert_eq!(own_app_name(&plain), None);
        // A bundle whose pin has no settings file.
        std::fs::remove_file(env.paths.pin_config("rush")).unwrap();
        assert_eq!(own_pin_config(&pin), None);
    }

    #[test]
    fn create_list_update_remove() {
        let env = Env::new("cycle");
        let mut rec = Recorder::new();
        let made = create(env.options("My Tool", &["htop"]), &env.paths, &mut rec.runner()).unwrap();
        let app = env.paths.apps_dir.join("My Tool.app");
        assert_eq!(made.app, app);
        assert_eq!(made.config, env.paths.pin_config("my-tool"));
        assert_eq!(std::fs::read(app.join("Contents/MacOS/my-tool")).unwrap(), b"not really a binary");
        assert!(
            std::fs::read_to_string(app.join("Contents/Info.plist"))
                .unwrap()
                .contains("red.forbes.terminal-decline.pin.my-tool")
        );
        assert_eq!(rec.programs(), ["cp", "xattr", "codesign", "lsregister"]);
        assert!(rec.calls[2].contains(&"red.forbes.terminal-decline.pin.my-tool".to_string()));

        assert_eq!(list(&env.paths), [(app.clone(), r#"["htop"]"#.to_string())]);

        // Same name again: updates in place.
        create(env.options("My Tool", &["top"]), &env.paths, &mut rec.runner()).unwrap();
        // Another name with the same id: refused.
        let mut o = env.options("my-tool", &["htop"]);
        o.dir = env.dir.join("elsewhere");
        let e = create(o, &env.paths, &mut rec.runner()).unwrap_err();
        assert!(e.contains("already uses the id \"my-tool\""), "{e}");
        assert!(!env.dir.join("elsewhere").exists());

        // A pin from before the rename: `--update` gives it the new id.
        let plist = app.join("Contents/Info.plist");
        let old = std::fs::read_to_string(&plist).unwrap().replace(ID_PREFIX, OLD_ID_PREFIX);
        std::fs::write(&plist, old).unwrap();
        let mut rec = Recorder::new();
        update(&env.paths, &mut rec.runner()).unwrap();
        assert_eq!(rec.programs(), ["cp", "xattr", "codesign", "lsregister"]);
        assert!(rec.calls[2].contains(&"red.forbes.terminal-decline.pin.my-tool".to_string()));
        let new = std::fs::read_to_string(&plist).unwrap();
        assert_eq!(new, info_plist("My Tool", "my-tool", false));
        assert!(!new.contains("red.forbes.decline."), "{new}");

        assert_eq!(remove("My Tool", &env.paths), Ok(app.clone()));
        assert!(!app.exists());
        assert!(!env.paths.pin_config("my-tool").exists());
        assert!(remove("My Tool", &env.paths).unwrap_err().contains("no pin named"));
    }

    /// `--remove` trusts nothing in the editable settings file: anything but
    /// a Terminal Decline pin bundle inside the apps dir is left alone.
    #[test]
    fn remove_deletes_only_real_pin_bundles() {
        let env = Env::new("remove");
        let victim = env.dir.join("victim");
        let pin_bin = |app: &Path| {
            std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            std::fs::write(app.join("Contents/MacOS/x"), b"").unwrap();
        };
        let outside_app = env.dir.join("Outside.app");
        pin_bin(&outside_app);
        let no_binary = env.paths.apps_dir.join("NoBinary.app");
        std::fs::create_dir_all(no_binary.join("Contents/MacOS")).unwrap();
        let not_app = env.paths.apps_dir.join("NotApp");
        pin_bin(&not_app);
        std::fs::create_dir_all(&victim).unwrap();
        let sneaky = env.paths.apps_dir.join("../Outside.app");

        for target in [
            &victim,
            &env.paths.apps_dir,
            &outside_app,
            &no_binary,
            &not_app,
            &sneaky,
            &env.dir.join("."),
        ] {
            std::fs::create_dir_all(env.paths.pins_dir()).unwrap();
            std::fs::write(env.paths.pin_config("x"), format!("pin_app = {:?}\n", target.to_str().unwrap())).unwrap();
            let e = remove("x", &env.paths).unwrap_err();
            assert!(e.contains("nothing removed"), "{e}");
            assert!(target.exists(), "{} deleted", target.display());
            assert!(env.paths.pin_config("x").exists(), "settings deleted for {}", target.display());
        }
        assert!(victim.exists() && outside_app.join("Contents/MacOS/x").exists());
    }

    #[test]
    fn dock_tile_url_is_percent_encoded() {
        assert_eq!(
            file_url(Path::new("/Users/a b/Applications/My Tool.app")),
            "file:///Users/a%20b/Applications/My%20Tool.app/"
        );
        assert_eq!(file_url(Path::new("/A&B/é#?%.app")), "file:///A%26B/%C3%A9%23%3F%25.app/");
        let tile = dock_tile(Path::new("/Apps/My Tool.app"));
        assert!(tile.contains("<string>file:///Apps/My%20Tool.app/</string>"), "{tile}");

        let env = Env::new("dock");
        let mut rec = Recorder::new();
        let mut o = env.options("My Tool", &["htop"]);
        o.dock = true;
        create(o, &env.paths, &mut rec.runner()).unwrap();
        assert_eq!(rec.programs(), ["cp", "xattr", "codesign", "lsregister", "defaults", "killall"]);
        let defaults = &rec.calls[4];
        let url = file_url(&env.paths.apps_dir.join("My Tool.app"));
        assert!(url.contains("My%20Tool.app/") && !url.contains(' '));
        assert_eq!(defaults[1..5], s(&["write", "com.apple.dock", "persistent-apps", "-array-add"]));
        assert!(defaults[5].contains(&url), "{defaults:?}");
    }

    #[test]
    fn codesign_failure_is_an_error_lsregister_failure_is_not() {
        let env = Env::new("sign");
        let mut rec = Recorder::new().failing("codesign");
        let e = create(env.options("Rush", &["htop"]), &env.paths, &mut rec.runner()).unwrap_err();
        assert!(e.contains("codesign failed"), "{e}");
        assert_eq!(rec.programs(), ["cp", "xattr", "codesign"]);

        let mut rec = Recorder::new().failing("lsregister").failing("xattr");
        create(env.options("Rush", &["htop"]), &env.paths, &mut rec.runner()).unwrap();

        // A runner that can't start a program at all counts as failure.
        let mut broken = |_: &mut Command| -> io::Result<ExitStatus> { Err(io::Error::other("no")) };
        let e = create(env.options("Rush", &["htop"]), &env.paths, &mut broken).unwrap_err();
        assert!(e.contains("codesign failed"), "{e}");

        let mut rec = Recorder::new().failing("defaults");
        let mut o = env.options("Rush", &["htop"]);
        o.dock = true;
        assert!(create(o, &env.paths, &mut rec.runner()).unwrap_err().contains("Dock"));
    }

    /// cp, xattr and codesign really run on a temp bundle, which must then
    /// pass `codesign --verify`; LaunchServices and the Dock are only
    /// recorded.
    #[test]
    fn real_bundle_is_signed() {
        let env = Env::new("real");
        let paths = Paths {
            exe: PathBuf::from("/usr/bin/true"),
            ..env.paths.clone()
        };
        let mut recorded = Vec::new();
        let mut runner = |cmd: &mut Command| {
            let program = cmd.get_program().to_string_lossy().into_owned();
            if ["lsregister", "defaults", "killall"].iter().any(|p| program.ends_with(p)) {
                recorded.push(program);
                return Ok(ExitStatus::from_raw(0));
            }
            cmd.status()
        };
        let mut o = env.options("Real Pin", &["htop"]);
        o.dock = true;
        let made = create(o, &paths, &mut runner).unwrap();
        assert_eq!(recorded.len(), 3, "{recorded:?}");
        let out = Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict", "--verbose=2"])
            .arg(&made.app)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let info = Command::new("/usr/bin/codesign")
            .args(["-d", "-v"])
            .arg(&made.app)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&info.stderr);
        assert!(text.contains("Identifier=red.forbes.terminal-decline.pin.real-pin"), "{text}");
    }
}
