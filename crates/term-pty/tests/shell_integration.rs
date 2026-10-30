//! The shell integration loaded into real login shells (zsh, and each
//! bash there is), with a temp HOME holding typical startup files: every
//! prompt is marked once (OSC 133 A and B, C and D around each command,
//! with its status) and reports its folder once (OSC 7), the user's own
//! files all ran, in the order the shell reads them, and nothing of ours
//! is left in the environment for the programs started from the shell.
//!
//! The integration directory is copied to a path with a space in it, as
//! in "Terminal Decline.app". Every wait has a deadline.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use term_core::Command as Mark;
use term_pty::{Command, Pty, Size};

const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: Size = Size {
    cols: 200,
    lines: 50,
    cell_width: 8,
    cell_height: 16,
};

/// A temp directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("terminal-decline-si-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir.canonicalize().unwrap())
    }

    fn join(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }

    fn str(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// `shell/` from the repository, copied under `home`'s temp dir at a
/// path with a space.
fn integration(home: &TempDir) -> PathBuf {
    let dir = home.join("App Resources/shell-integration");
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../shell"), &dir);
    dir
}

fn write(path: PathBuf, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A startup file's line noting that it ran, in `$HOME/ran`.
fn ran(name: &str) -> String {
    format!("echo {name} >> \"$HOME/ran\"\n")
}

struct Session {
    pty: Pty,
    rx: Receiver<Vec<u8>>,
    out: Vec<u8>,
    exited: bool,
}

impl Session {
    fn start(shell: &str, home: &TempDir, env: &[(&str, &str)]) -> Session {
        let mut cmd_env = vec![
            ("HOME".to_string(), home.str().to_string()),
            ("BASH_SILENCE_DEPRECATION_WARNING".into(), "1".into()),
            // Not the test runner's: empty is taken as unset.
            ("ZDOTDIR".into(), String::new()),
            ("ENV".into(), String::new()),
        ];
        cmd_env.extend(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let cmd = Command {
            cwd: Some(home.str().into()),
            env: cmd_env,
            shell: Some(shell.into()),
            integration: Some(integration(home)),
            ..Command::default()
        };
        let pty = Pty::spawn(&cmd, SIZE).unwrap();
        let mut reader = pty.reader().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = reader.read_batch(&mut buf);
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    return;
                }
            }
        });
        Session {
            pty,
            rx,
            out: Vec::new(),
            exited: false,
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }

    fn count(&self, needle: &str) -> usize {
        self.text().matches(needle).count()
    }

    fn wait_for(&mut self, what: &str, pred: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + DEADLINE;
        while !pred(self) {
            if self.exited {
                panic!("the shell exited before {what}; output: {:?}", self.text());
            }
            match self.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(bytes) => self.out.extend_from_slice(&bytes),
                Err(RecvTimeoutError::Disconnected) => self.exited = true,
                Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for {what}; output: {:?}", self.text()),
            }
        }
    }

    /// Type `line` and wait for the prompt after it (the `n`th).
    fn run(&mut self, line: &str, n: usize) {
        self.pty.writer().unwrap().write_all(format!("{line}\n").as_bytes()).unwrap();
        self.wait_for(&format!("prompt {n}, after {line:?}"), |s| s.count("\x1b]133;B\x07") >= n);
    }
}

