//! Dock pins: make any program its own macOS app.
//!
//! A pin is a small app bundle holding an APFS clone of this binary (no extra
//! disk space), its own bundle id (so macOS gives it its own Dock icon and
//! Cmd+Tab entry) and an optional icon. What it runs, and any config setting
//! it overrides, live in `~/.config/decline/pins/<id>.toml`, outside the
//! bundle, so editing them doesn't break the bundle's code signature. The
//! binary knows it's a pin from its own file name, which is the pin's id.

use std::path::{Path, PathBuf};
use std::process::Command;

const ID_PREFIX: &str = "red.forbes.decline.pin.";

/// The `Contents` directory of the bundle this binary runs from, if any.
fn own_contents_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents").then(|| contents.to_path_buf())
}

fn pins_dir() -> Option<PathBuf> {
    Some(crate::config::user_config_path()?.parent()?.join("pins"))
}

fn pin_config_path(id: &str) -> Option<PathBuf> {
    Some(pins_dir()?.join(format!("{id}.toml")))
}

/// When running as a pin: its config file.
pub fn own_pin_config() -> Option<PathBuf> {
    own_contents_dir()?;
    let exe = std::env::current_exe().ok()?;
    let id = exe.file_name()?.to_str()?;
    if id == "decline" {
        return None;
    }
    let p = pin_config_path(id)?;
    p.exists().then_some(p)
}

/// The app's display name when running from a bundle.
pub fn own_app_name() -> Option<String> {
    let app = own_contents_dir()?.parent()?.to_path_buf();
    Some(app.file_stem()?.to_string_lossy().into_owned())
}

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    s.trim_matches('-').to_string()
}

fn default_dir() -> PathBuf {
    Path::new(&std::env::var("HOME").unwrap_or_else(|_| ".".into())).join("Applications")
}

struct Options {
    name: String,
    icon: Option<PathBuf>,
    config: Option<PathBuf>,
    cwd: Option<PathBuf>,
    dir: PathBuf,
    dock: bool,
    command: Vec<String>,
}

pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => {
            println!("{}", crate::USAGE);
            return Ok(());
        }
        Some("--list") => return list(),
        Some("--update") => return update(),
        Some("--remove") => {
            let name = args.get(1).ok_or("--remove needs a name")?;
            let id = slug(name);
            let cfg = pin_config_path(&id).ok_or("no config directory")?;
            let app = pins()
                .into_iter()
                .find(|(i, _)| *i == id)
                .map(|(_, app)| app)
                .ok_or(format!("no pin named {name}"))?;
            if app.join("Contents/MacOS").join(&id).exists() {
                std::fs::remove_dir_all(&app).map_err(|e| e.to_string())?;
            }
            let _ = std::fs::remove_file(&cfg);
            println!("removed {}", app.display());
            return Ok(());
        }
        _ => {}
    }

    let mut o = Options {
        name: String::new(),
        icon: None,
        config: None,
        cwd: None,
        dir: default_dir(),
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
    if o.name.is_empty() || slug(&o.name).is_empty() {
        return Err("give the pin a name, e.g. `decline pin Rush -- ./rush`".into());
    }
    if o.command.is_empty() {
        return Err("give the program to run after --".into());
    }
    create(o)
}

/// Absolute path for a program given as a relative path (./rush); bare names
/// (htop) are left for PATH lookup at launch.
fn resolve_program(p: &str) -> Result<String, String> {
    if p.contains('/') {
        let abs = std::fs::canonicalize(p).map_err(|e| format!("{p}: {e}"))?;
        return Ok(abs.to_string_lossy().into_owned());
    }
    Ok(p.to_string())
}

fn create(mut o: Options) -> Result<(), String> {
    o.command[0] = resolve_program(&o.command[0])?;
    let slug = slug(&o.name);
    let app = o.dir.join(format!("{}.app", o.name));
    let config_path = pin_config_path(&slug).ok_or("no config directory")?;
    // Two pins with the same id would share a Dock identity.
    if let Some((_, other)) = pins().into_iter().find(|(id, other)| *id == slug && *other != app) {
        return Err(format!(
            "{} already uses the id {slug:?}; pick another name or remove it first",
            other.display()
        ));
    }
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos).map_err(|e| format!("{}: {e}", macos.display()))?;
    std::fs::create_dir_all(&resources).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(config_path.parent().unwrap()).map_err(|e| e.to_string())?;

    // pin.toml: the given config file, plus what to run.
    let mut table = match &o.config {
        Some(p) => std::fs::read_to_string(p)
            .map_err(|e| format!("{}: {e}", p.display()))?
            .parse::<toml::Table>()
            .map_err(|e| format!("{}: {e}", p.display()))?,
        None => toml::Table::new(),
    };
    table.insert(
        "command".into(),
        toml::Value::Array(o.command.iter().cloned().map(toml::Value::String).collect()),
    );
    table.entry("title").or_insert_with(|| toml::Value::String(o.name.clone()));
    table.insert("pin_app".into(), toml::Value::String(app.to_string_lossy().into_owned()));
    let cwd = match &o.cwd {
        Some(c) => Some(std::fs::canonicalize(c).map_err(|e| format!("{}: {e}", c.display()))?),
        // A relative program usually expects to run from where it lives.
        None if o.command[0].starts_with('/') => Path::new(&o.command[0]).parent().map(Path::to_path_buf),
        None => None,
    };
    if let Some(cwd) = cwd {
        table
            .entry("cwd")
            .or_insert_with(|| toml::Value::String(cwd.to_string_lossy().into_owned()));
    }
    let pin_toml = format!(
        "# Made by `decline pin`. Any config.toml setting can go here.\n{}",
        toml::to_string_pretty(&table).map_err(|e| e.to_string())?
    );
    std::fs::write(&config_path, pin_toml).map_err(|e| e.to_string())?;

    let has_icon = match &o.icon {
        Some(icon) => {
            make_icns(icon, &resources.join("icon.icns"))?;
            true
        }
        None => false,
    };
    std::fs::write(contents.join("Info.plist"), info_plist(&o.name, &slug, has_icon)).map_err(|e| e.to_string())?;
    install_binary(&macos.join(&slug))?;
    finish_bundle(&app, &slug)?;
    println!("created {}", app.display());
    println!("settings: {}", config_path.display());

    if o.dock {
        add_to_dock(&app)?;
        println!("added to the Dock");
    }
    Ok(())
}

