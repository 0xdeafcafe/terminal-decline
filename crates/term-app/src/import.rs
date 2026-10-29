//! `terminal-decline import`: settings from other terminals.
//!
//! - `terminal-decline import ghostty [PATH]` reads Ghostty's config (default
//!   `~/.config/ghostty/config`) and gives the Terminal Decline config for it:
//!   theme, palette and colours, fonts, opacity and blur, option-as-meta,
//!   copy-on-select, padding, window size, split colours and opacity, and
//!   keybinds, where Terminal Decline has the setting.
//! - `terminal-decline import iterm FILE.itermcolors` turns an iTerm2 colour preset
//!   into a theme file, `themes/<name>.toml`.
//!
//! Both print the TOML; `--write` also writes it, and an existing file is
//! only replaced with `--force`. Settings with no Terminal Decline equivalent
//! are listed as comments at the end.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::Paths;
use crate::keybind::Chord;

pub const USAGE: &str = "\
usage: terminal-decline import ghostty [PATH] [--write [--force]]
       terminal-decline import iterm FILE.itermcolors [--name NAME] [--write [--force]]

Prints the Terminal Decline config (ghostty) or theme (iterm) for another terminal's
settings. --write saves it (config.toml, or themes/NAME.toml); an existing
file is replaced only with --force.";

/// `terminal-decline import …` (the arguments after `import`).
pub fn run(args: &[String], paths: &Paths) -> Result<(), String> {
    let mut write = false;
    let mut force = false;
    let mut name = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--write" => write = true,
            "--force" => force = true,
            "--name" => {
                i += 1;
                name = Some(args.get(i).ok_or("--name needs a name")?.clone());
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            a if a.starts_with('-') => return Err(format!("unknown option {a:?}\n{USAGE}")),
            a => rest.push(a.to_string()),
        }
        i += 1;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    // The files to write, the config (or the theme) first.
    let files: Vec<(PathBuf, String)> = match rest.as_slice() {
        [kind, more @ ..] if kind == "ghostty" && more.len() <= 1 => {
            let path = more.first().map(PathBuf::from).unwrap_or_else(|| ghostty_config_path(&home));
            let config = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let import = ghostty(&config, &path, &ghostty_theme_dirs(&home));
            let mut files = vec![(paths.config_file(), import.config)];
            for (name, text) in import.themes {
                files.push((crate::theme::file_of(&paths.themes_dir(), &name), text));
            }
            files
        }
        [kind, file] if kind == "iterm" => {
            let path = PathBuf::from(file);
            let plist = read_plist(&path)?;
            let name = match name {
                Some(n) => n,
                None => theme_name_of(&path),
            };
            let theme = iterm(&plist, &path)?;
            vec![(crate::theme::file_of(&paths.themes_dir(), &name), theme)]
        }
        _ => return Err(USAGE.into()),
    };
    for (i, (target, text)) in files.iter().enumerate() {
        if files.len() > 1 {
            println!("{}# ---- {} ----", if i > 0 { "\n" } else { "" }, target.display());
        }
        print!("{text}");
    }
    if write {
        // Nothing is written if any file would be replaced without --force.
        if !force && let Some((target, _)) = files.iter().find(|(t, _)| t.exists()) {
            return Err(format!("{} exists; add --force to replace it", target.display()));
        }
        for (target, text) in &files {
            save(target, text, force)?;
            eprintln!("terminal-decline import: wrote {}", target.display());
        }
        if rest[0] == "iterm" {
            let name = files[0].0.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            eprintln!("terminal-decline import: use it with theme = {}", quote(&name));
        }
    }
    Ok(())
}

/// Writes `text` to `path`, refusing to replace a file unless `force`.
pub fn save(path: &Path, text: &str, force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!("{} exists; add --force to replace it", path.display()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn ghostty_dir(home: &Path) -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    base.join("ghostty")
}

/// Ghostty's config: the first of the places it reads that exists
/// (`~/.config/ghostty/config`, then its macOS folder; either named
/// `config.ghostty` or `config`), else the first.
fn ghostty_config_path(home: &Path) -> PathBuf {
    let mac = home.join("Library/Application Support/com.mitchellh.ghostty");
    let places = [
        ghostty_dir(home).join("config.ghostty"),
        ghostty_dir(home).join("config"),
        mac.join("config.ghostty"),
        mac.join("config"),
    ];
    places.iter().find(|p| p.is_file()).unwrap_or(&places[1]).clone()
}

/// Where Ghostty keeps themes: the user's, then the app's own.
fn ghostty_theme_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        ghostty_dir(home).join("themes"),
        PathBuf::from("/Applications/Ghostty.app/Contents/Resources/ghostty/themes"),
        home.join("Applications/Ghostty.app/Contents/Resources/ghostty/themes"),
    ]
}

