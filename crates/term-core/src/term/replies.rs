//! Colours a program can set and ask about, and the replies programs use to
//! find out what the terminal supports.
//!
//! - OSC 4 / 104: the 256-colour palette, set, asked for and reset; OSC 5 /
//!   105 the special colours (bold, underline, blink, reverse, italic),
//!   also reachable as OSC 4 colours 256–260, after the palette.
//! - OSC 10 / 11 / 12 (foreground, background, cursor): set and asked for;
//!   OSC 110 / 111 / 112 reset them.
//! - DECRQSS (`DCS $ q … ST`): the current SGR, cursor style and margins.
//! - XTGETTCAP (`DCS + q … ST`): terminfo capabilities by name, which is
//!   how Neovim learns about undercurl and true colour when `TERM` doesn't
//!   say.
//!
//! The colours the theme gives are the defaults; what a program sets goes
//! on top, and a reset goes back to the theme's. Colours are 0xRRGGBB.

use super::{CursorShape, Terminal};
use crate::cell::{Color, ColorKind};
use crate::style::{Attrs, Style, Underline};

/// Palette entries, then the special colours, then foreground, background
/// and cursor.
const PALETTE: usize = 256;
const SPECIAL: usize = PALETTE;
const SPECIALS: usize = 5;
const DYNAMIC: usize = SPECIAL + SPECIALS;
const SLOTS: usize = DYNAMIC + 3;

/// The longest DCS string kept for a reply; longer ones are dropped.
const DCS_MAX: usize = 1024;

/// The default palette: the ANSI colours of Terminal Decline's default
/// theme, then xterm's 6×6×6 cube and grey ramp (as the renderer's).
pub(super) fn default_palette() -> [u32; PALETTE] {
    const ANSI: [u32; 16] = [
        0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, //
        0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
    ];
    let mut p = [0; PALETTE];
    p[..16].copy_from_slice(&ANSI);
    let level = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
    for i in 0..216u32 {
        p[16 + i as usize] = level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6);
    }
    for i in 0..24u32 {
        let v = 8 + i * 10;
        p[232 + i as usize] = v << 16 | v << 8 | v;
    }
    p
}

/// The theme's colours and the ones programs set on top.
#[derive(Clone)]
pub(super) struct ColorTable {
    base: [u32; SLOTS],
    current: [u32; SLOTS],
    /// Which slots a program set (one bit each).
    set: [u64; SLOTS.div_ceil(64)],
}

impl ColorTable {
    pub(super) fn new(defaults: [u32; 3]) -> ColorTable {
        let mut base = [0; SLOTS];
        base[..PALETTE].copy_from_slice(&default_palette());
        base[SPECIAL..DYNAMIC].fill(defaults[0]);
        base[DYNAMIC..].copy_from_slice(&defaults);
        ColorTable {
            base,
            current: base,
            set: [0; SLOTS.div_ceil(64)],
        }
    }

    /// The same theme colours, with nothing a program set.
    pub(super) fn reset_all(&self) -> ColorTable {
        ColorTable {
            base: self.base,
            current: self.base,
            set: [0; SLOTS.div_ceil(64)],
        }
    }

    fn is_set(&self, slot: usize) -> bool {
        self.set[slot / 64] & 1 << (slot % 64) != 0
    }

    fn set_base(&mut self, slot: usize, rgb: u32) {
        self.base[slot] = rgb;
        if !self.is_set(slot) {
            self.current[slot] = rgb;
        }
    }

    fn set(&mut self, slot: usize, rgb: u32) {
        self.current[slot] = rgb;
        self.set[slot / 64] |= 1 << (slot % 64);
    }

    fn reset(&mut self, slot: usize) {
        self.current[slot] = self.base[slot];
        self.set[slot / 64] &= !(1 << (slot % 64));
    }
}

