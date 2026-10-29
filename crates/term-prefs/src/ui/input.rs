//! Turns the bytes a terminal sends into keys, mouse events, pastes and
//! the app's chord replies: legacy keys, CSI and SS3 sequences, kitty
//! `CSI … u` (flag 1, disambiguate), SGR mouse, bracketed paste and OSC
//! 7771.
//!
//! No timers: a lone Esc at the end of what was read is the Esc key (with
//! kitty flag 1 the terminal sends Esc as `CSI 27 u` anyway).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    /// Super (Cmd), which Terminal Decline never sends to programs.
    pub cmd: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: false,
        cmd: false,
    };

    /// From the `1 + bits` parameter of CSI and kitty sequences.
    fn from_param(p: u32) -> Mods {
        let bits = p.saturating_sub(1);
        Mods {
            shift: bits & 1 != 0,
            alt: bits & 2 != 0,
            ctrl: bits & 4 != 0,
            cmd: bits & 8 != 0,
        }
    }

    pub fn is_empty(self) -> bool {
        self == Mods::NONE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Press(u8),
    Release,
    Drag,
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Key(Key, Mods),
    /// Zero-based cell.
    Mouse {
        kind: MouseKind,
        x: usize,
        y: usize,
    },
    Paste(String),
    /// `OSC 7771 ; chord ; <chord> ST`: the app recorded a chord.
    Chord(String),
    /// `OSC 7771 ; cancelled ST`.
    ChordCancelled,
}

/// The decoder, holding an incomplete sequence between reads.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    paste: Option<Vec<u8>>,
}

enum Step {
    /// An event and the bytes it used.
    Event(Option<Event>, usize),
    /// More bytes are needed.
    Incomplete,
}

impl Decoder {
    /// Decodes `bytes` (one read's worth) into events.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.buf.len() {
            if let Some(paste) = &mut self.paste {
                // Inside a bracketed paste: everything up to `CSI 201 ~`.
                let rest = &self.buf[i..];
                match find(rest, b"\x1b[201~") {
                    Some(end) => {
                        paste.extend_from_slice(&rest[..end]);
                        let text = String::from_utf8_lossy(paste).into_owned();
                        self.paste = None;
                        out.push(Event::Paste(text));
                        i += end + 6;
                    }
                    None => {
                        // Keep a possible start of the terminator.
                        let keep = (0..rest.len().min(5))
                            .rev()
                            .find(|&k| k > 0 && b"\x1b[201~".starts_with(&rest[rest.len() - k..]))
                            .unwrap_or(0);
                        paste.extend_from_slice(&rest[..rest.len() - keep]);
                        i = self.buf.len() - keep;
                        break;
                    }
                }
                continue;
            }
            match step(&self.buf[i..]) {
                Step::Event(e, n) => {
                    if e == Some(Event::Paste(String::new())) {
                        self.paste = Some(Vec::new());
                    } else if let Some(e) = e {
                        out.push(e);
                    }
                    i += n;
                }
                Step::Incomplete => {
                    if self.buf[i..] == [0x1b] {
                        out.push(Event::Key(Key::Esc, Mods::NONE));
                        i += 1;
                    }
                    break;
                }
            }
        }
        self.buf.drain(..i);
        out
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn key(k: Key) -> Option<Event> {
    Some(Event::Key(k, Mods::NONE))
}

fn step(b: &[u8]) -> Step {
    match b[0] {
        0x1b => escape(b),
        b'\r' | b'\n' => Step::Event(key(Key::Enter), 1),
        b'\t' => Step::Event(key(Key::Tab), 1),
        0x7f | 0x08 => Step::Event(key(Key::Backspace), 1),
        0 => Step::Event(Some(Event::Key(Key::Char(' '), Mods { ctrl: true, ..Mods::NONE })), 1),
        c @ 1..=26 => Step::Event(
            Some(Event::Key(Key::Char((b'a' + c - 1) as char), Mods { ctrl: true, ..Mods::NONE })),
            1,
        ),
        c if c < 0x20 => Step::Event(None, 1),
        _ => {
            let len = match b[0] {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                0x80..=0xbf | 0xf8..=0xff => return Step::Event(None, 1),
                _ => 1,
            };
            if b.len() < len {
                return Step::Incomplete;
            }
            match std::str::from_utf8(&b[..len]).ok().and_then(|s| s.chars().next()) {
                Some(c) => Step::Event(key(Key::Char(c)), len),
                None => Step::Event(None, 1),
            }
        }
    }
}

