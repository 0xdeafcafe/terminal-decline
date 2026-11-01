//! The welcome line: one faint line at the top of a new tab or window,
//! written before the shell's first prompt as the "restored from …" line
//! is ([`crate::saved_history`]):
//!
//! ```text
//! Terminal Decline 0.0.1+fdc6d84 · 2 tabs · 27 MB · ⌘↑ jumps to the previous prompt
//! Update available: 5 new commits — press ⌘U to update
//! ```
//!
//! The first line names the build, the tabs open and the app's memory
//! (its footprint, as Activity Monitor's Memory column), and one tip, a
//! different one each time ([`next_tip`]); tips name the chord the user's
//! `[keybind]` gives an action, and are left out while it has none. The
//! second is there only while an update waits ([`Pending`]): new commits
//! on `main` for a build from source, a new build installed and not
//! restarted into, or a release Sparkle found. It names the key of Update
//! Now (Cmd+U by default, [`MenuAction::UpdateNow`]), and is an OSC 8 link
//! to `terminal-decline://update?token=…`: clicking it does the same.
//! Only a link with this process's token is ours ([`is_own_link`]); one
//! a program prints is refused like any other unknown scheme.
//!
//! "If not now, don't keep asking": once the update part has been shown
//! in [`SHOW_TIMES`] tabs, it stays away for `welcome_update_snooze`
//! hours, then shows again; a different update (newer commits, a build
//! installed, a new release) shows at once ([`Seen::update_shown`]). What
//! was shown is kept in `~/Library/Caches/terminal-decline/welcome`,
//! never under the test control.
//!
//! `welcome = "update-only"` keeps only the second line, `"off"` both.
//! Not written in restored tabs (they have their own line), Settings,
//! commands (`-e`, pins with a command, an editor tab), agent API sessions
//! or the quick terminal after its first open.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use crate::config::{Config, WelcomeMode};
use crate::keybind::{Chord, Keybinds, MenuAction};

/// Tabs the update part is shown in before it is snoozed.
pub const SHOW_TIMES: u32 = 3;

/// The scheme and host of the line's own link.
const LINK: &str = "terminal-decline://update";

/// An update waiting to be installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    /// A build from source: `main` is `ahead` commits on, at `head`.
    Commits { ahead: u32, head: String },
    /// A build from source installed while this one ran, not restarted
    /// into yet.
    Installed,
    /// A release Sparkle found; `ready` once it is downloaded and installs
    /// on quit.
    Release { version: String, ready: bool },
}

impl Pending {
    /// What tells one update from another, for the snooze: a newer one
    /// shows again at once.
    pub fn key(&self) -> String {
        match self {
            Pending::Commits { head, .. } => format!("commits:{head}"),
            Pending::Installed => "installed".into(),
            Pending::Release { version, ready } => format!("release:{version}:{ready}"),
        }
    }
}

/// A tip: its text, `{}` standing for the chord of `action` (no tip while
/// it has none).
struct Tip {
    text: &'static str,
    action: Option<MenuAction>,
}

const fn tip(text: &'static str, action: MenuAction) -> Tip {
    Tip {
        text,
        action: Some(action),
    }
}

const fn plain(text: &'static str) -> Tip {
    Tip { text, action: None }
}

/// `{}` in the quick terminal's tip is its hotkey.
const QUICK_TIP: &str = "{} shows the quick terminal";
/// The agent API's tip, only while it is on.
const CTL_TIP: &str = "terminal-decline ctl lets your programs drive tabs";

const TIPS: &[Tip] = &[
    tip("{} jumps to the previous prompt", MenuAction::PreviousPrompt),
    plain(QUICK_TIP),
    tip("{} copies the last command", MenuAction::CopyCommand),
    plain("Option-drag selects a rectangle"),
    tip("{} copies the last command's output", MenuAction::CopyOutput),
    tip("{} opens Settings", MenuAction::Settings),
    plain(CTL_TIP),
    tip("{} splits the pane", MenuAction::SplitRight),
    plain("Cmd-click opens a link"),
    tip("{} jumps to the last failed command", MenuAction::PreviousFailed),
    plain("Option-click on the command line moves the cursor there"),
    tip("{} zooms the pane", MenuAction::TogglePaneZoom),
    tip("{} selects the command's output", MenuAction::SelectOutput),
];

