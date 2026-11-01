//! Screens: the TUI's output fed into term-core's `Terminal`, a real
//! terminal as the test screen, at 80×24, at 20×5 and after a resize.
//! After every step the terminal must show exactly the frame the TUI
//! drew, with the cursor where it put it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use term_core::{Attrs, ColorKind, Term};
use term_prefs::app::{App, Effect};
use term_prefs::host::{Fake, Pin};
use term_prefs::ui::frame::Renderer;
use term_prefs::ui::input::Decoder;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("term-prefs-screen-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn file(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Screen {
    term: Term,
    renderer: Renderer,
    decoder: Decoder,
    now: Instant,
    sent: Vec<String>,
    edited: Vec<PathBuf>,
    quit: bool,
}

impl Screen {
    fn new(app: &mut App, w: usize, h: usize) -> Screen {
        app.resize(w, h);
        let mut s = Screen {
            term: Term::new(w, h, 0, term_core::ImagePool::process()),
            renderer: Renderer::default(),
            decoder: Decoder::default(),
            now: Instant::now(),
            sent: Vec::new(),
            edited: Vec::new(),
            quit: false,
        };
        s.show(app);
        s
    }

    /// Draws a frame into the terminal and checks the two agree.
    fn show(&mut self, app: &mut App) {
        for e in app.take_effects() {
            match e {
                Effect::Repaint => self.renderer.invalidate(),
                Effect::Send(s) => self.sent.push(s),
                Effect::Quit => self.quit = true,
                Effect::EditFile(p) => self.edited.push(p),
                Effect::Watch(_) => {}
            }
        }
        let frame = app.draw();
        let bytes = self.renderer.render(&frame);
        self.term.advance(bytes.as_bytes());
        for y in 0..frame.height {
            assert_eq!(
                self.row(y),
                frame.row_text(y).trim_end(),
                "row {y} differs from the frame:\n{}",
                self.text()
            );
            for x in 0..frame.width {
                let row = self.term.state.visible_row(y);
                let cell = row.cell(x);
                if !cell.is_spacer() {
                    assert_eq!(
                        cell.style().attrs.contains(Attrs::INVERSE),
                        frame.is_reverse(x, y),
                        "reverse video at {x},{y}"
                    );
                }
            }
        }
        let c = self.term.state.cursor();
        match frame.cursor {
            Some((x, y)) => {
                assert_eq!((c.col(), c.row()), (x, y), "cursor");
                assert!(self.term.state.modes().cursor_visible, "cursor hidden");
            }
            None => assert!(!self.term.state.modes().cursor_visible),
        }
    }

    fn row(&self, y: usize) -> String {
        self.term.state.visible_row(y).text().trim_end().to_string()
    }

    fn text(&self) -> String {
        (0..self.term.state.lines()).map(|y| self.row(y)).collect::<Vec<_>>().join("\n")
    }

    /// Feeds terminal input bytes, a second apart (no write is held back).
    fn send(&mut self, app: &mut App, bytes: &str) {
        for e in self.decoder.feed(bytes.as_bytes()) {
            self.now += Duration::from_secs(1);
            app.handle(e, self.now);
            self.show(app);
        }
    }

    fn resize(&mut self, app: &mut App, w: usize, h: usize) {
        self.term.state.resize(w, h);
        app.resize(w, h);
        self.show(app);
    }

    fn has(&self, s: &str) -> bool {
        self.text().contains(s)
    }

    /// The row the cursor is on.
    fn cursor_row(&self) -> String {
        self.row(self.term.state.cursor().row())
    }
}

const DOWN: &str = "\x1b[B";
const UP: &str = "\x1b[A";
const RIGHT: &str = "\x1b[C";
const LEFT: &str = "\x1b[D";
const ESC: &str = "\x1b[27u";
const ENTER: &str = "\r";

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

#[test]
fn the_layout_at_80_by_24() {
    let dir = Dir::new("layout");
    let cfg = dir.0.join("config.toml");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let s = Screen::new(&mut app, 80, 24);
    let t = s.text();
    // The header, then one card: sections beside the settings, the
    // selected setting under them.
    assert!(s.row(0).starts_with(" Settings  config.toml"), "{t}");
    assert!(s.row(1).starts_with("╭───────────────┬─ Appearance ─"), "{t}");
    assert!(s.row(2).starts_with("│ ▸ Appearance  │ ▸ Appearance        system"), "{t}");
    assert!(t.contains("│   Font        │"), "{t}");
    assert!(t.contains("Light theme       none"), "{t}");
    assert!(t.contains("Theme transition  fade"), "{t}");
    assert!(s.row(18).starts_with("├───────────────┴──"), "{t}");
    // The detail pane: name, value and where it comes from; the
    // description; the sample of the colours.
    assert!(s.row(19).contains("│ Appearance  system  ·  default"), "{t}");
    assert!(s.row(20).contains("Follow macOS"), "{t}");
    assert!(s.row(21).contains("Sample text dim bold"), "{t}");
    assert!(s.row(22).starts_with("╰───"), "{t}");
    assert_eq!(
        s.row(23),
        " ↑↓ move  ←→ section  enter edit  / search  r reset  u undo  ? help  q quit"
    );
    // The real cursor is on the selected row; nothing was written.
    assert!(s.cursor_row().contains("▸ Appearance"));
    assert!(!cfg.exists());
}

#[test]
fn moving_and_sections() {
    let dir = Dir::new("move");
    let cfg = dir.0.join("config.toml");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, RIGHT);
    assert!(s.has("┬─ Font ─"));
    assert!(s.cursor_row().contains("▸ Family"), "{}", s.text());
    s.send(&mut app, DOWN);
    assert!(s.cursor_row().contains("▸ Size") && s.cursor_row().contains("13"), "{}", s.text());
    assert!(s.cursor_row().contains("default"));
    s.send(&mut app, &RIGHT.repeat(4));
    assert!(s.has("┬─ Advanced ─"));
    assert!(s.has("+ add a variable"));
    // Title is a pin's setting only.
    assert!(!s.has("Title"));
    s.send(&mut app, RIGHT);
    assert!(s.has("┬─ Completion ─") && s.has("Completion list"), "{}", s.text());
    s.send(&mut app, RIGHT);
    assert!(s.has("┬─ Appearance ─"));
    s.send(&mut app, &LEFT.repeat(2));
    assert!(s.has("┬─ Advanced ─"));
    // Mouse: click "Keys" in the sections pane.
    s.send(&mut app, "\x1b[<0;5;5M\x1b[<0;5;5m");
    assert!(s.has("┬─ Keys ─"), "{}", s.text());
    assert!(s.has("New Tab") && s.has("⌘T") && s.has("Shell"), "{}", s.text());
    // Wheel down moves the selection, over the Shell menu's header.
    s.send(&mut app, "\x1b[<65;30;5M");
    assert!(
        s.cursor_row().contains("New Window") && s.cursor_row().contains("⌘N"),
        "{}",
        s.text()
    );
    s.send(&mut app, "?");
    assert!(s.has("╭ Help"));
    s.send(&mut app, "x");
    assert!(!s.has("╭ Help"));
    s.send(&mut app, "q");
    assert!(s.quit);
}

