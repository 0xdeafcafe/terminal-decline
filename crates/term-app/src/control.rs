//! Test-only remote control, for the smoke tests that drive the real app.
//!
//! Only when `DECLINE_TEST_DUMP=<file>` is set, a program in the terminal can
//! send `ESC ] 7770 ; <command> BEL` (or ST instead of BEL):
//!
//! - `dump`: after the next frame the GPU has finished, append one line of
//!   JSON to the file (see [`Dump`]);
//! - `resize C L`: resize the window to C columns × L lines;
//! - `font +1` / `font -1`: change the font size by one point;
//! - `tab`: open a new tab beside this window, running the same program;
//! - `front`: bring this window or tab to the front;
//! - `drop PATH[<tab>PATH…]`: as if those files were dragged onto the
//!   window (paths separated by tabs, so they may hold spaces);
//! - `close`: close the tab as its close button does (asking first with
//!   `confirm_close`);
//! - `split right` / `split down`: split the focused pane (Cmd+D,
//!   Cmd+Shift+D);
//! - `focus N` / `focus next|previous|left|right|up|down`: focus the
//!   Nth pane (from 0, in the order the dump lists them), or as the pane
//!   actions do;
//! - `close-pane`: close the focused pane as Cmd+W does;
//! - `drag N PX`: drag divider N (from 0, in tree order) so its line
//!   starts at PX (window pixels: x for side-by-side panes, y for
//!   stacked), as the mouse does;
//! - `resize-pane left|right|up|down`: move the focused pane's divider
//!   one cell, as Cmd+Ctrl+arrows do;
//! - `equalize`: equal shares (Cmd+Ctrl+=);
//! - `zoom`: zoom the focused pane, or end the zoom (Cmd+Shift+Enter);
//! - `save`: write what a restart would save (every window, its tabs and
//!   panes) to the dump file's path with `.saved` added, as JSON;
//! - `settings`: Terminal Decline > Settings… (Cmd+,);
//! - `type-settings TEXT`: type TEXT into the Settings tab, as keys would
//!   (`\r`, `\e` and `\\` stand for Return, Esc and a backslash);
//! - `key CODE [MODS]`: put a key press on the app's event queue, where a
//!   real key arrives: monitors and menus first (MODS: `cmd`, `ctrl`, `alt`,
//!   `shift` joined by `+`);
//! - `quit`: quit the app, without asking.
//!
//! While this is on, the window draws even when covered by other windows,
//! so a dump never waits for a frame that is never drawn.
//!
//! Commands run in order; one after a `dump` waits until the dump is
//! written. Without the env var the sequence is ignored like any unknown
//! OSC.

use serde::Serialize;

use crate::panes::{Dir, Direction, PaneId};

/// The env var that turns this on, naming the dump file.
pub const ENV: &str = "DECLINE_TEST_DUMP";
/// The OSC number.
pub const OSC: &str = "7770";

const PREFIX: &[u8] = b"\x1b]7770;";
/// Longer commands are dropped.
const MAX_LEN: usize = 256;

/// Finds control sequences in program output, across reads: `7770`'s
/// (by default), or another OSC's ([`ControlScanner::new`]).
#[derive(Debug)]
pub struct ControlScanner {
    prefix: &'static [u8],
    state: State,
    payload: Vec<u8>,
}

impl Default for ControlScanner {
    fn default() -> ControlScanner {
        ControlScanner::new(PREFIX)
    }
}

/// The preferences' OSC (see [`crate::prefs`]): `record-chord` asks the
/// app to record the next key, honoured only in the Settings tab.
pub const PREFS_PREFIX: &[u8] = b"\x1b]7771;";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// Matched this many bytes of [`PREFIX`].
    Prefix(usize),
    Payload,
    /// ESC inside the payload: `\` ends it (ST).
    PayloadEsc,
}