/// What the tips depend on.
pub struct TipContext<'a> {
    pub binds: &'a Keybinds,
    /// `quick_terminal_hotkey`, empty when off.
    pub quick_hotkey: &'a str,
    pub agent_api: bool,
}

/// Tip `i`'s text as it would be shown, `None` when it can't be (its
/// action unbound, the quick terminal or the agent API off).
fn tip_text(i: usize, c: &TipContext) -> Option<String> {
    let t = TIPS.get(i)?;
    if let Some(a) = t.action {
        let chord = c.binds.chords(a).into_iter().next()?;
        return Some(t.text.replace("{}", &chord.symbols()));
    }
    match t.text {
        QUICK_TIP => {
            let chord = Chord::parse(c.quick_hotkey).ok()?;
            Some(QUICK_TIP.replace("{}", &chord.symbols()))
        }
        CTL_TIP => c.agent_api.then(|| CTL_TIP.to_string()),
        text => Some(text.to_string()),
    }
}

/// The tip after `last`: the next one in the list that can be shown,
/// wrapping, so never the same twice in a row while two can be.
pub fn next_tip(last: Option<usize>, c: &TipContext) -> Option<(usize, String)> {
    let n = TIPS.len();
    let start = last.filter(|&l| l < n).map_or(0, |l| l + 1);
    (0..n).map(|k| (start + k) % n).find_map(|i| tip_text(i, c).map(|t| (i, t)))
}

/// The first line's parts, before fitting.
pub struct Info {
    /// `Terminal Decline 0.0.1+fdc6d84`.
    pub app: String,
    /// Tabs open, this one included.
    pub tabs: usize,
    /// The app's footprint, in bytes.
    pub footprint: u64,
    pub tip: Option<String>,
}

/// `Terminal Decline 0.4.0 · 2 tabs · 27 MB · ⌘↑ jumps to the previous
/// prompt`, losing its last parts while it is wider than `cols`.
pub fn info_line(info: &Info, cols: usize) -> String {
    let tabs = match info.tabs {
        1 => "1 tab".to_string(),
        n => format!("{n} tabs"),
    };
    let mb = format!("{} MB", info.footprint.div_ceil(1 << 20));
    let mut parts: Vec<&str> = vec![&info.app, &tabs, &mb];
    if let Some(t) = &info.tip {
        parts.push(t);
    }
    fit(&parts, " · ", cols)
}

/// `parts` joined by `sep`, dropping the last ones while wider than
/// `cols` (the first is cut with `…` if it alone is).
fn fit(parts: &[&str], sep: &str, cols: usize) -> String {
    let width = |s: &str| s.chars().count();
    let mut n = parts.len();
    while n > 1 && width(&parts[..n].join(sep)) > cols {
        n -= 1;
    }
    let line = parts[..n].join(sep);
    cut(&line, cols)
}

fn cut(s: &str, cols: usize) -> String {
    if s.chars().count() <= cols || cols == 0 {
        return s.to_string();
    }
    s.chars().take(cols - 1).chain(['…']).collect()
}

/// The update line: `Update available: 5 new commits — press ⌘U to
/// update`, or, with Update Now unbound, `— click to update`.
pub fn update_line(p: &Pending, key: Option<&Chord>, cols: usize) -> String {
    let press = |what: &str| match key {
        Some(k) => format!("press {} to {what}", k.symbols()),
        None => format!("click to {what}"),
    };
    let app = crate::PRODUCT_NAME;
    let line = match p {
        Pending::Commits { ahead, .. } => {
            format!(
                "{} — {}",
                crate::update::source::available_title(crate::update::source::Relation::Behind, *ahead),
                press("update")
            )
        }
        Pending::Installed => format!("New build installed — {}", press("restart to update")),
        Pending::Release { version, ready: false } => format!("{app} {version} is available — {}", press("update")),
        Pending::Release { version, ready: true } => {
            format!("{app} {version} installs when you quit — {}", press("install it now"))
        }
    };
    cut(&line, cols)
}

/// What `~/Library/Caches/terminal-decline/welcome` keeps, as `key=value`
/// lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The update last shown ([`Pending::key`]).
    pub update: Option<String>,
    /// Tabs it has been shown in since it was new or its snooze ended.
    pub shown: u32,
    /// Not shown again before this (seconds since 1970).
    pub snoozed_until: u64,
    /// The tip last shown.
    pub tip: Option<usize>,
}

