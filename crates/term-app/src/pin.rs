//! Dock pins: make any program its own macOS app (docs/design/pins.md).
//!
//! A pin is a small app bundle with its own bundle id (so macOS gives it
//! its own Dock icon, Cmd+Tab entry and permissions) and an icon. Made from
//! Terminal Decline.app, it holds a copy of the app's pin stub, which loads
//! the installed app's library, so the bundle never changes again and
//! neither do its permissions ([`bundle`]). Made from a bare binary
//! (`cargo install`), it holds an APFS clone of that binary instead (a
//! "clone pin", re-cloned by `--update`). What it runs, and any config
//! setting it overrides, live in `~/.config/terminal-decline/pins/<id>.toml`,
//! outside the bundle; what made it is in `pins/<id>.pin.toml` ([`record`]).
//!
//! Every location comes in through [`Paths`] and every other program (cp,
//! xattr, codesign, LaunchServices, sips, iconutil, defaults, killall, the
//! editor) runs through a [`Runner`], so tests use temp dirs and record the
//! commands.

pub mod bundle;
pub mod record;
pub mod status;

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::paths::Paths;
use record::{Method, Record};
use status::Status;

pub const ID_PREFIX: &str = "red.forbes.terminal-decline.pin.";

/// Pins' bundle ids before the rename to Terminal Decline.
pub const OLD_ID_PREFIX: &str = "red.forbes.decline.pin.";

/// The executable's own names: `terminal-decline`, and the `decline` alias
/// the cask links (and builds from before the rename). Never pin ids.
pub(crate) const OWN_NAMES: [&str; 2] = ["terminal-decline", "decline"];

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// Runs a command to completion. The real one is [`system_runner`].
pub type Runner<'a> = dyn FnMut(&mut Command) -> io::Result<ExitStatus> + 'a;

/// Not a program: a [`Runner`] command with this name and a bundle path
/// registers the bundle with LaunchServices (`LSRegisterURL`, which also
/// makes it show in Spotlight). Tests record it like any other.
pub const REGISTER: &str = "LSRegisterURL";

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn LSRegisterURL(url: term_pin::cf::CFTypeRef, update: u8) -> i32;
}

fn ls_register(app: &Path) -> bool {
    let Some(url) = term_pin::cf::url(app, true) else { return false };
    // SAFETY: a live CFURL.
    unsafe { LSRegisterURL(url.get(), 1) == 0 }
}

pub fn system_runner(cmd: &mut Command) -> io::Result<ExitStatus> {
    use std::os::unix::process::ExitStatusExt;
    if cmd.get_program() == REGISTER {
        let ok = cmd.get_args().next().is_some_and(|a| ls_register(Path::new(a)));
        return Ok(ExitStatus::from_raw(if ok { 0 } else { 1 << 8 }));
    }
    cmd.status()
}

pub(crate) fn succeeds(run: &mut Runner, cmd: &mut Command) -> bool {
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
    List {
        json: bool,
    },
    /// Clone pins only: re-cloned, or made stub pins from the app.
    Update {
        yes: bool,
    },
    /// Every pin, or the one named.
    Repair {
        name: Option<String>,
        yes: bool,
    },
    Edit(String),
    Remove(String),
    Create(Options),
}

/// `args` after the first, which may only be the given flags (and, if
/// `name`, one pin name). Returns (name, flags seen).
fn flags_and_name(first: &str, rest: &[String], allowed: &[&str], name: bool) -> Result<(Option<String>, Vec<String>), String> {
    let (mut n, mut seen) = (None, Vec::new());
    for a in rest {
        if allowed.contains(&a.as_str()) {
            seen.push(a.clone());
        } else if name && n.is_none() && !a.starts_with('-') {
            check_name(a)?;
            n = Some(a.clone());
        } else {
            return Err(format!("unexpected {a:?} after {first}"));
        }
    }
    Ok((n, seen))
}