fn escape(b: &[u8]) -> Step {
    let Some(&next) = b.get(1) else {
        return Step::Incomplete;
    };
    match next {
        b'[' => csi(b),
        b'O' => match b.get(2) {
            None => Step::Incomplete,
            Some(&c) => Step::Event(
                match c {
                    b'A' => key(Key::Up),
                    b'B' => key(Key::Down),
                    b'C' => key(Key::Right),
                    b'D' => key(Key::Left),
                    b'H' => key(Key::Home),
                    b'F' => key(Key::End),
                    b'P'..=b'S' => key(Key::F(c - b'P' + 1)),
                    b'M' => key(Key::Enter),
                    _ => None,
                },
                3,
            ),
        },
        b']' => osc(b),
        0x1b => Step::Event(key(Key::Esc), 1),
        _ => {
            // Legacy Alt+key: Esc then the key.
            match step(&b[1..]) {
                Step::Event(Some(Event::Key(k, mut m)), n) => {
                    m.alt = true;
                    Step::Event(Some(Event::Key(k, m)), n + 1)
                }
                Step::Event(e, n) => Step::Event(e, n + 1),
                Step::Incomplete => Step::Incomplete,
            }
        }
    }
}

fn osc(b: &[u8]) -> Step {
    // Up to BEL or ESC \.
    let body = &b[2..];
    let (end, term) = match (body.iter().position(|&c| c == 0x07), find(body, b"\x1b\\")) {
        (Some(a), Some(e)) if a < e => (a, 1),
        (_, Some(e)) => (e, 2),
        (Some(a), None) => (a, 1),
        (None, None) => return Step::Incomplete,
    };
    let text = String::from_utf8_lossy(&body[..end]);
    let event = match text.strip_prefix("7771;") {
        Some("cancelled") => Some(Event::ChordCancelled),
        Some(rest) => rest.strip_prefix("chord;").map(|c| Event::Chord(c.to_string())),
        None => None,
    };
    Step::Event(event, 2 + end + term)
}

fn csi(b: &[u8]) -> Step {
    // Parameters and intermediates, then a final byte in 0x40..=0x7e.
    let Some(fin) = b[2..].iter().position(|&c| (0x40..=0x7e).contains(&c)) else {
        if b.len() > 64 {
            return Step::Event(None, b.len());
        }
        return Step::Incomplete;
    };
    let n = 2 + fin + 1;
    let params = &b[2..2 + fin];
    let last = b[2 + fin];
    if params.first() == Some(&b'<') {
        return Step::Event(mouse(&params[1..], last), n);
    }
    let text = String::from_utf8_lossy(params);
    // `code:alternates;mods:event;text`: only the first of each matters.
    let fields: Vec<Vec<u32>> = text
        .split(';')
        .map(|f| f.split(':').map(|p| p.parse().unwrap_or(0)).collect())
        .collect();
    let num = |i: usize| fields.get(i).and_then(|f| f.first()).copied().unwrap_or(0);
    let mods = Mods::from_param(num(1).max(1));
    let with = |k: Key| Some(Event::Key(k, mods));
    let event = match last {
        b'A' => with(Key::Up),
        b'B' => with(Key::Down),
        b'C' => with(Key::Right),
        b'D' => with(Key::Left),
        b'H' => with(Key::Home),
        b'F' => with(Key::End),
        b'Z' => key(Key::BackTab),
        b'P'..=b'S' => with(Key::F(last - b'P' + 1)),
        b'~' => match num(0) {
            200 => Some(Event::Paste(String::new())),
            1 | 7 => with(Key::Home),
            2 => with(Key::Insert),
            3 => with(Key::Delete),
            4 | 8 => with(Key::End),
            5 => with(Key::PageUp),
            6 => with(Key::PageDown),
            n @ 11..=15 => with(Key::F((n - 10) as u8)),
            n @ 17..=21 => with(Key::F((n - 11) as u8)),
            n @ 23..=24 => with(Key::F((n - 12) as u8)),
            _ => None,
        },
        b'u' => {
            let k = match num(0) {
                27 => Some(Key::Esc),
                13 => Some(Key::Enter),
                9 => Some(Key::Tab),
                127 | 8 => Some(Key::Backspace),
                c => char::from_u32(c).filter(|c| !c.is_control()).map(Key::Char),
            };
            k.and_then(with)
        }
        _ => None,
    };
    Step::Event(event, n)
}