impl Seen {
    pub fn parse(text: &str) -> Seen {
        let mut s = Seen::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k {
                "update" if !v.is_empty() => s.update = Some(v.to_string()),
                "shown" => s.shown = v.parse().unwrap_or(0),
                "snoozed_until" => s.snoozed_until = v.parse().unwrap_or(0),
                "tip" => s.tip = v.parse().ok(),
                _ => {}
            }
        }
        s
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        if let Some(u) = &self.update {
            out.push_str(&format!("update={u}\n"));
        }
        out.push_str(&format!("shown={}\nsnoozed_until={}\n", self.shown, self.snoozed_until));
        if let Some(t) = self.tip {
            out.push_str(&format!("tip={t}\n"));
        }
        out
    }

    /// Whether the update `key` is shown in a tab opening `now`, counting
    /// it if so: a new update always is; the same one until it has been
    /// shown [`SHOW_TIMES`] times, then not for `snooze` seconds (0: never
    /// snoozed).
    pub fn update_shown(&mut self, key: &str, now: u64, snooze: u64) -> bool {
        if self.update.as_deref() != Some(key) {
            *self = Seen {
                update: Some(key.to_string()),
                tip: self.tip,
                ..Seen::default()
            };
        }
        if now < self.snoozed_until {
            return false;
        }
        self.shown += 1;
        if self.shown >= SHOW_TIMES && snooze > 0 {
            self.shown = 0;
            self.snoozed_until = now + snooze;
        }
        true
    }
}

thread_local! {
    /// What was shown, loaded from the file at first use.
    static SEEN: RefCell<Option<Seen>> = const { RefCell::new(None) };
    /// The update the test control pretends waits (`fake-update`).
    static FAKE: RefCell<Option<Option<Pending>>> = const { RefCell::new(None) };
    /// Times Update Now ran with a fake update (the dump's `update_now`).
    static FAKE_RUNS: Cell<u32> = const { Cell::new(0) };
    /// Windows are being reopened after a restart or launch.
    static RESTORING: Cell<bool> = const { Cell::new(false) };
    /// The quick terminal has been opened once.
    static QUICK_OPENED: Cell<bool> = const { Cell::new(false) };
    /// A Sparkle update found: its version, and whether it is ready.
    static RELEASE: RefCell<Option<(String, bool)>> = const { RefCell::new(None) };
}

fn testing() -> bool {
    std::env::var_os(crate::control::ENV).is_some_and(|v| !v.is_empty())
}

fn seen_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(Path::new(&home).join("Library/Caches/terminal-decline/welcome"))
}

fn with_seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
    SEEN.with(|s| {
        let mut s = s.borrow_mut();
        let seen = s.get_or_insert_with(|| match testing() {
            true => Seen::default(),
            false => seen_file()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map_or_else(Seen::default, |t| Seen::parse(&t)),
        });
        let before = seen.clone();
        let r = f(seen);
        if *seen != before
            && !testing()
            && let Some(p) = seen_file()
        {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, seen.text());
        }
        r
    })
}

/// Sparkle found `version` (`ready`: downloaded, installing on quit).
pub fn release_found(version: &str, ready: bool) {
    RELEASE.with(|r| {
        let mut r = r.borrow_mut();
        // Found again after it was ready: still ready.
        let ready = ready || r.as_ref().is_some_and(|(v, was)| v == version && *was);
        *r = Some((version.to_string(), ready));
    });
}

/// The test control's `fake-update`: pretend `p` waits (`None`: nothing).
pub fn fake(p: Option<Pending>) {
    FAKE.with(|f| *f.borrow_mut() = Some(p));
}

/// Times Update Now ran with a fake update waiting.
pub fn fake_runs() -> u32 {
    FAKE_RUNS.with(Cell::get)
}

/// The update waiting, if any: the test control's, else a new build
/// installed, else new commits, else a release.
pub fn pending() -> Option<Pending> {
    if let Some(fake) = FAKE.with(|f| f.borrow().clone()) {
        return fake;
    }
    if crate::app::new_build_pending() {
        return Some(Pending::Installed);
    }
    if let Some((ahead, head)) = crate::update::dogfood::available() {
        return Some(Pending::Commits { ahead, head });
    }
    RELEASE
        .with(|r| r.borrow().clone())
        .map(|(version, ready)| Pending::Release { version, ready })
}

