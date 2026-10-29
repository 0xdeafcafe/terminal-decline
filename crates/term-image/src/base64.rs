//! Streaming base64 (standard alphabet), for kitty payloads and iTerm2
//! files.
//!
//! Input may be split anywhere: decoding in any slices gives the same bytes
//! as decoding in one go. Lenient where senders differ:
//! - ASCII whitespace is skipped (older `imgcat`s wrap at 76 columns);
//! - `=` ends a group early, and more data may follow it (iTerm2's
//!   `FilePart`s are padded one by one);
//! - a missing final `=` is accepted ([`Base64::finish`]).
//!
//! kitty decodes each chunk on its own, so its caller makes a new decoder
//! (or calls [`Base64::finish`]) per chunk.

/// Not a base64 byte.
const BAD: u8 = 0xFF;
/// Whitespace, skipped.
const SKIP: u8 = 0xFE;
/// `=`.
const PAD: u8 = 0xFD;

static TABLE: [u8; 256] = {
    let mut t = [BAD; 256];
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut i = 0;
    while i < 64 {
        t[alphabet[i] as usize] = i as u8;
        i += 1;
    }
    t[b' ' as usize] = SKIP;
    t[b'\t' as usize] = SKIP;
    t[b'\n' as usize] = SKIP;
    t[b'\r' as usize] = SKIP;
    t[0x0C] = SKIP;
    t[b'=' as usize] = PAD;
    t
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base64Error {
    /// A byte outside the alphabet, whitespace and `=`.
    BadByte(u8),
    /// A group ended (by `=` or the end) after a single character, which
    /// carries less than a byte.
    Truncated,
}

impl std::fmt::Display for Base64Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Base64Error::BadByte(b) => write!(f, "bad base64 byte {b:#04x}"),
            Base64Error::Truncated => f.write_str("truncated base64"),
        }
    }
}

impl std::error::Error for Base64Error {}

/// Bytes decoded per call of the sink in [`Base64::feed_to`].
const BLOCK: usize = 3 * 1024;

/// A streaming decoder. After an error, everything is refused until
/// [`Base64::reset`].
#[derive(Clone, Debug, Default)]
pub struct Base64 {
    /// Sextets of the group in progress, high first.
    acc: u32,
    /// How many sextets `acc` holds (0..=3).
    have: u8,
    failed: Option<Base64Error>,
}

impl Base64 {
    pub fn new() -> Base64 {
        Base64::default()
    }

    pub fn reset(&mut self) {
        *self = Base64::default();
    }

    /// Decode `input`, appending to `out`.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), Base64Error> {
        out.reserve(input.len() / 4 * 3 + 3);
        self.feed_to(input, &mut |bytes| out.extend_from_slice(bytes))
    }

    /// Decode `input`, handing the bytes to `sink` in pieces of up to 3 KB
    /// (decoded on the stack, so no allocation).
    pub fn feed_to(&mut self, input: &[u8], sink: &mut dyn FnMut(&[u8])) -> Result<(), Base64Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        let mut buf = [0u8; BLOCK + 3];
        let mut n = 0;
        let mut i = 0;
        while i < input.len() {
            // Fast path: whole groups of four alphabet bytes.
            if self.have == 0 {
                while i + 4 <= input.len() && n + 3 <= BLOCK {
                    let a = TABLE[input[i] as usize] as u32;
                    let b = TABLE[input[i + 1] as usize] as u32;
                    let c = TABLE[input[i + 2] as usize] as u32;
                    let d = TABLE[input[i + 3] as usize] as u32;
                    if (a | b | c | d) >= 64 {
                        break;
                    }
                    let v = (a << 18) | (b << 12) | (c << 6) | d;
                    buf[n] = (v >> 16) as u8;
                    buf[n + 1] = (v >> 8) as u8;
                    buf[n + 2] = v as u8;
                    n += 3;
                    i += 4;
                }
                if n + 3 > BLOCK {
                    sink(&buf[..n]);
                    n = 0;
                    continue;
                }
                if i == input.len() {
                    break;
                }
            }
            // One byte at a time: whitespace, padding, a group split across
            // calls, or a bad byte.
            let byte = input[i];
            i += 1;
            match TABLE[byte as usize] {
                SKIP => {}
                PAD => match self.flush(&mut buf[n..]) {
                    Ok(k) => n += k,
                    Err(e) => {
                        self.failed = Some(e);
                        sink(&buf[..n]);
                        return Err(e);
                    }
                },
                BAD => {
                    let e = Base64Error::BadByte(byte);
                    self.failed = Some(e);
                    sink(&buf[..n]);
                    return Err(e);
                }
                v => {
                    self.acc = (self.acc << 6) | v as u32;
                    self.have += 1;
                    if self.have == 4 {
                        let v = self.acc;
                        buf[n] = (v >> 16) as u8;
                        buf[n + 1] = (v >> 8) as u8;
                        buf[n + 2] = v as u8;
                        n += 3;
                        self.acc = 0;
                        self.have = 0;
                    }
                }
            }
            if n + 3 > BLOCK {
                sink(&buf[..n]);
                n = 0;
            }
        }
        if n > 0 {
            sink(&buf[..n]);
        }
        Ok(())
    }

    /// End of the data: decode a final group that had no `=`.
    pub fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), Base64Error> {
        self.finish_to(&mut |bytes| out.extend_from_slice(bytes))
    }

    /// [`Base64::finish`] with a sink. The decoder is ready for new data
    /// afterwards (unless it failed).
    pub fn finish_to(&mut self, sink: &mut dyn FnMut(&[u8])) -> Result<(), Base64Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        let mut buf = [0u8; 3];
        match self.flush(&mut buf) {
            Ok(n) => {
                if n > 0 {
                    sink(&buf[..n]);
                }
                Ok(())
            }
            Err(e) => {
                self.failed = Some(e);
                Err(e)
            }
        }
    }

    /// Decode a partial group into `out`; returns the bytes written.
    fn flush(&mut self, out: &mut [u8]) -> Result<usize, Base64Error> {
        let (acc, have) = (self.acc, self.have);
        self.acc = 0;
        self.have = 0;
        match have {
            0 => Ok(0),
            1 => Err(Base64Error::Truncated),
            2 => {
                out[0] = (acc >> 4) as u8;
                Ok(1)
            }
            3 => {
                out[0] = (acc >> 10) as u8;
                out[1] = (acc >> 2) as u8;
                Ok(2)
            }
            _ => unreachable!(),
        }
    }
}

