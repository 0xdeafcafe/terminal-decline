//! What the app does with files and folders from the rest of macOS:
//! dragged onto a window, dropped on the Dock icon (or `open -a`), or sent
//! by a Services menu item. The AppKit side is in `app` and `view`; this is
//! the logic they share, tested on its own.

use std::path::{Path, PathBuf};

/// `path` quoted for a shell, the way Terminal.app does it: characters a
/// shell treats specially get a backslash (`My\ File.txt`). Control
/// characters (a newline in a file name) can't be typed as they are, since
/// a paste drops them, so a name with one is written as `$'…'` instead,
/// which bash and zsh read.
pub fn shell_quote(path: &str) -> String {
    if path.is_empty() {
        return "''".into();
    }
    if path.chars().any(char::is_control) {
        let mut out = String::from("$'");
        for c in path.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '\'' => out.push_str("\\'"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                c if c.is_control() => {
                    // Each UTF-8 byte as \xHH (C1 controls take two).
                    let mut buf = [0u8; 4];
                    for b in c.encode_utf8(&mut buf).bytes() {
                        out.push_str(&format!("\\x{b:02x}"));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('\'');
        return out;
    }
    let mut out = String::with_capacity(path.len() + 4);
    for c in path.chars() {
        if !is_plain(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Characters no shell gives a meaning to, anywhere in a word.
fn is_plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ',' | ':' | '@' | '%') || (!c.is_ascii() && !c.is_whitespace())
}

/// The text typed for dropped files: each path quoted, separated by
/// spaces.
pub fn dropped_text<S: AsRef<str>>(paths: &[S]) -> String {
    paths.iter().map(|p| shell_quote(p.as_ref())).collect::<Vec<_>>().join(" ")
}

/// The folder a window opens in for something opened with the app: a
/// folder is itself, a file the folder it is in. `None` if it doesn't
/// exist.
pub fn folder_for(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_dir() {
        Some(path.to_path_buf())
    } else {
        path.parent().map(Path::to_path_buf)
    }
}

/// A new window or a new tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    Window,
    Tab,
}

/// macOS's "Prefer tabs when opening documents" (Desktop & Dock), as
/// `NSWindow.userTabbingPreference` gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPreference {
    /// "Never".
    Manual,
    Always,
    /// "In Full Screen" (the default).
    InFullScreen,
}

/// How a folder opened from the Dock or Finder opens: as a tab of the
/// window in front when the macOS setting asks for tabs (always, or while
/// that window is full screen), else as a window.
pub fn open_folder_as(pref: TabPreference, front: Option<FrontWindow>) -> Open {
    match (pref, front) {
        (_, None) => Open::Window,
        (TabPreference::Always, Some(_)) => Open::Tab,
        (TabPreference::InFullScreen, Some(f)) if f.full_screen => Open::Tab,
        _ => Open::Window,
    }
}

/// The window in front, for [`open_folder_as`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrontWindow {
    pub full_screen: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_stay_as_they_are() {
        for p in [
            "/usr/bin/env",
            "/tmp/a-b_c.d+e,f:g@h%i",
            "/Users/me/Café/日本語.txt",
            "relative/path",
        ] {
            assert_eq!(shell_quote(p), p);
        }
    }

    #[test]
    fn special_characters_get_a_backslash() {
        for (path, want) in [
            ("/tmp/My File.txt", r"/tmp/My\ File.txt"),
            ("/tmp/it's", r"/tmp/it\'s"),
            ("/tmp/\"q\"", r#"/tmp/\"q\""#),
            ("/tmp/a\\b", r"/tmp/a\\b"),
            ("/tmp/$HOME", r"/tmp/\$HOME"),
            ("/tmp/`x`", r"/tmp/\`x\`"),
            ("/tmp/a;b&c|d", r"/tmp/a\;b\&c\|d"),
            ("/tmp/(x)<y>", r"/tmp/\(x\)\<y\>"),
            ("/tmp/*?[a]{b}", r"/tmp/\*\?\[a\]\{b\}"),
            ("~/x", r"\~/x"),
            ("/tmp/#1 !2 =3 ^4", r"/tmp/\#1\ \!2\ \=3\ \^4"),
            // A no-break space is still a space to a reader.
            ("/tmp/a\u{a0}b", "/tmp/a\\\u{a0}b"),
        ] {
            assert_eq!(shell_quote(path), want, "{path:?}");
        }
    }

    #[test]
    fn control_characters_use_dollar_quotes() {
        assert_eq!(shell_quote("/tmp/a\nb"), r"$'/tmp/a\nb'");
        assert_eq!(shell_quote("/tmp/a\tb c's\\"), r"$'/tmp/a\tb c\'s\\'");
        assert_eq!(shell_quote("/tmp/\x1b[31m\r"), r"$'/tmp/\x1b[31m\r'");
        assert_eq!(shell_quote("/tmp/\u{85}"), r"$'/tmp/\xc2\x85'");
    }

    #[test]
    fn empty_path_is_an_empty_word() {
        assert_eq!(shell_quote(""), "''");
    }

    /// What `sh` reads back from the quoted text is the path itself.
    #[test]
    fn a_shell_reads_back_the_paths() {
        let paths = [
            "/tmp/My File.txt",
            "/tmp/it's \"here\"",
            "/tmp/$x `y` *",
            "~user/a b",
            "/tmp/a\\b",
            "/tmp/日本 語",
        ];
        let text = dropped_text(&paths);
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("for a in {text}; do printf '%s\\0' \"$a\"; done"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let got: Vec<_> = out
            .stdout
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        assert_eq!(got, paths);
    }

    #[test]
    fn dollar_quotes_read_back_in_bash_and_zsh() {
        let path = "/tmp/new\nline\tand 'quote'\\\x01";
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(shell).exists() {
                continue;
            }
            let out = std::process::Command::new(shell)
                .arg("-c")
                .arg(format!("printf '%s' {}", shell_quote(path)))
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), path, "{shell}");
        }
    }

    #[test]
    fn several_paths_are_separated_by_spaces() {
        assert_eq!(dropped_text(&["/a b", "/c"]), r"/a\ b /c");
        assert_eq!(dropped_text(&["/c"]), "/c");
        assert_eq!(dropped_text::<&str>(&[]), "");
    }

    #[test]
    fn files_open_in_their_folder() {
        let dir = std::env::temp_dir().join(format!("terminal-decline-desktop-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        assert_eq!(folder_for(&dir.join("sub")), Some(dir.join("sub")));
        assert_eq!(folder_for(&dir.join("file.txt")), Some(dir.clone()));
        assert_eq!(folder_for(&dir.join("missing")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tab_preference_picks_tab_or_window() {
        use TabPreference::*;
        let windowed = Some(FrontWindow { full_screen: false });
        let full = Some(FrontWindow { full_screen: true });
        for (pref, front, want) in [
            (Manual, None, Open::Window),
            (Manual, windowed, Open::Window),
            (Manual, full, Open::Window),
            (Always, None, Open::Window),
            (Always, windowed, Open::Tab),
            (Always, full, Open::Tab),
            (InFullScreen, None, Open::Window),
            (InFullScreen, windowed, Open::Window),
            (InFullScreen, full, Open::Tab),
        ] {
            assert_eq!(open_folder_as(pref, front), want, "{pref:?} {front:?}");
        }
    }
}