/// Terminal Decline › Update Now, Cmd+U, or a click on the line: install
/// the update waiting. Nothing when none is.
pub fn update_now() {
    if FAKE.with(|f| f.borrow().is_some()) {
        if pending().is_some() {
            FAKE_RUNS.with(|n| n.set(n.get() + 1));
        }
        return;
    }
    match pending() {
        Some(Pending::Installed) => crate::app::restart_to_update(),
        Some(Pending::Commits { .. }) => crate::update::dogfood::update_now(),
        Some(Pending::Release { .. }) => crate::update::sparkle::check_now(),
        None => {}
    }
}

/// While windows are reopened (`f`), new tabs get no welcome: they are
/// restored, with their own line.
pub fn restoring<R>(f: impl FnOnce() -> R) -> R {
    let was = RESTORING.with(|r| r.replace(true));
    let out = f();
    RESTORING.with(|r| r.set(was));
    out
}

/// This process's token for the line's link: random, so only the app's
/// own lines carry it.
fn token() -> &'static str {
    static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TOKEN.get_or_init(|| {
        let mut b = [0u8; 16];
        unsafe { libc::arc4random_buf(b.as_mut_ptr().cast(), b.len()) };
        b.iter().map(|x| format!("{x:02x}")).collect()
    })
}

/// The update line's link.
pub fn link() -> String {
    format!("{LINK}?token={}", token())
}

/// Whether `uri` is a link this process wrote (the update line's): a
/// program's `terminal-decline://` link never is.
pub fn is_own_link(uri: &str) -> bool {
    uri == link()
}

/// What kind of tab is opening, for [`lines`].
pub struct Opening {
    /// It runs the login shell (no `-e` command, no pin command, not
    /// Settings, not an agent API session).
    pub shell: bool,
    pub quick: bool,
    /// It has history or a saved layout from before a restart.
    pub restored: bool,
    /// Tabs open, this one included.
    pub tabs: usize,
}

/// The lines a new tab starts with: either may be missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    /// Version, tabs, memory, a tip.
    pub info: Option<String>,
    /// The update waiting, and the key that installs it.
    pub update: Option<String>,
}

/// The lines a tab opening so starts with, counting the update part as
/// shown. None under [`restoring`], nor in the quick terminal after its
/// first open.
pub fn lines(config: &Config, o: &Opening, cols: usize) -> Lines {
    let mut out = Lines::default();
    if config.welcome == WelcomeMode::Off || !o.shell || o.restored || RESTORING.with(Cell::get) {
        return out;
    }
    if o.quick && QUICK_OPENED.with(|q| q.replace(true)) {
        return out;
    }
    let binds = config.keybinds();
    if config.welcome == WelcomeMode::Full {
        let c = TipContext {
            binds: &binds,
            quick_hotkey: &config.quick_terminal_hotkey,
            agent_api: config.agent_api,
        };
        let tip = with_seen(|s| {
            let t = next_tip(s.tip, &c);
            if let Some((i, _)) = &t {
                s.tip = Some(*i);
            }
            t.map(|(_, text)| text)
        });
        let info = Info {
            app: app_version(),
            tabs: o.tabs,
            footprint: crate::control::phys_footprint(),
            tip,
        };
        out.info = Some(info_line(&info, cols));
    }
    if let Some(p) = pending() {
        let snooze = u64::from(config.welcome_update_snooze) * 3600;
        if with_seen(|s| s.update_shown(&p.key(), crate::saved_history::now(), snooze)) {
            let key = binds.chords(MenuAction::UpdateNow).into_iter().next();
            out.update = Some(update_line(&p, key.as_ref(), cols));
        }
    }
    out
}

/// `Terminal Decline 0.4.0`, with the commit a build from source was
/// built from: `Terminal Decline 0.0.1+fdc6d84`.
fn app_version() -> String {
    let version = crate::update::sparkle::own_version();
    let commit = crate::reload::own_bundle()
        .and_then(|b| crate::update::source::of_bundle(&b))
        .map(|s| s.commit[..s.commit.len().min(7)].to_string());
    match commit {
        Some(c) if !version.contains(&c) => format!("{} {version}+{c}", crate::PRODUCT_NAME),
        _ => format!("{} {version}", crate::PRODUCT_NAME),
    }
}