#[test]
fn numbers_step_within_their_range_and_esc_cancels() {
    let dir = Dir::new("number");
    let cfg = dir.file("config.toml", "# mine\nfont_size = 13 # big\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("{RIGHT}{DOWN}{ENTER}"));
    assert!(s.cursor_row().contains("‹ 13 ›  (6–72)"), "{}", s.text());
    s.send(&mut app, &format!("{RIGHT}{RIGHT}"));
    assert_eq!(read(&cfg), "# mine\nfont_size = 15 # big\n");
    s.send(&mut app, ESC);
    assert_eq!(read(&cfg), "# mine\nfont_size = 13 # big\n");
    // Typed, out of range: an error, nothing written.
    s.send(&mut app, &format!("{ENTER}99{ENTER}"));
    assert!(s.has("error: \"99\" isn't a number from 6 to 72"), "{}", s.text());
    assert_eq!(read(&cfg), "# mine\nfont_size = 13 # big\n");
    s.send(&mut app, &format!("\x7f\x7f14.5{ENTER}"));
    assert_eq!(read(&cfg), "# mine\nfont_size = 14.5 # big\n");
    assert!(s.cursor_row().contains("14.5") && s.cursor_row().contains("set here"));
    // The bottom of the range holds.
    s.send(&mut app, &format!("{ENTER}{}", LEFT.repeat(20)));
    assert!(read(&cfg).contains("font_size = 6 # big"));
}

#[test]
fn held_back_writes_end_with_the_last_value() {
    let dir = Dir::new("throttle");
    let cfg = dir.file("config.toml", "font_size = 13\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("{RIGHT}{DOWN}{ENTER}"));
    // Key repeat: 15 ms apart.
    let t0 = s.now;
    for i in 1..=5u32 {
        app.handle(
            term_prefs::ui::input::Event::Key(term_prefs::ui::input::Key::Right, Default::default()),
            t0 + Duration::from_millis(15 * i as u64),
        );
    }
    assert_eq!(read(&cfg), "font_size = 14\n", "only the first step is written at once");
    let due = app.deadline().expect("a write is held back");
    app.tick(due - Duration::from_millis(1));
    assert_eq!(read(&cfg), "font_size = 14\n");
    app.tick(due);
    assert_eq!(read(&cfg), "font_size = 18\n");
    assert_eq!(app.deadline(), None);
    s.show(&mut app);
}

#[test]
fn toggles_choices_text_and_colours() {
    let dir = Dir::new("editors");
    let cfg = dir.0.join("config.toml");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    // Toggle, through search.
    s.send(&mut app, &format!("/copy on{ENTER}"));
    assert_eq!(read(&cfg), "copy_on_select = true\n");
    assert!(s.has("Window › Copy on select") && s.cursor_row().contains("on"), "{}", s.text());
    s.send(&mut app, ESC);
    // Choice list with a filter.
    s.send(&mut app, &format!("/option{ENTER}"));
    assert!(s.has("╭ Option as Meta ─") && s.has("│ ❯ "), "{}", s.text());
    assert!(s.has("▸ none"));
    s.send(&mut app, "le");
    assert!(s.has("▸ left") && !s.has("both"), "{}", s.text());
    s.send(&mut app, ENTER);
    assert!(read(&cfg).contains("option_as_meta = \"left\""));
    s.send(&mut app, ESC);
    // Text.
    s.send(&mut app, &format!("/bold face{ENTER}Menlo Bold"));
    assert!(s.cursor_row().contains("Menlo Bold"));
    s.send(&mut app, ENTER);
    assert!(read(&cfg).contains("font_bold = \"Menlo Bold\""));
    s.send(&mut app, ESC);
    // Colour: a bad one is an error, a good one gets its swatch.
    s.send(&mut app, &format!("/foreground{ENTER}red{ENTER}"));
    assert!(s.has("error: \"red\" is not a colour"), "{}", s.text());
    assert!(!read(&cfg).contains("foreground"));
    s.send(&mut app, &format!("{ESC}{ENTER}ff0000{ENTER}"));
    assert!(read(&cfg).contains("[colors]\nforeground = \"#ff0000\""), "{}", read(&cfg));
    // Theme.
    s.send(&mut app, ESC);
    // A theme with no partner on one side is both; then a pair.
    s.send(&mut app, &format!("/dark theme{ENTER}tokyo{ENTER}"));
    assert!(read(&cfg).contains("theme = \"tokyo-night\""), "{}", read(&cfg));
    s.send(&mut app, ESC);
    s.send(&mut app, &format!("/light theme{ENTER}slope light{ENTER}"));
    assert!(
        read(&cfg).contains("theme = { light = \"slope-light\", dark = \"tokyo-night\" }"),
        "{}",
        read(&cfg)
    );
    s.send(&mut app, ESC);
    assert!(s.has("Slope Light") && s.has("Tokyo Night"), "{}", s.text());
    let cfg_text = read(&cfg);
    assert_eq!(fake_notices(&fake, &cfg, &cfg_text), Vec::<String>::new());
}

fn fake_notices(fake: &Fake, cfg: &Path, text: &str) -> Vec<String> {
    use term_prefs::Host;
    fake.check(&[cfg.to_path_buf()], 0, text).notices
}

#[test]
fn fonts_and_lists() {
    let dir = Dir::new("fonts");
    let cfg = dir.0.join("config.toml");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    assert_eq!(*fake.font_scans.borrow(), 0, "fonts are scanned when the picker opens");
    s.send(&mut app, &format!("{RIGHT}{ENTER}"));
    assert_eq!(*fake.font_scans.borrow(), 1);
    // Every font: the monospaced ones, then the others under a header.
    let t = s.text();
    let at = |name: &str| (0..24).find(|&y| s.row(y).contains(name)).unwrap_or_else(|| panic!("{name}: {t}"));
    assert!(at("Monospaced") < at("Menlo") && at("SF Mono") < at("Not monospaced"), "{t}");
    assert!(
        at("Not monospaced") < at("Helvetica") && at("Helvetica") < at("Apple Color Emoji"),
        "{t}"
    );
    s.send(&mut app, &format!("sf{ENTER}"));
    assert_eq!(read(&cfg), "font_family = \"SF Mono\"\n");
    // A typed name that isn't installed.
    s.send(&mut app, &format!("{ENTER}Comic Code{ENTER}"));
    assert!(s.has("error: no font named \"Comic Code\" is installed"), "{}", s.text());
    assert_eq!(*fake.font_scans.borrow(), 2, "read again each time the list opens");
    // The fallback list: add two, reorder, remove.
    s.send(&mut app, &format!("{DOWN}{DOWN}{ENTER}"));
    assert!(s.has("(empty: a adds)"), "{}", s.text());
    s.send(&mut app, &format!("aemoji{ENTER}"));
    s.send(&mut app, &format!("amenlo{ENTER}"));
    assert_eq!(
        read(&cfg),
        "font_family = \"SF Mono\"\nfont_fallback = [\"Apple Color Emoji\", \"Menlo\"]\n"
    );
    s.send(&mut app, "K");
    assert_eq!(
        read(&cfg),
        "font_family = \"SF Mono\"\nfont_fallback = [\"Menlo\", \"Apple Color Emoji\"]\n"
    );
    s.send(&mut app, "d");
    assert_eq!(read(&cfg), "font_family = \"SF Mono\"\nfont_fallback = [\"Apple Color Emoji\"]\n");
    s.send(&mut app, ESC);
    assert!(s.cursor_row().contains("Apple Color Emoji"), "{}", s.text());
}

#[test]
fn shortcuts_typed() {
    let dir = Dir::new("keys");
    let cfg = dir.file("config.toml", "[keybind]\n\"command+K\" = \"new_tab\" # mine\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &RIGHT.repeat(2));
    // Every action under its menu, with its shortcuts as the menus show
    // them: New Tab has ⌘K too, so Clear Scrollback has none.
    let t = s.text();
    assert!(t.contains("Shell") && t.contains("Edit") && t.contains("Window"), "{t}");
    let row = |s: &Screen, what: &str| s.row(line(s, what));
    assert!(
        row(&s, "New Tab").contains("⌘T  ⌘K") && row(&s, "New Tab").contains("set here"),
        "{t}"
    );
    assert!(row(&s, "Clear Scrollback").contains("none"), "{t}");
    assert!(row(&s, "Find").contains("none"), "an action with no default: {t}");
    // Give ⌘K back to Clear Scrollback (found by search): it is New
    // Tab's, so it asks; replacing restores the default, which deletes
    // the entry.
    s.send(&mut app, &format!("/clear scr{ENTER}"));
    assert!(s.has("type the shortcut"), "{}", s.text());
    s.send(&mut app, &format!("cmd+k{ENTER}"));
    assert!(s.has("⌘K is New Tab's: give it to Clear Scrollback instead?"), "{}", s.text());
    assert_eq!(s.row(23), " r replace  esc cancel");
    s.send(&mut app, "r");
    assert_eq!(read(&cfg), "[keybind]\n");
    s.send(&mut app, ESC);
    // Clearing a default writes "none"; resetting deletes the entry.
    s.send(&mut app, &format!("{DOWN}{DOWN}{DOWN}"));
    assert!(s.cursor_row().contains("Close") && s.cursor_row().contains("⌘W"), "{}", s.text());
    assert!(s.row(23).contains("enter record  a add  ⌫ clear  r default"), "{}", s.row(23));
    s.send(&mut app, "\x7f");
    assert_eq!(read(&cfg), "[keybind]\n\"cmd+w\" = \"none\"\n");
    assert!(
        s.cursor_row().contains("none") && s.cursor_row().contains("set here"),
        "{}",
        s.text()
    );
    s.send(&mut app, "r");
    assert_eq!(read(&cfg), "[keybind]\n");
    assert!(s.has("Close reset to ⌘W"), "{}", s.text());
    // New Tab on ⌘N: swap, and New Window takes ⌘T.
    s.send(&mut app, &format!("{UP}c"));
    assert!(s.cursor_row().contains("New Tab"), "{}", s.text());
    s.send(&mut app, &format!("cmd+n{ENTER}"));
    assert!(
        s.has("⌘N is New Window's: swap (it gets ⌘T), or replace (it loses it)?"),
        "{}",
        s.text()
    );
    s.send(&mut app, "s");
    assert_eq!(read(&cfg), "[keybind]\n\"cmd+n\" = \"new_tab\"\n\"cmd+t\" = \"new_window\"\n");
    assert!(s.has("New Tab is now on ⌘N, New Window on ⌘T"), "{}", s.text());
    s.send(&mut app, "u");
    assert_eq!(read(&cfg), "[keybind]\n");
    // Or replace: New Window has no shortcut then.
    s.send(&mut app, &format!("{ENTER}cmd+n{ENTER}r"));
    assert_eq!(read(&cfg), "[keybind]\n\"cmd+n\" = \"new_tab\"\n\"cmd+t\" = \"none\"\n");
    s.send(&mut app, "u");
    // Adding one keeps ⌘T.
    s.send(&mut app, &format!("acmd+j{ENTER}"));
    assert_eq!(read(&cfg), "[keybind]\n\"cmd+j\" = \"new_tab\"\n");
    assert!(s.cursor_row().contains("⌘T  ⌘J"), "{}", s.text());
    s.send(&mut app, "u");
    // A plain key types: it can't be a shortcut. Nor a bad spelling.
    s.send(&mut app, &format!("ck{ENTER}"));
    assert!(s.has("error: K types: a shortcut needs ⌘, ⌃ or ⌥"), "{}", s.text());
    s.send(&mut app, &format!("{ESC}chyper+k{ENTER}"));
    assert!(s.has("error: \"hyper+k\": unknown modifier"), "{}", s.text());
    s.send(&mut app, ESC);
    assert_eq!(read(&cfg), "[keybind]\n");
    // A key that sends text, with escapes; Shift+Enter may (it isn't a
    // menu shortcut).
    s.send(&mut app, "\x1b[F");
    assert!(s.cursor_row().contains("+ add a key that sends text"), "{}", s.text());
    s.send(&mut app, &format!("{ENTER}ctrl+t{ENTER}"));
    assert!(s.has("the text ⌃T sends"), "{}", s.text());
    s.send(&mut app, &format!("\\e[116;5u{ENTER}"));
    assert_eq!(read(&cfg), "[keybind]\n\"ctrl+t\" = { text = \"\\u001B[116;5u\" }\n");
    assert!(s.has("⌃T") && s.has("sends \"\\u{1b}[116;5u\""), "{}", s.text());
    s.send(&mut app, &format!("ashift+enter{ENTER}\\n{ENTER}"));
    assert!(read(&cfg).contains("\"shift+enter\" = { text = "), "{}", read(&cfg));
    // A chord in use asks first.
    s.send(&mut app, &format!("acmd+n{ENTER}"));
    assert!(s.has("⌘N is New Window: replace? (y/n)"), "{}", s.text());
    s.send(&mut app, "n");
    assert!(s.has("cancelled"));
    let text = read(&cfg);
    assert_eq!(fake_notices(&fake, &cfg, &text), Vec::<String>::new());
}

/// In the app, choosing an action waits for the shortcut and the app
/// records it, before the menus see it (`OSC 7771 ; record-chord`). Keys
/// the list needs still work: Esc cancels (the app says so), ⌫ clears,
/// `t` types it instead; a plain key is refused and recording goes on.
#[test]
fn shortcuts_recorded_by_the_app() {
    let dir = Dir::new("record");
    let cfg = dir.0.join("config.toml");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, true);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("{RIGHT}{RIGHT}{DOWN}{DOWN}"));
    assert!(s.cursor_row().contains("New Tab"), "{}", s.text());
    // Change New Tab's shortcut: Enter, then the app's reply.
    s.send(&mut app, ENTER);
    assert_eq!(s.sent, ["\x1b]7771;record-chord\x1b\\"]);
    assert!(s.has("press the shortcut") && s.cursor_row().contains("press the shortcut…"));
    assert!(
        s.row(23).contains("press the shortcut  ⌫ clear  t type it  esc cancel"),
        "{}",
        s.row(23)
    );
    s.send(&mut app, "\x1b]7771;chord;ctrl+cmd+t\x1b\\");
    assert_eq!(read(&cfg), "[keybind]\n\"ctrl+cmd+t\" = \"new_tab\"\n\"cmd+t\" = \"none\"\n");
    assert!(s.has("New Tab is now on ⌃⌘T"), "{}", s.text());
    assert!(s.cursor_row().contains("⌃⌘T"), "{}", s.text());
    // Moving New Window onto a shortcut in use asks first; Esc cancels.
    s.send(&mut app, UP);
    assert!(s.cursor_row().contains("New Window"), "{}", s.text());
    s.send(&mut app, "c\x1b]7771;chord;ctrl+cmd+t\x1b\\");
    assert!(
        s.has("⌃⌘T is New Tab's: swap (it gets ⌘N), or replace (it loses it)?"),
        "{}",
        s.text()
    );
    assert_eq!(s.row(23), " s swap  r replace  esc cancel");
    s.send(&mut app, ESC);
    assert_eq!(read(&cfg), "[keybind]\n\"ctrl+cmd+t\" = \"new_tab\"\n\"cmd+t\" = \"none\"\n");
    // The app cancels (Esc while recording).
    s.send(&mut app, "c\x1b]7771;cancelled\x1b\\");
    assert!(s.has("cancelled"));
    // ⌫ while recording clears the shortcut.
    s.send(&mut app, "c\x1b]7771;chord;backspace\x1b\\");
    assert!(read(&cfg).contains("\"cmd+n\" = \"none\""), "{}", read(&cfg));
    assert!(s.has("New Window has no shortcut now"), "{}", s.text());
    s.send(&mut app, "u");
    // t types it instead.
    s.send(&mut app, "c\x1b]7771;chord;t\x1b\\");
    assert!(s.has("type the shortcut"), "{}", s.text());
    s.send(&mut app, ESC);
    // A plain key types: refused, and the app is asked again.
    let asked = s.sent.len();
    s.send(&mut app, "c\x1b]7771;chord;x\x1b\\");
    assert!(s.has("error: X types"), "{}", s.text());
    assert_eq!(s.sent.len(), asked + 2, "{:?}", s.sent);
    assert_eq!(s.sent.last().unwrap(), "\x1b]7771;record-chord\x1b\\");
    s.send(&mut app, "\x1b]7771;cancelled\x1b\\");
    // A fixed shortcut is taken from its menu item, and the list says so.
    s.send(&mut app, "c\x1b]7771;chord;cmd+q\x1b\\");
    assert!(s.has("New Window is now on ⌘Q (it takes ⌘Q from Quit)"), "{}", s.text());
    assert!(read(&cfg).contains("\"cmd+q\" = \"new_window\""), "{}", read(&cfg));
    assert!(read(&cfg).contains("\"cmd+n\" = \"none\""), "{}", read(&cfg));
    // u undoes both halves of the move.
    s.send(&mut app, "u");
    assert_eq!(read(&cfg), "[keybind]\n\"ctrl+cmd+t\" = \"new_tab\"\n\"cmd+t\" = \"none\"\n");
    // Swapping: New Window takes ⌃⌘T, New Tab ⌘N.
    s.send(&mut app, "c\x1b]7771;chord;ctrl+cmd+t\x1b\\s");
    assert_eq!(
        read(&cfg),
        "[keybind]\n\"ctrl+cmd+t\" = \"new_window\"\n\"cmd+t\" = \"none\"\n\"cmd+n\" = \"new_tab\"\n"
    );
    let text = read(&cfg);
    assert_eq!(fake_notices(&fake, &cfg, &text), Vec::<String>::new());
}

#[test]
fn undo_and_reset() {
    let dir = Dir::new("undo");
    let original = "# mine\npadding = 2 # tight\nfont_size = 13\n";
    let cfg = dir.file("config.toml", original);
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, "/padding");
    assert!(s.cursor_row().contains("2") && s.cursor_row().contains("set here"), "{}", s.text());
    s.send(&mut app, ESC);
    s.send(&mut app, "/padding");
    s.send(&mut app, ESC);
    // Reset from the Appearance section's last row.
    s.send(&mut app, "\x1b[F");
    assert!(s.cursor_row().contains("Padding"), "{}", s.text());
    s.send(&mut app, "r");
    assert_eq!(read(&cfg), "font_size = 13\n");
    assert!(
        s.has("padding reset (default); its comment went with it · u undoes"),
        "{}",
        s.text()
    );
    assert!(s.cursor_row().contains("6") && s.cursor_row().contains("default"));
    s.send(&mut app, "u");
    assert_eq!(read(&cfg), original);
    assert!(s.has("undone"));
    s.send(&mut app, "u");
    assert!(s.has("nothing to undo"));
}