impl ControlScanner {
    /// A scanner for `ESC ] <number> ;` sequences, `prefix` being those
    /// bytes.
    pub fn new(prefix: &'static [u8]) -> ControlScanner {
        ControlScanner {
            prefix,
            state: State::Ground,
            payload: Vec::new(),
        }
    }

    /// Scan `bytes`, pushing `(end, command)` for each sequence that ends in
    /// them, `end` being the offset just past its last byte.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<(usize, String)>) {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground {
                match bytes[i..].iter().position(|&b| b == 0x1b) {
                    Some(k) => i += k,
                    None => return,
                }
            }
            let b = bytes[i];
            self.state = match self.state {
                State::Ground => State::Prefix(1),
                State::Prefix(k) if b == self.prefix[k] => {
                    if k + 1 == self.prefix.len() {
                        self.payload.clear();
                        State::Payload
                    } else {
                        State::Prefix(k + 1)
                    }
                }
                State::Prefix(_) if b == 0x1b => State::Prefix(1),
                State::Prefix(_) => State::Ground,
                State::Payload => match b {
                    0x07 => {
                        out.push((i + 1, String::from_utf8_lossy(&self.payload).into_owned()));
                        State::Ground
                    }
                    0x1b => State::PayloadEsc,
                    _ if self.payload.len() >= MAX_LEN => State::Ground,
                    _ => {
                        self.payload.push(b);
                        State::Payload
                    }
                },
                State::PayloadEsc if b == b'\\' => {
                    out.push((i + 1, String::from_utf8_lossy(&self.payload).into_owned()));
                    State::Ground
                }
                State::PayloadEsc if b == 0x1b => State::Prefix(1),
                State::PayloadEsc => State::Ground,
            };
            i += 1;
        }
    }
}

/// A parsed control command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    Dump,
    Resize {
        cols: u16,
        lines: u16,
    },
    /// Change the font size by this many points.
    Font(i32),
    /// Open a tab beside this window (Shell > New Tab).
    Tab,
    /// Bring this window or tab to the front (as a notification click does).
    Front,
    /// Files dropped on the window.
    Drop(Vec<String>),
    /// The tab's close button.
    Close,
    /// Shell > Split Right / Split Down.
    Split(Dir),
    /// Focus a pane.
    Focus(FocusTarget),
    /// Shell > Close: the focused pane.
    ClosePane,
    /// Drag divider `index` (tree order) so its line starts at `to`
    /// pixels.
    Drag {
        index: usize,
        to: i32,
    },
    /// Window > Move Divider ….
    ResizePane(Direction),
    /// Window > Equalize Panes.
    Equalize,
    /// Window > Zoom Pane.
    Zoom,
    /// Write the windows as a restart saves them.
    Save,
    /// Terminal Decline > Settings….
    Settings,
    /// Bytes typed into the Settings tab.
    TypeSettings(Vec<u8>),
    /// A key event for the app: key code and `NSEventModifierFlags`.
    Key {
        keycode: u16,
        flags: u64,
    },
    Quit,
}

/// Which pane `focus` moves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusTarget {
    /// The Nth in tree order, from 0.
    Index(usize),
    Next,
    Previous,
    Towards(Direction),
}

/// `\r`, `\e` and `\\` as the bytes they stand for.
fn unescape(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        match bytes.next() {
            Some(b'r') => out.push(b'\r'),
            Some(b'e') => out.push(0x1b),
            Some(other) => out.push(other),
            None => out.push(b'\\'),
        }
    }
    out
}

