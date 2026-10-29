//! Format sniffing and sizes from the first bytes of an image, so the
//! terminal knows an image's size (and moves the cursor by it) before it
//! is decoded (images.md §6.1). PNG, JPEG and GIF only; anything else is
//! refused before a decoder sees a byte.
//!
//! - PNG: the `IHDR` chunk, in the first 33 bytes.
//! - GIF: the logical screen descriptor, in the first 10 bytes.
//! - JPEG: the frame header (`SOFn`), after any number of segments, and
//!   the EXIF orientation from `APP1`. Orientations 5–8 turn the image a
//!   quarter, so the size reported is the size shown, width and height
//!   swapped.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
}

/// What the header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probe {
    pub format: Format,
    /// The size as shown, after the orientation.
    pub width: u32,
    pub height: u32,
    /// EXIF orientation, 1–8 (1 = as stored). Always 1 for PNG and GIF.
    pub orientation: u8,
}

impl Probe {
    /// The size as stored, before the orientation.
    pub fn stored_size(&self) -> (u32, u32) {
        if self.orientation >= 5 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// The bytes end before the header does; more might settle it.
    NeedMore,
    /// Not PNG, JPEG or GIF (TIFF, PDF, HEIC, SVG, …).
    Unsupported,
    /// The format's signature, but a broken or zero-sized header.
    Corrupt,
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ProbeError::NeedMore => "image header truncated",
            ProbeError::Unsupported => "unsupported image format",
            ProbeError::Corrupt => "corrupt image header",
        })
    }
}