#[test]
fn a_file_that_does_not_parse_is_read_only() {
    let dir = Dir::new("broken");
    let broken = "font_size = = 3\n";
    let cfg = dir.file("config.toml", broken);
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    assert!(s.row(1).starts_with(" error: config.toml doesn't parse"), "{}", s.text());
    assert!(s.row(1).contains("line 1"), "{}", s.text());
    s.send(&mut app, "/copy on");
    s.send(&mut app, ENTER);
    assert!(s.has("error: config.toml can't be edited here"), "{}", s.text());
    assert_eq!(read(&cfg), broken);
    s.send(&mut app, &format!("{ESC}e"));
    // `e` hands the file to the editor (the runner's part).
    assert_eq!(s.edited, std::slice::from_ref(&cfg));
    // Fixed by hand meanwhile: reload says so.
    std::fs::write(&cfg, "font_size = 15\n").unwrap();
    app.reload();
    s.show(&mut app);
    assert!(s.has("config.toml changed on disk: reloaded"), "{}", s.text());
    assert!(!s.has("doesn't parse"));
}

#[test]
fn the_editor_setting_is_read_from_the_layers() {
    let dir = Dir::new("editor");
    let low = dir.file("low.toml", "editor = 'nano'\n");
    let high = dir.file("high.toml", "");
    let fake = Fake::new(high.clone());
    let app = App::new(&fake, vec![low.clone(), high.clone()], 1, false);
    assert_eq!(app.editor_setting(), "nano");
    std::fs::write(&high, "editor = 'code -w'\n").unwrap();
    let app = App::new(&fake, vec![low, high], 1, false);
    assert_eq!(app.editor_setting(), "code -w");
    let none = dir.file("none.toml", "");
    let app = App::new(&fake, vec![none], 0, false);
    assert_eq!(app.editor_setting(), "");
}