impl Control {
    pub fn parse(s: &str) -> Option<Control> {
        if let Some(paths) = s.trim_start().strip_prefix("drop ") {
            let paths: Vec<String> = paths.split('\t').filter(|p| !p.is_empty()).map(str::to_owned).collect();
            return (!paths.is_empty()).then_some(Control::Drop(paths));
        }
        if let Some(text) = s.trim_start().strip_prefix("type-settings ") {
            return Some(Control::TypeSettings(unescape(text)));
        }
        let mut words = s.split_whitespace();
        let c = match words.next()? {
            "dump" => Control::Dump,
            "quit" => Control::Quit,
            "tab" => Control::Tab,
            "front" => Control::Front,
            "close" => Control::Close,
            "close-pane" => Control::ClosePane,
            "equalize" => Control::Equalize,
            "zoom" => Control::Zoom,
            "drag" => Control::Drag {
                index: words.next()?.parse().ok()?,
                to: words.next()?.parse().ok()?,
            },
            "resize-pane" => Control::ResizePane(match words.next()? {
                "left" => Direction::Left,
                "right" => Direction::Right,
                "up" => Direction::Up,
                "down" => Direction::Down,
                _ => return None,
            }),
            "save" => Control::Save,
            "split" => Control::Split(match words.next()? {
                "right" => Dir::Right,
                "down" => Dir::Down,
                _ => return None,
            }),
            "focus" => Control::Focus(match words.next()? {
                "next" => FocusTarget::Next,
                "previous" => FocusTarget::Previous,
                "left" => FocusTarget::Towards(Direction::Left),
                "right" => FocusTarget::Towards(Direction::Right),
                "up" => FocusTarget::Towards(Direction::Up),
                "down" => FocusTarget::Towards(Direction::Down),
                n => FocusTarget::Index(n.parse().ok()?),
            }),
            "settings" => Control::Settings,
            "resize" => Control::Resize {
                cols: words.next()?.parse().ok().filter(|&c| c > 0)?,
                lines: words.next()?.parse().ok().filter(|&l| l > 0)?,
            },
            "font" => Control::Font(words.next()?.parse().ok()?),
            "key" => {
                let keycode = words.next()?.parse().ok()?;
                let mut flags = 0;
                if let Some(mods) = words.next() {
                    for m in mods.split('+') {
                        flags |= match m {
                            "cmd" => 1 << 20,
                            "alt" => 1 << 19,
                            "ctrl" => 1 << 18,
                            "shift" => 1 << 17,
                            _ => return None,
                        };
                    }
                }
                Control::Key { keycode, flags }
            }
            _ => return None,
        };
        words.next().is_none().then_some(c)
    }
}

/// One dump, written as a line of JSON.
#[derive(Clone, Debug, Serialize)]
pub struct Dump {
    /// 1 for the first dump of this window, then 2, …
    pub seq: u64,
    /// Which window (or tab) this is: 1 for the first opened, then 2, …
    pub window: usize,
    /// Tabs in this window's tab group (1 without tabs).
    pub tabs: usize,
    /// This tab's place in the tab bar, from 0 (0 without tabs).
    pub tab: usize,
    /// Whether the window can be seen (a background tab can't).
    pub visible: bool,
    /// Times the window was hidden or covered and gave back its drawables.
    pub hidden: u64,
    /// Top padding in pixels: the title bar, the tab bar while one shows,
    /// and the configured padding.
    pub pad_top: u32,
    /// Notifications so far, for the whole app (see [`crate::notify`]).
    pub notifications: crate::notify::Stats,
    /// Whether a sheet (the question `confirm_close` asks) is showing.
    pub sheet: bool,
    /// The Dock icon's badge (progress).
    pub badge: Option<String>,
    /// Whether the app is active, and whether it holds Secure Keyboard
    /// Entry (only while active, when turned on).
    pub active: bool,
    pub secure_input: bool,
    /// Whether the Settings tab's preferences have started and read keys
    /// (its terminal is on the alternate screen, which they switch to once
    /// the tty is raw), and whether it is recording a chord. False with no
    /// Settings tab. For the whole app, like `notifications`.
    pub settings_ready: bool,
    pub settings_recording: bool,
    /// `Terminal::dump()` of the focused pane: the visible rows and a
    /// status line.
    pub screen: String,
    pub cols: usize,
    pub lines: usize,
    /// Every pane, in tree order (left to right, top to bottom).
    pub panes: Vec<PaneDump>,
    /// The dividers drawn between them.
    pub dividers: usize,
    /// The zoomed pane's id, while one fills the window.
    pub zoomed: Option<PaneId>,
    /// Frames the GPU has finished drawing.
    pub frames: u64,
    /// The last frame's glyph and rectangle (background, decoration)
    /// instance counts.
    pub glyphs: usize,
    pub rects: usize,
    /// Side lengths of the text and colour glyph textures, pixels.
    pub mask_texture: u32,
    pub color_texture: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub font_size: f64,
    pub scale: f64,
    /// The theme's foreground and background in effect, as `#rrggbb`.
    pub foreground: String,
    pub background: String,
    /// `phys_footprint` from `task_info`, bytes.
    pub phys_footprint: u64,
}