/// A TOML string.
fn quote(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// `rrggbb` or `#rrggbb` as `#rrggbb`.
fn hex(v: &str) -> Option<String> {
    let v = v.trim();
    let h = v.strip_prefix('#').unwrap_or(v);
    (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", h.to_ascii_lowercase()))
}

/// Colours read from a config or theme, by Terminal Decline's names.
#[derive(Default, Debug, Clone, PartialEq)]
struct ColorSet {
    foreground: Option<String>,
    background: Option<String>,
    cursor: Option<String>,
    selection: Option<String>,
    split_divider: Option<String>,
    palette: BTreeMap<usize, String>,
}

impl ColorSet {
    fn is_empty(&self) -> bool {
        *self == ColorSet::default()
    }

    /// The lines of a `[colors]` table or theme file.
    fn write(&self, out: &mut String) {
        for (k, v) in [
            ("foreground", &self.foreground),
            ("background", &self.background),
            ("cursor", &self.cursor),
            ("selection", &self.selection),
            ("split_divider", &self.split_divider),
        ] {
            if let Some(v) = v {
                let _ = writeln!(out, "{k} = {}", quote(v));
            }
        }
        if let Some(&last) = self.palette.keys().last() {
            let entries: Vec<String> = (0..=last).map(|i| quote(self.palette.get(&i).map_or("", String::as_str))).collect();
            let _ = writeln!(out, "palette = [{}]", entries.join(", "));
        }
    }
}

/// Ghostty's `key = value` lines, in order, quotes taken off values.
fn ghostty_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v);
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

/// The colours a Ghostty config or theme sets; the rest of its lines are
/// left for the caller.
fn ghostty_colour(key: &str, value: &str, colors: &mut ColorSet) -> Option<Result<(), String>> {
    let slot = match key {
        "foreground" => &mut colors.foreground,
        "background" => &mut colors.background,
        "cursor-color" => &mut colors.cursor,
        "selection-background" => &mut colors.selection,
        "split-divider-color" => &mut colors.split_divider,
        "palette" => {
            let Some((n, c)) = value.split_once('=') else {
                return Some(Err("not N=COLOUR".into()));
            };
            let Ok(n) = n.trim().parse::<usize>() else {
                return Some(Err("not N=COLOUR".into()));
            };
            if n >= 16 {
                return Some(Err("Terminal Decline sets only the 16 ANSI colours".into()));
            }
            return Some(match hex(c) {
                Some(c) => {
                    colors.palette.insert(n, c);
                    Ok(())
                }
                None => Err("only #rrggbb colours are imported".into()),
            });
        }
        _ => return None,
    };
    Some(match hex(value) {
        Some(c) => {
            *slot = Some(c);
            Ok(())
        }
        None => Err("only #rrggbb colours are imported".into()),
    })
}

/// A theme name compared loosely: `Catppuccin Mocha`, `catppuccin-mocha`
/// and `catppuccin_mocha` match.
fn loose(name: &str) -> String {
    let l: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    l.strip_prefix("builtin").map(str::to_string).unwrap_or(l)
}

fn built_in_named(name: &str) -> Option<&'static str> {
    crate::theme::BUILT_IN.iter().map(|(n, _)| *n).find(|n| loose(n) == loose(name))
}

/// A Ghostty theme as a Terminal Decline theme file, from a path or the theme
/// folders.
fn ghostty_theme(name: &str, dirs: &[PathBuf]) -> Option<String> {
    let candidates: Vec<PathBuf> = if Path::new(name).is_absolute() {
        vec![PathBuf::from(name)]
    } else {
        dirs.iter().map(|d| d.join(name)).collect()
    };
    let text = candidates.iter().find_map(|p| std::fs::read_to_string(p).ok())?;
    let mut colors = ColorSet::default();
    let mut skipped = Vec::new();
    for (k, v) in ghostty_lines(&text) {
        match ghostty_colour(&k, &v, &mut colors) {
            Some(Ok(())) => {}
            Some(Err(why)) => skipped.push(format!("# {k} = {v}  ({why})")),
            None => skipped.push(format!("# {k} = {v}  (no Terminal Decline setting does this)")),
        }
    }
    let mut out = format!("# Imported from the Ghostty theme {name:?} by `terminal-decline import ghostty`.\n");
    colors.write(&mut out);
    if !skipped.is_empty() {
        out.push_str("\n# Not imported:\n");
        for line in skipped {
            out.push_str(&line);
            out.push('\n');
        }
    }
    Some(out)
}

/// A Terminal Decline theme name for a Ghostty one: `Harbour Lights` →
/// `harbour-lights`.
fn theme_file_name(name: &str) -> String {
    let base = Path::new(name).file_name().map_or(name.into(), |n| n.to_string_lossy());
    let words: Vec<String> = base
        .split(|c: char| !c.is_alphanumeric() && c != '+' && c != '_')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() { "imported".into() } else { words.join("-") }
}

/// What `terminal-decline import ghostty` writes: the config, and a theme
/// file for each Ghostty theme it names that isn't one of Terminal Decline's
/// (name, TOML).
#[derive(Debug)]
pub struct GhosttyImport {
    pub config: String,
    pub themes: Vec<(String, String)>,
}

/// A Ghostty key name as Terminal Decline's.
fn ghostty_key(key: &str) -> Option<String> {
    let k = key.strip_prefix("key_").or_else(|| key.strip_prefix("digit_")).unwrap_or(key);
    let k = k.strip_prefix("arrow_").unwrap_or(k);
    let named = match k {
        "equal" => "=",
        "minus" => "-",
        "plus" => "plus",
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "backslash" => "\\",
        "semicolon" => ";",
        "quote" | "apostrophe" => "'",
        "bracket_left" | "left_bracket" => "[",
        "bracket_right" | "right_bracket" => "]",
        "grave_accent" | "backquote" => "`",
        "page_up" | "page_down" | "enter" | "tab" | "escape" | "backspace" | "delete" | "insert" | "home" | "end" | "up" | "down"
        | "left" | "right" | "space" => k,
        k if k.chars().count() == 1 => k,
        k if k.starts_with('f') && k[1..].parse::<u8>().is_ok() => k,
        _ => return None,
    };
    Some(named.to_string())
}

/// A Ghostty trigger (`super+shift+left_bracket`) as a Terminal Decline chord.
fn ghostty_trigger(trigger: &str) -> Option<String> {
    let (mods, key) = if let Some(m) = trigger.strip_suffix("++") {
        (m, "+")
    } else {
        trigger.rsplit_once('+').unwrap_or(("", trigger))
    };
    let mut text = String::new();
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        let m = match m {
            "super" | "cmd" | "command" => "cmd",
            "ctrl" | "control" => "ctrl",
            "alt" | "opt" | "option" => "alt",
            "shift" => "shift",
            _ => return None,
        };
        text.push_str(m);
        text.push('+');
    }
    text.push_str(&ghostty_key(key)?);
    Chord::parse(&text).ok().map(|c| c.to_string())
}