fn mouse(params: &[u8], last: u8) -> Option<Event> {
    let text = std::str::from_utf8(params).ok()?;
    let mut it = text.split(';').map(|p| p.parse::<usize>().ok());
    let (b, x, y) = (it.next()??, it.next()??, it.next()??);
    let kind = if b & 64 != 0 {
        if b & 1 == 0 { MouseKind::ScrollUp } else { MouseKind::ScrollDown }
    } else if last == b'm' {
        MouseKind::Release
    } else if b & 32 != 0 {
        MouseKind::Drag
    } else {
        MouseKind::Press((b & 3) as u8)
    };
    Some(Event::Mouse {
        kind,
        x: x.saturating_sub(1),
        y: y.saturating_sub(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<Event> {
        Decoder::default().feed(bytes)
    }

    fn k(key: Key) -> Event {
        Event::Key(key, Mods::NONE)
    }

    #[test]
    fn legacy_and_kitty_keys() {
        assert_eq!(keys(b"a\r\x7f"), [k(Key::Char('a')), k(Key::Enter), k(Key::Backspace)]);
        assert_eq!(keys(b"\x1b"), [k(Key::Esc)]);
        assert_eq!(keys(b"\x1b[27u"), [k(Key::Esc)]);
        assert_eq!(
            keys(b"\x1b[A\x1bOB\x1b[1;5C"),
            [k(Key::Up), k(Key::Down), Event::Key(Key::Right, Mods { ctrl: true, ..Mods::NONE })]
        );
        assert_eq!(keys(b"\x1b[97;3u"), [Event::Key(Key::Char('a'), Mods { alt: true, ..Mods::NONE })]);
        assert_eq!(keys(b"\x1b[13;5u"), [Event::Key(Key::Enter, Mods { ctrl: true, ..Mods::NONE })]);
        assert_eq!(keys(b"\x1bx"), [Event::Key(Key::Char('x'), Mods { alt: true, ..Mods::NONE })]);
        assert_eq!(keys(b"\x03"), [Event::Key(Key::Char('c'), Mods { ctrl: true, ..Mods::NONE })]);
        assert_eq!(keys("é".as_bytes()), [k(Key::Char('é'))]);
        assert_eq!(keys(b"\x1b[3~\x1b[5~\x1b[Z"), [k(Key::Delete), k(Key::PageUp), k(Key::BackTab)]);
    }

    #[test]
    fn sequences_split_across_reads() {
        let mut d = Decoder::default();
        assert_eq!(d.feed(b"\x1b[1;"), []);
        assert_eq!(d.feed(b"2A"), [Event::Key(Key::Up, Mods { shift: true, ..Mods::NONE })]);
        assert_eq!(d.feed(&"日".as_bytes()[..1]), []);
        assert_eq!(d.feed(&"日".as_bytes()[1..]), [k(Key::Char('日'))]);
    }

    #[test]
    fn mouse_paste_and_chord_replies() {
        assert_eq!(
            keys(b"\x1b[<0;5;3M\x1b[<0;5;3m\x1b[<65;1;1M"),
            [
                Event::Mouse {
                    kind: MouseKind::Press(0),
                    x: 4,
                    y: 2
                },
                Event::Mouse {
                    kind: MouseKind::Release,
                    x: 4,
                    y: 2
                },
                Event::Mouse {
                    kind: MouseKind::ScrollDown,
                    x: 0,
                    y: 0
                },
            ]
        );
        let mut d = Decoder::default();
        assert_eq!(d.feed(b"\x1b[200~hello\x1b[20"), []);
        assert_eq!(d.feed(b"1~x"), [Event::Paste("hello".into()), k(Key::Char('x'))]);
        assert_eq!(keys(b"\x1b]7771;chord;ctrl+cmd+k\x1b\\"), [Event::Chord("ctrl+cmd+k".into())]);
        assert_eq!(keys(b"\x1b]7771;cancelled\x07"), [Event::ChordCancelled]);
        assert_eq!(keys(b"\x1b]11;rgb:0/0/0\x07a"), [k(Key::Char('a'))]);
    }
}