/// Decode all of `input` at once.
pub fn decode(input: &[u8]) -> Result<Vec<u8>, Base64Error> {
    let mut d = Base64::new();
    let mut out = Vec::with_capacity(input.len() / 4 * 3 + 3);
    d.feed(input, &mut out)?;
    d.finish(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(data: &[u8]) -> Vec<u8> {
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        for c in data.chunks(3) {
            let v = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            out.push(alphabet[(v >> 18) as usize & 63]);
            out.push(alphabet[(v >> 12) as usize & 63]);
            out.push(if c.len() > 1 { alphabet[(v >> 6) as usize & 63] } else { b'=' });
            out.push(if c.len() > 2 { alphabet[v as usize & 63] } else { b'=' });
        }
        out
    }

    #[test]
    fn known_values() {
        assert_eq!(decode(b"").unwrap(), b"");
        assert_eq!(decode(b"Zg==").unwrap(), b"f");
        assert_eq!(decode(b"Zm8=").unwrap(), b"fo");
        assert_eq!(decode(b"Zm9v").unwrap(), b"foo");
        assert_eq!(decode(b"Zm9vYmFy").unwrap(), b"foobar");
        // No padding at the end.
        assert_eq!(decode(b"Zm9vYg").unwrap(), b"foob");
        assert_eq!(decode(b"Zm9vYmE").unwrap(), b"fooba");
        // Whitespace and line wrapping.
        assert_eq!(decode(b"Zm9v\r\nYmFy\n").unwrap(), b"foobar");
        assert_eq!(decode(b" Z m 9 v ").unwrap(), b"foo");
        // Padded pieces joined together (iTerm2 FileParts).
        assert_eq!(decode(b"Zg==Zm8=Zm9v").unwrap(), b"ffofoo");
        // Extra padding is harmless.
        assert_eq!(decode(b"Zm9v==").unwrap(), b"foo");
    }

    #[test]
    fn errors() {
        assert_eq!(decode(b"Zm9v*"), Err(Base64Error::BadByte(b'*')));
        assert_eq!(decode(b"Z"), Err(Base64Error::Truncated));
        assert_eq!(decode(b"Zm9vY="), Err(Base64Error::Truncated));
        assert_eq!(decode(b"Zm-_"), Err(Base64Error::BadByte(b'-')));
        assert_eq!(decode(b"Zm9v\x1b"), Err(Base64Error::BadByte(0x1b)));
        // A failed decoder stays failed.
        let mut d = Base64::new();
        let mut out = Vec::new();
        assert!(d.feed(b"Zm9v!", &mut out).is_err());
        assert_eq!(out, b"foo");
        assert!(d.feed(b"Zm9v", &mut out).is_err());
        d.reset();
        assert!(d.feed(b"Zm9v", &mut out).is_ok());
    }

    #[test]
    fn every_split_point_gives_the_same_bytes() {
        let data: Vec<u8> = (0..=255u8).chain(0..=100).collect();
        for tail in 0..3 {
            let data = &data[..data.len() - tail];
            let mut enc = encode(data);
            enc.insert(40, b'\n');
            let whole = decode(&enc).unwrap();
            assert_eq!(whole, data);
            for split in 0..=enc.len() {
                let mut d = Base64::new();
                let mut out = Vec::new();
                d.feed(&enc[..split], &mut out).unwrap();
                d.feed(&enc[split..], &mut out).unwrap();
                d.finish(&mut out).unwrap();
                assert_eq!(out, data, "split at {split}");
            }
            // And three pieces, byte by byte for the middle one.
            let mut d = Base64::new();
            let mut out = Vec::new();
            for b in &enc {
                d.feed(std::slice::from_ref(b), &mut out).unwrap();
            }
            d.finish(&mut out).unwrap();
            assert_eq!(out, data);
        }
    }

    #[test]
    fn padding_may_end_any_chunk() {
        // kitty: each chunk decoded on its own, so each may end in `=`.
        let mut d = Base64::new();
        let mut out = Vec::new();
        for chunk in [&b"Zg=="[..], b"Zm8=", b"Zm9v"] {
            d.feed(chunk, &mut out).unwrap();
            d.finish(&mut out).unwrap();
        }
        assert_eq!(out, b"ffofoo");
    }

    #[test]
    fn large_input_crosses_blocks() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i * 7 + i / 13) as u8).collect();
        let enc = encode(&data);
        assert_eq!(decode(&enc).unwrap(), data);
        let mut pieces = 0;
        let mut d = Base64::new();
        let mut out = Vec::new();
        d.feed_to(&enc, &mut |b| {
            pieces += 1;
            assert!(b.len() <= BLOCK);
            out.extend_from_slice(b);
        })
        .unwrap();
        assert_eq!(out, data);
        assert!(pieces >= data.len() / BLOCK);
    }
}
