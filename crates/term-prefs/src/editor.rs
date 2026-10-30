//! Which text editor opens a file, and the command lines that run it.
//!
//! The order is the `editor` setting, then `$VISUAL`, then `$EDITOR`,
//! then the first of nano, vim and vi found on the PATH. An app started
//! from the Dock has none of the user's shell environment, so the editor
//! is run through their login shell (which reads their own profile), and
//! the Settings screen asks that shell once when its own environment has
//! neither variable.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Tried in this order when nothing else names an editor.
pub const FALLBACKS: [&str; 3] = ["nano", "vim", "vi"];

/// How long the login shell has to say what `$VISUAL`/`$EDITOR` are.
const ASK_TIMEOUT: Duration = Duration::from_millis(1500);

/// `value` unless it is empty or only spaces.
fn nonblank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// The editor to run, as shell words (it may have arguments): the
/// `setting`, else `visual`, else `editor`, else (`login`, asked only
/// now) the same two from the login shell, else the first of nano, vim
/// and vi that `on_path` finds, else `vi`.
pub fn resolve(
    setting: &str,
    visual: Option<&str>,
    editor: Option<&str>,
    login: impl FnOnce() -> String,
    on_path: impl Fn(&str) -> bool,
) -> String {
    if let Some(e) = nonblank(Some(setting)).or(nonblank(visual)).or(nonblank(editor)) {
        return e.to_string();
    }
    let asked = login();
    if let Some(e) = nonblank(Some(&asked)) {
        return e.to_string();
    }
    FALLBACKS.iter().find(|e| on_path(e)).unwrap_or(&"vi").to_string()
}