/// One pane in a dump.
#[derive(Clone, Debug, Serialize)]
pub struct PaneDump {
    pub id: PaneId,
    /// Where it is in the window: x, y, width, height in pixels.
    pub rect: [i32; 4],
    pub cols: usize,
    pub lines: usize,
    pub focused: bool,
    /// Shown: not hidden by another pane's zoom.
    pub shown: bool,
    /// As [`Dump::screen`].
    pub screen: String,
}

/// This process's memory footprint as Activity Monitor and `footprint`
/// count it (`phys_footprint` from `task_info(TASK_VM_INFO)`).
// libc marks its mach functions deprecated in favour of the mach2 crate.
#[allow(deprecated)]
pub fn phys_footprint() -> u64 {
    // task_vm_info_data_t up to phys_footprint, with room for later fields.
    #[repr(C)]
    struct TaskVmInfo {
        virtual_size: u64,
        region_count: i32,
        page_size: i32,
        /// resident_size … compressed_lifetime
        sizes: [u64; 16],
        phys_footprint: u64,
        rest: [u64; 40],
    }
    const TASK_VM_INFO: u32 = 22;
    let mut info: TaskVmInfo = unsafe { std::mem::zeroed() };
    let mut count = (std::mem::size_of::<TaskVmInfo>() / std::mem::size_of::<libc::natural_t>()) as libc::mach_msg_type_number_t;
    let r = unsafe { libc::task_info(libc::mach_task_self(), TASK_VM_INFO, (&raw mut info).cast(), &mut count) };
    if r != 0 { 0 } else { info.phys_footprint }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_in_pieces(bytes: &[u8], piece: usize) -> Vec<(usize, String)> {
        let mut s = ControlScanner::default();
        let mut out = Vec::new();
        let mut all = Vec::new();
        for (i, chunk) in bytes.chunks(piece).enumerate() {
            s.feed(chunk, &mut out);
            all.extend(out.drain(..).map(|(end, c)| (i * piece + end, c)));
        }
        all
    }

    #[test]
    fn finds_commands_split_anywhere() {
        let bytes = b"one\x1b]7770;dump\x07two\x1b]0;title\x07\x1b]7770;resize 3 4\x1b\\three\x1b[31m\x1b]7770;quit\x07";
        let want = vec![
            (15, "dump".to_string()),
            (47, "resize 3 4".to_string()),
            (bytes.len(), "quit".to_string()),
        ];
        for piece in 1..=bytes.len() {
            assert_eq!(scan_in_pieces(bytes, piece), want, "in pieces of {piece}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for bytes in [
            &b"\x1b]777;dump\x07"[..],
            b"\x1b]77701;dump\x07",
            b"\x1b]7770dump\x07",
            b"\x1b[7770;dump\x07",
            b"\x1b]7770;dump\x1bx",
            b"\x1b]7770;dump",
        ] {
            assert_eq!(scan_in_pieces(bytes, bytes.len()), [], "{bytes:?}");
        }
        let long = [&b"\x1b]7770;"[..], &[b'a'; 300], b"\x07"].concat();
        assert_eq!(scan_in_pieces(&long, 7), []);
        // An ESC that abandons one sequence can start the next.
        assert_eq!(scan_in_pieces(b"\x1b]77\x1b]7770;quit\x07", 1), [(16, "quit".to_string())]);
        assert_eq!(scan_in_pieces(b"\x1b]7770;du\x1b\x1b]7770;quit\x07", 1), [(22, "quit".to_string())]);
    }

    #[test]
    fn finds_the_preferences_osc() {
        let mut s = ControlScanner::new(PREFS_PREFIX);
        let mut out = Vec::new();
        s.feed(
            b"\x1b]7770;dump\x07\x1b[?1049h\x1b]7771;record-chord\x1b\\x\x1b]7771;other\x07",
            &mut out,
        );
        let found: Vec<&str> = out.iter().map(|(_, c)| c.as_str()).collect();
        assert_eq!(found, ["record-chord", "other"]);
    }

    #[test]
    fn parses_commands() {
        for (text, want) in [
            ("dump", Some(Control::Dump)),
            ("quit", Some(Control::Quit)),
            (" quit ", Some(Control::Quit)),
            ("resize 80 24", Some(Control::Resize { cols: 80, lines: 24 })),
            ("resize 80", None),
            ("resize 0 24", None),
            ("resize 80 24 1", None),
            ("resize x y", None),
            ("font +1", Some(Control::Font(1))),
            ("font -2", Some(Control::Font(-2))),
            ("font", None),
            ("tab", Some(Control::Tab)),
            ("tab 2", None),
            ("front", Some(Control::Front)),
            ("close", Some(Control::Close)),
            ("close-pane", Some(Control::ClosePane)),
            ("drag 0 300", Some(Control::Drag { index: 0, to: 300 })),
            ("drag 1", None),
            ("drag x 3", None),
            ("resize-pane left", Some(Control::ResizePane(Direction::Left))),
            ("resize-pane down", Some(Control::ResizePane(Direction::Down))),
            ("resize-pane", None),
            ("resize-pane 3", None),
            ("equalize", Some(Control::Equalize)),
            ("zoom", Some(Control::Zoom)),
            ("zoom 1", None),
            ("save", Some(Control::Save)),
            ("split right", Some(Control::Split(Dir::Right))),
            ("split down", Some(Control::Split(Dir::Down))),
            ("split", None),
            ("split up", None),
            ("focus 2", Some(Control::Focus(FocusTarget::Index(2)))),
            ("focus next", Some(Control::Focus(FocusTarget::Next))),
            ("focus previous", Some(Control::Focus(FocusTarget::Previous))),
            ("focus left", Some(Control::Focus(FocusTarget::Towards(Direction::Left)))),
            ("focus down", Some(Control::Focus(FocusTarget::Towards(Direction::Down)))),
            ("focus", None),
            ("focus x", None),
            ("drop /a b\t/c", Some(Control::Drop(vec!["/a b".into(), "/c".into()]))),
            ("drop /x", Some(Control::Drop(vec!["/x".into()]))),
            ("drop ", None),
            ("drop", None),
            ("settings", Some(Control::Settings)),
            (
                "key 45 cmd+shift",
                Some(Control::Key {
                    keycode: 45,
                    flags: 1 << 20 | 1 << 17,
                }),
            ),
            ("key 53", Some(Control::Key { keycode: 53, flags: 0 })),
            ("key 45 hyper", None),
            (
                "type-settings /fg\\r\\e\\\\x",
                Some(Control::TypeSettings(b"/fg\r\x1b\\x".to_vec())),
            ),
            ("dump now", None),
            ("rm -rf", None),
            ("", None),
        ] {
            assert_eq!(Control::parse(text), want, "{text:?}");
        }
    }

    #[test]
    fn footprint_is_plausible() {
        let f = phys_footprint();
        assert!(f > 100_000 && f < 1 << 40, "{f}");
    }
}
