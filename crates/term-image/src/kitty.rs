//! kitty graphics control keys: the part of `ESC _ G <keys> ; <payload>
//! ESC \` before the `;`, parsed into a [`Command`] (images.md §3.1).
//!
//! `key=value` pairs separated by `,`; each key one ASCII letter; values
//! unsigned or signed 32-bit integers, or single letters. A repeated key
//! takes the last value. Unknown keys are ignored (kitty drops the whole
//! command instead; being lenient lets a newer client's extra key work).
//! What the keys mean is term-core's business; this only reads them.

/// Keys longer than this are refused (cells §14).
pub const MAX_KEYS_LEN: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Action {
    /// `t`: transmit.
    #[default]
    Transmit,
    /// `T`: transmit and place.
    TransmitPlace,
    /// `p`: place.
    Place,
    /// `d`: delete.
    Delete,
    /// `q`: query.
    Query,
    /// `f`, `a`, `c`: animation frames, animation control, composition.
    /// Parsed, not supported in 1.0.
    Frame,
    Animate,
    Compose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Format {
    /// `f=24`: RGB.
    Rgb,
    /// `f=32`: RGBA (the default).
    #[default]
    Rgba,
    /// `f=100`: PNG.
    Png,
}

impl Format {
    /// Bytes per pixel of the raw formats.
    pub fn bytes_per_pixel(self) -> Option<u32> {
        match self {
            Format::Rgb => Some(3),
            Format::Rgba => Some(4),
            Format::Png => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Medium {
    /// `t=d`: in the payload.
    #[default]
    Direct,
    /// `t=f`: a file named by the payload.
    File,
    /// `t=t`: a temporary file, removed once read.
    TempFile,
    /// `t=s`: a POSIX shared-memory object.
    SharedMemory,
}

/// What `a=d` deletes: `d=` and the keys it uses. Lower case deletes
/// placements; upper case also frees images left with none (`free`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delete {
    pub what: DeleteWhat,
    pub free: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteWhat {
    /// `a`: all placements on the visible screen.
    All,
    /// `i`: of the image `i` (only placement `p` if not 0).
    Id,
    /// `n`: of the newest image with number `I` (and `p`).
    Number,
    /// `r`: of images with ids `x..=y`.
    Range { from: u32, to: u32 },
    /// `c`: intersecting the cursor's cell.
    Cursor,
    /// `p`: intersecting cell (`x`, `y`), 1-based.
    Cell { x: u32, y: u32 },
    /// `q`: as `p`, with z-index `z`.
    CellZ { x: u32, y: u32, z: i32 },
    /// `x`: intersecting column `x`, 1-based.
    Column(u32),
    /// `y`: intersecting row `y`, 1-based.
    Row(u32),
    /// `z`: with z-index `z`.
    Z(i32),
    /// `f`: animation frames (does nothing in 1.0).
    Frames,
}

/// One graphics command's keys, with kitty's defaults for those not given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Command {
    pub action: Action,
    /// `q`: 0, 1 (no OK), 2 (no errors either). Larger values read as 2.
    pub quiet: u8,
    pub format: Format,
    pub medium: Medium,
    /// `o=z`.
    pub compressed: bool,
    /// `m=1`: more chunks follow.
    pub more: bool,
    /// `i` (0 = none).
    pub image_id: u32,
    /// `I` (0 = none).
    pub image_number: u32,
    /// `p` (0 = none).
    pub placement_id: u32,
    /// `s`, `v`: the size of raw data.
    pub data_width: u32,
    pub data_height: u32,
    /// `S`, `O`: bytes to read from a file or shared memory, and from where.
    pub file_size: u64,
    pub file_offset: u64,
    /// `x`, `y`, `w`, `h`: the source rectangle (0 width or height = to the edge).
    pub src: [u32; 4],
    /// `X`, `Y`: the offset within the first cell, in pixels.
    pub cell_offset: (u16, u16),
    /// `c`, `r`: the size in cells (0 = from the image).
    pub cols: u16,
    pub rows: u16,
    /// `C=1`: don't move the cursor.
    pub no_cursor_move: bool,
    /// `U=1`: a virtual placement for Unicode placeholders.
    pub virtual_: bool,
    /// `z`.
    pub z: i32,
    /// For `a=d`: what to delete (default `d=a`).
    pub delete: Option<Delete>,
    /// `P`, `Q`, `H` or `V` (relative placements) or an animation action seen.
    pub unsupported: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// Over [`MAX_KEYS_LEN`].
    TooLong,
    /// Not `key=value`, a number that doesn't fit, a letter value that
    /// isn't allowed, or `i` and `I` together. Replied as `EINVAL`.
    Invalid(&'static str),
}

impl KeyError {
    /// The code kitty replies with.
    pub fn code(&self) -> &'static str {
        "EINVAL"
    }
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::TooLong => f.write_str("control keys too long"),
            KeyError::Invalid(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for KeyError {}

/// How a key's value is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ignored,
    Letter,
    Unsigned,
    Signed,
}

/// Index of a key letter: `A`–`Z` are 0–25, `a`–`z` 26–51.
fn index(key: u8) -> Option<usize> {
    match key {
        b'A'..=b'Z' => Some((key - b'A') as usize),
        b'a'..=b'z' => Some((key - b'a') as usize + 26),
        _ => None,
    }
}

static KINDS: [Kind; 52] = {
    let mut k = [Kind::Ignored; 52];
    let letters = b"adto";
    let unsigned = b"fmiIpqwhxysvSOcrXYCUPQ";
    let signed = b"zHV";
    let mut i = 0;
    while i < letters.len() {
        k[letters[i] as usize - b'a' as usize + 26] = Kind::Letter;
        i += 1;
    }
    let mut i = 0;
    while i < unsigned.len() {
        let c = unsigned[i];
        let at = if c >= b'a' { c - b'a' + 26 } else { c - b'A' } as usize;
        k[at] = Kind::Unsigned;
        i += 1;
    }
    let mut i = 0;
    while i < signed.len() {
        let c = signed[i];
        let at = if c >= b'a' { c - b'a' + 26 } else { c - b'A' } as usize;
        k[at] = Kind::Signed;
        i += 1;
    }
    k
};

/// The raw keys of one command: a value per key letter, if given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    /// Letters as their byte, numbers as their value (signed ones as `i32`
    /// bits).
    values: [u32; 52],
    given: u64,
}

impl Keys {
    /// Parse the keys (the bytes before the payload's `;`) into a command.
    pub fn parse(keys: &[u8]) -> Result<Command, KeyError> {
        Keys::read(keys)?.command()
    }

    /// Read the keys without interpreting them.
    pub fn read(keys: &[u8]) -> Result<Keys, KeyError> {
        if keys.len() > MAX_KEYS_LEN {
            return Err(KeyError::TooLong);
        }
        let mut out = Keys { values: [0; 52], given: 0 };
        for pair in keys.split(|&b| b == b',') {
            if pair.is_empty() {
                continue;
            }
            let Some(eq) = pair.iter().position(|&b| b == b'=') else {
                return Err(KeyError::Invalid("key without a value"));
            };
            let (key, value) = (&pair[..eq], &pair[eq + 1..]);
            let at = match key {
                [k] => index(*k),
                _ => None,
            };
            // Unknown and multi-letter keys are skipped.
            let Some(at) = at else { continue };
            let v = match KINDS[at] {
                Kind::Ignored => continue,
                Kind::Letter => match value {
                    [c] if c.is_ascii_alphabetic() => *c as u32,
                    _ => return Err(KeyError::Invalid("bad letter value")),
                },
                Kind::Unsigned => unsigned(value).ok_or(KeyError::Invalid("bad number"))?,
                Kind::Signed => signed(value).ok_or(KeyError::Invalid("bad number"))? as u32,
            };
            out.values[at] = v;
            out.given |= 1 << at;
        }
        Ok(out)
    }

    /// The value of key `key`, if given (signed keys as `i32` bits).
    pub fn get(&self, key: u8) -> Option<u32> {
        let at = index(key)?;
        (self.given & (1 << at) != 0).then_some(self.values[at])
    }

    fn num(&self, key: u8) -> u32 {
        self.get(key).unwrap_or(0)
    }

    fn int(&self, key: u8) -> i32 {
        self.num(key) as i32
    }

    fn u16(&self, key: u8) -> u16 {
        self.num(key).min(u16::MAX as u32) as u16
    }

    /// Interpret the keys, with kitty's defaults.
    pub fn command(&self) -> Result<Command, KeyError> {
        let action = match self.get(b'a').map(|v| v as u8) {
            None | Some(b't') => Action::Transmit,
            Some(b'T') => Action::TransmitPlace,
            Some(b'p') => Action::Place,
            Some(b'd') => Action::Delete,
            Some(b'q') => Action::Query,
            Some(b'f') => Action::Frame,
            Some(b'a') => Action::Animate,
            Some(b'c') => Action::Compose,
            Some(_) => return Err(KeyError::Invalid("unknown action")),
        };
        let mut c = Command {
            action,
            ..Command::default()
        };
        c.quiet = self.num(b'q').min(2) as u8;
        c.format = match self.get(b'f') {
            None | Some(32) => Format::Rgba,
            Some(24) => Format::Rgb,
            Some(100) => Format::Png,
            Some(_) => return Err(KeyError::Invalid("unknown format")),
        };
        c.medium = match self.get(b't').map(|v| v as u8) {
            None | Some(b'd') => Medium::Direct,
            Some(b'f') => Medium::File,
            Some(b't') => Medium::TempFile,
            Some(b's') => Medium::SharedMemory,
            Some(_) => return Err(KeyError::Invalid("unknown transmission medium")),
        };
        c.compressed = match self.get(b'o').map(|v| v as u8) {
            None => false,
            Some(b'z') => true,
            Some(_) => return Err(KeyError::Invalid("unknown compression")),
        };
        c.more = self.num(b'm') == 1;
        c.image_id = self.num(b'i');
        c.image_number = self.num(b'I');
        if c.image_id != 0 && c.image_number != 0 {
            return Err(KeyError::Invalid("both i and I"));
        }
        c.placement_id = self.num(b'p');
        c.data_width = self.num(b's');
        c.data_height = self.num(b'v');
        c.file_size = self.num(b'S') as u64;
        c.file_offset = self.num(b'O') as u64;
        c.src = [self.num(b'x'), self.num(b'y'), self.num(b'w'), self.num(b'h')];
        c.cell_offset = (self.u16(b'X'), self.u16(b'Y'));
        c.cols = self.u16(b'c');
        c.rows = self.u16(b'r');
        c.no_cursor_move = self.num(b'C') == 1;
        c.virtual_ = self.num(b'U') == 1;
        c.z = self.int(b'z');
        if c.action == Action::Delete {
            let d = self.get(b'd').map(|v| v as u8).unwrap_or(b'a');
            let (x, y, z) = (self.num(b'x'), self.num(b'y'), self.int(b'z'));
            let what = match d.to_ascii_lowercase() {
                b'a' => DeleteWhat::All,
                b'i' => DeleteWhat::Id,
                b'n' => DeleteWhat::Number,
                b'r' => DeleteWhat::Range { from: x, to: y },
                b'c' => DeleteWhat::Cursor,
                b'p' => DeleteWhat::Cell { x, y },
                b'q' => DeleteWhat::CellZ { x, y, z },
                b'x' => DeleteWhat::Column(x),
                b'y' => DeleteWhat::Row(y),
                b'z' => DeleteWhat::Z(z),
                b'f' => DeleteWhat::Frames,
                _ => return Err(KeyError::Invalid("unknown delete specifier")),
            };
            c.delete = Some(Delete {
                what,
                free: d.is_ascii_uppercase(),
            });
        }
        c.unsupported =
            matches!(c.action, Action::Frame | Action::Animate | Action::Compose) || b"PQHV".iter().any(|&k| self.get(k).is_some());
        Ok(c)
    }
}

fn unsigned(v: &[u8]) -> Option<u32> {
    if v.is_empty() || v.len() > 10 {
        // Leading zeros past ten digits are still a number in kitty, but
        // nobody sends them.
        return None;
    }
    let mut n: u64 = 0;
    for &b in v {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (b - b'0') as u64;
    }
    u32::try_from(n).ok()
}

fn signed(v: &[u8]) -> Option<i32> {
    match v {
        [b'-', rest @ ..] => {
            let n = unsigned(rest)? as i64;
            i32::try_from(-n).ok()
        }
        _ => i32::try_from(unsigned(v)?).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Command {
        Keys::parse(s.as_bytes()).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn defaults() {
        let c = parse("");
        assert_eq!(c, Command::default());
        assert_eq!(c.action, Action::Transmit);
        assert_eq!(c.format, Format::Rgba);
        assert_eq!(c.medium, Medium::Direct);
        assert_eq!(c.delete, None);
    }

    #[test]
    fn one_row_per_key() {
        type Check = fn(&Command) -> bool;
        let rows: &[(&str, Check)] = &[
            ("a=t", |c| c.action == Action::Transmit),
            ("a=T", |c| c.action == Action::TransmitPlace),
            ("a=p", |c| c.action == Action::Place),
            ("a=d", |c| c.action == Action::Delete),
            ("a=q", |c| c.action == Action::Query),
            ("a=f", |c| c.action == Action::Frame && c.unsupported),
            ("a=a", |c| c.action == Action::Animate && c.unsupported),
            ("a=c", |c| c.action == Action::Compose && c.unsupported),
            ("q=0", |c| c.quiet == 0),
            ("q=1", |c| c.quiet == 1),
            ("q=2", |c| c.quiet == 2),
            ("q=9", |c| c.quiet == 2),
            ("f=24", |c| c.format == Format::Rgb),
            ("f=32", |c| c.format == Format::Rgba),
            ("f=100", |c| c.format == Format::Png),
            ("t=d", |c| c.medium == Medium::Direct),
            ("t=f", |c| c.medium == Medium::File),
            ("t=t", |c| c.medium == Medium::TempFile),
            ("t=s", |c| c.medium == Medium::SharedMemory),
            ("o=z", |c| c.compressed),
            ("m=1", |c| c.more),
            ("m=0", |c| !c.more),
            ("i=4294967295", |c| c.image_id == u32::MAX),
            ("I=7", |c| c.image_number == 7),
            ("p=3", |c| c.placement_id == 3),
            ("s=10,v=20", |c| (c.data_width, c.data_height) == (10, 20)),
            ("S=4096,O=128", |c| (c.file_size, c.file_offset) == (4096, 128)),
            ("x=1,y=2,w=3,h=4", |c| c.src == [1, 2, 3, 4]),
            ("X=5,Y=6", |c| c.cell_offset == (5, 6)),
            ("X=100000", |c| c.cell_offset.0 == u16::MAX),
            ("c=80,r=24", |c| (c.cols, c.rows) == (80, 24)),
            ("C=1", |c| c.no_cursor_move),
            ("U=1", |c| c.virtual_),
            ("z=-1073741825", |c| c.z == -1_073_741_825),
            ("z=-2147483648", |c| c.z == i32::MIN),
            ("z=2147483647", |c| c.z == i32::MAX),
            ("P=1", |c| c.unsupported),
            ("Q=1", |c| c.unsupported),
            ("H=-3", |c| c.unsupported),
            ("V=2", |c| c.unsupported),
            // Unknown and multi-letter keys are ignored.
            ("e=5,ab=1,a=p", |c| c.action == Action::Place),
            ("Z=letters", |c| *c == Command::default()),
            // Repeats take the last value; empty items are skipped.
            ("i=1,i=2", |c| c.image_id == 2),
            (",,i=1,,", |c| c.image_id == 1),
        ];
        for (s, check) in rows {
            assert!(check(&parse(s)), "{s}: {:?}", parse(s));
        }
    }

    #[test]
    fn deletes() {
        let d = |s: &str| parse(s).delete.unwrap();
        assert_eq!(
            d("a=d"),
            Delete {
                what: DeleteWhat::All,
                free: false
            }
        );
        assert_eq!(
            d("a=d,d=A"),
            Delete {
                what: DeleteWhat::All,
                free: true
            }
        );
        assert_eq!(
            d("a=d,d=i,i=10"),
            Delete {
                what: DeleteWhat::Id,
                free: false
            }
        );
        assert_eq!(d("a=d,d=I,i=10,p=2").what, DeleteWhat::Id);
        assert_eq!(
            d("a=d,d=N,I=3"),
            Delete {
                what: DeleteWhat::Number,
                free: true
            }
        );
        assert_eq!(d("a=d,d=r,x=3,y=9").what, DeleteWhat::Range { from: 3, to: 9 });
        assert_eq!(d("a=d,d=c").what, DeleteWhat::Cursor);
        assert_eq!(d("a=d,d=p,x=3,y=4").what, DeleteWhat::Cell { x: 3, y: 4 });
        assert_eq!(
            d("a=d,d=Q,x=3,y=4,z=-2"),
            Delete {
                what: DeleteWhat::CellZ { x: 3, y: 4, z: -2 },
                free: true
            }
        );
        assert_eq!(d("a=d,d=x,x=7").what, DeleteWhat::Column(7));
        assert_eq!(d("a=d,d=Y,y=7").what, DeleteWhat::Row(7));
        assert_eq!(d("a=d,d=z,z=5").what, DeleteWhat::Z(5));
        assert_eq!(d("a=d,d=f").what, DeleteWhat::Frames);
        // `d` without `a=d` is just read.
        assert_eq!(parse("d=i").delete, None);
    }

    #[test]
    fn errors() {
        let bad = [
            "i=1,I=2",
            "a=x",
            "a=",
            "a=tt",
            "a=1",
            "f=16",
            "t=x",
            "o=g",
            "i=4294967296",
            "i=-1",
            "i=1x",
            "i=",
            "i",
            "z=2147483648",
            "z=-2147483649",
            "z=--1",
            "a=d,d=k",
        ];
        for s in bad {
            assert!(matches!(Keys::parse(s.as_bytes()), Err(KeyError::Invalid(_))), "{s}");
        }
        assert_eq!(Keys::parse(&b"i=1,".repeat(1025)), Err(KeyError::TooLong));
        assert!(Keys::parse(&b"q=2,".repeat(1024)).is_ok());
        assert_eq!(KeyError::Invalid("x").code(), "EINVAL");
    }

    #[test]
    fn examples_from_the_protocol_document() {
        // Each from kitty's graphics-protocol.rst.
        let c = parse("f=24,s=10,v=20");
        assert_eq!((c.format, c.data_width, c.data_height), (Format::Rgb, 10, 20));
        let c = parse("f=100,t=f");
        assert_eq!((c.format, c.medium), (Format::Png, Medium::File));
        let c = parse("f=24,s=10,v=20,o=z");
        assert!(c.compressed);
        let c = parse("a=q,i=31,s=1,v=1,f=24,t=d");
        assert_eq!((c.action, c.image_id), (Action::Query, 31));
        let c = parse("f=100,m=1");
        assert!(c.more);
        let c = parse("m=0");
        assert!(!c.more);
        let c = parse("a=T,f=100,i=10,c=10,r=5");
        assert_eq!((c.action, c.cols, c.rows), (Action::TransmitPlace, 10, 5));
        let c = parse("a=p,i=10,p=2,x=10,y=10,w=40,h=40,X=3,Y=4,z=-1");
        assert_eq!((c.placement_id, c.src, c.cell_offset, c.z), (2, [10, 10, 40, 40], (3, 4), -1));
        let c = parse("a=T,U=1,i=42,c=10,r=5,q=2");
        assert!(c.virtual_ && c.quiet == 2);
        let c = parse("a=d,d=p,x=3,y=4");
        assert_eq!(c.delete.unwrap().what, DeleteWhat::Cell { x: 3, y: 4 });
        let c = parse("a=p,i=1,p=7,P=10,Q=11,H=-2,V=3");
        assert!(c.unsupported);
        let c = parse("i=31,I=0,a=q");
        assert_eq!(c.image_id, 31);
    }

    #[test]
    fn raw_keys() {
        let k = Keys::read(b"a=T,z=-5,q=1").unwrap();
        assert_eq!(k.get(b'a'), Some(b'T' as u32));
        assert_eq!(k.get(b'z'), Some(-5i32 as u32));
        assert_eq!(k.get(b'i'), None);
        assert_eq!(k.get(b'1'), None);
    }
}