/// Ghostty's string escapes (Zig's): `\x1b`, `\n`, `\r`, `\t`, `\\`,
/// `\"`, `\u{…}`.
fn unescape(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'e' => out.push('\x1b'),
            'x' => {
                let h: String = chars.by_ref().take(2).collect();
                out.push(char::from(u8::from_str_radix(&h, 16).ok()?));
            }
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let h: String = chars.by_ref().take_while(|&c| c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// A Ghostty action as the value of a Terminal Decline `[keybind]` entry.
fn ghostty_action(action: &str) -> Option<String> {
    let (name, arg) = action.split_once(':').unwrap_or((action, ""));
    let menu = match name {
        "new_window" => "new_window",
        "new_tab" => "new_tab",
        "close_surface" => "close",
        "close_tab" => "close_tab",
        "close_window" => "close_window",
        "copy_to_clipboard" => "copy",
        "paste_from_clipboard" => "paste",
        "select_all" => "select_all",
        "clear_screen" => "clear_scrollback",
        "increase_font_size" => "font_bigger",
        "decrease_font_size" => "font_smaller",
        "reset_font_size" => "font_reset",
        "toggle_fullscreen" => "toggle_full_screen",
        "next_tab" => "next_tab",
        "previous_tab" => "previous_tab",
        "start_search" => "find",
        "new_split" => match arg {
            "right" => "split_right",
            "down" => "split_down",
            _ => return None,
        },
        "goto_split" => match arg {
            "left" => "focus_pane_left",
            "right" => "focus_pane_right",
            "up" | "top" => "focus_pane_up",
            "down" | "bottom" => "focus_pane_down",
            "next" => "focus_pane_next",
            "previous" => "focus_pane_previous",
            _ => return None,
        },
        // `resize_split:left,10`: Ghostty's amount is in points; Terminal
        // Decline's keys move a divider one cell.
        "resize_split" => match arg.split_once(',').map_or(arg, |(d, _)| d) {
            "left" => "resize_pane_left",
            "right" => "resize_pane_right",
            "up" => "resize_pane_up",
            "down" => "resize_pane_down",
            _ => return None,
        },
        "equalize_splits" => "equalize_panes",
        // `jump_to_prompt:-1`: a count, its sign the way.
        "jump_to_prompt" => match arg.parse::<i32>().ok()? {
            n if n < 0 => "previous_prompt",
            n if n > 0 => "next_prompt",
            _ => return None,
        },
        "toggle_split_zoom" => "toggle_pane_zoom",
        "unbind" => "none",
        "text" => return Some(format!("{{ text = {} }}", quote(&unescape(arg)?))),
        "csi" => return Some(format!("{{ text = {} }}", quote(&format!("\x1b[{}", unescape(arg)?)))),
        "esc" => return Some(format!("{{ text = {} }}", quote(&format!("\x1b{}", unescape(arg)?)))),
        _ => return None,
    };
    Some(quote(menu))
}

/// A `keybind = trigger=action` value: the trigger ends at the first `=`
/// that isn't the key itself (`super+==increase_font_size:1`).
fn split_keybind(v: &str) -> Option<(&str, &str)> {
    let bytes = v.as_bytes();
    let at = (0..bytes.len()).find(|&i| bytes[i] == b'=' && i > 0 && bytes[i - 1] != b'+')?;
    Some((&v[..at], &v[at + 1..]))
}

/// A Ghostty `font-feature` entry: Terminal Decline's syntax (`liga`,
/// `+liga`, `-liga`, `liga=0`) or CSS's, which Ghostty also takes
/// (`"liga" off`, `liga on`, `cv05 2`).
fn ghostty_feature(entry: &str) -> Option<(term_font::Tag, u32)> {
    if let Some(f) = term_font::Tag::parse_feature(entry) {
        return Some(f);
    }
    let (tag, value) = entry.split_once(char::is_whitespace)?;
    let tag = term_font::Tag::new(tag.trim_matches(|c| c == '"' || c == '\''))?;
    let value = match value.trim() {
        "on" => 1,
        "off" => 0,
        n => n.parse().ok()?,
    };
    Some((tag, value))
}

/// A feature as `font_features` spells it: `liga`, `-liga` or `cv05=2`.
fn feature_text((tag, value): (term_font::Tag, u32)) -> String {
    match value {
        0 => format!("-{tag}"),
        1 => tag.to_string(),
        n => format!("{tag}={n}"),
    }
}

/// The Terminal Decline config for Ghostty config `text` (read from `path`),
/// with themes looked for in `theme_dirs`.
pub fn ghostty(text: &str, path: &Path, theme_dirs: &[PathBuf]) -> GhosttyImport {
    let mut top: Vec<(&str, String)> = Vec::new();
    let set = |k: &'static str, v: String, top: &mut Vec<(&str, String)>| {
        top.retain(|(key, _)| *key != k);
        top.push((k, v));
    };
    let mut fonts: Vec<String> = Vec::new();
    let mut features: Vec<String> = Vec::new();
    let mut themes: Vec<(String, String)> = Vec::new();
    // A theme by Terminal Decline's name: built in, else a Ghostty theme made
    // into a theme file.
    let mut theme_named = |name: &str| -> Option<String> {
        if let Some(b) = built_in_named(name) {
            return Some(b.to_string());
        }
        let text = ghostty_theme(name, theme_dirs)?;
        let file = theme_file_name(name);
        themes.retain(|(n, _)| *n != file);
        themes.push((file.clone(), text));
        Some(file)
    };
    let mut colors = ColorSet::default();
    let mut keybinds: Vec<(String, String)> = Vec::new();
    let mut skipped: Vec<(String, String, String)> = Vec::new();
    let mut padding_x = None;
    for (key, value) in ghostty_lines(text) {
        let mut skip = |why: &str| skipped.push((key.clone(), value.clone(), why.to_string()));
        if let Some(r) = ghostty_colour(&key, &value, &mut colors) {
            if let Err(why) = r {
                skip(&why);
            }
            continue;
        }
        let number = value.parse::<f64>().ok().filter(|n| n.is_finite());
        match key.as_str() {
            "theme" => {
                let pair = value
                    .split(',')
                    .map(str::trim)
                    .fold((None, None, None), |(l, d, one), part| match part.split_once(':') {
                        Some(("light", n)) => (Some(n.trim().to_string()), d, one),
                        Some(("dark", n)) => (l, Some(n.trim().to_string()), one),
                        _ => (l, d, Some(part.to_string())),
                    });
                match pair {
                    (Some(l), Some(d), None) => match (theme_named(&l), theme_named(&d)) {
                        (Some(l), Some(d)) => set("theme", format!("{{ light = {}, dark = {} }}", quote(&l), quote(&d)), &mut top),
                        _ => skip("no built-in Terminal Decline theme or Ghostty theme file for one of these"),
                    },
                    (None, None, Some(name)) => match theme_named(&name) {
                        Some(t) => set("theme", quote(&t), &mut top),
                        None => skip("no built-in Terminal Decline theme or Ghostty theme file of that name"),
                    },
                    _ => skip("not a theme name or light:NAME,dark:NAME"),
                }
            }
            "font-family" if value.is_empty() => fonts.clear(),
            "font-family" => fonts.push(value.clone()),
            // Repeatable; empty clears the list. One line may hold several,
            // comma-separated.
            "font-feature" if value.is_empty() => features.clear(),
            "font-feature" => {
                for entry in value.split(',').map(str::trim).filter(|e| !e.is_empty()) {
                    match ghostty_feature(entry) {
                        Some(f) => {
                            features.retain(|old| term_font::Tag::parse_feature(old).map(|(t, _)| t) != Some(f.0));
                            features.push(feature_text(f));
                        }
                        None => skipped.push((key.clone(), entry.to_string(), "not a font feature".to_string())),
                    }
                }
            }
            "font-family-bold" => set("font_bold", quote(&value), &mut top),
            "font-family-italic" => set("font_italic", quote(&value), &mut top),
            "font-family-bold-italic" => set("font_bold_italic", quote(&value), &mut top),
            "font-size" => match number.filter(|n| *n > 0.0) {
                Some(n) => set("font_size", n.to_string(), &mut top),
                None => skip("not a size"),
            },
            "adjust-cell-height" => match value.strip_suffix('%').and_then(|p| p.trim().parse::<f64>().ok()) {
                Some(p) if p > -100.0 => set("line_height", ((100.0 + p) / 100.0).to_string(), &mut top),
                _ => skip("only a percentage maps (to line_height)"),
            },
            "adjust-cell-width" => match value.strip_suffix('%').and_then(|p| p.trim().parse::<f64>().ok()) {
                Some(p) => set("cell_width", quote(&format!("{p}%")), &mut top),
                _ => skip("only a percentage maps (Ghostty's other unit is pixels, Terminal Decline's points)"),
            },
            "background-opacity" => match number {
                Some(n) => set("opacity", n.clamp(0.0, 1.0).to_string(), &mut top),
                None => skip("not a number"),
            },
            "background-blur" | "background-blur-radius" => match value.as_str() {
                "true" => set("blur", "true".into(), &mut top),
                "false" | "0" => set("blur", "false".into(), &mut top),
                _ if number.is_some_and(|n| n > 0.0) => set("blur", "true".into(), &mut top),
                _ => skip("not a blur Terminal Decline has"),
            },
            "macos-option-as-alt" => match value.as_str() {
                "true" => set("option_as_meta", quote("both"), &mut top),
                "false" => set("option_as_meta", quote("none"), &mut top),
                "left" | "right" => set("option_as_meta", quote(&value), &mut top),
                _ => skip("not true, false, left or right"),
            },
            "copy-on-select" => match value.as_str() {
                "true" | "clipboard" => set("copy_on_select", "true".into(), &mut top),
                "false" => set("copy_on_select", "false".into(), &mut top),
                _ => skip("not true, false or clipboard"),
            },
            "window-padding-x" => match number {
                Some(n) if n >= 0.0 => {
                    padding_x = Some(n);
                    set("padding", n.to_string(), &mut top);
                }
                _ => skip("Terminal Decline has one padding for all sides"),
            },
            "window-padding-y" => {
                if number.is_none() || number != padding_x {
                    skip("Terminal Decline has one padding for all sides (window-padding-x is used)");
                }
            }
            "unfocused-split-opacity" => match number {
                // Ghostty's own range.
                Some(n) => set("unfocused_pane_opacity", n.clamp(0.15, 1.0).to_string(), &mut top),
                None => skip("not a number"),
            },
            "window-width" => match value.parse::<u16>() {
                Ok(n) if n > 0 => set("columns", n.to_string(), &mut top),
                _ => skip("not a number of columns"),
            },
            "window-height" => match value.parse::<u16>() {
                Ok(n) if n > 0 => set("lines", n.to_string(), &mut top),
                _ => skip("not a number of lines"),
            },
            "keybind" => {
                let Some((trigger, action)) = split_keybind(&value) else {
                    skip("not TRIGGER=ACTION");
                    continue;
                };
                let trigger = trigger.strip_prefix("performable:").unwrap_or(trigger);
                if trigger.contains('>') {
                    skip("key sequences aren't supported");
                } else if trigger.contains(':') {
                    skip("global, all and unconsumed keybinds aren't supported");
                } else {
                    match (ghostty_trigger(trigger), ghostty_action(action)) {
                        (None, _) => skip("a key Terminal Decline can't bind"),
                        (_, None) => skip("no Terminal Decline action does this"),
                        (Some(chord), Some(act)) => {
                            keybinds.retain(|(c, _)| *c != chord);
                            keybinds.push((chord, act));
                        }
                    }
                }
            }
            "scrollback-limit" => skip("Ghostty counts bytes, Terminal Decline lines (set scrollback)"),
            "command" => skip("Terminal Decline's command is a list: command = [\"program\", \"arg\"]"),
            "config-file" => skip("other files aren't followed; import them too"),
            _ => skip("no Terminal Decline setting does this"),
        }
    }
    if let Some((first, rest)) = fonts.split_first() {
        set("font_family", quote(first), &mut top);
        if !rest.is_empty() {
            let list: Vec<String> = rest.iter().map(|f| quote(f)).collect();
            set("font_fallback", format!("[{}]", list.join(", ")), &mut top);
        }
    }
    if !features.is_empty() {
        let list: Vec<String> = features.iter().map(|f| quote(f)).collect();
        set("font_features", format!("[{}]", list.join(", ")), &mut top);
    }

    let mut out = format!("# Imported from {} by `terminal-decline import ghostty`.\n", path.display());
    const ORDER: [&str; 19] = [
        "theme",
        "font_family",
        "font_fallback",
        "font_features",
        "font_bold",
        "font_italic",
        "font_bold_italic",
        "font_size",
        "line_height",
        "cell_width",
        "padding",
        "opacity",
        "blur",
        "unfocused_pane_opacity",
        "option_as_meta",
        "copy_on_select",
        "columns",
        "lines",
        "scrollback",
    ];
    for k in ORDER {
        if let Some((_, v)) = top.iter().find(|(key, _)| *key == k) {
            let _ = writeln!(out, "{k} = {v}");
        }
    }
    if !colors.is_empty() {
        out.push_str("\n[colors]\n");
        colors.write(&mut out);
    }
    if !keybinds.is_empty() {
        out.push_str("\n[keybind]\n");
        for (chord, act) in &keybinds {
            let _ = writeln!(out, "{} = {act}", quote(chord));
        }
    }
    if !skipped.is_empty() {
        out.push_str("\n# Not imported:\n");
        for (k, v, why) in &skipped {
            let _ = writeln!(out, "# {k} = {v}  ({why})");
        }
    }
    GhosttyImport { config: out, themes }
}

/// A property list value, as much of it as colour presets use.
#[derive(Debug, Clone, PartialEq)]
pub enum Plist {
    Dict(BTreeMap<String, Plist>),
    Array(Vec<Plist>),
    Real(f64),
    Str(String),
    Bool(bool),
}

/// Reads a plist file, XML or binary (converted with `plutil`).
fn read_plist(path: &Path) -> Result<Plist, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = if bytes.starts_with(b"bplist") {
        let out = std::process::Command::new("/usr/bin/plutil")
            .args(["-convert", "xml1", "-o", "-"])
            .arg(path)
            .output()
            .map_err(|e| format!("plutil: {e}"))?;
        if !out.status.success() {
            return Err(format!("{}: not a property list", path.display()));
        }
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    parse_plist(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Parses an XML property list.
pub fn parse_plist(text: &str) -> Result<Plist, String> {
    let mut p = XmlParser { s: text, at: 0 };
    loop {
        match p.tag()? {
            Some(Tag::Open(name)) if name == "plist" => break,
            Some(_) => {}
            None => return Err("no <plist>".into()),
        }
    }
    match p.tag()? {
        Some(Tag::Open(name)) => p.value(&name),
        _ => Err("empty <plist>".into()),
    }
}

enum Tag {
    Open(String),
    Close(String),
    Empty(String),
}

struct XmlParser<'a> {
    s: &'a str,
    at: usize,
}

fn unentity(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

impl XmlParser<'_> {
    /// The next tag, skipping text, comments, `<?…?>` and `<!…>`.
    fn tag(&mut self) -> Result<Option<Tag>, String> {
        loop {
            let Some(open) = self.s[self.at..].find('<') else { return Ok(None) };
            self.at += open;
            let rest = &self.s[self.at..];
            if rest.starts_with("<!--") {
                let end = rest.find("-->").ok_or("unclosed comment")?;
                self.at += end + 3;
                continue;
            }
            let end = rest.find('>').ok_or("unclosed tag")?;
            let inner = &rest[1..end];
            self.at += end + 1;
            if inner.starts_with('?') || inner.starts_with('!') {
                continue;
            }
            let name_of = |s: &str| s.split_whitespace().next().unwrap_or("").to_string();
            return Ok(Some(if let Some(n) = inner.strip_prefix('/') {
                Tag::Close(name_of(n))
            } else if let Some(n) = inner.strip_suffix('/') {
                Tag::Empty(name_of(n))
            } else {
                Tag::Open(name_of(inner))
            }));
        }
    }

    /// The text up to `</name>`.
    fn text(&mut self, name: &str) -> Result<String, String> {
        let close = format!("</{name}>");
        let end = self.s[self.at..].find(&close).ok_or_else(|| format!("unclosed <{name}>"))?;
        let t = unentity(self.s[self.at..self.at + end].trim());
        self.at += end + close.len();
        Ok(t)
    }

    /// The value whose opening tag `<name>` was just read.
    fn value(&mut self, name: &str) -> Result<Plist, String> {
        match name {
            "dict" => {
                let mut map = BTreeMap::new();
                loop {
                    match self.tag()? {
                        Some(Tag::Close(n)) if n == "dict" => return Ok(Plist::Dict(map)),
                        Some(Tag::Open(n)) if n == "key" => {
                            let key = self.text("key")?;
                            let v = match self.tag()? {
                                Some(Tag::Open(n)) => self.value(&n)?,
                                Some(Tag::Empty(n)) => empty_value(&n),
                                _ => return Err(format!("no value for key {key:?}")),
                            };
                            map.insert(key, v);
                        }
                        _ => return Err("bad <dict>".into()),
                    }
                }
            }
            "array" => {
                let mut items = Vec::new();
                loop {
                    match self.tag()? {
                        Some(Tag::Close(n)) if n == "array" => return Ok(Plist::Array(items)),
                        Some(Tag::Open(n)) => items.push(self.value(&n)?),
                        Some(Tag::Empty(n)) => items.push(empty_value(&n)),
                        _ => return Err("bad <array>".into()),
                    }
                }
            }
            "real" | "integer" => {
                let t = self.text(name)?;
                t.parse()
                    .map(Plist::Real)
                    .map_err(|_| format!("<{name}>{t}</{name}> is not a number"))
            }
            other => Ok(Plist::Str(self.text(other)?)),
        }
    }
}

fn empty_value(name: &str) -> Plist {
    match name {
        "true" => Plist::Bool(true),
        "false" => Plist::Bool(false),
        "dict" => Plist::Dict(BTreeMap::new()),
        "array" => Plist::Array(Vec::new()),
        _ => Plist::Str(String::new()),
    }
}

/// A theme name from a preset's file name: `Dracula+.itermcolors` →
/// `dracula+`, spaces as dashes.
fn theme_name_of(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "imported".into());
    stem.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-")
}

/// An iTerm2 colour as `#rrggbb` in sRGB, converting from the colour space
/// the preset names (older presets, without one, are calibrated RGB).
fn iterm_colour(entry: &Plist) -> Option<String> {
    let Plist::Dict(d) = entry else { return None };
    let comp = |k: &str| match d.get(k) {
        Some(Plist::Real(v)) => Some(v.clamp(0.0, 1.0)),
        _ => None,
    };
    let (r, g, b) = (comp("Red Component")?, comp("Green Component")?, comp("Blue Component")?);
    let space = match d.get("Color Space") {
        Some(Plist::Str(s)) => s.as_str(),
        _ => "Calibrated",
    };
    let (r, g, b) = to_srgb(space, r, g, b);
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b)))
}

