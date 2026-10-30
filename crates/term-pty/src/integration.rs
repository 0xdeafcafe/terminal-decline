//! Shell integration loaded into the shells the app starts, the way
//! Ghostty and kitty do it: the user sources nothing, and their startup
//! files still run as they always do (docs/shell-integration.md).
//!
//! The integration directory is `shell/` in the repository, bundled as
//! `Contents/Resources/shell-integration`. Each shell is pointed at it
//! its own way, with the user's own setting kept in a `DECLINE_*`
//! variable that the shell's side puts back and removes:
//!
//! - zsh: `ZDOTDIR` is `<dir>/zsh`, whose startup files source the
//!   user's own, one by one, with their ZDOTDIR while each runs, then
//!   `decline.zsh` after the user's `.zshrc` (the user's ZDOTDIR, if set,
//!   in `DECLINE_ZSH_ZDOTDIR`).
//! - bash: an interactive shell that isn't a login shell, with
//!   `--rcfile <dir>/bash/decline-inject.bash` in place of `~/.bashrc`;
//!   that file reads what bash would have (for a login shell, as
//!   `DECLINE_BASH_LOGIN` says: `/etc/profile`, then `~/.bash_profile`,
//!   `~/.bash_login` or `~/.profile`), then `decline.bash`
//!   (`DECLINE_BASH_INJECT`). Not `--posix` with `ENV`: macOS's bash 3.2
//!   doesn't read ENV in POSIX mode.
//! - fish: `<dir>` first in `XDG_DATA_DIRS`, so fish reads
//!   `<dir>/fish/vendor_conf.d/decline.fish`, which takes it out again
//!   (`DECLINE_FISH_XDG_DIR`).
//!
//! Only a login shell the app starts, or a command that is just a shell
//! (`["fish"]`); any other command, and shells started from those, are
//! left alone.

use std::path::Path;

/// A shell the integration knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

impl Shell {
    /// The shell `program` (a path or a name, `-zsh` as a login shell's
    /// argv[0] too) is, if it is one of ours.
    pub fn of(program: &str) -> Option<Shell> {
        let name = Path::new(program).file_name()?.to_str()?;
        match name.strip_prefix('-').unwrap_or(name) {
            "zsh" => Some(Shell::Zsh),
            "bash" => Some(Shell::Bash),
            "fish" => Some(Shell::Fish),
            _ => None,
        }
    }
}

/// The variables the shells' side reads and removes. Inherited ones (an
/// app started from a shell that was being set up) are dropped first.
const OURS: [&str; 4] = [
    "DECLINE_ZSH_ZDOTDIR",
    "DECLINE_BASH_INJECT",
    "DECLINE_BASH_LOGIN",
    "DECLINE_FISH_XDG_DIR",
];

/// XDG_DATA_DIRS when unset, as the XDG spec has it.
const XDG_DATA_DIRS_DEFAULT: &str = "/usr/local/share:/usr/share";

/// Environment variables, as names and values.
pub type Env = Vec<(String, String)>;

/// What starting `shell` (a `login` shell or not) with the integration
/// in `dir` adds: arguments after the shell's argv[0], and variables to
/// set, given the environment `env` it would otherwise get. `None` if
/// `dir` isn't a UTF-8 path.
pub fn injection(shell: Shell, dir: &Path, env: &[(String, String)], login: bool) -> Option<(Vec<String>, Env)> {
    let dir = dir.to_str()?.trim_end_matches('/');
    let get = |key: &str| {
        env.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    };
    let mut args = Vec::new();
    let mut set = Env::new();
    match shell {
        Shell::Zsh => {
            // An empty ZDOTDIR would have zsh read /.zshrc: taken as unset.
            if let Some(user) = get("ZDOTDIR") {
                set.push(("DECLINE_ZSH_ZDOTDIR".into(), user.into()));
            }
            set.push(("ZDOTDIR".into(), format!("{dir}/zsh")));
        }
        Shell::Bash => {
            args.extend(["--rcfile".into(), format!("{dir}/bash/decline-inject.bash")]);
            set.push(("DECLINE_BASH_INJECT".into(), "1".into()));
            if login {
                set.push(("DECLINE_BASH_LOGIN".into(), "1".into()));
            }
        }
        Shell::Fish => {
            let rest = get("XDG_DATA_DIRS").unwrap_or(XDG_DATA_DIRS_DEFAULT);
            set.push(("DECLINE_FISH_XDG_DIR".into(), dir.into()));
            set.push(("XDG_DATA_DIRS".into(), format!("{dir}:{rest}")));
        }
    }
    Some((args, set))
}