/// A device control string being collected for a reply.
pub(super) struct Dcs {
    kind: DcsKind,
    data: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DcsKind {
    /// `DCS $ q`: DECRQSS.
    Decrqss,
    /// `DCS + q`: XTGETTCAP.
    Xtgettcap,
}

/// Which of foreground, background and cursor (OSC 10, 11, 12).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DynamicColor {
    Foreground,
    Background,
    Cursor,
}

impl Terminal {
    /// The palette in effect, 0xRRGGBB: the theme's (see
    /// [`set_default_palette`](Self::set_default_palette)) with what
    /// programs set (OSC 4) on top.
    pub fn palette(&self) -> &[u32; PALETTE] {
        self.colors.current[..PALETTE].try_into().unwrap()
    }

    /// The theme's palette, the defaults OSC 104 goes back to.
    pub fn default_palette(&self) -> &[u32; PALETTE] {
        self.colors.base[..PALETTE].try_into().unwrap()
    }

    /// The theme's palette, the defaults OSC 104 goes back to. Entries a
    /// program set keep the program's colour.
    pub fn set_default_palette(&mut self, palette: &[u32; PALETTE]) {
        for (i, &rgb) in palette.iter().enumerate() {
            self.colors.set_base(i, rgb);
        }
        self.colors_touched();
    }

    /// The foreground, background or cursor colour in effect, 0xRRGGBB:
    /// the theme's (see [`set_default_colors`](Self::set_default_colors))
    /// unless a program set it (OSC 10–12).
    pub fn dynamic_color(&self, which: DynamicColor) -> u32 {
        self.colors.current[DYNAMIC + which as usize]
    }

    /// Foreground, background and cursor colours as 0xRRGGBB from the
    /// theme: the defaults OSC 110–112 go back to.
    pub fn default_colors(&self) -> [u32; 3] {
        self.colors.base[DYNAMIC..].try_into().unwrap()
    }

    pub fn set_default_colors(&mut self, colors: [u32; 3]) {
        for (i, rgb) in colors.into_iter().enumerate() {
            self.colors.set_base(DYNAMIC + i, rgb);
        }
        // The special colours default to the foreground.
        for slot in SPECIAL..DYNAMIC {
            self.colors.set_base(slot, colors[0]);
        }
        self.colors_touched();
    }

    /// Whether a program changed a colour (OSC 4, 10–12 or a reset) since
    /// the last call, or the defaults changed: the screen needs drawing
    /// again with [`palette`](Self::palette) and
    /// [`dynamic_color`](Self::dynamic_color).
    pub fn take_colors_changed(&mut self) -> bool {
        std::mem::take(&mut self.colors_changed)
    }

    fn colors_touched(&mut self) {
        self.colors_changed = true;
        self.generation = self.generation.wrapping_add(1);
    }

    /// OSC 4, 5, 10–12, 104, 105 and 110–112. Returns whether `params`
    /// was one of them.
    pub(super) fn osc_color(&mut self, params: &[&[u8]]) -> bool {
        let Some(code) = params.first().and_then(|c| std::str::from_utf8(c).ok()?.parse::<u16>().ok()) else {
            return false;
        };
        let rest = &params[1..];
        match code {
            4 | 5 => {
                let offset = if code == 5 { SPECIAL } else { 0 };
                for [index, spec] in rest.as_chunks::<2>().0 {
                    let Some(n) = number(index) else { break };
                    let slot = n + offset;
                    if slot >= DYNAMIC {
                        break;
                    }
                    self.color_spec(slot, spec, |s| format!("{code};{n};{s}"));
                }
            }
            10..=19 => {
                for (i, spec) in rest.iter().enumerate() {
                    let n = code as usize + i;
                    if n > 12 {
                        break;
                    }
                    self.color_spec(DYNAMIC + n - 10, spec, |s| format!("{n};{s}"));
                }
            }
            104 | 105 => {
                let offset = if code == 105 { SPECIAL } else { 0 };
                let all = if code == 105 { SPECIAL..DYNAMIC } else { 0..PALETTE };
                if rest.iter().all(|p| p.is_empty()) {
                    all.for_each(|slot| self.colors.reset(slot));
                } else {
                    for slot in rest.iter().filter_map(|p| number(p)).map(|n| n + offset) {
                        if slot < DYNAMIC {
                            self.colors.reset(slot);
                        }
                    }
                }
                self.colors_touched();
            }
            110..=112 => {
                self.colors.reset(DYNAMIC + (code - 110) as usize);
                self.colors_touched();
            }
            _ => return false,
        }
        true
    }