/// `terminal-decline pin …` arguments (after `pin`). `apps_dir` is the
/// default for `--dir`.
pub fn parse_args(args: &[String], apps_dir: &Path) -> Result<Request, String> {
    let first = args.first().map(String::as_str);
    let rest = args.get(1..).unwrap_or_default();
    match first {
        None | Some("-h" | "--help") => return Ok(Request::Help),
        Some("--list") => {
            let (_, f) = flags_and_name("--list", rest, &["--json"], false)?;
            return Ok(Request::List { json: !f.is_empty() });
        }
        Some("--update") => {
            let (_, f) = flags_and_name("--update", rest, &["--yes", "-y"], false)?;
            return Ok(Request::Update { yes: !f.is_empty() });
        }
        Some("--repair") => {
            let (name, f) = flags_and_name("--repair", rest, &["--yes", "-y"], true)?;
            return Ok(Request::Repair { name, yes: !f.is_empty() });
        }
        Some("--edit") => {
            let (name, _) = flags_and_name("--edit", rest, &[], true)?;
            return Ok(Request::Edit(name.ok_or("--edit needs a name")?));
        }
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
        Request::List { json } => {
            let rows = list(paths, &term_pin::find::launch_services);
            let app = installed_app();
            if json {
                println!("{}", list_json(&rows, app.as_ref()));
            } else {
                print!("{}", list_text(&rows, app.as_ref()));
            }
        }
        Request::Update { yes } => update(paths, run, &mut confirmer(yes))?,
        Request::Repair { name, yes } => repair(name.as_deref(), paths, run, &mut confirmer(yes), &term_pin::find::launch_services)?,
        Request::Edit(name) => {
            let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
            let editor = ["VISUAL", "EDITOR"]
                .iter()
                .filter_map(std::env::var_os)
                .map(|v| v.to_string_lossy().into_owned())
                .find(|v| !v.trim().is_empty())
                .filter(|_| tty);
            for e in edit(&name, paths, editor.as_deref(), run)? {
                eprintln!("{e}");
            }
        }
        Request::Remove(name) => {
            let app = remove(&name, paths)?;
            println!("removed {}", app.display());
        }
        Request::Create(o) => {
            let made = create(o, paths, run)?;
            if made.rebuilt {
                println!("created {}", made.app.display());
            } else {
                println!("updated the settings of {} (the app is unchanged)", made.app.display());
            }
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
    let _ = std::fs::remove_file(record::path(&paths.pins_dir(), &id));
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
    /// Whether the bundle was made (or remade); `false` when only the
    /// settings of an existing stub pin were rewritten (§5.2).
    pub rebuilt: bool,
}

pub fn create(o: Options, paths: &Paths, run: &mut Runner) -> Result<Created, String> {
    check_name(&o.name)?;
    let source = bundle::Source::of(paths).transpose()?;
    if source.is_none() {
        clone_source(&paths.exe)?;
    }
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
    std::fs::create_dir_all(paths.pins_dir()).map_err(|e| e.to_string())?;
    let pin_toml = format!(
        "# Made by `terminal-decline pin`. Any config.toml setting can go here.\n{}",
        toml::to_string_pretty(&table).map_err(|e| e.to_string())?
    );

    let rebuilt = match &source {
        // An existing stub pin of this id: only its settings change, so
        // it keeps its permissions (§5.2).
        Some(_) if stub_pin_id(&app).as_deref() == Some(slug.as_str()) => {
            if o.icon.is_some() {
                eprintln!("warning: {} keeps its icon; to change it, remove the pin and make it again", o.name);
            }
            std::fs::write(&config_path, pin_toml).map_err(|e| e.to_string())?;
            false
        }
        Some(src) => {
            std::fs::write(&config_path, pin_toml).map_err(|e| e.to_string())?;
            let (icns, plan) = match &o.icon {
                Some(_) => (None, None),
                None => generated_icon(&o.name),
            };
            let icon = match (&o.icon, &icns) {
                (Some(file), _) => bundle::Icon::Image(file),
                (None, Some(bytes)) => bundle::Icon::Icns(bytes),
                (None, None) => bundle::Icon::None,
            };
            bundle::build(src, &o.name, &slug, &app, icon, run)?;
            stub_record(src, &app, plan).write(&paths.pins_dir(), &slug)?;
            true
        }
        None => {
            std::fs::write(&config_path, pin_toml).map_err(|e| e.to_string())?;
            make_clone(&o.name, &slug, &app, o.icon.as_deref(), &paths.exe, run)?;
            clone_record(&app).write(&paths.pins_dir(), &slug)?;
            true
        }
    };
    if o.dock {
        add_to_dock(&app, run)?;
    }
    Ok(Created {
        app,
        config: config_path,
        dock: o.dock,
        rebuilt,
    })
}

/// The `TDPinId` of the stub pin at `app`, if it is one.
fn stub_pin_id(app: &Path) -> Option<String> {
    let info = term_pin::cf::InfoPlist::read(app)?;
    info.integer("TDPinAbi")?;
    info.string("TDPinId")
}

/// A new pin's icon by the Slope rule (term-icon), and its plan for the
/// record; `(None, None)` with a warning if it can't be drawn.
fn generated_icon(name: &str) -> (Option<Vec<u8>>, Option<term_icon::PinIcon>) {
    match term_icon::generate(name).and_then(|plan| Ok((term_icon::render_icns(&plan)?, plan))) {
        Ok((icns, plan)) => (Some(icns), Some(plan)),
        Err(e) => {
            eprintln!("warning: no icon for {name}: {e}");
            (None, None)
        }
    }
}

fn stub_record(src: &bundle::Source, app: &Path, icon: Option<term_icon::PinIcon>) -> Record {
    Record {
        stub: term_pin::abi::ABI_LATEST,
        method: Method::Stub,
        made_by: src.made_by.clone(),
        requirement: src.requirement.kind().into(),
        pin_app: app.to_path_buf(),
        icon,
    }
}

fn clone_record(app: &Path) -> Record {
    Record {
        stub: 0,
        method: Method::Clone,
        made_by: env!("CARGO_PKG_VERSION").into(),
        requirement: String::new(),
        pin_app: app.to_path_buf(),
        icon: None,
    }
}

/// A clone pin (§5.6): this bare binary cloned into the bundle.
fn make_clone(name: &str, slug: &str, app: &Path, icon: Option<&Path>, exe: &Path, run: &mut Runner) -> Result<(), String> {
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos).map_err(|e| format!("{}: {e}", macos.display()))?;
    std::fs::create_dir_all(&resources).map_err(|e| e.to_string())?;
    if let Some(icon) = icon {
        make_icns(icon, &resources.join("icon.icns"), run)?;
    }
    std::fs::write(contents.join("Info.plist"), info_plist(name, slug, icon.is_some())).map_err(|e| e.to_string())?;
    install_binary(exe, &macos.join(slug), run)?;
    finish_bundle(app, slug, run)
}

/// Clone (APFS copy-on-write) `exe` into the bundle. Written to a new file
/// and renamed into place: overwriting a signed binary that may be running
/// gets it killed on Apple Silicon.
fn install_binary(exe: &Path, dest: &Path, run: &mut Runner) -> Result<(), String> {
    let exe = clone_source(exe)?;
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

/// The binary a clone pin copies: `exe`, unless it is the launcher of an
/// app bundle whose code is in `Contents/Frameworks/libterminal_decline.dylib`
/// (pins.md §2.1). A copy of the launcher would look for the library
/// inside the pin. Only reached when the app has no pin stub to make
/// stub pins from ([`bundle::Source::of`]).
fn clone_source(exe: &Path) -> Result<PathBuf, String> {
    let exe = exe.canonicalize().map_err(|e| format!("{}: {e}", exe.display()))?;
    let launcher = exe
        .parent()
        .and_then(Path::parent)
        .is_some_and(|contents| contents.join("Frameworks/libterminal_decline.dylib").is_file());
    if launcher {
        return Err(format!(
            "{} is the app's launcher, which can't be copied into a pin, and the app has no pin \
             stub ({}); reinstall Terminal Decline",
            exe.display(),
            bundle::STUB_IN_APP
        ));
    }
    Ok(exe)
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

pub(crate) fn xml_escape(s: &str) -> String {
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
pub(crate) fn make_icns(src: &Path, dest: &Path, run: &mut Runner) -> Result<(), String> {
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

/// Every pin: (id, app path), from the pins' settings files (not their
/// records), sorted.
pub fn pins(paths: &Paths) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(paths.pins_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml") && !record::is_record(p))
        .filter_map(|p| {
            let id = p.file_stem()?.to_str()?.to_string();
            let t = std::fs::read_to_string(&p).ok()?.parse::<toml::Table>().ok()?;
            Some((id, PathBuf::from(t.get("pin_app")?.as_str()?)))
        })
        .collect();
    out.sort();
    out
}

/// One pin, for `--list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    /// The bundle's name without `.app`.
    pub name: String,
    pub app: PathBuf,
    /// Its `command` setting, as TOML.
    pub command: String,
    pub status: Status,
}

/// Every pin and its state. `lookup` asks LaunchServices for a bundle id
/// ([`term_pin::find::launch_services`]).
pub fn list(paths: &Paths, lookup: &dyn Fn(&str) -> Vec<PathBuf>) -> Vec<Row> {
    pins(paths)
        .into_iter()
        .map(|(id, app)| {
            let command = std::fs::read_to_string(paths.pin_config(&id))
                .ok()
                .and_then(|c| c.parse::<toml::Table>().ok())
                .and_then(|t| t.get("command").cloned())
                .map(|c| c.to_string())
                .unwrap_or_default();
            let record = Record::read(&paths.pins_dir(), &id);
            let status = status::of(&id, &app, record.as_ref(), lookup);
            let name = app.file_stem().map_or_else(|| id.clone(), |s| s.to_string_lossy().into_owned());
            Row {
                id,
                name,
                app,
                command,
                status,
            }
        })
        .collect()
}

/// The Terminal Decline.app pins would load, found the way the stub finds
/// it (crates/term-pin/FROZEN.md): (path, version, build).
pub fn installed_app() -> Option<(PathBuf, String, String)> {
    let home = term_pin::find::home();
    let mut notes = Vec::new();
    term_pin::find::candidates(home.as_deref(), bundle::APP_BUNDLE_ID, term_pin::find::launch_services, &mut notes)
        .into_iter()
        .find(|c| term_pin::find::library(&c.app, bundle::APP_BUNDLE_ID).is_ok())
        .map(|c| {
            let info = term_pin::cf::InfoPlist::read(&c.app);
            let key = |k: &str| info.as_ref().and_then(|i| i.string(k)).unwrap_or_default();
            (c.app, key("CFBundleShortVersionString"), key("CFBundleVersion"))
        })
}

/// `--list`: the app line, then a table.
/// Nothing at all without pins.
pub fn list_text(rows: &[Row], app: Option<&(PathBuf, String, String)>) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = match app {
        Some((path, version, build)) => format!("using {} ({version}, build {build})\n", path.display()),
        None => "no Terminal Decline.app found: stub pins can't start\n".to_string(),
    };
    let pin = |r: &Row| match &r.status {
        Status::Moved(now) => format!("{} (was {})", crate::crash::tilde(now), crate::crash::tilde(&r.app)),
        Status::Missing => format!("(was {})", crate::crash::tilde(&r.app)),
        _ => crate::crash::tilde(&r.app),
    };
    let cells: Vec<[String; 4]> = rows
        .iter()
        .map(|r| [r.name.clone(), r.status.label(), r.command.clone(), pin(r)])
        .collect();
    let header = ["NAME", "STATUS", "COMMAND", "PIN"].map(String::from);
    let width = |i: usize| cells.iter().chain([&header]).map(|c| c[i].chars().count()).max().unwrap_or(0);
    let widths = [width(0), width(1), width(2)];
    for c in [&header].into_iter().chain(&cells) {
        for (i, w) in widths.iter().enumerate() {
            out.push_str(&format!("{:<w$}  ", c[i]));
        }
        out.push_str(&c[3]);
        out.push('\n');
    }
    out
}

/// `--list --json`, for scripts and the preferences' Pins section.
pub fn list_json(rows: &[Row], app: Option<&(PathBuf, String, String)>) -> String {
    let pins: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let mut v = serde_json::json!({
                "id": r.id,
                "name": r.name,
                "app": r.app,
                "command": r.command,
                "status": r.status.key(),
                "dev": matches!(r.status, Status::Ok { dev: true }),
            });
            match &r.status {
                Status::Moved(p) => v["moved_to"] = serde_json::json!(p),
                Status::Damaged(why) => v["why"] = serde_json::json!(why),
                _ => {}
            }
            v
        })
        .collect();
    let app = app.map(|(path, version, build)| serde_json::json!({"path": path, "version": version, "build": build}));
    serde_json::to_string_pretty(&serde_json::json!({ "app": app, "pins": pins })).expect("JSON of strings")
}

/// Asks before a rebuild that resets a pin's permissions (§5.2):
/// `question` → yes or no.
pub type Confirm<'a> = dyn FnMut(&str) -> bool + 'a;

/// `--yes` answers yes; otherwise the question is asked on the terminal,
/// and without one the answer is no.
fn confirmer(yes: bool) -> impl FnMut(&str) -> bool {
    move |question: &str| {
        if yes {
            return true;
        }
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("{question} [y/N] no (not a terminal; run with --yes to go ahead)");
            return false;
        }
        eprint!("{question} [y/N] ");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        matches!(line.trim(), "y" | "Y" | "yes" | "Yes")
    }
}