/// Start `shell` in `home`, run a few commands, and check the marks; the
/// output of the last one, which prints what `probe` expands to.
fn check(shell: &str, home: &TempDir, env: &[(&str, &str)], probe: &str) -> String {
    std::fs::create_dir_all(home.join("sub dir")).unwrap();
    let mut s = Session::start(shell, home, env);
    s.wait_for("the first prompt", |s| s.count("\x1b]133;B\x07") >= 1);
    s.run("cd 'sub dir'", 2);
    s.run("false", 3);
    s.run(&format!("echo \"<{probe}>\""), 4);
    // Settle: nothing more comes for a moment.
    std::thread::sleep(Duration::from_millis(300));
    s.wait_for("the last output", |_| true);
    while let Ok(bytes) = s.rx.try_recv() {
        s.out.extend_from_slice(&bytes);
    }
    let text = s.text();
    // Once per prompt, and once per command.
    for (mark, n) in [
        ("\x1b]133;A\x07", 4),
        ("\x1b]133;B\x07", 4),
        ("\x1b]133;C\x07", 3),
        ("\x1b]133;D;0\x07", 2),
        ("\x1b]133;D;1\x07", 1),
        ("\x1b]7;file://", 4),
    ] {
        assert_eq!(text.matches(mark).count(), n, "{shell}: {mark:?} in {text:?}");
    }
    // What term-core and the title status make of them: the command text
    // after B, the status, and the folder.
    let mut term = term_core::Term::new(SIZE.cols as usize, SIZE.lines as usize, 1000, term_core::ImagePool::process());
    term.advance(&s.out);
    let commands = term.state.take_commands();
    let started: Vec<Option<String>> = commands
        .iter()
        .filter_map(|c| match c {
            Mark::Started(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        started,
        [
            Some("cd 'sub dir'".to_string()),
            Some("false".into()),
            Some(format!("echo \"<{probe}>\""))
        ],
        "{shell}: {commands:?}"
    );
    let finished: Vec<&Mark> = commands.iter().filter(|c| matches!(c, Mark::Finished(_))).collect();
    assert_eq!(
        finished,
        [&Mark::Finished(Some(0)), &Mark::Finished(Some(1)), &Mark::Finished(Some(0))],
        "{shell}"
    );
    assert_eq!(term.state.cwd(), home.join("sub dir").to_str(), "{shell}");
    let start = text.rfind('<').expect("probe output");
    let end = text[start..].find('>').expect("probe output") + start;
    text[start + 1..end].to_string()
}

fn ran_files(home: &TempDir) -> String {
    std::fs::read_to_string(home.join("ran"))
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

const ZSH: &str = "/bin/zsh";

/// What zsh's startup leaves: our variables gone, ZDOTDIR the user's.
const ZSH_PROBE: &str = "${ZDOTDIR-unset}|${DECLINE_ZSH_ZDOTDIR-unset}|${+_decline_zdotdir}";

fn zsh_files(dir: &Path, zshrc_extra: &str) {
    write(dir.join(".zshenv"), &ran("zshenv"));
    write(dir.join(".zprofile"), &ran("zprofile"));
    write(dir.join(".zshrc"), &format!("{}{zshrc_extra}PS1='READY> '\n", ran("zshrc")));
    write(dir.join(".zlogin"), &ran("zlogin"));
}

#[test]
fn zsh_with_plain_startup_files() {
    let home = TempDir::new("zsh-plain");
    zsh_files(&home.0, "");
    let probe = check(ZSH, &home, &[], ZSH_PROBE);
    assert_eq!(probe, "unset|unset|0");
    assert_eq!(ran_files(&home), "zshenv zprofile zshrc zlogin");
}

#[test]
fn zsh_with_its_own_zdotdir() {
    let home = TempDir::new("zsh-zdotdir");
    let zdot = home.join("config/zsh");
    zsh_files(&zdot, "");
    let probe = check(ZSH, &home, &[("ZDOTDIR", zdot.to_str().unwrap())], ZSH_PROBE);
    assert_eq!(probe, format!("{}|unset|0", zdot.display()));
    assert_eq!(ran_files(&home), "zshenv zprofile zshrc zlogin");
}

#[test]
fn zsh_with_zdotdir_set_in_zshenv() {
    let home = TempDir::new("zsh-zdotdir-env");
    let zdot = home.join("dots");
    write(
        home.join(".zshenv"),
        &format!("{}export ZDOTDIR=\"$HOME/dots\"\n", ran("home-zshenv")),
    );
    write(zdot.join(".zprofile"), &ran("zprofile"));
    write(zdot.join(".zshrc"), &format!("{}PS1='READY> '\n", ran("zshrc")));
    let probe = check(ZSH, &home, &[], ZSH_PROBE);
    assert_eq!(probe, format!("{}|unset|0", zdot.display()));
    assert_eq!(ran_files(&home), "home-zshenv zprofile zshrc");
}

/// Powerlevel10k's instant prompt guard at the top of .zshrc, and a theme
/// that rebuilds PS1 in its own precmd, as p10k and others do.
#[test]
fn zsh_with_an_instant_prompt_and_a_theme() {
    let home = TempDir::new("zsh-p10k");
    write(home.join(".cache/p10k-instant-prompt-me.zsh"), "typeset -g _instant_prompt=1\n");
    let guard = r#"if [[ -r "${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-me.zsh" ]]; then
  source "${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-me.zsh"
fi
"#;
    let theme = "_theme_precmd() { PS1=\"READY> \" }\nautoload -Uz add-zsh-hook\nadd-zsh-hook precmd _theme_precmd\nsetopt prompt_subst\n";
    zsh_files(&home.0, &format!("{guard}{theme}"));
    let probe = check(ZSH, &home, &[], "${_instant_prompt-}|${precmd_functions[-1]}");
    assert_eq!(probe, "1|_decline_precmd");
    assert_eq!(ran_files(&home), "zshenv zprofile zshrc zlogin");
}

/// A .zshrc that already sources decline.zsh itself, as before the app
/// loaded it: still once.
#[test]
fn zsh_that_sources_the_snippet_itself() {
    let home = TempDir::new("zsh-sourced");
    let snippet = home.join("App Resources/shell-integration/decline.zsh");
    zsh_files(&home.0, &format!("source '{}'\n", snippet.display()));
    let probe = check(ZSH, &home, &[], "${(M)precmd_functions:#_decline_*}");
    assert_eq!(probe, "_decline_precmd");
    assert_eq!(ran_files(&home), "zshenv zprofile zshrc zlogin");
}

/// The bashes there are: macOS's 3.2 and Homebrew's.
fn bashes() -> Vec<&'static str> {
    let found: Vec<&str> = ["/bin/bash", "/opt/homebrew/bin/bash", "/usr/local/bin/bash"]
        .into_iter()
        .filter(|b| Path::new(b).exists())
        .collect();
    assert!(!found.is_empty());
    found
}

/// What bash's startup leaves: ENV as it was, ours gone.
const BASH_PROBE: &str = "${ENV-unset}|${DECLINE_BASH_INJECT-unset}|${DECLINE_BASH_LOGIN-unset}";

#[test]
fn bash_with_plain_startup_files() {
    for bash in bashes() {
        let home = TempDir::new(&format!("bash-plain{}", bash.replace('/', "-")));
        write(
            home.join(".bash_profile"),
            &format!("{}[ -r ~/.bashrc ] && . ~/.bashrc\n", ran("bash_profile")),
        );
        write(
            home.join(".bashrc"),
            &format!("{}PS1='READY> '\nPROMPT_COMMAND='history -a'\n", ran("bashrc")),
        );
        // Not read: .bash_profile comes first.
        write(home.join(".profile"), &ran("profile"));
        let probe = check(bash, &home, &[], BASH_PROBE);
        assert_eq!(probe, "|unset|unset", "{bash}");
        assert_eq!(ran_files(&home), "bash_profile bashrc", "{bash}");
    }
}

#[test]
fn bash_with_only_a_profile_and_its_own_env() {
    for bash in bashes() {
        let home = TempDir::new(&format!("bash-profile{}", bash.replace('/', "-")));
        write(home.join(".profile"), &format!("{}PS1='READY> '\n", ran("profile")));
        let shrc = home.join(".shrc");
        let probe = check(bash, &home, &[("ENV", shrc.to_str().unwrap())], BASH_PROBE);
        assert_eq!(probe, format!("{}|unset|unset", shrc.display()), "{bash}");
        assert_eq!(ran_files(&home), "profile", "{bash}");
    }
}

#[test]
fn bash_that_sources_the_snippet_itself() {
    for bash in bashes() {
        let home = TempDir::new(&format!("bash-sourced{}", bash.replace('/', "-")));
        let snippet = home.join("App Resources/shell-integration/decline.bash");
        write(
            home.join(".bash_profile"),
            &format!("{}PS1='READY> '\nsource '{}'\n", ran("bash_profile"), snippet.display()),
        );
        let probe = check(bash, &home, &[], "$PROMPT_COMMAND");
        assert_eq!(probe, "_decline_status; _decline_prompt", "{bash}");
        assert_eq!(ran_files(&home), "bash_profile", "{bash}");
    }
}