/// Load the integration in `dir` into what `Pty::spawn` runs: `argv`
/// (argv[0] first) and `env` as built for `command` (its argv, empty for
/// the login shell `shell`). A command other than a lone shell is left
/// as it is.
pub fn apply(dir: &Path, shell: &str, command: &[String], argv: &mut Vec<String>, env: &mut Vec<(String, String)>) {
    let (kind, hop) = match command {
        [] => (Shell::of(shell), false),
        // Run through the login shell (`shell -c 'exec "$0" "$@"' bash`).
        [program] => (Shell::of(program), true),
        _ => return,
    };
    let Some(kind) = kind else { return };
    env.retain(|(k, _)| !OURS.contains(&k.as_str()));
    let Some((args, mut set)) = injection(kind, dir, env, !hop) else {
        return;
    };
    if kind == Shell::Bash
        && !hop
        && let Some(name) = argv.first_mut()
    {
        // A login bash ignores --rcfile: our file reads the login files.
        *name = name.trim_start_matches('-').to_string();
    }
    if hop && kind == Shell::Zsh {
        // ZDOTDIR is for the zsh at the end, not the login shell that
        // starts it (when that is zsh too, it would read our files and
        // put the user's ZDOTDIR back): `env` sets it on the way.
        let Some(at) = set.iter().position(|(k, _)| k == "ZDOTDIR") else {
            return;
        };
        let (_, zdotdir) = set.remove(at);
        let program = argv.pop().unwrap_or_default();
        argv.extend(["/usr/bin/env".to_string(), format!("ZDOTDIR={zdotdir}"), program]);
    }
    argv.extend(args);
    for (k, v) in set {
        match env.binary_search_by(|(key, _)| key.as_str().cmp(&k)) {
            Ok(i) => env[i].1 = v,
            Err(i) => env.insert(i, (k, v)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{build_argv, build_env};

    fn kv(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// argv and env for `command` run by the login shell `shell`, from
    /// `inherited`, with the integration in `/App/Res/si`.
    fn run(shell: &str, command: &[&str], inherited: &[(&str, &str)]) -> (Vec<String>, Vec<(String, String)>) {
        let command = s(command);
        let mut argv = build_argv(shell, &command);
        let mut env = build_env(kv(inherited).into_iter(), &[]);
        apply(Path::new("/App/Res/si/"), shell, &command, &mut argv, &mut env);
        (argv, env)
    }

    #[test]
    fn shells_by_name() {
        assert_eq!(Shell::of("/bin/zsh"), Some(Shell::Zsh));
        assert_eq!(Shell::of("-zsh"), Some(Shell::Zsh));
        assert_eq!(Shell::of("/opt/homebrew/bin/bash"), Some(Shell::Bash));
        assert_eq!(Shell::of("fish"), Some(Shell::Fish));
        assert_eq!(Shell::of("/bin/sh"), None);
        assert_eq!(Shell::of("/usr/bin/zshell"), None);
        assert_eq!(Shell::of(""), None);
    }

    #[test]
    fn zsh_gets_our_zdotdir_and_keeps_the_users() {
        let (argv, env) = run("/bin/zsh", &[], &[("PATH", "/bin")]);
        assert_eq!(argv, s(&["-zsh"]));
        assert_eq!(get(&env, "ZDOTDIR"), Some("/App/Res/si/zsh"));
        assert_eq!(get(&env, "DECLINE_ZSH_ZDOTDIR"), None);
        let (_, env) = run("/bin/zsh", &[], &[("ZDOTDIR", "/Users/me/.config/zsh")]);
        assert_eq!(get(&env, "ZDOTDIR"), Some("/App/Res/si/zsh"));
        assert_eq!(get(&env, "DECLINE_ZSH_ZDOTDIR"), Some("/Users/me/.config/zsh"));
        // Empty is as good as unset.
        let (_, env) = run("/bin/zsh", &[], &[("ZDOTDIR", "")]);
        assert_eq!(get(&env, "DECLINE_ZSH_ZDOTDIR"), None);
    }

    #[test]
    fn bash_reads_our_rcfile_which_reads_the_login_files() {
        let (argv, env) = run("/bin/bash", &[], &[("ENV", "/Users/me/.shrc")]);
        assert_eq!(argv, s(&["bash", "--rcfile", "/App/Res/si/bash/decline-inject.bash"]));
        assert_eq!(get(&env, "DECLINE_BASH_INJECT"), Some("1"));
        assert_eq!(get(&env, "DECLINE_BASH_LOGIN"), Some("1"));
        assert_eq!(get(&env, "ENV"), Some("/Users/me/.shrc"));
    }

    #[test]
    fn fish_finds_our_vendor_config() {
        let (argv, env) = run("/opt/homebrew/bin/fish", &[], &[]);
        assert_eq!(argv, s(&["-fish"]));
        assert_eq!(get(&env, "XDG_DATA_DIRS"), Some("/App/Res/si:/usr/local/share:/usr/share"));
        assert_eq!(get(&env, "DECLINE_FISH_XDG_DIR"), Some("/App/Res/si"));
        let (_, env) = run("/opt/homebrew/bin/fish", &[], &[("XDG_DATA_DIRS", "/opt/share")]);
        assert_eq!(get(&env, "XDG_DATA_DIRS"), Some("/App/Res/si:/opt/share"));
    }

    #[test]
    fn terminal_variables_are_unchanged() {
        for shell in ["/bin/zsh", "/bin/bash", "/opt/homebrew/bin/fish"] {
            let (_, env) = run(shell, &[], &[("TERM_PROGRAM", "Apple_Terminal")]);
            let plain = build_env(kv(&[("TERM_PROGRAM", "Apple_Terminal")]).into_iter(), &[]);
            for key in ["TERM", "COLORTERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "LANG"] {
                assert_eq!(get(&env, key), get(&plain, key), "{shell} {key}");
            }
            let mut keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
            assert!(keys.is_sorted(), "{keys:?}");
            keys.dedup();
            assert_eq!(keys.len(), env.len());
        }
    }

    #[test]
    fn other_shells_and_commands_are_left_alone() {
        for (shell, command) in [
            ("/bin/sh", &[][..]),
            ("/bin/zsh", &["vim"][..]),
            ("/bin/zsh", &["zsh", "-c", "make"][..]),
            ("/bin/bash", &["/usr/bin/top"][..]),
        ] {
            let (argv, env) = run(shell, command, &[("ZDOTDIR", "/z")]);
            assert_eq!(argv, build_argv(shell, &s(command)), "{shell} {command:?}");
            assert_eq!(env, build_env(kv(&[("ZDOTDIR", "/z")]).into_iter(), &[]), "{shell} {command:?}");
        }
    }

    #[test]
    fn a_command_that_is_a_shell_gets_it_too() {
        // bash and fish: the login shell passes the setting on.
        let (argv, env) = run("/bin/zsh", &["bash"], &[]);
        assert_eq!(
            argv,
            s(&[
                "-zsh",
                "-c",
                "exec \"$0\" \"$@\"",
                "bash",
                "--rcfile",
                "/App/Res/si/bash/decline-inject.bash"
            ])
        );
        assert_eq!(get(&env, "DECLINE_BASH_INJECT"), Some("1"));
        assert_eq!(get(&env, "DECLINE_BASH_LOGIN"), None);
        let (argv, env) = run("/bin/zsh", &["/opt/homebrew/bin/fish"], &[]);
        assert_eq!(argv, build_argv("/bin/zsh", &s(&["/opt/homebrew/bin/fish"])));
        assert_eq!(get(&env, "DECLINE_FISH_XDG_DIR"), Some("/App/Res/si"));
        // zsh: ZDOTDIR only for the zsh at the end, set by env on the way.
        let (argv, env) = run("/bin/zsh", &["zsh"], &[("ZDOTDIR", "/z")]);
        assert_eq!(
            argv,
            s(&["-zsh", "-c", "exec \"$0\" \"$@\"", "/usr/bin/env", "ZDOTDIR=/App/Res/si/zsh", "zsh"])
        );
        assert_eq!(get(&env, "ZDOTDIR"), Some("/z"));
        assert_eq!(get(&env, "DECLINE_ZSH_ZDOTDIR"), Some("/z"));
    }

    #[test]
    fn inherited_settings_of_ours_are_dropped() {
        let (_, env) = run("/bin/zsh", &[], &[("DECLINE_ZSH_ZDOTDIR", "/stale"), ("DECLINE_BASH_INJECT", "1")]);
        assert_eq!(get(&env, "DECLINE_ZSH_ZDOTDIR"), None);
        assert_eq!(get(&env, "DECLINE_BASH_INJECT"), None);
    }
}