impl std::error::Error for ProbeError {}

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Which format `head` starts with, from its signature alone.
pub fn sniff(head: &[u8]) -> Option<Format> {
    if head.starts_with(PNG_SIGNATURE) {
        Some(Format::Png)
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else {
        None
    }
}

/// Format and size from the first bytes of an image. `NeedMore` means the
/// header runs past `head`; the caller decides how much to read before
/// giving up (the terminal looks at 64 KB at most).
pub fn probe(head: &[u8]) -> Result<Probe, ProbeError> {
    // A prefix of a signature isn't enough to refuse.
    let prefixes: [&[u8]; 4] = [PNG_SIGNATURE, &[0xFF, 0xD8, 0xFF], b"GIF87a", b"GIF89a"];
    match sniff(head) {
        Some(Format::Png) => png(head),
        Some(Format::Jpeg) => jpeg(head),
        Some(Format::Gif) => gif(head),
        None if prefixes.iter().any(|p| head.len() < p.len() && p.starts_with(head)) => Err(ProbeError::NeedMore),
        None => Err(ProbeError::Unsupported),
    }
}

fn be16(b: &[u8]) -> u32 {
    u16::from_be_bytes([b[0], b[1]]) as u32
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn sized(format: Format, width: u32, height: u32, orientation: u8) -> Result<Probe, ProbeError> {
    if width == 0 || height == 0 {
        return Err(ProbeError::Corrupt);
    }
    let (width, height) = if orientation >= 5 { (height, width) } else { (width, height) };
    Ok(Probe {
        format,
        width,
        height,
        orientation,
    })
}

fn png(head: &[u8]) -> Result<Probe, ProbeError> {
    // Signature, then the first chunk, which must be a 13-byte IHDR.
    if head.len() < 24 {
        return Err(ProbeError::NeedMore);
    }
    if be32(&head[8..]) != 13 || &head[12..16] != b"IHDR" {
        return Err(ProbeError::Corrupt);
    }
    let (w, h) = (be32(&head[16..]), be32(&head[20..]));
    // The PNG spec caps both at 2^31 - 1.
    if w > i32::MAX as u32 || h > i32::MAX as u32 {
        return Err(ProbeError::Corrupt);
    }
    sized(Format::Png, w, h, 1)
}

fn gif(head: &[u8]) -> Result<Probe, ProbeError> {
    if head.len() < 10 {
        return Err(ProbeError::NeedMore);
    }
    let w = u16::from_le_bytes([head[6], head[7]]) as u32;
    let h = u16::from_le_bytes([head[8], head[9]]) as u32;
    sized(Format::Gif, w, h, 1)
}

fn jpeg(head: &[u8]) -> Result<Probe, ProbeError> {
    let mut orientation = 1;
    let mut i = 2;
    loop {
        // Markers may be preceded by any number of 0xFF fill bytes.
        while head.get(i) == Some(&0xFF) && head.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        let (Some(&ff), Some(&marker)) = (head.get(i), head.get(i + 1)) else {
            return Err(ProbeError::NeedMore);
        };
        if ff != 0xFF {
            return Err(ProbeError::Corrupt);
        }
        i += 2;
        match marker {
            // Markers with no length: TEM, RSTn.
            0x01 | 0xD0..=0xD7 => continue,
            // A second SOI, EOI or SOS before any frame header.
            0xD8..=0xDA => return Err(ProbeError::Corrupt),
            _ => {}
        }
        let Some(len) = head.get(i..i + 2).map(be16) else {
            return Err(ProbeError::NeedMore);
        };
        let len = len as usize;
        if len < 2 {
            return Err(ProbeError::Corrupt);
        }
        match marker {
            // SOF0–SOF15, except DHT (C4), JPG (C8) and DAC (CC).
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                // length, precision, height, width.
                let Some(f) = head.get(i..i + 7) else {
                    return Err(ProbeError::NeedMore);
                };
                if len < 8 {
                    return Err(ProbeError::Corrupt);
                }
                return sized(Format::Jpeg, be16(&f[5..]), be16(&f[3..]), orientation);
            }
            // APP1: EXIF, for the orientation. A broken EXIF block is
            // ignored rather than refusing the image, as decoders do.
            0xE1 => {
                let Some(seg) = head.get(i + 2..i + len) else {
                    return Err(ProbeError::NeedMore);
                };
                if let Some(o) = exif_orientation(seg) {
                    orientation = o;
                }
            }
            _ => {}
        }
        i += len;
    }
}

/// The orientation tag (0x0112) of IFD0 in an `APP1` segment's contents,
/// if it is EXIF and the value is 1–8.
pub fn exif_orientation(seg: &[u8]) -> Option<u8> {
    let tiff = seg.strip_prefix(b"Exif\0\0")?;
    let le = match tiff.get(..4)? {
        [b'I', b'I', 42, 0] => true,
        [b'M', b'M', 0, 42] => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = tiff.get(at..at + 2)?;
        Some(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    for n in 0..count {
        let entry = ifd + 2 + n * 12;
        if u16_at(entry)? == 0x0112 {
            // Type SHORT (3), count 1: the value sits in the entry.
            if u16_at(entry + 2)? != 3 {
                return None;
            }
            let v = u16_at(entry + 8)?;
            return (1..=8).contains(&v).then_some(v as u8);
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut v = PNG_SIGNATURE.to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v.extend_from_slice(&[0; 4]); // CRC, not checked
        v
    }

    fn exif(orientation: u16, le: bool) -> Vec<u8> {
        let mut t = Vec::new();
        let p16 = |t: &mut Vec<u8>, v: u16| t.extend_from_slice(&if le { v.to_le_bytes() } else { v.to_be_bytes() });
        let p32 = |t: &mut Vec<u8>, v: u32| t.extend_from_slice(&if le { v.to_le_bytes() } else { v.to_be_bytes() });
        t.extend_from_slice(if le { b"II" } else { b"MM" });
        p16(&mut t, 42);
        p32(&mut t, 8);
        p16(&mut t, 2); // two entries
        // ImageWidth first, so the orientation isn't the first entry.
        p16(&mut t, 0x0100);
        p16(&mut t, 3);
        p32(&mut t, 1);
        p16(&mut t, 640);
        p16(&mut t, 0);
        p16(&mut t, 0x0112);
        p16(&mut t, 3);
        p32(&mut t, 1);
        p16(&mut t, orientation);
        p16(&mut t, 0);
        p32(&mut t, 0);
        let mut seg = b"Exif\0\0".to_vec();
        seg.extend(t);
        seg
    }

    pub fn jpeg_header(w: u16, h: u16, orientation: Option<u16>) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        // APP0 JFIF.
        v.extend_from_slice(&[0xFF, 0xE0, 0, 16]);
        v.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        if let Some(o) = orientation {
            let seg = exif(o, o % 2 == 0);
            v.extend_from_slice(&[0xFF, 0xE1]);
            v.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
            v.extend(seg);
        }
        // A DQT-like segment to skip, with fill bytes before the marker.
        v.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xDB, 0, 4, 0, 0]);
        // SOF2 (progressive).
        v.extend_from_slice(&[0xFF, 0xC2, 0, 17, 8]);
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        v
    }

    #[test]
    fn png_size() {
        let h = png_header(1624, 1018);
        assert_eq!(
            probe(&h),
            Ok(Probe {
                format: Format::Png,
                width: 1624,
                height: 1018,
                orientation: 1
            })
        );
        assert_eq!(probe(&h[..24]).unwrap().width, 1624);
        assert_eq!(probe(&h[..23]), Err(ProbeError::NeedMore));
        assert_eq!(probe(&png_header(0, 5)), Err(ProbeError::Corrupt));
        assert_eq!(probe(&png_header(u32::MAX, 5)), Err(ProbeError::Corrupt));
        let mut bad = h.clone();
        bad[12] = b'X';
        assert_eq!(probe(&bad), Err(ProbeError::Corrupt));
    }

    #[test]
    fn gif_size() {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&[0x40, 0x01, 0xF0, 0x00, 0xF7, 0, 0]);
        assert_eq!(
            probe(&g),
            Ok(Probe {
                format: Format::Gif,
                width: 320,
                height: 240,
                orientation: 1
            })
        );
        assert_eq!(probe(&g[..9]), Err(ProbeError::NeedMore));
        g[3..6].copy_from_slice(b"87a");
        assert_eq!(probe(&g).unwrap().format, Format::Gif);
        assert_eq!(probe(b"GIF89a\0\0\x10\0"), Err(ProbeError::Corrupt));
    }

    #[test]
    fn jpeg_sizes_and_orientations() {
        let plain = jpeg_header(4032, 3024, None);
        assert_eq!(
            probe(&plain),
            Ok(Probe {
                format: Format::Jpeg,
                width: 4032,
                height: 3024,
                orientation: 1
            })
        );
        for o in 1..=8u16 {
            let p = probe(&jpeg_header(4032, 3024, Some(o))).unwrap();
            assert_eq!(p.orientation, o as u8);
            if o >= 5 {
                assert_eq!((p.width, p.height), (3024, 4032), "orientation {o}");
            } else {
                assert_eq!((p.width, p.height), (4032, 3024), "orientation {o}");
            }
            assert_eq!(p.stored_size(), (4032, 3024));
        }
        // Out-of-range orientation values are ignored.
        assert_eq!(probe(&jpeg_header(10, 20, Some(9))).unwrap().orientation, 1);
        assert_eq!(probe(&jpeg_header(10, 20, Some(0))).unwrap().orientation, 1);
    }

    #[test]
    fn truncated_jpeg_needs_more_at_every_length() {
        let h = jpeg_header(800, 600, Some(6));
        for n in 0..h.len() - 10 {
            assert_eq!(probe(&h[..n]), Err(ProbeError::NeedMore), "length {n}");
        }
    }

    #[test]
    fn garbage_and_other_formats_are_refused() {
        let others: [&[u8]; 8] = [
            b"II*\0\x08\0\0\0",                            // TIFF
            b"MM\0*\0\0\0\x08",                            // TIFF
            b"%PDF-1.7\n",                                 // PDF
            b"\0\0\0\x18ftypheic\0\0\0\0mif1heic",         // HEIC
            b"<?xml version=\"1.0\"?><svg>",               // SVG
            b"<svg xmlns=\"http://www.w3.org/2000/svg\">", // SVG
            b"RIFF\0\0\0\0WEBPVP8 ",                       // WebP
            b"BM\x36\0\0\0",                               // BMP
        ];
        for o in others {
            assert_eq!(probe(o), Err(ProbeError::Unsupported), "{:?}", o.escape_ascii());
        }
        assert_eq!(probe(b""), Err(ProbeError::NeedMore));
        assert_eq!(probe(b"\x89PN"), Err(ProbeError::NeedMore));
        assert_eq!(probe(b"\xFF\xD8\xFF\xD9"), Err(ProbeError::Corrupt));
        assert_eq!(probe(b"\xFF\xD8\xFF\xE0\0\x01"), Err(ProbeError::Corrupt));
        assert_eq!(probe(b"\xFF\xD8\xFF\xE0\0\x02\x12\x34"), Err(ProbeError::Corrupt));
    }

    #[test]
    fn broken_exif_is_ignored() {
        assert_eq!(exif_orientation(b"Exif\0\0II*\0\xff\xff\xff\xff"), None);
        assert_eq!(exif_orientation(b"Exif\0\0XX*\0"), None);
        assert_eq!(exif_orientation(b"http://ns.adobe.com/xap/1.0/\0"), None);
        assert_eq!(exif_orientation(&exif(6, true)), Some(6));
        assert_eq!(exif_orientation(&exif(3, false)), Some(3));
        let e = exif(6, true);
        for n in 0..e.len() {
            let _ = exif_orientation(&e[..n]);
        }
    }
}
