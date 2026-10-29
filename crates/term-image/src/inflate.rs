//! zlib inflate (kitty's `o=z`) with a cap on the output, so a small
//! stream that inflates to gigabytes costs nothing: it is refused as soon
//! as the output passes the cap, and at most the cap has been handed on.
//!
//! Uses miniz_oxide (pure Rust). Holds about 43 KB of state (miniz's window
//! and tables plus a 32 KB output block), whatever the size of the data.

use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InflateError {
    /// The output would pass the cap.
    TooLarge,
    /// Not a zlib stream, or corrupt (bad header, bad data, bad checksum).
    Corrupt,
    /// [`Inflate::finish`] before the end of the stream.
    Truncated,
}

impl std::fmt::Display for InflateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            InflateError::TooLarge => "inflated data too large",
            InflateError::Corrupt => "corrupt zlib data",
            InflateError::Truncated => "truncated zlib data",
        })
    }
}

impl std::error::Error for InflateError {}

const BLOCK: usize = 32 * 1024;

/// A streaming zlib decoder. After an error, everything is refused.
pub struct Inflate {
    state: Box<InflateState>,
    out: Box<[u8; BLOCK]>,
    cap: usize,
    total: usize,
    done: bool,
    failed: Option<InflateError>,
}

impl std::fmt::Debug for Inflate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inflate")
            .field("cap", &self.cap)
            .field("total", &self.total)
            .field("done", &self.done)
            .finish()
    }
}

impl Inflate {
    /// A decoder that refuses output past `cap` bytes.
    pub fn new(cap: usize) -> Inflate {
        Inflate {
            state: InflateState::new_boxed(DataFormat::Zlib),
            out: Box::new([0; BLOCK]),
            cap,
            total: 0,
            done: false,
            failed: None,
        }
    }

    /// Bytes handed on so far.
    pub fn total_out(&self) -> usize {
        self.total
    }

    /// True once the end of the stream has been seen.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Inflate `input`, appending to `out`.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), InflateError> {
        self.feed_to(input, &mut |bytes| out.extend_from_slice(bytes))
    }

    /// Inflate `input`, handing the output to `sink` in pieces of up to
    /// 32 KB. Bytes after the end of the stream are ignored.
    pub fn feed_to(&mut self, mut input: &[u8], sink: &mut dyn FnMut(&[u8])) -> Result<(), InflateError> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        while !self.done {
            let r = inflate(&mut self.state, input, &mut self.out[..], MZFlush::None);
            input = &input[r.bytes_consumed..];
            if r.bytes_written > 0 {
                if self.total + r.bytes_written > self.cap {
                    return Err(self.fail(InflateError::TooLarge));
                }
                self.total += r.bytes_written;
                sink(&self.out[..r.bytes_written]);
            }
            match r.status {
                Ok(MZStatus::StreamEnd) => self.done = true,
                Ok(_) => {
                    // A full output block may mean more output is waiting.
                    if input.is_empty() && r.bytes_written < BLOCK {
                        break;
                    }
                }
                // No progress: it wants more input.
                Err(MZError::Buf) => break,
                Err(_) => return Err(self.fail(InflateError::Corrupt)),
            }
        }
        Ok(())
    }

    /// The data has ended: an error unless the stream ended too.
    pub fn finish(&mut self) -> Result<(), InflateError> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        if self.done {
            Ok(())
        } else {
            Err(self.fail(InflateError::Truncated))
        }
    }

    fn fail(&mut self, e: InflateError) -> InflateError {
        self.failed = Some(e);
        e
    }
}

/// Inflate all of `input`, refusing more than `cap` bytes of output.
pub fn inflate_all(input: &[u8], cap: usize) -> Result<Vec<u8>, InflateError> {
    let mut z = Inflate::new(cap);
    let mut out = Vec::new();
    z.feed(input, &mut out)?;
    z.finish()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::compress_to_vec_zlib;

    #[test]
    fn round_trip_in_any_slices() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8 ^ (i >> 9) as u8).collect();
        let z = compress_to_vec_zlib(&data, 6);
        assert_eq!(inflate_all(&z, data.len()).unwrap(), data);
        for piece in [1, 7, 4096, 65536] {
            let mut d = Inflate::new(data.len());
            let mut out = Vec::new();
            for c in z.chunks(piece) {
                d.feed(c, &mut out).unwrap();
            }
            d.finish().unwrap();
            assert_eq!(out, data, "pieces of {piece}");
        }
    }

    #[test]
    fn a_bomb_is_refused_at_the_cap() {
        // 128 MB of zeros, compressed to about 128 KB (deflate's ratio
        // tops out near 1000:1), fed in 1 KB pieces, with a 1 MB cap. Built
        // in pieces so the test itself never holds the 128 MB.
        let zero = vec![0u8; 1 << 20];
        let mut enc =
            miniz_oxide::deflate::core::CompressorOxide::new(miniz_oxide::deflate::core::create_comp_flags_from_zip_params(9, 1, 0));
        let mut z = Vec::new();
        let mut buf = vec![0u8; 1 << 16];
        for i in 0..128 {
            let flush = if i == 127 {
                miniz_oxide::deflate::core::TDEFLFlush::Finish
            } else {
                miniz_oxide::deflate::core::TDEFLFlush::None
            };
            let mut input = &zero[..];
            loop {
                let (status, used, written) = miniz_oxide::deflate::core::compress(&mut enc, input, &mut buf, flush);
                z.extend_from_slice(&buf[..written]);
                input = &input[used..];
                if status == miniz_oxide::deflate::core::TDEFLStatus::Done || (input.is_empty() && written < buf.len()) {
                    break;
                }
            }
        }
        assert!(z.len() < 256 << 10, "{}", z.len());

        let cap = 1 << 20;
        let mut d = Inflate::new(cap);
        let mut seen = 0usize;
        let mut biggest = 0usize;
        let mut result = Ok(());
        for c in z.chunks(1024) {
            result = d.feed_to(c, &mut |b| {
                seen += b.len();
                biggest = biggest.max(b.len());
            });
            if result.is_err() {
                break;
            }
        }
        assert_eq!(result, Err(InflateError::TooLarge));
        assert!(seen <= cap);
        assert!(biggest <= BLOCK);
        // Stays failed.
        assert_eq!(d.feed(&z[..10], &mut Vec::new()), Err(InflateError::TooLarge));
    }

    #[test]
    fn truncated_and_corrupt() {
        let data = b"hello hello hello hello".repeat(100);
        let z = compress_to_vec_zlib(&data, 6);
        assert_eq!(inflate_all(&z[..z.len() - 6], 1 << 20), Err(InflateError::Truncated));
        assert_eq!(inflate_all(&z[..1], 1 << 20), Err(InflateError::Truncated));
        assert_eq!(inflate_all(b"", 1 << 20), Err(InflateError::Truncated));
        assert_eq!(inflate_all(b"not zlib at all", 1 << 20), Err(InflateError::Corrupt));
        // A bad checksum.
        let mut bad = z.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(inflate_all(&bad, 1 << 20), Err(InflateError::Corrupt));
        // Exactly at the cap is fine, one under is not.
        assert_eq!(inflate_all(&z, data.len()).unwrap(), data);
        assert_eq!(inflate_all(&z, data.len() - 1), Err(InflateError::TooLarge));
    }

    #[test]
    fn bytes_after_the_end_are_ignored() {
        let mut z = compress_to_vec_zlib(b"abc", 6);
        z.extend_from_slice(b"trailing");
        assert_eq!(inflate_all(&z, 10).unwrap(), b"abc");
    }
}