/// Clone (APFS copy-on-write) this binary into the bundle. Written to a new
/// file and renamed into place: overwriting a signed binary that may be
/// running gets it killed on Apple Silicon.
fn install_binary(dest: &Path) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    let cloned = Command::new("/bin/cp")
        .arg("-c")
        .arg(&exe)
        .arg(&tmp)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !cloned {
        // Different volume (no clone possible): plain copy.
        std::fs::copy(&exe, &tmp).map_err(|e| format!("copying binary: {e}"))?;
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("installing binary: {e}"))?;
    Ok(())
}

/// Drop the download quarantine copied from decline itself, sign the bundle
/// with its own identifier (sealing its Info.plist), and register it with
/// LaunchServices.
fn finish_bundle(app: &Path, id: &str) -> Result<(), String> {
    let _ = Command::new("/usr/bin/xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(app)
        .status();
    let identifier = format!("{ID_PREFIX}{id}");
    let ok = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-", "--identifier", &identifier])
        .arg(app)
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("codesign failed".into());
    }
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let registered = Command::new(lsregister)
        .arg("-f")
        .arg(app)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !registered {
        eprintln!("warning: LaunchServices registration failed; the Dock may not pick up the icon until you open the app once");
    }
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn info_plist(name: &str, slug: &str, has_icon: bool) -> String {
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
fn make_icns(src: &Path, dest: &Path) -> Result<(), String> {
    if src.extension().is_some_and(|e| e == "icns") {
        std::fs::copy(src, dest).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let set = std::env::temp_dir().join(format!("decline-{}.iconset", std::process::id()));
    std::fs::create_dir_all(&set).map_err(|e| e.to_string())?;
    for size in [16, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let px = (size * scale).to_string();
            let out = set.join(format!("icon_{size}x{size}{suffix}.png"));
            let ok = Command::new("/usr/bin/sips")
                .args(["-s", "format", "png", "-z", &px, &px])
                .arg(src)
                .arg("--out")
                .arg(&out)
                .stdout(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err(format!("could not read image {}", src.display()));
            }
        }
    }
    let ok = Command::new("/usr/bin/iconutil")
        .args(["-c", "icns"])
        .arg(&set)
        .arg("-o")
        .arg(dest)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::fs::remove_dir_all(&set);
    if ok { Ok(()) } else { Err("iconutil failed".into()) }
}

fn add_to_dock(app: &Path) -> Result<(), String> {
    let url = format!("file://{}/", app.display());
    let tile = format!(
        "<dict><key>tile-data</key><dict><key>file-data</key><dict><key>_CFURLString</key><string>{}</string><key>_CFURLStringType</key><integer>15</integer></dict></dict></dict>",
        xml_escape(&url)
    );
    let ok = Command::new("/usr/bin/defaults")
        .args(["write", "com.apple.dock", "persistent-apps", "-array-add", &tile])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("could not update the Dock settings".into());
    }
    let _ = Command::new("/usr/bin/killall").arg("Dock").status();
    Ok(())
}

/// Every pin: (id, app path), from the config directory.
fn pins() -> Vec<(String, PathBuf)> {
    let Some(dir) = pins_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
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

fn list() -> Result<(), String> {
    for (id, app) in pins() {
        let cfg = pin_config_path(&id)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        let cmd = cfg
            .parse::<toml::Table>()
            .ok()
            .and_then(|t| t.get("command").cloned())
            .map(|c| c.to_string())
            .unwrap_or_default();
        println!("{}\t{cmd}", app.display());
    }
    Ok(())
}

/// After upgrading decline, refresh the binary inside every pin.
fn update() -> Result<(), String> {
    for (id, app) in pins() {
        let bin = app.join("Contents/MacOS").join(&id);
        if !bin.exists() {
            eprintln!("skipping {}: not found", app.display());
            continue;
        }
        install_binary(&bin)?;
        finish_bundle(&app, &id)?;
        println!("updated {}", app.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Rush"), "rush");
        assert_eq!(slug("My Tool!"), "my-tool");
    }

    #[test]
    fn plist_escapes_name() {
        let p = info_plist("A & B", "a-b", true);
        assert!(p.contains("A &amp; B"));
        assert!(p.contains("red.forbes.decline.pin.a-b"));
        assert!(p.contains("CFBundleIconFile"));
    }
}