    /// Set slot `slot` to `spec`, or answer for it if `spec` is `?`.
    fn color_spec(&mut self, slot: usize, spec: &[u8], head: impl Fn(String) -> String) {
        if spec == b"?" {
            let rgb = self.colors.current[slot];
            let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
            let body = head(format!("rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}"));
            // Always ended with ST, whatever the query ended with: what
            // Terminal Decline has always sent, and what programs expect.
            self.replies.extend_from_slice(format!("\x1b]{body}\x1b\\").as_bytes());
        } else if let Some(rgb) = parse_color(spec) {
            self.colors.set(slot, rgb);
            self.colors_touched();
        }
    }

    /// A DCS string starts: collect it if it is a query we answer.
    pub(super) fn dcs_hook(&mut self, intermediates: &[u8], ignore: bool, action: char) {
        let kind = match (intermediates, action) {
            _ if ignore => None,
            ([b'$'], 'q') => Some(DcsKind::Decrqss),
            ([b'+'], 'q') => Some(DcsKind::Xtgettcap),
            _ => None,
        };
        self.dcs = kind.map(|kind| Dcs { kind, data: Vec::new() });
    }

    pub(super) fn dcs_put(&mut self, data: &[u8]) {
        if let Some(dcs) = &mut self.dcs {
            if dcs.data.len() + data.len() > DCS_MAX {
                self.dcs = None;
            } else {
                dcs.data.extend_from_slice(data);
            }
        }
    }

    pub(super) fn dcs_finish(&mut self, complete: bool) {
        let Some(dcs) = self.dcs.take() else { return };
        if !complete {
            return;
        }
        match dcs.kind {
            DcsKind::Decrqss => self.decrqss(&dcs.data),
            DcsKind::Xtgettcap => self.xtgettcap(&dcs.data),
        }
    }

    /// DECRQSS: `DCS 1 $ r <setting> ST` for what we know, `DCS 0 $ r ST`
    /// for the rest.
    fn decrqss(&mut self, what: &[u8]) {
        let setting = match what {
            b"m" => Some(format!("{}m", sgr_report(&self.cursor.pen))),
            b" q" => {
                let n = match self.cursor_shape {
                    CursorShape::Block => 1,
                    CursorShape::Underline => 3,
                    CursorShape::Bar => 5,
                } + !self.cursor_blink as u8;
                Some(format!("{n} q"))
            }
            b"r" => Some(format!("{};{}r", self.scroll_top + 1, self.scroll_bottom + 1)),
            _ => None,
        };
        let reply = match setting {
            Some(s) => format!("\x1bP1$r{s}\x1b\\"),
            None => "\x1bP0$r\x1b\\".to_string(),
        };
        self.replies.extend_from_slice(reply.as_bytes());
    }

    /// XTGETTCAP: one reply per name asked for, `DCS 1 + r name=value ST`
    /// (`name` alone for a flag), or `DCS 0 + r name ST` for one we don't
    /// have. Names and values are in hex.
    fn xtgettcap(&mut self, names: &[u8]) {
        for hex_name in names.split(|&b| b == b';') {
            let cap = hex_decode(hex_name).and_then(|name| termcap(&name));
            let name = String::from_utf8_lossy(hex_name);
            let reply = match cap {
                Some(Cap::Flag) => format!("\x1bP1+r{name}\x1b\\"),
                Some(Cap::Value(v)) => format!("\x1bP1+r{name}={}\x1b\\", hex_encode(v.as_bytes())),
                None => format!("\x1bP0+r{name}\x1b\\"),
            };
            self.replies.extend_from_slice(reply.as_bytes());
        }
    }
}