/// Components in colour space `space` (iTerm2's names: sRGB, Calibrated,
/// P3, Device) as sRGB, by AppKit.
fn to_srgb(space: &str, r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    use objc2_app_kit::{NSColor, NSColorSpace};
    let color = match space {
        "sRGB" => return (r, g, b),
        "P3" => NSColor::colorWithDisplayP3Red_green_blue_alpha(r, g, b, 1.0),
        "Device" => NSColor::colorWithDeviceRed_green_blue_alpha(r, g, b, 1.0),
        _ => NSColor::colorWithCalibratedRed_green_blue_alpha(r, g, b, 1.0),
    };
    match color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) {
        Some(c) => (c.redComponent(), c.greenComponent(), c.blueComponent()),
        None => (r, g, b),
    }
}

/// The Terminal Decline theme file for an iTerm2 colour preset (read from
/// `path`).
pub fn iterm(plist: &Plist, path: &Path) -> Result<String, String> {
    let Plist::Dict(entries) = plist else {
        return Err("not a colour preset (no dictionary of colours)".into());
    };
    let mut colors = ColorSet::default();
    let mut skipped = Vec::new();
    for (key, value) in entries {
        let Some(c) = iterm_colour(value) else {
            skipped.push(format!("# {key}: not a colour"));
            continue;
        };
        let ansi = key
            .strip_prefix("Ansi ")
            .and_then(|k| k.strip_suffix(" Color"))
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|&n| n < 16);
        match (key.as_str(), ansi) {
            (_, Some(n)) => {
                colors.palette.insert(n, c);
            }
            ("Foreground Color", _) => colors.foreground = Some(c),
            ("Background Color", _) => colors.background = Some(c),
            ("Cursor Color", _) => colors.cursor = Some(c),
            ("Selection Color", _) => colors.selection = Some(c),
            _ => skipped.push(format!("# {key} = {c}  (no Terminal Decline setting does this)")),
        }
    }
    if colors.is_empty() {
        return Err("no colours Terminal Decline uses in this preset".into());
    }
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = format!("# Imported from the iTerm2 colour preset {name:?} by `terminal-decline import iterm`.\n");
    colors.write(&mut out);
    if !skipped.is_empty() {
        out.push_str("\n# Not imported:\n");
        for s in skipped {
            out.push_str(&s);
            out.push('\n');
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{self, Palette};
    use crate::testutil::TempDir;
    use crate::theme::Appearance;

    const GHOSTTY: &str = include_str!("../tests/fixtures/ghostty-config");
    const ITERM: &str = include_str!("../tests/fixtures/sample.itermcolors");

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    /// `text` as a config file: what Terminal Decline makes of it, and its
    /// errors. What Terminal Decline makes of an import, written out, and its
    /// errors.
    fn load_import(import: &GhosttyImport) -> (config::Config, Vec<String>) {
        let dir = TempDir::new("import");
        let file = dir.join("config.toml");
        std::fs::write(&file, &import.config).unwrap();
        for (name, text) in &import.themes {
            save(&crate::theme::file_of(&dir.join("themes"), name), text, false).unwrap();
        }
        config::load(&[&file], &dir.join("themes"))
    }

    fn load(text: &str) -> (config::Config, Vec<String>) {
        load_import(&GhosttyImport {
            config: text.into(),
            themes: Vec::new(),
        })
    }

    #[test]
    fn ghostty_settings_map() {
        let import = ghostty(GHOSTTY, Path::new("/x/ghostty/config"), &[fixtures().join("ghostty-themes")]);
        let out = &import.config;
        let (c, errors) = load_import(&import);
        assert!(errors.is_empty(), "{errors:?}\n{out}");
        assert!(out.contains("\ntheme = \"harbour-lights\"\n"), "{out}");
        assert_eq!(import.themes.len(), 1);
        let (name, theme) = &import.themes[0];
        assert_eq!(name, "harbour-lights");
        assert!(theme.starts_with("# Imported from the Ghostty theme \"Harbour Lights\""), "{theme}");
        assert!(
            theme.contains("# selection-foreground = ffffff  (no Terminal Decline setting does this)"),
            "{theme}"
        );
        assert_eq!(c.font_family, "JetBrains Mono");
        assert_eq!(c.font_fallback, ["Symbols Nerd Font Mono"]);
        assert_eq!(c.font_bold.as_deref(), Some("JetBrains Mono ExtraBold"));
        assert_eq!(c.font_size, 14.5);
        assert_eq!(c.font_features, ["zero", "ss01", "cv05=2", "calt"], "the last calt wins");
        assert_eq!(c.line_height, 1.2);
        assert_eq!(c.cell_width, term_font::CellWidth::Percent(5.0));
        assert_eq!(c.opacity, 0.92);
        assert!(c.blur && c.copy_on_select);
        assert_eq!(c.option_as_meta, config::OptionAsMeta::Left);
        assert_eq!(c.padding, 8.0);
        assert_eq!((c.columns, c.lines), (120, 36));
        // The Ghostty theme's colours, with the config's on top.
        let p = c.palette(Appearance::Dark);
        assert_eq!(p.background, 0x0b0f14, "from the theme");
        assert_eq!(p.foreground, 0xcdd6f4, "the config's own");
        assert_eq!(p.cursor, 0xf5e0dc);
        assert_eq!(p.selection, 0x353749);
        assert_eq!(p.ansi[0], 0x101418);
        assert_eq!(p.ansi[1], 0xff5555, "the config's palette wins");
        assert_eq!(p.ansi[2], 0x27ae60);
        assert_eq!(p.ansi[3], Palette::DEFAULT.ansi[3], "unset stays Terminal Decline's");
        assert_eq!(p.ansi[9], 0xff6e6e);
        assert_eq!(p.ansi[15], 0xfafafa);
        // Keybinds.
        use crate::keybind::{Bound, Chord, MenuAction};
        let b = c.keybinds();
        let get = |s: &str| b.get(&Chord::parse(s).unwrap()).cloned();
        assert_eq!(get("cmd+k"), Some(Bound::Menu(MenuAction::ClearScrollback)));
        assert_eq!(get("cmd+shift+["), Some(Bound::Menu(MenuAction::PreviousTab)));
        assert_eq!(get("cmd+="), Some(Bound::Menu(MenuAction::FontBigger)));
        assert_eq!(get("ctrl+shift+t"), Some(Bound::Text("\x1b[116;6u".into())));
        assert_eq!(get("alt+left"), Some(Bound::Text("\x1bb".into())));
        assert_eq!(get("cmd+d"), None, "unbind");
        assert_eq!(get("cmd+t"), Some(Bound::Menu(MenuAction::NewTab)));
        // Splits.
        for (chord, action) in [
            ("cmd+shift+enter", MenuAction::TogglePaneZoom),
            ("cmd+shift+d", MenuAction::SplitDown),
            ("cmd+alt+left", MenuAction::FocusPaneLeft),
            ("cmd+]", MenuAction::FocusPaneNext),
            ("cmd+ctrl+up", MenuAction::ResizePaneUp),
            ("cmd+ctrl+=", MenuAction::EqualizePanes),
            ("cmd+shift+w", MenuAction::CloseWindow),
        ] {
            assert_eq!(get(chord), Some(Bound::Menu(action)), "{chord}");
        }
        assert_eq!(get("cmd+ctrl+d"), None, "no new pane to the left");
        assert_eq!(p.divider(), 0x44475a);
        assert_eq!(c.unfocused_pane_opacity, 0.7);
        // What doesn't map is listed.
        let notes = &out[out.find("# Not imported:").expect(out)..];
        for k in [
            "font-thicken = true",
            "font-feature = nope!",
            "cursor-text = #11111b",
            "palette = 200=#123456",
            "scrollback-limit = 10000000",
            "command = /opt/homebrew/bin/fish",
            "keybind = ctrl+a>n=next_tab",
            "keybind = global:ctrl+grave_accent=toggle_quick_terminal",
            "keybind = super+ctrl+d=new_split:left",
        ] {
            assert!(notes.contains(&format!("# {k}  (")), "{k} missing:\n{notes}");
        }
        assert_eq!(notes.lines().count(), 10, "{notes}");
    }

    #[test]
    fn ghostty_themes_map_to_built_in_ones() {
        let out = ghostty("theme = Catppuccin Mocha\n", Path::new("c"), &[]).config;
        assert!(out.contains("\ntheme = \"catppuccin-mocha\"\n"), "{out}");
        let out = ghostty("theme = light:Solarized Light,dark:TokyoNight\n", Path::new("c"), &[]).config;
        assert!(
            out.contains("theme = { light = \"solarized-light\", dark = \"tokyo-night\" }"),
            "{out}"
        );
        let (c, errors) = load(&out);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.palette(Appearance::Light).background, 0xfdf6e3);
        // A pair of Ghostty's own themes become theme files.
        let dirs = [fixtures().join("ghostty-themes")];
        let import = ghostty("theme = light:Solarized Light,dark:Harbour Lights\n", Path::new("c"), &dirs);
        assert!(
            import
                .config
                .contains("theme = { light = \"solarized-light\", dark = \"harbour-lights\" }"),
            "{}",
            import.config
        );
        let (c, errors) = load_import(&import);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.palette(Appearance::Dark).background, 0x0b0f14);
        let out = ghostty("theme = light:Nope,dark:TokyoNight\ntheme = Unknown Theme\n", Path::new("c"), &[]).config;
        assert!(!out.contains("\ntheme ="), "{out}");
        assert!(out.contains("# theme = Unknown Theme  (no built-in"), "{out}");
        assert!(out.contains("# theme = light:Nope,dark:TokyoNight  ("), "{out}");
    }

    #[test]
    fn ghostty_keys_and_text() {
        assert_eq!(ghostty_trigger("super+shift+arrow_up").as_deref(), Some("shift+cmd+up"));
        assert_eq!(ghostty_trigger("ctrl+digit_1").as_deref(), Some("ctrl+1"));
        assert_eq!(ghostty_trigger("super+plus").as_deref(), Some("cmd+plus"));
        assert_eq!(ghostty_trigger("hyper+a"), None);
        assert_eq!(ghostty_trigger("super+numpad_add"), None);
        assert_eq!(split_keybind("super+==x"), Some(("super+=", "x")));
        assert_eq!(split_keybind("a=b=c"), Some(("a", "b=c")));
        assert_eq!(unescape(r"\x1b[A\u{1F600}\n").as_deref(), Some("\x1b[A\u{1F600}\n"));
        assert_eq!(unescape(r"\q"), None);
        assert_eq!(ghostty_action("csi:A").as_deref(), Some("{ text = \"\\u001B[A\" }"));
        assert_eq!(ghostty_action("goto_tab:1"), None);
        for (ghostty, ours) in [
            ("new_split:right", Some("split_right")),
            ("new_split:down", Some("split_down")),
            ("new_split:auto", None),
            ("goto_split:previous", Some("focus_pane_previous")),
            ("goto_split:top", Some("focus_pane_up")),
            ("goto_split:bottom", Some("focus_pane_down")),
            ("resize_split:right,20", Some("resize_pane_right")),
            ("resize_split:down", Some("resize_pane_down")),
            ("resize_split:sideways,3", None),
            ("equalize_splits", Some("equalize_panes")),
            ("toggle_split_zoom", Some("toggle_pane_zoom")),
            ("jump_to_prompt:-1", Some("previous_prompt")),
            ("jump_to_prompt:2", Some("next_prompt")),
            ("jump_to_prompt:0", None),
            ("close_surface", Some("close")),
            ("close_tab", Some("close_tab")),
        ] {
            assert_eq!(ghostty_action(ghostty), ours.map(quote), "{ghostty}");
        }
    }

    #[test]
    fn plist_parses() {
        let p = parse_plist(ITERM).unwrap();
        let Plist::Dict(d) = &p else { panic!("{p:?}") };
        assert_eq!(d.len(), 9);
        let Some(Plist::Dict(red)) = d.get("Ansi 1 Color") else { panic!() };
        assert_eq!(red.get("Red Component"), Some(&Plist::Real(0.8)));
        assert_eq!(red.get("Color Space"), Some(&Plist::Str("sRGB".into())));
        let small =
            "<plist><dict><key>a &amp; b</key><true/><key>l</key><array><integer>1</integer><string>x</string></array></dict></plist>";
        let Plist::Dict(d) = parse_plist(small).unwrap() else { panic!() };
        assert_eq!(d.get("a & b"), Some(&Plist::Bool(true)));
        assert_eq!(d.get("l"), Some(&Plist::Array(vec![Plist::Real(1.0), Plist::Str("x".into())])));
        assert!(parse_plist("<dict></dict>").is_err());
        assert!(parse_plist("<plist><dict><key>a</key>").is_err());
    }

    #[test]
    fn iterm_presets_become_themes() {
        let out = iterm(&parse_plist(ITERM).unwrap(), Path::new("Sample.itermcolors")).unwrap();
        let c = crate::theme::parse(&out).unwrap_or_else(|e| panic!("{e}\n{out}"));
        assert_eq!(c.background.as_deref(), Some("#101820"));
        assert_eq!(c.foreground.as_deref(), Some("#ccddee"));
        assert_eq!(c.selection.as_deref(), Some("#334d66"));
        assert_eq!(c.palette.len(), 16);
        assert_eq!(c.palette[0], "#000000");
        assert_eq!(c.palette[1], "#cc3333");
        assert_eq!(c.palette[2], "", "unset: the default's");
        assert_eq!(c.palette[15], "#ffffff");
        // Calibrated grey, converted to sRGB: still a mid grey.
        let cursor = config::parse_hex(c.cursor.as_deref().unwrap()).unwrap();
        let (r, g, b) = (cursor >> 16, cursor >> 8 & 0xff, cursor & 0xff);
        assert!(r == g && g == b && (0x80..=0xa0).contains(&r), "{cursor:06x}");
        assert!(
            out.contains("# Bold Color = #ffffff  (no Terminal Decline setting does this)"),
            "{out}"
        );
        assert!(out.contains("# Cursor Text Color = #000000  ("), "{out}");
        assert!(iterm(&parse_plist("<plist><dict></dict></plist>").unwrap(), Path::new("x")).is_err());
    }

    #[test]
    fn writing_never_replaces_without_force() {
        let dir = TempDir::new("import-save");
        let file = dir.join("themes/x.toml");
        save(&file, "a = 1\n", false).unwrap();
        let e = save(&file, "a = 2\n", false).unwrap_err();
        assert!(e.ends_with("exists; add --force to replace it"), "{e}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a = 1\n");
        save(&file, "a = 2\n", true).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a = 2\n");
        assert_eq!(
            theme_name_of(Path::new("/a/Solarized Dark Higher.itermcolors")),
            "solarized-dark-higher"
        );
    }

    #[test]
    fn the_command_writes_where_it_should() {
        let dir = TempDir::new("import-run");
        let paths = Paths {
            config_dir: dir.join("terminal-decline"),
            apps_dir: dir.join("Applications"),
            exe: PathBuf::from("terminal-decline"),
        };
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let iterm = fixtures().join("sample.itermcolors");
        let iterm = iterm.to_str().unwrap();
        run(&args(&["iterm", iterm]), &paths).unwrap();
        assert!(!paths.themes_dir().exists(), "printed only");
        run(&args(&["iterm", iterm, "--write"]), &paths).unwrap();
        assert!(paths.themes_dir().join("sample.toml").exists());
        assert!(run(&args(&["iterm", iterm, "--write"]), &paths).unwrap_err().contains("--force"));
        run(&args(&["iterm", iterm, "--write", "--name", "mine"]), &paths).unwrap();
        assert!(paths.themes_dir().join("mine.toml").exists());
        let ghostty = fixtures().join("ghostty-config");
        // Its theme comes from a Ghostty themes folder the test can't point
        // at, so it is listed as not imported: only the config is written.
        run(&args(&["ghostty", ghostty.to_str().unwrap(), "--write"]), &paths).unwrap();
        assert!(paths.config_file().exists());
        assert!(run(&args(&["ghostty", ghostty.to_str().unwrap(), "--write"]), &paths).is_err());
        run(&args(&["ghostty", ghostty.to_str().unwrap(), "--write", "--force"]), &paths).unwrap();
        assert!(run(&args(&["nope"]), &paths).is_err());
        assert!(run(&args(&["iterm"]), &paths).is_err());
    }
}
