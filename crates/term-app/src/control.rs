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
//! - `quit`: quit the app.
//!
//! While this is on, the window draws even when covered by other windows,
//! so a dump never waits for a frame that is never drawn.
//!
//! Commands run in order; one after a `dump` waits until the dump is
//! written. Without the env var the sequence is ignored like any unknown
//! OSC.

use serde::Serialize;

/// The env var that turns this on, naming the dump file.
pub const ENV: &str = "DECLINE_TEST_DUMP";
/// The OSC number.
pub const OSC: &str = "7770";

const PREFIX: &[u8] = b"\x1b]7770;";
/// Longer commands are dropped.
const MAX_LEN: usize = 256;

/// Finds control sequences in program output, across reads.
#[derive(Debug, Default)]
pub struct ControlScanner {
    state: State,
    payload: Vec<u8>,
}

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
                State::Prefix(k) if b == PREFIX[k] => {
                    if k + 1 == PREFIX.len() {
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
    Quit,
}

impl Control {
    pub fn parse(s: &str) -> Option<Control> {
        let mut words = s.split_whitespace();
        let c = match words.next()? {
            "dump" => Control::Dump,
            "quit" => Control::Quit,
            "tab" => Control::Tab,
            "resize" => Control::Resize {
                cols: words.next()?.parse().ok().filter(|&c| c > 0)?,
                lines: words.next()?.parse().ok().filter(|&l| l > 0)?,
            },
            "font" => Control::Font(words.next()?.parse().ok()?),
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
    /// The Dock icon's badge (progress).
    pub badge: Option<String>,
    /// Whether the app is active, and whether it holds Secure Keyboard
    /// Entry (only while active, when turned on).
    pub active: bool,
    pub secure_input: bool,
    /// `Terminal::dump()`: the visible rows and a status line.
    pub screen: String,
    pub cols: usize,
    pub lines: usize,
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
    /// `phys_footprint` from `task_info`, bytes.
    pub phys_footprint: u64,
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