fn reset_question(name: &str) -> String {
    format!("macOS will ask again for any permissions {name} has (camera, folders, …). Continue?")
}

/// The clone or damaged pin `id` at `app`, made again as a stub pin in
/// place (§5.3): its name, icon file and settings kept, the current id
/// prefix.
fn rebuild_as_stub(src: &bundle::Source, id: &str, app: &Path, paths: &Paths, run: &mut Runner) -> Result<(), String> {
    let name = app.file_stem().map_or_else(|| id.to_string(), |s| s.to_string_lossy().into_owned());
    let old_record = Record::read(&paths.pins_dir(), id);
    let renamed = std::fs::read_to_string(app.join("Contents/Info.plist")).is_ok_and(|p| p.contains(OLD_ID_PREFIX));
    let kept = std::fs::read(app.join("Contents/Resources/icon.icns")).ok();
    let (icns, plan) = match (kept, old_record.and_then(|r| r.icon)) {
        (Some(bytes), plan) => (Some(bytes), plan),
        (None, Some(plan)) => (term_icon::render_icns(&plan).ok(), Some(plan)),
        (None, None) => generated_icon(&name),
    };
    let icon = icns.as_deref().map_or(bundle::Icon::None, bundle::Icon::Icns);
    bundle::build(src, &name, id, app, icon, run)?;
    stub_record(src, app, plan).write(&paths.pins_dir(), id)?;
    if renamed {
        // Its Secure Keyboard Entry choice, kept under the old id.
        crate::secure_input::carry_over(&format!("{ID_PREFIX}{id}"));
    }
    Ok(())
}

