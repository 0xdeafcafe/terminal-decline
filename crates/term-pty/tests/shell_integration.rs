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
use term_pty::{Command, Pty, Reports, Size};

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
        Session::start_with(shell, home, env, None)
    }

    fn start_with(shell: &str, home: &TempDir, env: &[(&str, &str)], reports: Option<Reports>) -> Session {
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
            reports,
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

    fn send(&mut self, bytes: &str) {
        self.pty.writer().unwrap().write_all(bytes.as_bytes()).unwrap();
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
        // With the command line (zsh, fish) or without (bash).
        ("\x1b]133;C", 3),
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
    // zsh and fish send each command line whole (`cmdline_url`); bash
    // has none to send.
    let lines: Vec<&str> = commands
        .iter()
        .filter_map(|c| match c {
            Mark::CommandLine(l) => Some(l.as_str()),
            _ => None,
        })
        .collect();
    if shell.contains("bash") {
        assert!(lines.is_empty(), "{shell}: {lines:?}");
    } else {
        let probe_line = format!("echo \"<{probe}>\"");
        assert_eq!(lines, ["cd 'sub dir'", "false", probe_line.as_str()], "{shell}");
    }
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

/// zsh's command line arrives whole and as typed, whatever is in it: a
/// `;` (which would end the parameter), a `%` and what looks like an
/// escape, an alias as typed.
#[test]
fn zsh_sends_the_command_line_as_typed() {
    let home = TempDir::new("zsh-cmdline");
    zsh_files(&home.0, "alias hi='echo hello'\n");
    let mut s = Session::start(ZSH, &home, &[]);
    s.wait_for("the first prompt", |s| s.count("\x1b]133;B\x07") >= 1);
    let line = "hi 'a;b' 100% %41 é";
    s.run(line, 2);
    let mut term = term_core::Term::new(SIZE.cols as usize, SIZE.lines as usize, 1000, term_core::ImagePool::process());
    term.advance(&s.out);
    let lines: Vec<String> = term
        .state
        .take_commands()
        .into_iter()
        .filter_map(|c| match c {
            Mark::CommandLine(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(lines, [line]);
    assert!(s.text().contains("hello a;b 100%"), "{:?}", s.text());
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

/// macOS's /etc/zshrc sets `HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history`, and
/// it runs while ZDOTDIR is still ours: history must still be the user's
/// (up-arrow and Ctrl+R found nothing when it was the app bundle's), in
/// their ZDOTDIR if they have one, and a HISTFILE their .zshrc sets wins.
#[test]
fn zsh_history_stays_the_users() {
    let sets = std::fs::read_to_string("/etc/zshrc").is_ok_and(|t| t.contains("HISTFILE=${ZDOTDIR:-$HOME}"));
    if !sets {
        return;
    }
    let home = TempDir::new("zsh-histfile");
    zsh_files(&home.0, "");
    let probe = check(ZSH, &home, &[], "${HISTFILE-unset}");
    assert_eq!(probe, format!("{}/.zsh_history", home.str()));

    let home = TempDir::new("zsh-histfile-zdotdir");
    let zdot = home.join("config/zsh");
    zsh_files(&zdot, "");
    let probe = check(ZSH, &home, &[("ZDOTDIR", zdot.to_str().unwrap())], "${HISTFILE-unset}");
    assert_eq!(probe, format!("{}/.zsh_history", zdot.display()));

    let home = TempDir::new("zsh-histfile-own");
    zsh_files(&home.0, "HISTFILE=$HOME/.my_history\n");
    let probe = check(ZSH, &home, &[], "${HISTFILE-unset}");
    assert_eq!(probe, format!("{}/.my_history", home.str()));
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// The output so far, parsed by a terminal that knows the token.
fn parsed(s: &Session) -> term_core::Term {
    let mut term = term_core::Term::new(SIZE.cols as usize, SIZE.lines as usize, 1000, term_core::ImagePool::process());
    term.state.set_report_token(TOKEN.as_bytes().try_into().ok());
    term.advance(&s.out);
    term
}

/// zsh given a token reports the line being edited on every redraw, with
/// `;`, `%` and controls encoded and the cursor in bytes, and once what it
/// can run; the token is in no environment and no listing; another
/// pre-redraw widget the user had still runs.
#[test]
fn zsh_reports_the_command_line_with_its_token() {
    let home = TempDir::new("zsh-reports");
    let rc = "zle-line-pre-redraw() { echo pre >> \"$HOME/ran\" }\nzle -N zle-line-pre-redraw\nsetopt autocd\nalias ll='ls -l'\n";
    zsh_files(&home.0, rc);
    let reports = Reports {
        token: TOKEN.into(),
        features: "highlight,list".into(),
    };
    let mut s = Session::start_with(ZSH, &home, &[], Some(reports));
    s.wait_for("the first prompt", |s| s.count("\x1b]133;B\x07") >= 1);
    s.send("echo \"a;b%\" é");
    s.wait_for("the line's report", |s| s.text().contains("%3Bb%25\" é\x07"));
    let term = parsed(&s);
    let r = term.state.line_report().expect("a line report");
    assert_eq!(r.text, "echo \"a;b%\" é");
    assert_eq!((r.cursor, r.keymap), (r.text.len(), term_core::Keymap::Insert));
    // Ctrl+A: the cursor moves to the start.
    s.send("\x01");
    s.wait_for("the cursor's report", |s| s.text().contains(";main;0;echo"));
    assert_eq!(parsed(&s).state.line_report().unwrap().cursor, 0);
    // Run it: the report goes with the C mark.
    s.send("\x05\n");
    s.wait_for("prompt 2", |s| s.count("\x1b]133;B\x07") >= 2);
    let mut term = parsed(&s);
    let env = term.state.take_shell_env().expect("an env report");
    assert!(env.aliases.contains(&"ll".to_string()), "{env:?}");
    assert!(env.autocd);
    assert!(env.builtins.contains(&"cd".to_string()));
    assert!(env.reserved_words.contains(&"then".to_string()));
    assert!(!env.functions.iter().any(|f| f.starts_with('_')), "{:?}", env.functions);
    // macOS's /etc/zshrc sets HISTFILE; without it zsh has none.
    if std::fs::read_to_string("/etc/zshrc").is_ok_and(|t| t.contains("HISTFILE=")) {
        assert_eq!(env.histfile, Some(format!("{}/.zsh_history", home.str())));
    }
    // Once per change: the second prompt sent no second env report.
    assert_eq!(s.count("\x1b]7701;env;"), 1);
    // A continuation line reports nothing of its own.
    s.send("for x in 1; do\n");
    s.wait_for("the continuation's report", |s| s.text().contains(";cont;0;\x07"));
    // Finished, not interrupted: a Ctrl+C that arrives before the line
    // editor has the terminal can be lost (CI's macOS runner), and with
    // it the next prompt.
    s.run("true; done", 3);
    // The probe's output comes before the prompt after it.
    s.run("printenv DECLINE_TOKEN DECLINE_COMPLETION; echo \"<${+DECLINE_TOKEN}|$(set | grep -c 0123456789abcdef)|$(typeset | grep -c 0123456789abcdef)>\"", 4);
    while let Ok(bytes) = s.rx.try_recv() {
        s.out.extend_from_slice(&bytes);
    }
    assert!(s.text().contains("<0|0|0>"), "{:?}", s.text());
    assert!(ran_files(&home).contains("pre"), "the user's own pre-redraw widget ran");
}

/// A choice taken from the app's completion list (`ESC [ 7701 ~` and its
/// payload) replaces the word in zsh, quoted by zsh, keeping the folder
/// typed; a payload that doesn't fit the line is ignored.
#[test]
fn zsh_takes_a_choice_from_the_list() {
    let home = TempDir::new("zsh-take");
    zsh_files(&home.0, "");
    let reports = Reports {
        token: TOKEN.into(),
        features: "highlight,list".into(),
    };
    let mut s = Session::start_with(ZSH, &home, &[], Some(reports));
    s.wait_for("the first prompt", |s| s.count("\x1b]133;B\x07") >= 1);
    let line = |s: &Session| parsed(s).state.line_report().map(|r| r.text.clone());
    s.send("cat src/ma");
    s.wait_for("the typed line", |s| line(s).as_deref() == Some("cat src/ma"));
    s.send("\x1b[7701~4;4;macros/;\x07");
    s.wait_for("the folder taken", |s| line(s).as_deref() == Some("cat src/macros/"));
    // Ctrl+U clears the line; a name zsh must quote, with `;` encoded.
    s.send("\x15echo my");
    s.wait_for("the second line", |s| line(s).as_deref() == Some("echo my"));
    s.send("\x1b[7701~5;0;my file%3Bx; \x07");
    s.wait_for("the quoted name", |s| line(s).as_deref() == Some("echo 'my file;x' "));
    // Past the line's end: nothing changes, and typing carries on.
    s.send("\x1b[7701~99;0;zzz; \x07z");
    s.wait_for("typing after it", |s| line(s).as_deref() == Some("echo 'my file;x' z"));
}

/// Tab asks zsh: with several matches zsh inserts what they share, lists
/// nothing itself, and reports them; taking one by its number has zsh
/// insert it, quoted by zsh. With one match zsh inserts it as before.
#[test]
fn zsh_tab_reports_its_matches_and_takes_one() {
    let home = TempDir::new("zsh-tab");
    zsh_files(&home.0, "autoload -Uz compinit\ncompinit -u -D\n");
    for f in ["alpha1.txt", "alpha2.txt", "x a;b", "x c", "only.md"] {
        write(home.join(f), "");
    }
    let reports = Reports {
        token: TOKEN.into(),
        features: "highlight,list,tab".into(),
    };
    let mut s = Session::start_with(ZSH, &home, &[], Some(reports));
    s.wait_for("the first prompt", |s| s.count("\x1b]133;B\x07") >= 1);
    let line = |s: &Session| parsed(s).state.line_report().map(|r| r.text.clone());
    let matches = |s: &Session| parsed(s).state.take_shell_matches().map(|m| m.shown);
    s.send("cat al");
    s.wait_for("the typed line", |s| line(s).as_deref() == Some("cat al"));
    s.send("\t");
    s.wait_for("what they share", |s| line(s).as_deref() == Some("cat alpha"));
    s.wait_for("the matches", |s| matches(s).is_some());
    assert_eq!(matches(&s).unwrap(), ["alpha1.txt", "alpha2.txt"]);
    s.send("\x1b[7701~#2\x07");
    s.wait_for("the second taken", |s| line(s).as_deref() == Some("cat alpha2.txt "));
    // A name zsh must quote.
    s.send("\x15cat x");
    s.wait_for("the second line", |s| line(s).as_deref() == Some("cat x"));
    let before = s.count("\x1b]7701;matches;");
    s.send("\t");
    s.wait_for("the second matches", |s| s.count("\x1b]7701;matches;") > before);
    s.send("\x1b[7701~#1\x07");
    s.wait_for("the quoted name", |s| line(s).as_deref() == Some("cat x\\ a\\;b "));
    // One match: inserted, nothing reported.
    let before = s.count("\x1b]7701;matches;");
    s.send("\x15cat on\t");
    s.wait_for("the one match", |s| line(s).as_deref() == Some("cat only.md "));
    assert_eq!(s.count("\x1b]7701;matches;"), before);
}

/// Without a token (another terminal, or nothing in the app wants the
/// reports) zsh sends none.
#[test]
fn zsh_without_a_token_reports_nothing() {
    let home = TempDir::new("zsh-no-reports");
    zsh_files(&home.0, "");
    let mut s = Session::start(ZSH, &home, &[]);
    s.wait_for("the first prompt", |s| s.count("\x1b]133;B\x07") >= 1);
    s.send("echo hi");
    s.run("", 2);
    assert_eq!(s.count("\x1b]7701;"), 0, "{:?}", s.text());
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