/// Write `lines` into a new pane's terminal, faint, each on a line of
/// its own; the update line is a link to [`link`].
pub fn write(term: &mut term_core::Term, lines: &Lines) {
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    if let Some(info) = &lines.info {
        term.advance(format!("\x1b[0;2m{}\x1b[0m\r\n", clean(info)).as_bytes());
    }
    if let Some(update) = &lines.update {
        let link = link();
        term.advance(format!("\x1b[0;2m\x1b]8;;{link}\x1b\\{}\x1b]8;;\x1b\\\x1b[0m\r\n", clean(update)).as_bytes());
    }
    term.state.take_replies();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(binds: &Keybinds) -> TipContext<'_> {
        TipContext {
            binds,
            quick_hotkey: crate::quick::DEFAULT_HOTKEY,
            agent_api: true,
        }
    }

    #[test]
    fn the_info_line() {
        let info = Info {
            app: "Terminal Decline 0.0.1+fdc6d84".into(),
            tabs: 2,
            footprint: 26 * (1 << 20) + 5,
            tip: Some("⌘↑ jumps to the previous prompt".into()),
        };
        assert_eq!(
            info_line(&info, 200),
            "Terminal Decline 0.0.1+fdc6d84 · 2 tabs · 27 MB · ⌘↑ jumps to the previous prompt"
        );
        // Narrow: the tip goes first, then the memory.
        assert_eq!(info_line(&info, 60), "Terminal Decline 0.0.1+fdc6d84 · 2 tabs · 27 MB");
        assert_eq!(info_line(&info, 40), "Terminal Decline 0.0.1+fdc6d84 · 2 tabs");
        assert_eq!(info_line(&info, 10), "Terminal …");
        let one = Info {
            tabs: 1,
            tip: None,
            ..info
        };
        assert_eq!(info_line(&one, 200), "Terminal Decline 0.0.1+fdc6d84 · 1 tab · 27 MB");
    }

    #[test]
    fn the_update_line() {
        let u = Chord::parse("cmd+u").unwrap();
        let commits = Pending::Commits {
            ahead: 5,
            head: "abc".into(),
        };
        assert_eq!(
            update_line(&commits, Some(&u), 200),
            "Update available: 5 new commits — press ⌘U to update"
        );
        assert_eq!(
            update_line(&commits, None, 200),
            "Update available: 5 new commits — click to update"
        );
        assert_eq!(
            update_line(&Pending::Installed, Some(&u), 200),
            "New build installed — press ⌘U to restart to update"
        );
        let release = |ready| Pending::Release {
            version: "0.6.0".into(),
            ready,
        };
        assert_eq!(
            update_line(&release(false), Some(&u), 200),
            "Terminal Decline 0.6.0 is available — press ⌘U to update"
        );
        assert_eq!(
            update_line(&release(true), Some(&u), 200),
            "Terminal Decline 0.6.0 installs when you quit — press ⌘U to install it now"
        );
        assert_eq!(update_line(&commits, Some(&u), 20).chars().count(), 20);
        // Each update is told apart.
        assert_ne!(release(false).key(), release(true).key());
        assert_ne!(commits.key(), Pending::Installed.key());
    }

    #[test]
    fn tips_rotate_never_twice_in_a_row() {
        let binds = Keybinds::default();
        let c = ctx(&binds);
        let (first, text) = next_tip(None, &c).unwrap();
        assert_eq!((first, text.as_str()), (0, "⌘↑ jumps to the previous prompt"));
        let mut last = Some(first);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..TIPS.len() * 2 {
            let (i, text) = next_tip(last, &c).unwrap();
            assert_ne!(Some(i), last, "{text}");
            seen.insert(text);
            last = Some(i);
        }
        assert_eq!(seen.len(), TIPS.len(), "every tip comes round: {seen:?}");
        assert!(seen.contains("⌃` shows the quick terminal"), "{seen:?}");
        assert!(seen.contains("⇧⌘C copies the last command"), "{seen:?}");
        // From a stale index (a longer list before), it starts again.
        assert_eq!(next_tip(Some(999), &c).map(|t| t.0), Some(0));
    }

    #[test]
    fn tips_follow_the_keys_and_settings() {
        let toml = "[keybind]\n\"cmd+up\" = \"none\"\n\"ctrl+p\" = \"copy_command\"\n\"shift+cmd+c\" = \"none\"\n";
        #[derive(serde::Deserialize)]
        struct T {
            keybind: crate::keybind::KeybindConfig,
        }
        let t: T = toml::from_str(toml).unwrap();
        let binds = Keybinds::new(&t.keybind);
        let c = TipContext {
            binds: &binds,
            quick_hotkey: "",
            agent_api: false,
        };
        let all: Vec<String> = (0..TIPS.len()).filter_map(|i| tip_text(i, &c)).collect();
        assert!(!all.iter().any(|t| t.contains("previous prompt")), "unbound: {all:?}");
        assert!(all.contains(&"⌃P copies the last command".to_string()), "{all:?}");
        assert!(!all.iter().any(|t| t.contains("quick terminal") || t.contains("ctl")), "{all:?}");
        // The first tip unbound: the rotation starts at the next.
        assert_eq!(next_tip(None, &c).map(|t| t.0), Some(2));
    }

    #[test]
    fn the_update_part_snoozes_after_three_tabs() {
        let mut s = Seen::default();
        let day = 24 * 3600;
        let t0 = 1_000_000;
        for n in 0..SHOW_TIMES {
            assert!(s.update_shown("commits:a", t0 + u64::from(n), day), "tab {n}");
        }
        // Snoozed: the fourth tab, and any for a day.
        assert!(!s.update_shown("commits:a", t0 + 10, day));
        assert!(!s.update_shown("commits:a", t0 + day - 1, day));
        // Then three more.
        for n in 0..SHOW_TIMES {
            assert!(s.update_shown("commits:a", t0 + day + 10 + u64::from(n), day));
        }
        assert!(!s.update_shown("commits:a", t0 + day + 20, day));
        // A newer update (more commits) un-snoozes at once.
        assert!(s.update_shown("commits:b", t0 + day + 21, day));
        assert_eq!(s.shown, 1);
        // So does the build being installed.
        assert!(s.update_shown("installed", t0 + day + 22, day));
    }

    #[test]
    fn no_snooze_with_zero() {
        let mut s = Seen::default();
        for n in 0..10 {
            assert!(s.update_shown("installed", n, 0));
        }
    }

    #[test]
    fn seen_reads_back() {
        let s = Seen {
            update: Some("release:0.6.0:false".into()),
            shown: 2,
            snoozed_until: 1234,
            tip: Some(3),
        };
        assert_eq!(Seen::parse(&s.text()), s);
        assert_eq!(Seen::parse("junk\nshown=x\n"), Seen::default());
        // The tip survives a new update.
        let mut s = s;
        s.update_shown("installed", 0, 10);
        assert_eq!((s.tip, s.shown, s.update.as_deref()), (Some(3), 1, Some("installed")));
    }

    #[test]
    fn only_our_own_link_is_ours() {
        let ours = link();
        assert!(ours.starts_with("terminal-decline://update?token=") && ours.len() > LINK.len() + 30);
        assert!(is_own_link(&ours));
        assert!(!is_own_link("terminal-decline://update"));
        assert!(!is_own_link("terminal-decline://update?token=00000000000000000000000000000000"));
        // A program's copy is refused as any unknown scheme is.
        assert!(crate::links::to_open("terminal-decline://update", "mac").is_err());
    }

    #[test]
    fn the_lines_are_faint_and_the_update_is_a_link() {
        let mut t = term_core::Term::new(100, 5, 100, term_core::ImagePool::process());
        let lines = Lines {
            info: Some("Terminal Decline 0.4.0 · 1 tab".into()),
            update: Some("Update available".into()),
        };
        write(&mut t, &lines);
        let dump = t.state.dump();
        assert!(dump.starts_with("Terminal Decline 0.4.0 · 1 tab"), "{dump}");
        assert!(dump.contains("Update available"), "{dump}");
        let p = term_core::Point { line: 1, col: 2 };
        assert_eq!(t.state.link_at(p).map(|l| l.uri), Some(link()));
        assert!(t.state.link_at(term_core::Point { line: 0, col: 2 }).is_none());
    }
}