/// Whether `name` is an executable file in a directory of `path`.
pub fn in_path(name: &str, path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path)
        .any(|dir| std::fs::metadata(dir.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// What the login shell says `$VISUAL` (else `$EDITOR`) is, or empty. It
/// is asked with `shell -l -i -c`, so the user's profile and rc files
/// apply, and given a short time to answer.
pub fn ask_login_shell(shell: &str, timeout: Duration) -> String {
    let child = Command::new(shell)
        .args(["-l", "-i", "-c", r#"printf %s "${VISUAL:-$EDITOR}""#])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return String::new() };
    let Some(mut out) = child.stdout.take() else { return String::new() };
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return String::new(),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    // The last line: a profile may print before the answer.
    let said = reader.join().unwrap_or_default();
    said.lines().last().unwrap_or("").trim().to_string()
}

/// The user's login shell.
pub fn login_shell() -> String {
    match std::env::var("SHELL") {
        Ok(s) if !s.is_empty() => s,
        _ => "/bin/zsh".into(),
    }
}

/// [`resolve`] for the real environment. The login shell is asked at
/// most once per process.
pub fn resolve_here(setting: &str) -> String {
    static LOGIN: OnceLock<String> = OnceLock::new();
    let var = |k| std::env::var(k).ok();
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    resolve(
        setting,
        var("VISUAL").as_deref(),
        var("EDITOR").as_deref(),
        || LOGIN.get_or_init(|| ask_login_shell(&login_shell(), ASK_TIMEOUT)).clone(),
        |e| in_path(e, &path),
    )
}

/// The command line that edits `file` in `editor` (shell words),
/// through `/bin/sh`: for running in this terminal.
pub fn command_line(editor: &str, file: &Path) -> Command {
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(file);
    c
}

/// The shell script that runs the editor on `$1`, in an environment
/// where `$VISUAL` and `$EDITOR` are the user's own: the `setting` if
/// there is one, else those, else nano, vim or vi.
pub fn script(setting: &str) -> String {
    if let Some(e) = nonblank(Some(setting)) {
        return format!("exec {e} \"$1\"");
    }
    [
        r#"if [ -n "${VISUAL:-}" ]; then exec $VISUAL "$1"; fi"#,
        r#"if [ -n "${EDITOR:-}" ]; then exec $EDITOR "$1"; fi"#,
        r#"for e in nano vim vi; do command -v "$e" >/dev/null 2>&1 && exec "$e" "$1"; done"#,
        r#"echo "No editor found: set the editor setting, \$VISUAL or \$EDITOR."; read -r _"#,
    ]
    .join("\n")
}

/// The program and arguments for a new tab that edits `file`: the user's
/// login shell, interactive so that its rc files set `$VISUAL` and
/// `$EDITOR`, runs [`script`] under `/bin/sh` (the script and the file
/// travel in the environment, so any login shell, fish too, can start
/// it). The tab closes when the editor exits.
pub fn tab_argv(shell: &str, setting: &str, file: &Path) -> Vec<String> {
    vec![
        "/usr/bin/env".into(),
        format!("TD_EDITOR_SCRIPT={}", script(setting)),
        format!("TD_EDITOR_FILE={}", file.display()),
        shell.into(),
        "-l".into(),
        "-i".into(),
        "-c".into(),
        r#"exec /bin/sh -c "$TD_EDITOR_SCRIPT" sh "$TD_EDITOR_FILE""#.into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(setting: &str, visual: Option<&str>, editor: Option<&str>, login: &str, have: &[&str]) -> String {
        resolve(setting, visual, editor, || login.to_string(), |e| have.contains(&e))
    }

    #[test]
    fn the_setting_wins_then_visual_then_editor() {
        assert_eq!(pick("code -w", Some("vim"), Some("nano"), "emacs", &["nano"]), "code -w");
        assert_eq!(pick("", Some("vim"), Some("nano"), "emacs", &["nano"]), "vim");
        assert_eq!(pick("  ", None, Some("nano -w"), "emacs", &[]), "nano -w");
        assert_eq!(pick("", Some(" "), Some(""), "emacs", &["nano"]), "emacs");
    }

    #[test]
    fn the_login_shell_is_asked_only_when_needed() {
        let asked = std::cell::Cell::new(0);
        let ask = || {
            asked.set(asked.get() + 1);
            String::new()
        };
        resolve("x", None, None, ask, |_| false);
        resolve("", Some("y"), None, ask, |_| false);
        assert_eq!(asked.get(), 0);
        resolve("", None, None, ask, |_| false);
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn nano_then_vim_then_vi() {
        assert_eq!(pick("", None, None, "", &["vi", "vim", "nano"]), "nano");
        assert_eq!(pick("", None, None, "", &["vi", "vim"]), "vim");
        assert_eq!(pick("", None, None, "", &["vi"]), "vi");
        assert_eq!(pick("", None, None, "", &[]), "vi");
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("td-editor-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_path_is_searched() {
        let dir = temp("path");
        std::fs::write(dir.join("plain"), "").unwrap();
        executable(&dir.join("mynano"), "");
        let path = format!("/nonexistent:{}", dir.display());
        assert!(in_path("mynano", &path));
        assert!(!in_path("plain", &path), "not executable");
        assert!(!in_path("absent", &path));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_script_follows_the_same_order() {
        assert_eq!(script("code -w"), "exec code -w \"$1\"");
        let s = script("");
        let at = |needle: &str| s.find(needle).unwrap();
        assert!(at("$VISUAL") < at("$EDITOR") && at("$EDITOR") < at("nano vim vi"));
    }

    #[test]
    fn the_tab_runs_the_script_through_the_login_shell() {
        let argv = tab_argv("/bin/zsh", "", Path::new("/u/config.toml"));
        assert_eq!(argv[0], "/usr/bin/env");
        assert_eq!(argv[2], "TD_EDITOR_FILE=/u/config.toml");
        assert_eq!(argv[3..7], ["/bin/zsh", "-l", "-i", "-c"]);
        assert!(argv[7].contains("$TD_EDITOR_SCRIPT"));
        assert_eq!(argv.len(), 8);
    }

    #[test]
    fn the_command_line_quotes_the_file() {
        let c = command_line("vim -f", Path::new("/a b/c.toml"));
        let args: Vec<_> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-c", "vim -f \"$1\"", "sh", "/a b/c.toml"]);
    }

    #[test]
    fn a_shell_that_answers_is_heard_and_one_that_hangs_is_not_waited_for() {
        let dir = temp("ask");
        let says = dir.join("says");
        executable(&says, "echo banner; printf %s 'hx --wait'");
        assert_eq!(ask_login_shell(&says.display().to_string(), Duration::from_secs(5)), "hx --wait");
        let hangs = dir.join("hangs");
        executable(&hangs, "sleep 30");
        let start = Instant::now();
        assert_eq!(ask_login_shell(&hangs.display().to_string(), Duration::from_millis(200)), "");
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(ask_login_shell("/nonexistent/shell", Duration::from_millis(200)), "");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
