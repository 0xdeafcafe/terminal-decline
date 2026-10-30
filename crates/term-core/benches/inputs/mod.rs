//! The parse inputs, the kinds of output terminals actually see: shared
//! by the `parse` bench and the `cpubench` example.

/// Every input, `size` bytes or a line more, by name.
pub fn all(size: usize) -> [(&'static str, Vec<u8>); 6] {
    [
        ("plain", plain(size)),
        ("coloured", coloured(size)),
        ("tui", tui(size)),
        ("seq", seq(size)),
        ("unicode", unicode(size)),
        ("clusters", clusters(size)),
    ]
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Plain text lines, like `cat` of source code.
fn plain(size: usize) -> Vec<u8> {
    let mut r = Rng(1);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let len = 20 + (r.next() % 80) as usize;
        for _ in 0..len {
            out.push(b' ' + (r.next() % 94) as u8);
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Coloured output, like `ls --color` or compiler errors.
fn coloured(size: usize) -> Vec<u8> {
    let mut r = Rng(2);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        for _ in 0..6 {
            let c = r.next() % 256;
            out.extend_from_slice(format!("\x1b[38;5;{c}m").as_bytes());
            for _ in 0..(3 + r.next() % 10) {
                out.push(b'a' + (r.next() % 26) as u8);
            }
            out.extend_from_slice(b"\x1b[0m ");
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Full-screen redraws with cursor moves and truecolour, like vim or htop.
fn tui(size: usize) -> Vec<u8> {
    let mut r = Rng(3);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let row = 1 + r.next() % 50;
        let col = 1 + r.next() % 150;
        let (cr, cg, cb) = (r.next() % 256, r.next() % 256, r.next() % 256);
        out.extend_from_slice(format!("\x1b[{row};{col}H\x1b[48;2;{cr};{cg};{cb}m").as_bytes());
        for _ in 0..(5 + r.next() % 40) {
            out.push(b'a' + (r.next() % 26) as u8);
        }
        out.extend_from_slice(b"\x1b[0m\x1b[K");
    }
    out
}

/// Short numbered lines, like `seq 1 1000000`: mostly line feeds and scrolling.
fn seq(size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    let mut i = 1u64;
    while out.len() < size {
        out.extend_from_slice(format!("{i}\r\n").as_bytes());
        i += 1;
    }
    out
}

/// Wide characters and emoji.
fn unicode(size: usize) -> Vec<u8> {
    let s = "漢字かなカナ한글 😀 Привет мир ┌─┐│└┘ ";
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        out.extend_from_slice(s.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Grapheme clusters: combining marks, emoji presentation, skin tones,
/// flags, keycaps, ZWJ sequences and an Indic conjunct, among plain text.
fn clusters(size: usize) -> Vec<u8> {
    let s = "cafe\u{301} ❤\u{fe0f} 👍🏽 🇬🇧 1\u{fe0f}\u{20e3} 👨\u{200d}👩\u{200d}👧 क\u{94d}ष ok done ";
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        out.extend_from_slice(s.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}