#[test]
fn small_screens_and_resizes() {
    let dir = Dir::new("small");
    let cfg = dir.0.join("config.toml");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 20, 5);
    let t = s.text();
    // One column: the section, its settings, the description, the keys.
    assert_eq!(s.row(0), " Appearance 1/7", "{t}");
    assert_eq!(s.row(1), " ▸ Appearan… system", "{t}");
    assert_eq!(s.row(2), "   Light th… none …", "{t}");
    assert_eq!(s.row(3), " Follow macOS, or …", "{t}");
    assert_eq!(s.row(4), " ? help  q quit", "{t}");
    assert!(s.cursor_row().contains("▸"));
    s.send(&mut app, DOWN);
    assert!(s.row(2).contains("▸ Light"), "{}", s.text());
    s.send(&mut app, RIGHT);
    assert_eq!(s.row(0), " Font 2/7", "{}", s.text());
    s.resize(&mut app, 80, 24);
    assert!(s.has("│ ▸ Font        │") && s.cursor_row().contains("▸ Family"), "{}", s.text());
    s.resize(&mut app, 50, 12);
    assert!(s.row(0) == " Font 2/7" && s.cursor_row().contains("▸ Family"), "{}", s.text());
    s.resize(&mut app, 12, 4);
    s.resize(&mut app, 11, 3);
    assert!(s.has("too small"));
    s.resize(&mut app, 20, 5);
    s.send(&mut app, ENTER);
    s.send(&mut app, ESC);
    s.send(&mut app, "\x0c");
}

