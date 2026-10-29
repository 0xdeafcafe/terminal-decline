//! Screens: the TUI's output fed into term-core's `Terminal`, a real
//! terminal as the test screen, at 80×24, at 20×5 and after a resize.
//! After every step the terminal must show exactly the frame the TUI
//! drew, with the cursor where it put it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use term_core::{Attrs, Term};
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
    assert!(s.row(0).starts_with("┌ Sections ─"), "{t}");
    assert!(s.row(0).contains("┐┌ Appearance ─"), "{t}");
    assert!(s.row(1).starts_with("│ ▸ Appearance  ││ ▸ Theme"), "{t}");
    assert!(t.contains("│   Font"), "{t}");
    assert!(t.contains("Light/dark pair   off"), "{t}");
    assert!(t.contains("Blur") || t.contains("Colour"), "{t}");
    assert!(s.row(23).contains("↑↓ move  ←→ section  enter edit  / search"), "{t}");
    // The description of the selected row, and the preview strip.
    assert!(s.row(22).contains("A built-in theme"), "{t}");
    assert!(s.row(21).contains("Preview  Sample text"), "{t}");
    // The real cursor is on the selected row; nothing was written.
    assert!(s.cursor_row().contains("▸ Theme"));
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
    assert!(s.has("┌ Font ─"));
    assert!(s.cursor_row().contains("▸ Family"), "{}", s.text());
    s.send(&mut app, DOWN);
    assert!(s.cursor_row().contains("▸ Size") && s.cursor_row().contains("13"), "{}", s.text());
    assert!(s.cursor_row().contains("default"));
    s.send(&mut app, &RIGHT.repeat(4));
    assert!(s.has("┌ Advanced ─"));
    assert!(s.has("+ add a variable"));
    // Title is a pin's setting only.
    assert!(!s.has("Title"));
    s.send(&mut app, RIGHT);
    assert!(s.has("┌ Appearance ─"));
    s.send(&mut app, LEFT);
    assert!(s.has("┌ Advanced ─"));
    // Mouse: click "Keys" in the sections pane.
    s.send(&mut app, "\x1b[<0;5;4M\x1b[<0;5;4m");
    assert!(s.has("┌ Keys ─"), "{}", s.text());
    assert!(s.has("cmd+t") && s.has("New Tab"));
    // Wheel down moves the selection.
    s.send(&mut app, "\x1b[<65;30;5M");
    assert!(s.cursor_row().contains("cmd+n"), "{}", s.text());
    s.send(&mut app, "?");
    assert!(s.has("┌ Help"));
    s.send(&mut app, "x");
    assert!(!s.has("┌ Help"));
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
    assert!(s.has("filter:"), "{}", s.text());
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
    s.send(&mut app, &format!("/theme{ENTER}tokyo{ENTER}"));
    assert!(read(&cfg).contains("theme = \"tokyo-night\""), "{}", read(&cfg));
    s.send(&mut app, &format!("{DOWN}{ENTER}"));
    assert!(
        read(&cfg).contains("theme = { light = \"tokyo-night\", dark = \"tokyo-night\" }"),
        "{}",
        read(&cfg)
    );
    assert!(s.has("Light theme") && s.has("Dark theme"), "{}", s.text());
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
    assert!(s.has("SF Mono") && !s.has("Helvetica"), "{}", s.text());
    s.send(&mut app, "\t");
    assert!(s.has("Helvetica (not monospace: text will look uneven)"), "{}", s.text());
    s.send(&mut app, &format!("sf{ENTER}"));
    assert_eq!(read(&cfg), "font_family = \"SF Mono\"\n");
    // A typed name that isn't installed.
    s.send(&mut app, &format!("{ENTER}Comic Code{ENTER}"));
    assert!(s.has("error: no font named \"Comic Code\" is installed"), "{}", s.text());
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
    assert!(s.has("cmd+k") && s.has("set here"), "{}", s.text());
    // Rebind cmd+k (found by its parsed chord) to Find.
    s.send(&mut app, &format!("/cmd+k{ENTER}find{ENTER}"));
    assert_eq!(read(&cfg), "[keybind]\n\"command+K\" = \"find\" # mine\n");
    // Restoring the default deletes the entry.
    s.send(&mut app, &format!("{ENTER}clear_scr{ENTER}"));
    assert_eq!(read(&cfg), "[keybind]\n");
    s.send(&mut app, ESC);
    // Unbinding a default writes "none"; restoring it deletes the entry.
    s.send(&mut app, &format!("{DOWN}{DOWN}{DOWN}"));
    assert!(s.cursor_row().contains("cmd+w") && s.cursor_row().contains("Close"), "{}", s.text());
    s.send(&mut app, "d");
    assert_eq!(read(&cfg), "[keybind]\n\"cmd+w\" = \"none\"\n");
    assert!(
        s.cursor_row().contains("none") && s.cursor_row().contains("set here"),
        "{}",
        s.text()
    );
    s.send(&mut app, "r");
    assert_eq!(read(&cfg), "[keybind]\n");
    // Add a shortcut by typing it.
    s.send(&mut app, "a");
    assert!(s.has("type the shortcut"), "{}", s.text());
    s.send(&mut app, &format!("hyper+k{ENTER}"));
    assert!(s.has("error: \"hyper+k\": unknown modifier"), "{}", s.text());
    s.send(&mut app, &format!("{ESC}a"));
    s.send(&mut app, &format!("cmd+shift+k{ENTER}"));
    assert!(s.has("shift+cmd+k does"), "{}", s.text());
    s.send(&mut app, &format!("new window{ENTER}"));
    assert_eq!(read(&cfg), "[keybind]\n\"shift+cmd+k\" = \"new_window\"\n");
    // A chord in use asks first.
    s.send(&mut app, &format!("acmd+n{ENTER}"));
    assert!(s.has("cmd+n is New Window: replace? (y/n)"), "{}", s.text());
    s.send(&mut app, "n");
    assert!(s.has("cancelled"));
    // Text to send, with escapes.
    s.send(&mut app, &format!("a{}", "ctrl+t\r"));
    s.send(&mut app, &format!("send text{ENTER}\\e[116;5u{ENTER}"));
    assert_eq!(
        read(&cfg),
        "[keybind]\n\"shift+cmd+k\" = \"new_window\"\n\"ctrl+t\" = { text = \"\\u001B[116;5u\" }\n"
    );
}