enum Cap {
    Flag,
    Value(String),
}

/// The terminfo capabilities XTGETTCAP answers for: the ones programs ask
/// about because `xterm-256color` lacks them (undercurl, underline
/// colour, true colour), and a few they check.
fn termcap(name: &[u8]) -> Option<Cap> {
    let v = |s: &str| Some(Cap::Value(s.to_string()));
    match name {
        b"TN" | b"name" => v("xterm-256color"),
        b"Co" | b"colors" => v("256"),
        b"RGB" | b"Tc" | b"Su" => Some(Cap::Flag),
        b"Smulx" => v("\x1b[4:%p1%dm"),
        b"Setulc" => v("\x1b[58:2::%p1%{65536}%/%d:%p1%{256}%/%{255}%&%d:%p1%{255}%&%dm"),
        b"setrgbf" => v("\x1b[38:2::%p1%d:%p2%d:%p3%dm"),
        b"setrgbb" => v("\x1b[48:2::%p1%d:%p2%d:%p3%dm"),
        b"Ss" => v("\x1b[%p1%d q"),
        b"Se" => v("\x1b[2 q"),
        b"Ms" => v("\x1b]52;%p1%s;%p2%s\x07"),
        b"smxx" => v("\x1b[9m"),
        b"rmxx" => v("\x1b[29m"),
        b"Smol" => v("\x1b[53m"),
        b"Rmol" => v("\x1b[55m"),
        b"Sync" => v("\x1b[?2026%?%p1%{1}%-%tl%eh%;"),
        b"BE" => v("\x1b[?2004h"),
        b"BD" => v("\x1b[?2004l"),
        b"fe" => v("\x1b[?1004h"),
        b"fd" => v("\x1b[?1004l"),
        _ => None,
    }
}

/// The SGR that gives `pen`, as DECRQSS reports it: `0` then each
/// attribute (`0;1;38;2;255;0;0`).
pub(super) fn sgr_report(pen: &Style) -> String {
    let mut s = String::from("0");
    let attrs = [
        (Attrs::BOLD, "1"),
        (Attrs::DIM, "2"),
        (Attrs::ITALIC, "3"),
        (Attrs::BLINK, "5"),
        (Attrs::INVERSE, "7"),
        (Attrs::HIDDEN, "8"),
        (Attrs::STRIKE, "9"),
        (Attrs::OVERLINE, "53"),
    ];
    for (a, code) in attrs {
        if pen.attrs.contains(a) {
            s.push(';');
            s.push_str(code);
        }
    }
    match pen.underline {
        Underline::None => {}
        Underline::Single => s.push_str(";4"),
        u => s.push_str(&format!(";4:{}", u as u8)),
    }
    let mut color = |c: Color, base: u16, bright: u16, extended: u16| match c.kind() {
        ColorKind::Default => {}
        ColorKind::Indexed(i) if i < 8 && base != 0 => s.push_str(&format!(";{}", base + i as u16)),
        ColorKind::Indexed(i) if i < 16 && base != 0 => s.push_str(&format!(";{}", bright + i as u16 - 8)),
        ColorKind::Indexed(i) => s.push_str(&format!(";{extended};5;{i}")),
        ColorKind::Rgb(r, g, b) => s.push_str(&format!(";{extended};2;{r};{g};{b}")),
    };
    color(pen.fg, 30, 90, 38);
    color(pen.bg, 40, 100, 48);
    color(pen.underline_color, 0, 0, 58);
    s
}