/// Points pin `id`'s settings and record at `now`, where it was moved.
fn moved_to(id: &str, now: &Path, paths: &Paths) -> Result<(), String> {
    let file = paths.pin_config(id);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut doc: toml::Table = text.parse().map_err(|e| format!("{}: {e}", file.display()))?;
    doc.insert("pin_app".into(), toml::Value::String(now.to_string_lossy().into_owned()));
    let header: String = text.lines().take_while(|l| l.starts_with('#')).map(|l| format!("{l}\n")).collect();
    let body = toml::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&file, format!("{header}{body}")).map_err(|e| format!("{}: {e}", file.display()))?;
    if let Some(mut r) = Record::read(&paths.pins_dir(), id) {
        r.pin_app = now.to_path_buf();
        r.write(&paths.pins_dir(), id)?;
    }
    Ok(())
}

/// `pin --repair [NAME]` (§5.3): registers each pin with LaunchServices
/// again, follows pins that were moved, and remakes old clone pins and
/// damaged ones as stub pins, after asking.
pub fn repair(
    name: Option<&str>,
    paths: &Paths,
    run: &mut Runner,
    confirm: &mut Confirm,
    lookup: &dyn Fn(&str) -> Vec<PathBuf>,
) -> Result<(), String> {
    let want = name.map(slug);
    let rows: Vec<Row> = list(paths, lookup)
        .into_iter()
        .filter(|r| want.as_ref().is_none_or(|w| *w == r.id))
        .collect();
    if let (Some(n), true) = (name, rows.is_empty()) {
        return Err(format!("no pin named {n}"));
    }
    let source = bundle::Source::of(paths).transpose()?;
    let mut failed = Vec::new();
    for r in rows {
        let register = |run: &mut Runner, app: &Path| {
            if !succeeds(run, Command::new(REGISTER).arg(app)) {
                eprintln!("warning: couldn't register {} with LaunchServices", app.display());
            }
        };
        match &r.status {
            Status::Ok { .. } => {
                register(run, &r.app);
                println!("{}: ok", r.name);
            }
            Status::Moved(now) => {
                moved_to(&r.id, now, paths)?;
                register(run, now);
                println!("{}: moved to {}; its settings follow", r.name, now.display());
            }
            Status::Old | Status::Damaged(_) => {
                let Some(src) = &source else {
                    println!(
                        "{}: {}; run `terminal-decline pin --repair` from Terminal Decline.app to rebuild it",
                        r.name,
                        r.status.label()
                    );
                    continue;
                };
                if let Status::Damaged(why) = &r.status {
                    println!("{}: damaged ({why})", r.name);
                }
                if !confirm(&reset_question(&r.name)) {
                    println!("{}: left as it is", r.name);
                    continue;
                }
                match rebuild_as_stub(src, &r.id, &r.app, paths, run) {
                    Ok(()) => println!("{}: rebuilt as a stub pin (restart it if it is running)", r.name),
                    Err(e) => {
                        eprintln!("{}: {e}", r.name);
                        failed.push(r.name.clone());
                    }
                }
            }
            Status::Clone => {
                register(run, &r.app);
                println!("{}: a clone pin; `pin --update` re-clones it", r.name);
            }
            Status::NeedsNewer(v) => println!("{}: needs a Terminal Decline with pin interface {v}", r.name),
            Status::Missing => println!(
                "{}: missing (was {}); `pin --remove {}` forgets it",
                r.name,
                r.app.display(),
                r.name
            ),
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("couldn't repair {}", failed.join(", ")))
    }
}