#[test]
fn pins_open_their_own_file_over_yours() {
    let dir = Dir::new("pins");
    let cfg = dir.file("config.toml", "font_size = 16\ncommand = ['zsh']\n");
    let pin = dir.file("htop.toml", "command = ['htop']\n");
    let fake = Fake::new(cfg.clone());
    fake.pins.borrow_mut().push(Pin {
        name: "htop".into(),
        command: "htop".into(),
        icon: None,
        config: pin.clone(),
    });
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &RIGHT.repeat(4));
    assert!(s.has("┬─ Pins") && s.has("htop"), "{}", s.text());
    s.send(&mut app, ENTER);
    assert!(s.has("editing htop's settings"), "{}", s.text());
    assert!(s.row(0).contains("htop.toml (a pin's own settings)"), "{}", s.text());
    assert!(!s.has("Pins"), "no Pins section in a pin's file");
    s.send(&mut app, RIGHT);
    assert!(s.has("16") && s.has("inherited from config.toml"), "{}", s.text());
    // command can't be reset in a pin; its title is shown.
    s.send(&mut app, &RIGHT.repeat(3));
    assert!(s.has("Title"), "{}", s.text());
    assert!(
        s.cursor_row().contains("htop") && s.cursor_row().contains("set here"),
        "{}",
        s.text()
    );
    s.send(&mut app, "r");
    assert!(s.has("error: command can't be reset in a pin"), "{}", s.text());
    assert_eq!(read(&pin), "command = ['htop']\n");
    // Esc goes back to the user's own file.
    s.send(&mut app, ESC);
    assert!(s.has("back to your own settings") && s.has("│ ▸ Appearance"), "{}", s.text());
    s.send(&mut app, &format!("{}d", RIGHT.repeat(4)));
    assert!(s.has("Remove the pin htop? Its Dock tile stays. (y/n)"), "{}", s.text());
    s.send(&mut app, "y");
    assert!(s.has("removed htop; its Dock tile stays"), "{}", s.text());
    assert!(s.has("No pins"));
}