/// A colour in X11's forms that programs use: `#rgb`, `#rrggbb`,
/// `#rrrgggbbb`, `#rrrrggggbbbb` (the high bits of each), or
/// `rgb:r/g/b` with 1–4 hex digits each (scaled). Not the CIE, TekHVC and
/// `rgbi:` forms, or colour names.
pub(super) fn parse_color(spec: &[u8]) -> Option<u32> {
    let hex = |s: &[u8]| -> Option<u32> {
        if s.is_empty() || s.len() > 4 || !s.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        u32::from_str_radix(std::str::from_utf8(s).ok()?, 16).ok()
    };
    let (r, g, b) = if let Some(digits) = spec.strip_prefix(b"#") {
        let n = digits.len() / 3;
        if !(1..=4).contains(&n) || digits.len() != n * 3 {
            return None;
        }
        // Each is the high bits of a 16-bit value.
        let at = |i: usize| hex(&digits[i * n..(i + 1) * n]).map(|v| (v << (16 - 4 * n)) >> 8);
        (at(0)?, at(1)?, at(2)?)
    } else {
        let mut parts = spec.strip_prefix(b"rgb:")?.split(|&b| b == b'/');
        let mut scaled = || -> Option<u32> {
            let p = parts.next()?;
            let v = hex(p)?;
            let max = (1u32 << (4 * p.len())) - 1;
            Some((v * 0xFFFF / max) >> 8)
        };
        let rgb = (scaled()?, scaled()?, scaled()?);
        if parts.next().is_some() {
            return None;
        }
        rgb
    };
    Some(r << 16 | g << 8 | b)
}

fn number(s: &[u8]) -> Option<usize> {
    if s.is_empty() || s.len() > 4 {
        return None;
    }
    std::str::from_utf8(s).ok()?.parse().ok()
}

fn hex_decode(s: &[u8]) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_forms() {
        assert_eq!(parse_color(b"#fff"), Some(0xf0f0f0));
        assert_eq!(parse_color(b"#888"), Some(0x808080));
        assert_eq!(parse_color(b"#f0f0f0"), Some(0xf0f0f0));
        assert_eq!(parse_color(b"#f00f00f00"), Some(0xf0f0f0));
        assert_eq!(parse_color(b"#800080008000"), Some(0x808080));
        assert_eq!(parse_color(b"#aaaabbbbcccc"), Some(0xaabbcc));
        assert_eq!(parse_color(b"rgb:f0f0/0000/8080"), Some(0xf00080));
        assert_eq!(parse_color(b"rgb:f/0/8"), Some(0xff0088));
        assert_eq!(parse_color(b"rgb:ff/00/80"), Some(0xff0080));
        assert_eq!(parse_color(b"rgb:fff/000/800"), Some(0xff0080));
        for bad in [
            &b"#ff"[..],
            b"#fffff",
            b"rgb:ff/00",
            b"rgb:ff/00/00/00",
            b"rgb:fffff/0/0",
            b"red",
            b"#ggg",
            b"",
        ] {
            assert_eq!(parse_color(bad), None, "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn sgr_reports() {
        assert_eq!(sgr_report(&Style::DEFAULT), "0");
        let pen = Style {
            fg: Color::indexed(1),
            bg: Color::indexed(12),
            underline_color: Color::rgb(1, 2, 3),
            attrs: Attrs::BOLD | Attrs::ITALIC,
            underline: Underline::Curly,
            ..Style::DEFAULT
        };
        assert_eq!(sgr_report(&pen), "0;1;3;4:3;31;104;58;2;1;2;3");
        let pen = Style {
            fg: Color::indexed(200),
            bg: Color::rgb(10, 20, 30),
            underline_color: Color::indexed(3),
            ..Style::DEFAULT
        };
        assert_eq!(sgr_report(&pen), "0;38;5;200;48;2;10;20;30;58;5;3");
    }

    #[test]
    fn default_palette_matches_xterm() {
        let p = default_palette();
        assert_eq!(p[16], 0x000000);
        assert_eq!(p[21], 0x0000ff);
        assert_eq!(p[196], 0xff0000);
        assert_eq!(p[231], 0xffffff);
        assert_eq!(p[232], 0x080808);
        assert_eq!(p[255], 0xeeeeee);
    }
}