#[test]
fn shortcuts_recorded_by_the_app() {
    let dir = Dir::new("record");
    let cfg = dir.0.join("config.toml");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, true);
    let mut s = Screen::new(&mut app, 80, 24);
    s.send(&mut app, &format!("{RIGHT}{RIGHT}{DOWN}{DOWN}"));
    assert!(s.cursor_row().contains("cmd+t"), "{}", s.text());
    // Change New Tab's shortcut: c, then the app's reply.
    s.send(&mut app, "c");
    assert_eq!(s.sent, ["\x1b]7771;record-chord\x1b\\"]);
    assert!(s.has("press the shortcut"));
    s.send(&mut app, "\x1b]7771;chord;ctrl+cmd+t\x1b\\");
    assert_eq!(read(&cfg), "[keybind]\n\"ctrl+cmd+t\" = \"new_tab\"\n\"cmd+t\" = \"none\"\n");
    assert!(s.has("New Tab is now on ctrl+cmd+t"), "{}", s.text());
    // Moving New Window onto a chord in use asks first.
    s.send(&mut app, "\x1b[A");
    assert!(s.cursor_row().contains("cmd+n"), "{}", s.text());
    s.send(&mut app, "c\x1b]7771;chord;ctrl+cmd+t\x1b\\");
    assert!(s.has("ctrl+cmd+t is New Tab: replace? (y/n)"), "{}", s.text());
    s.send(&mut app, "n");
    assert_eq!(read(&cfg), "[keybind]\n\"ctrl+cmd+t\" = \"new_tab\"\n\"cmd+t\" = \"none\"\n");
    // The app cancels.
    s.send(&mut app, "c\x1b]7771;cancelled\x1b\\");
    assert!(s.has("cancelled"));
    // A fixed shortcut is taken from its menu item, and the TUI says so.
    s.send(&mut app, "c\x1b]7771;chord;cmd+q\x1b\\");
    assert!(s.has("it takes cmd+q from Quit"), "{}", s.text());
    assert!(read(&cfg).contains("\"cmd+q\" = \"new_window\""), "{}", read(&cfg));
    assert!(read(&cfg).contains("\"cmd+n\" = \"none\""), "{}", read(&cfg));
    // u undoes both halves of the move.
    s.send(&mut app, "u");
    assert_eq!(read(&cfg), "[keybind]\n\"ctrl+cmd+t\" = \"new_tab\"\n\"cmd+t\" = \"none\"\n");
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
    assert!(s.row(0).starts_with("error: config.toml doesn't parse"), "{}", s.text());
    assert!(s.row(0).contains("line 1"), "{}", s.text());
    s.send(&mut app, "/copy on");
    s.send(&mut app, ENTER);
    assert!(s.has("error: config.toml can't be edited here"), "{}", s.text());
    assert_eq!(read(&cfg), broken);
    s.send(&mut app, &format!("{ESC}e"));
    // `e` hands the file to $EDITOR (the runner's part).
    assert_eq!(s.edited, std::slice::from_ref(&cfg));
    // Fixed by hand meanwhile: reload says so.
    std::fs::write(&cfg, "font_size = 15\n").unwrap();
    app.reload();
    s.show(&mut app);
    assert!(s.has("config.toml changed on disk: reloaded"), "{}", s.text());
    assert!(!s.has("doesn't parse"));
}

#[test]
fn small_screens_and_resizes() {
    let dir = Dir::new("small");
    let cfg = dir.0.join("config.toml");
    let fake = Fake::new(cfg.clone());
    let mut app = App::new(&fake, vec![cfg.clone()], 0, false);
    let mut s = Screen::new(&mut app, 20, 5);
    let t = s.text();
    assert!(s.row(0).starts_with("┌ Appearance (1/6)"), "{t}");
    assert!(s.row(1).starts_with("│ ▸ Theme"), "{t}");
    assert!(s.cursor_row().contains("▸"));
    s.send(&mut app, DOWN);
    assert!(s.row(1).contains("▸ Light/d"), "{}", s.text());
    s.send(&mut app, RIGHT);
    assert!(s.row(0).starts_with("┌ Font (2/6)"), "{}", s.text());
    s.resize(&mut app, 80, 24);
    assert!(s.has("┌ Sections") && s.cursor_row().contains("▸ Family"), "{}", s.text());
    s.resize(&mut app, 50, 12);
    assert!(s.has("┌ Sections"), "{}", s.text());
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
    assert!(s.has("┌ Pins") && s.has("htop"), "{}", s.text());
    s.send(&mut app, ENTER);
    assert!(s.has("editing htop's settings"), "{}", s.text());
    assert!(s.has("Appearance · htop.toml"), "{}", s.text());
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
    assert!(s.has("back to your own settings") && s.has("┌ Sections"), "{}", s.text());
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