#[test]
fn the_file_watch_wakes_a_reload() {
    let dir = Dir::new("watch");
    let cfg = dir.file("config.toml", "font_size = 13\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("{RIGHT}{DOWN}{ENTER}{RIGHT}{ESC}"));
    // Our own write says nothing when it comes back.
    s.send(&mut app, &format!("{ENTER}{RIGHT}{ENTER}"));
    app.reload();
    s.show(&mut app);
    assert!(!s.has("changed on disk"), "{}", s.text());
    std::fs::write(&cfg, "font_size = 20\n").unwrap();
    app.reload();
    s.show(&mut app);
    assert!(s.has("changed on disk") && s.cursor_row().contains("20"), "{}", s.text());
}

/// Run by name in any other terminal (`terminal-decline settings`, no
/// `--in-app`), nothing is sent to the terminal: themes preview in the
/// tab alone, and a shortcut is typed, not recorded.
#[test]
fn outside_the_app_nothing_is_sent() {
    let dir = Dir::new("outside");
    let cfg = dir.file("config.toml", "theme = \"tokyo-night\"\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("/dark theme{ENTER}{UP}{UP}"));
    s.send(&mut app, ESC);
    s.send(&mut app, &format!("{ENTER}{UP}{ENTER}"));
    assert!(read(&cfg).contains("solarized-dark"), "the theme list was used: {}", read(&cfg));
    s.send(&mut app, ESC);
    s.send(&mut app, &format!("{RIGHT}{RIGHT}{DOWN}{DOWN}c"));
    assert!(s.has("type the shortcut"), "{}", s.text());
    assert!(s.sent.is_empty(), "nothing for an app that isn't there: {:?}", s.sent);
}

/// While a theme is previewed, this tab picks its colours from what
/// windows show: with `[colors]` setting black on black, a theme whose own
/// colours are readable still gets this tab's own readable text.
#[test]
fn a_preview_keeps_the_colors_overrides_readable() {
    let dir = Dir::new("preview-overrides");
    let cfg = dir.file(
        "config.toml",
        "theme = \"tokyo-night\"\n[colors]\nforeground = \"#000000\"\nbackground = \"#000000\"\n",
    );
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, true);
    let mut s = Screen::new(&mut app, 80, 24);
    let ink = |s: &Screen| {
        let y = (0..24).find(|&y| s.row(y).contains("Slope Dark")).expect("the list");
        let x = s.row(y).find("Slope Dark").map(|b| s.row(y)[..b].chars().count()).unwrap();
        s.term.state.visible_row(y).cell(x).style().fg.kind()
    };
    s.send(&mut app, &format!("/dark theme{ENTER}{UP}"));
    assert_eq!(s.sent, ["\x1b]7771;theme-preview;show;solarized-dark\x1b\\"]);
    // Solarized Dark's own colours are readable; windows show black on
    // black, so this tab draws in its own colours.
    assert_eq!(ink(&s), ColorKind::Rgb(0xe8, 0xe8, 0xe8), "{}", s.text());
    s.send(&mut app, ESC);
}

/// Moving through a theme list previews each theme in every window (the
/// app hears `OSC 7771 ; theme-preview`); Esc puts the old one back and
/// writes nothing, Enter writes it and keeps it shown until the reload.
#[test]
fn theme_lists_preview_live() {
    let dir = Dir::new("preview");
    let cfg = dir.file("config.toml", "theme = \"tokyo-night\"\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, true);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("/dark theme{ENTER}"));
    assert!(s.sent.is_empty(), "opening the list previews nothing: {:?}", s.sent);
    s.send(&mut app, UP);
    assert_eq!(s.sent, ["\x1b]7771;theme-preview;show;solarized-dark\x1b\\"]);
    s.send(&mut app, UP);
    assert_eq!(s.sent.last().unwrap(), "\x1b]7771;theme-preview;show;slope-dark\x1b\\");
    // Typing a filter previews the first match.
    s.send(&mut app, "toky");
    assert_eq!(s.sent.last().unwrap(), "\x1b]7771;theme-preview;show;tokyo-night\x1b\\");
    let n = s.sent.len();
    s.send(&mut app, "o");
    assert_eq!(s.sent.len(), n, "the same theme isn't sent twice");
    s.send(&mut app, ESC);
    assert_eq!(s.sent.last().unwrap(), "\x1b]7771;theme-preview;cancel\x1b\\");
    assert_eq!(read(&cfg), "theme = \"tokyo-night\"\n");
    // Again, and Enter keeps it.
    s.send(&mut app, &format!("{ENTER}{UP}"));
    assert_eq!(s.sent.last().unwrap(), "\x1b]7771;theme-preview;show;solarized-dark\x1b\\");
    s.send(&mut app, ENTER);
    assert_eq!(s.sent.last().unwrap(), "\x1b]7771;theme-preview;keep\x1b\\");
    assert_eq!(read(&cfg), "theme = { light = \"tokyo-night\", dark = \"solarized-dark\" }\n");
    // Each theme in the list shows its colours: `Aa` in its text colour
    // on its background, then its ANSI colours.
    s.send(&mut app, &format!("{ESC}/dark theme{ENTER}"));
    let t = s.text();
    let y = (0..24).find(|&y| s.row(y).contains("Slope Dark")).expect(&t);
    let row = s.row(y);
    assert!(row.contains("Slope Dark") && row.contains(" Aa "), "{t}");
    let x = row.chars().position(|c| c == 'A').unwrap();
    let cell = s.term.state.visible_row(y).cell(x).style();
    assert_eq!(cell.bg.kind(), ColorKind::Rgb(0x09, 0x2d, 0x2b), "Slope Dark's background");
    // This tab is drawn in the terminal's own colours, so it follows the
    // preview (and a transition) with every other window: the
    // selection is the theme's accent colour behind the background's.
    let y = (0..24).find(|&y| s.row(y).contains("▸ Solarized Dark")).expect(&t);
    let x = s.row(y).chars().position(|c| c == '▸').unwrap();
    let sel = s.term.state.visible_row(y).cell(x).style();
    assert!(sel.attrs.contains(Attrs::INVERSE));
    assert!(matches!(sel.fg.kind(), ColorKind::Indexed(_)), "{:?}", sel.fg.kind());
    s.send(&mut app, &format!("{ESC}{ESC}"));
    // At 20×5 the list still works.
    s.resize(&mut app, 20, 5);
    s.send(&mut app, &format!("/dark theme{ENTER}{UP}"));
    assert!(s.has("❯") && s.cursor_row().contains("❯"), "{}", s.text());
    s.send(&mut app, ESC);
    // Run by hand (not in the app), nothing is sent.
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("/dark theme{ENTER}{UP}{ESC}"));
    assert!(s.sent.is_empty(), "{:?}", s.sent);
}

/// The row of a group header (a popup cell holding just `name`).
fn header(s: &Screen, name: &str) -> Option<usize> {
    (0..24).find(|&y| s.row(y).split('│').any(|c| c.trim() == name))
}

/// The row of the first line containing `what`.
fn line(s: &Screen, what: &str) -> usize {
    (0..24)
        .find(|&y| s.row(y).contains(what))
        .unwrap_or_else(|| panic!("{what}: {}", s.text()))
}

/// Theme lists show two faint headers, Dark and Light, the side's own
/// group first. Arrow keys skip the headers, a search keeps only the
/// headers of groups with a match, and the preview follows each move.
#[test]
fn theme_lists_group_by_background() {
    let dir = Dir::new("groups");
    let cfg = dir.file("config.toml", "theme = \"tokyo-night\"\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, true);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("/dark theme{ENTER}"));
    let t = s.text();
    let (hd, hl) = (header(&s, "Dark").expect(&t), header(&s, "Light").expect(&t));
    let (slope_dark, solarized, tokyo, slope_light) = (
        line(&s, "Slope Dark"),
        line(&s, "Solarized Dark"),
        line(&s, "Tokyo Night"),
        line(&s, "Slope Light"),
    );
    assert!(
        hd < slope_dark && slope_dark < solarized && solarized < tokyo && tokyo < hl && hl < slope_light,
        "{t}"
    );
    assert!(s.row(tokyo).contains("▸"), "starts on the current theme\n{t}");
    // Down passes the Light header onto the first light theme; Up comes
    // back over it.
    s.send(&mut app, DOWN);
    assert!(s.row(line(&s, "Slope Light")).contains("▸"), "{}", s.text());
    assert_eq!(s.sent.last().unwrap(), "\x1b]7771;theme-preview;show;slope-light\x1b\\");
    s.send(&mut app, UP);
    assert!(s.row(line(&s, "Tokyo Night")).contains("▸"), "{}", s.text());
    // Search works across both groups, headers only where something matches.
    s.send(&mut app, "slope");
    let t = s.text();
    assert!(header(&s, "Dark").is_some() && header(&s, "Light").is_some(), "{t}");
    s.send(&mut app, "-l");
    let t = s.text();
    assert!(header(&s, "Dark").is_none() && header(&s, "Light").is_some(), "{t}");
    assert_eq!(s.sent.last().unwrap(), "\x1b]7771;theme-preview;show;slope-light\x1b\\");
    s.send(&mut app, &format!("{ESC}{ESC}"));
    // The light picker has the Light group first.
    s.send(&mut app, &format!("/light theme{ENTER}"));
    let t = s.text();
    let (hd, hl) = (header(&s, "Dark").expect(&t), header(&s, "Light").expect(&t));
    assert!(
        hl < line(&s, "Slope Light") && line(&s, "Slope Light") < hd && hd < line(&s, "Slope Dark"),
        "{t}"
    );
    s.send(&mut app, &format!("{ESC}{ESC}"));
    // At 20×5 the selection stays in view, headers or not.
    s.resize(&mut app, 20, 5);
    s.send(&mut app, &format!("/dark theme{ENTER}{DOWN}{UP}{UP}{UP}"));
    assert!(s.has("▸"), "{}", s.text());
    s.send(&mut app, &format!("{ESC}{ESC}"));
}

/// The font list at 80×24 and at 20×5: both groups' headers when there is
/// room, the selection always shown, and searching keeps the header of
/// what matches.
#[test]
fn font_picker_screens() {
    let dir = Dir::new("font-screens");
    let cfg = dir.file("config.toml", "font_family = \"SF Mono\"\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("{RIGHT}{ENTER}"));
    assert!(
        s.has("Font family") && s.has("Monospaced") && s.has("Not monospaced"),
        "{}",
        s.text()
    );
    assert!(s.has("▸ SF Mono"), "the current font is selected: {}", s.text());
    s.send(&mut app, "helv");
    assert!(
        s.has("Not monospaced") && !s.has("Monospaced") && s.has("▸ Helvetica"),
        "{}",
        s.text()
    );
    s.send(&mut app, ESC);
    s.resize(&mut app, 20, 5);
    s.send(&mut app, ENTER);
    assert!(s.has("SF Mono"), "{}", s.text());
    for _ in 0..3 {
        s.send(&mut app, DOWN);
    }
    assert!(s.has("▸ Apple Co"), "scrolled to the selection: {}", s.text());
    s.send(&mut app, ESC);
    assert_eq!(read(&cfg), "font_family = \"SF Mono\"\n");
}

/// Moving through the font list, or typing to filter it, changes nothing
/// in any window: nothing is sent to the app and nothing is written. The
/// font under the selection gets a sample in the list once it has rested
/// for 200 ms; Esc leaves everything as it was, Enter writes the font.
#[test]
fn font_lists_show_a_sample_and_change_nothing_until_enter() {
    use term_prefs::app::SAMPLE_AFTER;
    let dir = Dir::new("font-sample");
    let cfg = dir.file("config.toml", "font_family = \"Menlo\"\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, true);
    app.set_cell_pixels(8, 17);
    let mut s = Screen::new(&mut app, 100, 30);
    s.send(&mut app, &format!("{RIGHT}{ENTER}"));
    assert!(s.has("╭ Font family"), "{}", s.text());
    // The current font's sample comes after the pause too.
    assert_eq!(app.deadline(), Some(s.now + SAMPLE_AFTER));
    assert!(fake.samples.borrow().is_empty());
    // Moving quickly: each step puts the sample off again.
    s.send(&mut app, &format!("{DOWN}{DOWN}"));
    assert!(s.sent.is_empty(), "nothing restyles the app: {:?}", s.sent);
    assert!(fake.samples.borrow().is_empty(), "no sample while moving");
    let due = app.deadline().expect("a sample is due");
    assert_eq!(due, s.now + SAMPLE_AFTER);
    app.tick(due - Duration::from_millis(1));
    s.show(&mut app);
    assert!(fake.samples.borrow().is_empty());
    app.tick(due);
    s.show(&mut app);
    assert_eq!(*fake.samples.borrow(), ["SF Mono"], "only where it rested");
    assert!(s.has("SF Mono  monospaced"), "{}", s.text());
    let frame = app.draw();
    let image = frame.image.as_ref().expect("the sample's picture");
    assert_eq!((image.cols, image.rows), (image.width as usize / 8, image.height as usize / 17));
    assert_eq!(image.rgb.len(), (image.width * image.height * 3) as usize);
    // Drawn again, it isn't drawn by the host again.
    s.show(&mut app);
    assert_eq!(fake.samples.borrow().len(), 1);
    // Typing to filter: still nothing sent, and a font that isn't
    // monospaced says so.
    s.send(&mut app, "helv");
    app.tick(s.now + SAMPLE_AFTER);
    s.show(&mut app);
    assert!(s.has("Helvetica  not monospaced: letters won't line up"), "{}", s.text());
    assert!(s.sent.is_empty(), "{:?}", s.sent);
    // Esc: nothing written, nothing to put back.
    s.send(&mut app, ESC);
    assert!(s.sent.is_empty(), "{:?}", s.sent);
    assert_eq!(read(&cfg), "font_family = \"Menlo\"\n");
    assert!(app.draw().image.is_none(), "the picture goes with the list");
    assert_eq!(app.deadline(), None);
    // Again, and Enter writes it (the app's reload applies it).
    s.send(&mut app, &format!("{ENTER}{DOWN}{DOWN}{ENTER}"));
    assert_eq!(read(&cfg), "font_family = \"SF Mono\"\n");
    assert!(s.sent.is_empty(), "{:?}", s.sent);

    // The bold face: the family's own faces, sampled the same way.
    s.send(&mut app, &format!("{DOWN}{DOWN}{DOWN}"));
    assert!(s.cursor_row().contains("Bold face"), "{}", s.text());
    s.send(&mut app, ENTER);
    assert!(s.has("the family's own") && s.has("SF Mono Bold Italic"), "{}", s.text());
    assert!(!s.has("Menlo Bold"), "only the family's faces: {}", s.text());
    app.tick(s.now + SAMPLE_AFTER);
    s.show(&mut app);
    assert_eq!(fake.samples.borrow().last().unwrap(), "SF Mono", "unset: the family's own face");
    s.send(&mut app, &format!("{DOWN}{DOWN}"));
    app.tick(s.now + SAMPLE_AFTER);
    s.show(&mut app);
    assert_eq!(fake.samples.borrow().last().unwrap(), "SF Mono Bold");
    s.send(&mut app, ENTER);
    assert_eq!(read(&cfg), "font_family = \"SF Mono\"\nfont_bold = \"SF Mono Bold\"\n");
    // A face of another family is typed.
    s.send(&mut app, &format!("{ENTER}Menlo Bold{ENTER}"));
    assert_eq!(read(&cfg), "font_family = \"SF Mono\"\nfont_bold = \"Menlo Bold\"\n");
    // Back to the family's own: unset.
    s.send(&mut app, &format!("{ENTER}own{ENTER}"));
    assert_eq!(read(&cfg), "font_family = \"SF Mono\"\n");
    assert!(s.sent.is_empty(), "{:?}", s.sent);
    // Run by hand, there is no picture (other terminals may not show
    // one), but the name and whether it is monospaced are still there.
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 100, 30);
    s.send(&mut app, &format!("{RIGHT}{ENTER}"));
    app.tick(s.now + SAMPLE_AFTER);
    s.show(&mut app);
    assert!(s.has("SF Mono  monospaced") && app.draw().image.is_none(), "{}", s.text());
}

/// The font list is read each time it opens, so a font installed while
/// Settings is open is there the next time; nothing has to restart.
#[test]
fn a_font_installed_meanwhile_is_listed() {
    let dir = Dir::new("font-installed");
    let cfg = dir.file("config.toml", "");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("{RIGHT}{ENTER}comic"));
    assert!(s.has("❯ comic") && !s.has("Comic Code"), "{}", s.text());
    s.send(&mut app, ESC);
    fake.fonts.borrow_mut().push(term_prefs::Font {
        family: "Comic Code".into(),
        monospace: true,
        faces: vec!["Comic Code Regular".into()],
    });
    s.send(&mut app, &format!("{ENTER}comic"));
    assert!(s.has("▸ Comic Code"), "{}", s.text());
    assert_eq!(*fake.font_scans.borrow(), 2);
    s.send(&mut app, ENTER);
    assert_eq!(read(&cfg), "font_family = \"Comic Code\"\n");
}

/// Names are shown whole; a wide window shows each setting's description
/// beside it, dim, and a narrow one doesn't (the line under the list
/// still has the selected one's).
#[test]
fn wide_windows_show_whole_names_and_descriptions() {
    let dir = Dir::new("wide");
    let cfg = dir.file("config.toml", "");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 160, 40);
    s.send(&mut app, RIGHT);
    let t = s.text();
    let line = |s: &Screen, what: &str| (0..40).find(|&y| s.row(y).contains(what)).unwrap_or_else(|| panic!("{what}: {t}"));
    let y = line(&s, "Strikethrough thickness");
    assert!(s.row(y).contains("Added to the strikethrough's thickness"), "{t}");
    let y = line(&s, "Bold is bright");
    let x = s.row(y).find("Draw bold text").map(|b| s.row(y)[..b].chars().count()).expect(&t);
    assert!(s.term.state.visible_row(y).cell(x).style().attrs.contains(Attrs::DIM), "dim: {t}");
    // The descriptions line up in one column.
    let col = |what: &str| {
        let r = s.row(line(&s, what));
        r.find(what).map(|b| r[..b].chars().count())
    };
    assert_eq!(col("Added to the strikethrough's"), col("Draw bold text"), "{t}");
    s.resize(&mut app, 80, 24);
    let t = s.text();
    assert!(s.has("Strikethrough thickness") || s.has("Family"), "{t}");
    assert!(!s.has("Draw bold text in the first"), "no column at 80: {t}");
}

/// A key held in a theme list previews at most every 100 ms, and always
/// where it stops: each preview re-themes every window.
#[test]
fn a_held_key_previews_themes_at_most_every_100_ms() {
    use term_prefs::app::PREVIEW_EVERY;
    use term_prefs::ui::input::{Event, Key};
    let dir = Dir::new("preview-pace");
    let cfg = dir.file("config.toml", "theme = \"tokyo-night\"\n");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, true);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("/dark theme{ENTER}"));
    let sends = |app: &mut App| -> Vec<String> {
        app.take_effects()
            .into_iter()
            .filter_map(|e| match e {
                Effect::Send(s) => Some(s),
                _ => None,
            })
            .collect()
    };
    let up = || Event::Key(Key::Up, Default::default());
    let t0 = s.now;
    app.handle(up(), t0);
    assert_eq!(
        sends(&mut app),
        ["\x1b]7771;theme-preview;show;solarized-dark\x1b\\"],
        "the first at once"
    );
    // Key repeat, 30 ms apart: held back.
    app.handle(up(), t0 + Duration::from_millis(30));
    assert!(sends(&mut app).is_empty());
    assert_eq!(app.deadline(), Some(t0 + PREVIEW_EVERY));
    app.tick(t0 + PREVIEW_EVERY);
    assert_eq!(
        sends(&mut app),
        ["\x1b]7771;theme-preview;show;slope-dark\x1b\\"],
        "where it stopped"
    );
    // Enter while one is held: written, and nothing more is previewed.
    app.handle(
        Event::Key(Key::Down, Default::default()),
        t0 + PREVIEW_EVERY + Duration::from_millis(10),
    );
    assert!(app.deadline().is_some());
    app.handle(
        Event::Key(Key::Enter, Default::default()),
        t0 + PREVIEW_EVERY + Duration::from_millis(20),
    );
    assert_eq!(sends(&mut app), ["\x1b]7771;theme-preview;keep\x1b\\"]);
    assert_eq!(app.deadline(), None);
    assert!(read(&cfg).contains("solarized-dark"), "{}", read(&cfg));
    s.show(&mut app);
}

/// Moving and scrolling work from what was read: the pins are read once
/// for the section, not on every key, and again only when the files are.
#[test]
fn moving_reads_nothing_again() {
    let dir = Dir::new("no-rereads");
    let cfg = dir.file("config.toml", "");
    let fake = Fake::new(cfg.clone());
    for name in ["htop", "lazygit", "btop"] {
        fake.pins.borrow_mut().push(Pin {
            name: name.into(),
            command: name.into(),
            icon: None,
            config: dir.0.join(format!("{name}.toml")),
        });
    }
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &RIGHT.repeat(4));
    assert!(s.has("┬─ Pins") && s.has("lazygit"), "{}", s.text());
    let reads = *fake.pin_reads.borrow();
    for _ in 0..5 {
        s.send(&mut app, &format!("{DOWN}{UP}\x1b[<65;30;5M\x1b[<64;30;5M"));
    }
    assert_eq!(*fake.pin_reads.borrow(), reads, "no re-reads while moving");
    app.reload();
    s.show(&mut app);
    assert_eq!(*fake.pin_reads.borrow(), reads + 1, "read again with the files");
}
