//! What to run, and the pure pieces of turning that into `execve` arguments.

use std::collections::BTreeMap;
use std::path::Path;

/// A program to run on a PTY.
#[derive(Clone, Debug, Default)]
pub struct Command {
    /// Program and arguments. Empty = an interactive login shell.
    pub argv: Vec<String>,
    /// Working directory. Missing or nonexistent = HOME.
    pub cwd: Option<String>,
    /// Extra environment; overrides everything else.
    pub env: Vec<(String, String)>,
    /// The shell to use instead of the user's login shell.
    pub shell: Option<String>,
}

/// The environment for the child: `inherited`, then ours, then `extra`.
///
/// Our TERM, COLORTERM and TERM_PROGRAM(_VERSION) replace inherited values;
/// the caller's `extra` replaces ours. LANG gets a UTF-8 default when unset:
/// apps started from the Dock get no locale, and without one shells and
/// programs fall back to ASCII. Keys are unique; the result is sorted.
pub fn build_env(inherited: impl Iterator<Item = (String, String)>, extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: BTreeMap<String, String> = inherited.collect();
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    env.insert("TERM_PROGRAM".into(), "decline".into());
    env.insert("TERM_PROGRAM_VERSION".into(), env!("CARGO_PKG_VERSION").into());
    env.entry("LANG".into()).or_insert_with(|| "en_US.UTF-8".into());
    env.extend(extra.iter().cloned());
    env.into_iter().collect()
}

/// The argv to pass to `shell` (argv[0] first; the program is `shell`).
///
/// An empty `argv` gives an interactive login shell. Otherwise the program
/// runs through a login shell, `shell -c 'exec "$0" "$@"' argv...`, so it
/// gets the user's PATH and environment even when started from the Dock
/// (where PATH is only /usr/bin:/bin:/usr/sbin:/sbin). A leading '-' in
/// argv[0] is what makes a shell a login shell.
pub fn build_argv(shell: &str, argv: &[String]) -> Vec<String> {
    let name = Path::new(shell)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "sh".into());
    let mut out = vec![format!("-{name}")];
    if !argv.is_empty() {
        out.push("-c".into());
        out.push("exec \"$0\" \"$@\"".into());
        out.extend(argv.iter().cloned());
    }
    out
}

/// The directory to start in: `cwd` if it is an existing directory,
/// otherwise `home`. With no cwd the app's own would be used, which is `/`
/// when started from the Dock, so that falls back to `home` too.
pub fn resolve_cwd(cwd: Option<&str>, home: Option<&str>) -> Option<String> {
    cwd.filter(|d| Path::new(d).is_dir()).or(home).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn kv(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn env_from_empty_inherited_like_a_dock_launch() {
        let env = build_env(std::iter::empty(), &[]);
        assert_eq!(get(&env, "TERM"), Some("xterm-256color"));
        assert_eq!(get(&env, "COLORTERM"), Some("truecolor"));
        assert_eq!(get(&env, "TERM_PROGRAM"), Some("decline"));
        assert_eq!(get(&env, "TERM_PROGRAM_VERSION"), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(get(&env, "LANG"), Some("en_US.UTF-8"));
        assert_eq!(env.len(), 5, "{env:?}");
    }

    #[test]
    fn env_ours_override_inherited() {
        let inherited = kv(&[
            ("TERM", "dumb"),
            ("COLORTERM", "no"),
            ("TERM_PROGRAM", "Apple_Terminal"),
            ("TERM_PROGRAM_VERSION", "1"),
            ("PATH", "/bin"),
        ]);
        let env = build_env(inherited.into_iter(), &[]);
        assert_eq!(get(&env, "TERM"), Some("xterm-256color"));
        assert_eq!(get(&env, "COLORTERM"), Some("truecolor"));
        assert_eq!(get(&env, "TERM_PROGRAM"), Some("decline"));
        assert_eq!(get(&env, "TERM_PROGRAM_VERSION"), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(get(&env, "PATH"), Some("/bin"));
    }

    #[test]
    fn env_caller_overrides_ours_and_inherited() {
        let inherited = kv(&[("FOO", "old"), ("LANG", "C")]);
        let extra = kv(&[("TERM", "xterm"), ("FOO", "new"), ("LANG", "fr_FR.UTF-8")]);
        let env = build_env(inherited.into_iter(), &extra);
        assert_eq!(get(&env, "TERM"), Some("xterm"));
        assert_eq!(get(&env, "FOO"), Some("new"));
        assert_eq!(get(&env, "LANG"), Some("fr_FR.UTF-8"));
    }

    #[test]
    fn env_keeps_inherited_lang() {
        let env = build_env(kv(&[("LANG", "de_DE.UTF-8")]).into_iter(), &[]);
        assert_eq!(get(&env, "LANG"), Some("de_DE.UTF-8"));
    }

    #[test]
    fn env_keys_are_unique() {
        let inherited = kv(&[("A", "1"), ("A", "2"), ("TERM", "x")]);
        let extra = kv(&[("B", "1"), ("B", "2"), ("A", "3")]);
        let env = build_env(inherited.into_iter(), &extra);
        let mut keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "{env:?}");
        assert_eq!(get(&env, "A"), Some("3"));
        assert_eq!(get(&env, "B"), Some("2"));
    }

    #[test]
    fn empty_argv_is_a_login_shell() {
        assert_eq!(build_argv("/bin/zsh", &[]), s(&["-zsh"]));
        assert_eq!(build_argv("/usr/local/bin/fish", &[]), s(&["-fish"]));
        assert!(build_argv("/", &[])[0].starts_with('-'));
    }

    #[test]
    fn argv_runs_through_the_shell() {
        let argv = build_argv("/bin/sh", &s(&["vim", "a file.txt"]));
        assert_eq!(argv, s(&["-sh", "-c", "exec \"$0\" \"$@\"", "vim", "a file.txt"]));
    }

    #[test]
    fn cwd_existing_is_kept() {
        let tmp = std::env::temp_dir();
        let tmp = tmp.to_str().unwrap();
        assert_eq!(resolve_cwd(Some(tmp), Some("/home")).as_deref(), Some(tmp));
    }

    #[test]
    fn cwd_missing_falls_back_to_home() {
        assert_eq!(
            resolve_cwd(Some("/nonexistent/decline-test"), Some("/home/me")).as_deref(),
            Some("/home/me")
        );
    }

    #[test]
    fn cwd_none_falls_back_to_home() {
        assert_eq!(resolve_cwd(None, Some("/home/me")).as_deref(), Some("/home/me"));
        assert_eq!(resolve_cwd(None, None), None);
    }
}