/// `pin --update` (§5.3): stub pins are left alone (they load the
/// installed app); clone pins are made stub pins when run from the app
/// (after asking), else re-cloned from this binary as before.
pub fn update(paths: &Paths, run: &mut Runner, confirm: &mut Confirm) -> Result<(), String> {
    let source = bundle::Source::of(paths).transpose()?;
    if source.is_none() {
        clone_source(&paths.exe)?;
    }
    for (id, app) in pins(paths) {
        let name = app.file_stem().map_or_else(|| id.clone(), |s| s.to_string_lossy().into_owned());
        if status::is_stub_pin(&app) {
            println!("{name} is up to date (it loads the installed Terminal Decline)");
            continue;
        }
        let bin = app.join("Contents/MacOS").join(&id);
        if !bin.is_file() {
            eprintln!("skipping {}: not found", app.display());
            continue;
        }
        if let Some(src) = &source {
            if confirm(&reset_question(&name)) {
                rebuild_as_stub(src, &id, &app, paths, run)?;
                println!("rebuilt {} as a stub pin", app.display());
            } else {
                println!("{name}: left as it is");
            }
            continue;
        }
        // The bundle is `<name>.app`; its icon, if any, stays as it is.
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

/// `pin --edit NAME` (§5.2): opens the pin's settings in `editor`
/// (`$VISUAL`/`$EDITOR`, run through the shell so it may have
/// arguments), else in the default text editor, then returns what the
/// config loader finds wrong with them. Only the settings change, so the
/// pin keeps its permissions; a running pin reloads them.
pub fn edit(name: &str, paths: &Paths, editor: Option<&str>, run: &mut Runner) -> Result<Vec<String>, String> {
    let id = slug(name);
    let file = paths.pin_config(&id);
    if !pins(paths).iter().any(|(i, _)| *i == id) {
        return Err(format!("no pin named {name}"));
    }
    let opened = match editor {
        Some(ed) => succeeds(
            run,
            Command::new("/bin/sh").arg("-c").arg(format!("{ed} \"$1\"")).arg("sh").arg(&file),
        ),
        None => {
            let ok = succeeds(run, Command::new("/usr/bin/open").arg("-t").arg(&file));
            if ok {
                println!(
                    "opened {} in your text editor; a running {name} reloads it when saved",
                    file.display()
                );
            }
            ok
        }
    };
    if !opened {
        return Err(format!("couldn't open an editor for {}", file.display()));
    }
    let (_, errors) = crate::config::load(&[&file], &paths.themes_dir());
    Ok(errors.into_iter().map(|e| format!("{}: {e}", file.display())).collect())
}

/// The first window's notice while clone pins from before stubs remain
/// (§5.4); `None` when there are none.
pub fn migration_notice(paths: &Paths) -> Option<String> {
    // Only Info.plists and records are read: no signature checks at launch.
    let old = pins(paths)
        .into_iter()
        .filter(|(id, app)| {
            term_pin::cf::InfoPlist::read(app).is_some_and(|i| i.integer("TDPinAbi").is_none())
                && Record::read(&paths.pins_dir(), id).is_none_or(|r| r.method != Method::Clone)
        })
        .count();
    match old {
        0 => None,
        1 => Some(
            "1 pin still runs its own copy of an older version. Run `terminal-decline pin --repair` to update it; \
             macOS will ask once more for its permissions."
                .into(),
        ),
        n => Some(format!(
            "{n} pins still run their own copy of an older version. Run `terminal-decline pin --repair` to update \
             them; macOS will ask once more for their permissions."
        )),
    }
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
                app_bundle: None,
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
        assert_eq!(parse_args(&s(&["--list"]), apps), Ok(Request::List { json: false }));
        assert_eq!(parse_args(&s(&["--list", "--json"]), apps), Ok(Request::List { json: true }));
        assert_eq!(parse_args(&s(&["--update"]), apps), Ok(Request::Update { yes: false }));
        assert_eq!(parse_args(&s(&["--update", "--yes"]), apps), Ok(Request::Update { yes: true }));
        assert_eq!(parse_args(&s(&["--repair"]), apps), Ok(Request::Repair { name: None, yes: false }));
        assert_eq!(
            parse_args(&s(&["--repair", "Rush", "--yes"]), apps),
            Ok(Request::Repair {
                name: Some("Rush".into()),
                yes: true
            })
        );
        assert_eq!(parse_args(&s(&["--edit", "Rush"]), apps), Ok(Request::Edit("Rush".into())));
        assert!(err(&["--edit"]).contains("needs a name"));
        assert!(err(&["--list", "x"]).contains("unexpected"));
        assert!(err(&["--repair", "a", "b"]).contains("unexpected"));
        assert!(err(&["--update", "--json"]).contains("unexpected"));
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
    fn the_launcher_makes_no_clone_pins() {
        let env = Env::new("launcher");
        let app = env.dir.join("Terminal Decline.app/Contents");
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Frameworks")).unwrap();
        std::fs::write(app.join("Frameworks/libterminal_decline.dylib"), "").unwrap();
        let launcher = app.join("MacOS/terminal-decline");
        std::fs::write(&launcher, "").unwrap();
        let paths = Paths {
            exe: launcher,
            ..env.paths.clone()
        };
        let mut ran = 0;
        let mut runner = |_: &mut Command| {
            ran += 1;
            Ok(ExitStatus::default())
        };
        let e = create(env.options("Rush", &["/bin/echo"]), &paths, &mut runner).unwrap_err();
        assert!(e.contains("launcher"), "{e}");
        assert!(!paths.pin_config("rush").exists() && !env.paths.apps_dir.join("Rush.app").exists());
        assert!(update(&paths, &mut runner, &mut |_| true).unwrap_err().contains("launcher"));
        assert_eq!(ran, 0);
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

        let rows = list(&env.paths, &|_| Vec::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (&rows[0].app, rows[0].command.as_str(), &rows[0].status),
            (&app, r#"["htop"]"#, &Status::Clone)
        );
        assert_eq!(Record::read(&env.paths.pins_dir(), "my-tool").unwrap().method, Method::Clone);

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
        update(&env.paths, &mut rec.runner(), &mut |_| panic!("no question for clones")).unwrap();
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

    /// A stand-in Terminal Decline.app with a pin stub and a library:
    /// copies of `/usr/bin/true` (a real Mach-O, signed without a team).
    fn fake_app(env: &Env) -> Paths {
        let app = env.dir.join("Terminal Decline.app");
        let contents = app.join("Contents");
        for d in ["Helpers", "Frameworks", "MacOS"] {
            std::fs::create_dir_all(contents.join(d)).unwrap();
        }
        std::fs::copy("/usr/bin/true", contents.join("Helpers/terminal-decline-pin")).unwrap();
        std::fs::copy("/usr/bin/true", contents.join("Frameworks/libterminal_decline.dylib")).unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>CFBundleIdentifier</key><string>red.forbes.terminal-decline</string>\
             <key>CFBundleShortVersionString</key><string>0.4.0</string>\
             <key>CFBundleVersion</key><string>1234</string></dict></plist>\n",
        )
        .unwrap();
        Paths {
            exe: contents.join("MacOS/terminal-decline"),
            app_bundle: Some(app),
            ..env.paths.clone()
        }
    }

    /// Real cp, xattr and codesign; LaunchServices and the Dock recorded.
    fn real_runner(recorded: &mut Vec<String>) -> impl FnMut(&mut Command) -> io::Result<ExitStatus> + '_ {
        move |cmd: &mut Command| {
            let program = cmd.get_program().to_string_lossy().into_owned();
            if [REGISTER, "lsregister", "defaults", "killall"].iter().any(|p| program.ends_with(p)) {
                recorded.push(program);
                return Ok(ExitStatus::from_raw(0));
            }
            cmd.status()
        }
    }

    fn codesign(args: &[&str], path: &Path) -> String {
        let out = Command::new("/usr/bin/codesign").args(args).arg(path).output().unwrap();
        assert!(out.status.success(), "codesign {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
    }

    fn uuids(exe: &Path) -> String {
        let out = Command::new("/usr/bin/dwarfdump").arg("--uuid").arg(exe).output().unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.split(' ').nth(1).unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// From the app, a pin is the app's stub in a new bundle (§3.1): its
    /// own UUID, the frozen Info.plist, a generated icon, one hardened
    /// ad-hoc signature with the pin entitlements, a record.
    #[test]
    fn stub_pins_are_made_from_the_apps_stub() {
        let env = Env::new("stub");
        let paths = fake_app(&env);
        let mut recorded = Vec::new();
        let made = create(env.options("Rush", &["htop"]), &paths, &mut real_runner(&mut recorded)).unwrap();
        assert!(made.rebuilt);
        assert_eq!(recorded, [REGISTER]);
        let app = env.paths.apps_dir.join("Rush.app");
        let exe = app.join("Contents/MacOS/rush");
        assert!(exe.is_file());
        assert!(!bundle::staging_for(&app).exists(), "the staging copy is gone");
        let stub = paths.app_bundle.as_ref().unwrap().join(bundle::STUB_IN_APP);
        let (before, after) = (uuids(&stub), uuids(&exe));
        assert!(!after.is_empty() && before != after, "{before} vs {after}");

        codesign(&["--verify", "--strict"], &app);
        let d = codesign(&["-d", "-v"], &app);
        assert!(d.contains("Identifier=red.forbes.terminal-decline.pin.rush"), "{d}");
        assert!(d.contains("(adhoc,runtime)") || d.contains("flags=0x10002(adhoc,runtime)"), "{d}");
        let ents = codesign(&["-d", "--entitlements", "-", "--xml"], &app);
        assert!(ents.contains("com.apple.security.cs.disable-library-validation"), "{ents}");
        assert!(ents.contains("com.apple.security.device.camera"), "{ents}");

        let info = term_pin::cf::InfoPlist::read(&app).unwrap();
        assert_eq!(info.integer("TDPinAbi"), Some(1));
        assert_eq!(info.string("TDPinId").as_deref(), Some("rush"));
        assert_eq!(info.string("TDAppBundleId").as_deref(), Some("red.forbes.terminal-decline"));
        assert_eq!(info.string("TDLibraryRequirement"), Some(term_pin::abi::dev_requirement()));
        assert!(app.join("Contents/Resources/icon.icns").metadata().unwrap().len() > 1000);

        let r = Record::read(&env.paths.pins_dir(), "rush").unwrap();
        assert_eq!((r.stub, r.method, r.requirement.as_str()), (1, Method::Stub, "dev"));
        assert_eq!((r.made_by.as_str(), r.pin_app.as_path()), ("0.4.0 (1234)", app.as_path()));
        let icon = r.icon.unwrap();
        assert_eq!((icon.canonical.as_str(), icon.initial.as_str()), ("rush", "R"));
        assert!(!icon.outline.is_empty());

        let rows = list(&paths, &|_| Vec::new());
        assert_eq!(rows[0].status, Status::Ok { dev: true });

        // Again with another command: only the settings change.
        let hash = codesign(&["-d", "-v"], &app);
        let made = create(env.options("Rush", &["top"]), &paths, &mut real_runner(&mut Vec::new())).unwrap();
        assert!(!made.rebuilt);
        assert_eq!(codesign(&["-d", "-v"], &app), hash);
        assert!(std::fs::read_to_string(paths.pin_config("rush")).unwrap().contains("\"top\""));

        // `--update` never writes into a stub pin.
        let mut rec = Recorder::new();
        update(&paths, &mut rec.runner(), &mut |_| panic!("no question")).unwrap();
        assert!(rec.calls.is_empty(), "{:?}", rec.calls);
        assert_eq!(codesign(&["-d", "-v"], &app), hash);

        // `--repair` of a good pin registers it again, nothing more.
        let mut rec = Recorder::new();
        repair(None, &paths, &mut rec.runner(), &mut |_| panic!("no question"), &|_| Vec::new()).unwrap();
        assert_eq!(rec.programs(), [REGISTER]);
        assert_eq!(codesign(&["-d", "-v"], &app), hash);

        assert_eq!(remove("Rush", &paths), Ok(app.clone()));
        assert!(!record::path(&env.paths.pins_dir(), "rush").exists());
    }

    /// The order of steps and the signing options, recorded.
    #[test]
    fn stub_pin_steps() {
        let env = Env::new("stub-steps");
        let paths = fake_app(&env);
        let mut rec = Recorder::new();
        let mut o = env.options("Rush", &["htop"]);
        o.dock = true;
        create(o, &paths, &mut rec.runner()).unwrap();
        assert_eq!(rec.programs(), ["cp", "xattr", "codesign", REGISTER, "defaults", "killall"]);
        let sign = rec.calls[2].join(" ");
        assert!(
            sign.starts_with("codesign --force --sign - --options runtime --entitlements ")
                && sign.contains("--identifier red.forbes.terminal-decline.pin.rush")
                && sign.ends_with(".Rush.pin-new"),
            "{sign}"
        );
        assert_eq!(rec.calls[3][1], env.paths.apps_dir.join("Rush.app").to_string_lossy());

        // A refused signature leaves nothing behind.
        let mut rec = Recorder::new().failing("codesign");
        let e = create(env.options("Other", &["htop"]), &paths, &mut rec.runner()).unwrap_err();
        assert!(e.contains("codesign failed"), "{e}");
        assert!(!env.paths.apps_dir.join("Other.app").exists());
        assert!(!bundle::staging_for(&env.paths.apps_dir.join("Other.app")).exists());
        assert!(!env.paths.apps_dir.join(".Other.entitlements.plist").exists());
    }

    /// Clone pins from before stubs: noticed, rebuilt as stub pins by
    /// `--repair` (after asking) keeping their icon file and settings, and
    /// by `--update` from the app.
    #[test]
    fn old_clone_pins_migrate_to_stubs() {
        let env = Env::new("migrate");
        let bare = Paths {
            exe: PathBuf::from("/usr/bin/true"),
            ..env.paths.clone()
        };
        let mut recorded = Vec::new();
        create(env.options("Logs", &["tail", "-f"]), &bare, &mut real_runner(&mut recorded)).unwrap();
        create(env.options("Top", &["htop"]), &bare, &mut real_runner(&mut recorded)).unwrap();
        let logs = env.paths.apps_dir.join("Logs.app");
        // As made before records existed, one with an icon, one from
        // before the rename.
        for id in ["logs", "top"] {
            std::fs::remove_file(record::path(&env.paths.pins_dir(), id)).unwrap();
        }
        std::fs::write(logs.join("Contents/Resources/icon.icns"), b"kept icon").unwrap();
        let top_plist = env.paths.apps_dir.join("Top.app/Contents/Info.plist");
        let old = std::fs::read_to_string(&top_plist).unwrap().replace(ID_PREFIX, OLD_ID_PREFIX);
        std::fs::write(&top_plist, old).unwrap();
        let settings = std::fs::read_to_string(env.paths.pin_config("logs")).unwrap();

        let paths = fake_app(&env);
        let notice = migration_notice(&paths).unwrap();
        assert!(notice.starts_with("2 pins still run their own copy"), "{notice}");
        assert!(list(&paths, &|_| Vec::new()).iter().all(|r| r.status == Status::Old));

        // Saying no changes nothing.
        let mut rec = Recorder::new();
        repair(None, &paths, &mut rec.runner(), &mut |_| false, &|_| Vec::new()).unwrap();
        assert!(rec.calls.is_empty(), "{:?}", rec.calls);

        let mut asked = Vec::new();
        repair(
            Some("Logs"),
            &paths,
            &mut real_runner(&mut Vec::new()),
            &mut |q: &str| {
                asked.push(q.to_string());
                true
            },
            &|_| Vec::new(),
        )
        .unwrap();
        assert_eq!(asked.len(), 1);
        assert!(
            asked[0].contains("macOS will ask again for any permissions Logs has"),
            "{}",
            asked[0]
        );
        assert!(status::is_stub_pin(&logs));
        assert_eq!(std::fs::read(logs.join("Contents/Resources/icon.icns")).unwrap(), b"kept icon");
        assert_eq!(std::fs::read_to_string(env.paths.pin_config("logs")).unwrap(), settings);
        assert_eq!(Record::read(&env.paths.pins_dir(), "logs").unwrap().method, Method::Stub);
        codesign(&["--verify", "--strict"], &logs);
        assert_eq!(
            migration_notice(&paths).as_deref().map(|n| n.starts_with("1 pin still runs")),
            Some(true)
        );

        // `--update` from the app: the other one, with the current id.
        update(&paths, &mut real_runner(&mut Vec::new()), &mut |_| true).unwrap();
        let top = env.paths.apps_dir.join("Top.app");
        let info = term_pin::cf::InfoPlist::read(&top).unwrap();
        assert_eq!(
            info.string("CFBundleIdentifier").as_deref(),
            Some("red.forbes.terminal-decline.pin.top")
        );
        assert!(info.integer("TDPinAbi").is_some());
        assert!(
            Record::read(&env.paths.pins_dir(), "top").unwrap().icon.is_some(),
            "an icon generated"
        );
        assert_eq!(migration_notice(&paths), None);
    }

    /// A pin moved in Finder: found through LaunchServices, and its
    /// settings follow it on `--repair`.
    #[test]
    fn moved_and_missing_pins() {
        let env = Env::new("moved");
        let paths = fake_app(&env);
        create(env.options("Rush", &["htop"]), &paths, &mut real_runner(&mut Vec::new())).unwrap();
        let was = env.paths.apps_dir.join("Rush.app");
        let now = env.dir.join("Elsewhere/Rush.app");
        std::fs::create_dir_all(now.parent().unwrap()).unwrap();
        std::fs::rename(&was, &now).unwrap();
        let ls = |id: &str| {
            if id == "red.forbes.terminal-decline.pin.rush" {
                vec![now.clone()]
            } else {
                Vec::new()
            }
        };
        let rows = list(&paths, &ls);
        assert_eq!(rows[0].status, Status::Moved(now.clone()));
        assert_eq!(list(&paths, &|_| Vec::new())[0].status, Status::Missing);
        let text = list_text(&rows, None);
        assert!(text.contains("pin moved: --repair") && text.contains("(was "), "{text}");
        let json: serde_json::Value = serde_json::from_str(&list_json(&rows, None)).unwrap();
        assert_eq!(json["pins"][0]["status"], "moved");
        assert_eq!(json["pins"][0]["name"], "Rush");

        let mut rec = Recorder::new();
        repair(None, &paths, &mut rec.runner(), &mut |_| panic!("no question"), &ls).unwrap();
        assert_eq!(rec.programs(), [REGISTER]);
        assert_eq!(pins(&paths), [("rush".to_string(), now.clone())]);
        assert_eq!(Record::read(&env.paths.pins_dir(), "rush").unwrap().pin_app, now);
        assert_eq!(list(&paths, &|_| Vec::new())[0].status, Status::Ok { dev: true });

        // A damaged pin: an edited Info.plist breaks the seal.
        let plist = now.join("Contents/Info.plist");
        let text = std::fs::read_to_string(&plist)
            .unwrap()
            .replace("<string>Rush</string>", "<string>Rusty</string>");
        std::fs::write(&plist, text).unwrap();
        assert!(matches!(list(&paths, &|_| Vec::new())[0].status, Status::Damaged(_)));
    }

    #[test]
    fn list_text_lines_up() {
        let row = |name: &str, status| Row {
            id: slug(name),
            name: name.into(),
            app: PathBuf::from(format!("/Apps/{name}.app")),
            command: r#"["htop"]"#.into(),
            status,
        };
        let rows = [row("Rush", Status::Ok { dev: false }), row("Logs", Status::Old)];
        let app = (
            PathBuf::from("/Applications/Terminal Decline.app"),
            "0.4.0".to_string(),
            "1234".to_string(),
        );
        assert_eq!(
            list_text(&rows, Some(&app)),
            "using /Applications/Terminal Decline.app (0.4.0, build 1234)\n\
             NAME  STATUS             COMMAND   PIN\n\
             Rush  ok                 [\"htop\"]  /Apps/Rush.app\n\
             Logs  old: run --repair  [\"htop\"]  /Apps/Logs.app\n"
        );
    }

    #[test]
    fn edit_opens_the_settings_and_checks_them() {
        let env = Env::new("edit");
        let mut rec = Recorder::new();
        create(env.options("Rush", &["htop"]), &env.paths, &mut rec.runner()).unwrap();
        let file = env.paths.pin_config("rush");
        let mut rec = Recorder::new();
        assert_eq!(edit("Rush", &env.paths, Some("vim -f"), &mut rec.runner()), Ok(vec![]));
        assert_eq!(rec.calls[0][..3], s(&["sh", "-c", "vim -f \"$1\""]));
        assert_eq!(rec.calls[0][4], file.to_string_lossy());
        let mut rec = Recorder::new();
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, format!("font_sise = 3\n{text}")).unwrap();
        let errors = edit("Rush", &env.paths, None, &mut rec.runner());
        assert_eq!(rec.programs(), ["open"]);
        assert!(
            errors.as_ref().is_ok_and(|e| e.len() == 1 && e[0].contains("font_sise")),
            "{errors:?}"
        );
        assert!(
            edit("Nope", &env.paths, None, &mut rec.runner())
                .unwrap_err()
                .contains("no pin named")
        );
    }
}
