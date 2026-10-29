//! PNG, JPEG, GIF and WebP (first frame of animated ones) decoded in pure
//! Rust, so a bad image from a program can't crash every window through a
//! platform decoder (Alex's decision, images.md §12 question 3).
//!
//! [`decode`] writes premultiplied BGRA into a [`PixelTarget`] already
//! allocated at the size wanted ([`fit`] picks it from the header), so an
//! image is decoded at the size it is drawn: a 12-megapixel photo shown
//! at window size never exists at full size here.
//!
//! - Rows stream from the decoder through an area-averaging scaler and
//!   the EXIF orientation into the target, so beyond the target the
//!   memory used is a few rows, plus what each decoder holds:
//!   - PNG (the `png` crate): a row (all of it if interlaced);
//!   - JPEG (`jpeg-decoder`): the whole image at the decoder's output
//!     size, which its DCT scaling brings down by up to 8× each way first
//!     (a 4032 × 3024 photo drawn at 1000 px wide is decoded at 1008 ×
//!     756);
//!   - GIF (`gif`): the first frame as palette indices, a byte a pixel;
//!   - WebP (`image-webp`): the whole image as RGB or RGBA at full size
//!     (it has no scaled decoding), plus its own working copy: the YUV
//!     planes (lossy), an RGBA frame (lossless), or the canvas and a frame
//!     (animated). Checked against the limit before decoding.
//! - Each decoder's own allocations are capped by [`Options::transient`],
//!   and so are the scaler's rows together with the decoder's.
//! - Colour: converted to Display P3 from the image's ICC profile, or from
//!   sRGB if it has none ([`Options::display_p3`]); gamma chunks are
//!   ignored.

use crate::color::ToP3;
use crate::probe::{self, Format, Probe};
use crate::{PixelTarget, premultiply};
use std::io::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Most bytes a decoder may allocate for itself, beyond the target.
    pub transient: usize,
    /// Convert the pixels from the image's colour space (its ICC profile,
    /// or sRGB if it has none) to Display P3 ([`crate::color`]). On by
    /// default.
    pub display_p3: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            transient: 64 << 20,
            display_p3: true,
        }
    }
}

/// The conversion for an image with ICC profile `icc`, if any is needed.
fn colour_for(icc: Option<&[u8]>, options: &Options, own: &mut Option<ToP3>) -> bool {
    if !options.display_p3 {
        return false;
    }
    if let Some(t) = icc.and_then(ToP3::from_icc) {
        let identity = t.is_identity();
        *own = Some(t);
        return !identity;
    }
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Not PNG, JPEG, GIF or WebP, or a header that doesn't say its size.
    Unsupported,
    /// The decoder refused the data.
    Corrupt(String),
    /// The decoder would pass [`Options::transient`].
    TooLarge,
    /// The data ended early.
    Truncated,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Unsupported => f.write_str("unsupported image"),
            DecodeError::Corrupt(why) => write!(f, "corrupt image: {why}"),
            DecodeError::TooLarge => f.write_str("image too large to decode"),
            DecodeError::Truncated => f.write_str("image data truncated"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// How a decode went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Every pixel written had alpha 255.
    pub opaque: bool,
    /// The image's own size, as shown (after the orientation).
    pub source: (u32, u32),
}

/// The size to decode an image at: its own size, shrunk to fit inside
/// `max` keeping its aspect ratio (never enlarged), at least 1 × 1.
pub fn fit(probe: &Probe, max: (u32, u32)) -> (u32, u32) {
    fit_size((probe.width, probe.height), max)
}

/// [`fit`] for a plain size.
pub fn fit_size((w, h): (u32, u32), (mw, mh): (u32, u32)) -> (u32, u32) {
    if w <= mw && h <= mh {
        return (w.max(1), h.max(1));
    }
    let (w64, h64, mw64, mh64) = (w as u64, h as u64, mw.max(1) as u64, mh.max(1) as u64);
    // The tighter of the two scales.
    if w64 * mh64 >= h64 * mw64 {
        (mw.max(1), ((h64 * mw64 + w64 / 2) / w64).max(1) as u32)
    } else {
        (((w64 * mh64 + h64 / 2) / h64).max(1) as u32, mh.max(1))
    }
}

/// Decode `bytes` into `into`, scaled to `into`'s width and height.
pub fn decode(bytes: &[u8], into: &mut dyn PixelTarget, limits: &Options) -> Result<Decoded, DecodeError> {
    let p = match probe::probe(bytes) {
        Ok(p) => p,
        Err(probe::ProbeError::NeedMore) => return Err(DecodeError::Truncated),
        Err(probe::ProbeError::Unsupported) => return Err(DecodeError::Unsupported),
        Err(probe::ProbeError::Corrupt) => return Err(DecodeError::Corrupt("header".into())),
    };
    if into.width() == 0 || into.height() == 0 {
        return Ok(Decoded {
            opaque: true,
            source: (p.width, p.height),
        });
    }
    let opaque = match p.format {
        Format::Png => png(bytes, into, limits)?,
        Format::Jpeg => jpeg(bytes, &p, into, limits)?,
        Format::Gif => gif(bytes, into, limits)?,
        Format::Webp => webp(bytes, into, limits)?,
    };
    Ok(Decoded {
        opaque,
        source: (p.width, p.height),
    })
}

fn png(bytes: &[u8], into: &mut dyn PixelTarget, limits: &Options) -> Result<bool, DecodeError> {
    let mut d = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: limits.transient });
    d.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = d.read_info().map_err(png_error)?;
    let (sw, sh) = (reader.info().width, reader.info().height);
    let interlaced = reader.info().interlaced;
    let (color, _) = reader.output_color_type();
    let layout = match color {
        png::ColorType::Grayscale => Src::Gray,
        png::ColorType::GrayscaleAlpha => Src::GrayAlpha,
        png::ColorType::Rgb => Src::Rgb,
        png::ColorType::Rgba => Src::Rgba,
        png::ColorType::Indexed => return Err(DecodeError::Corrupt("palette not expanded".into())),
    };
    let mut own = None;
    let convert = colour_for(reader.info().icc_profile.as_deref(), limits, &mut own);
    let colour = convert.then(|| own.as_ref().unwrap_or(ToP3::srgb()));
    // The decoder's own rows: its output row, and the raw rows (this one
    // and the last, for unfiltering) it keeps as it inflates.
    let raw = reader.info().raw_row_length();
    let held = reader
        .output_line_size(sw)
        .ok_or(DecodeError::TooLarge)?
        .saturating_add(raw.saturating_mul(2));
    let mut sink = Sink::new((sw, sh), 1, colour, into, held, limits)?;
    if interlaced {
        let size = reader.output_buffer_size().ok_or(DecodeError::TooLarge)?;
        if size > limits.transient {
            return Err(DecodeError::TooLarge);
        }
        let mut buf = vec![0; size];
        let info = reader.next_frame(&mut buf).map_err(png_error)?;
        for row in buf[..info.buffer_size()].chunks_exact(info.line_size).take(sh as usize) {
            sink.row(layout, row);
        }
    } else {
        while let Some(row) = reader.next_row().map_err(png_error)? {
            sink.row(layout, row.data());
        }
    }
    sink.finish()
}

fn png_error(e: png::DecodingError) -> DecodeError {
    match e {
        png::DecodingError::LimitsExceeded => DecodeError::TooLarge,
        png::DecodingError::IoError(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => DecodeError::Truncated,
        e => DecodeError::Corrupt(e.to_string()),
    }
}

fn jpeg(bytes: &[u8], p: &Probe, into: &mut dyn PixelTarget, limits: &Options) -> Result<bool, DecodeError> {
    let mut d = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    d.set_max_decoding_buffer_size(limits.transient);
    // Ask for the stored-orientation size of the target; the decoder picks
    // the smallest DCT scale at least that large.
    let (tw, th) = if p.orientation >= 5 {
        (into.height(), into.width())
    } else {
        (into.width(), into.height())
    };
    let clamp = |v: u32| v.min(u16::MAX as u32) as u16;
    d.scale(clamp(tw), clamp(th)).map_err(jpeg_error)?;
    let info = d.info().ok_or(DecodeError::Corrupt("no frame".into()))?;
    let (sw, sh) = (info.width as u32, info.height as u32);
    let px = info.pixel_format.pixel_bytes();
    let estimate = sw as usize * sh as usize * px;
    if estimate > limits.transient {
        return Err(DecodeError::TooLarge);
    }
    let data = d.decode().map_err(jpeg_error)?;
    let layout = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => Src::Gray,
        jpeg_decoder::PixelFormat::L16 => Src::Gray16,
        jpeg_decoder::PixelFormat::RGB24 => Src::Rgb,
        jpeg_decoder::PixelFormat::CMYK32 => Src::Cmyk,
    };
    let line = sw as usize * px;
    if line == 0 || data.len() < line * sh as usize {
        return Err(DecodeError::Truncated);
    }
    let icc = d.icc_profile();
    let mut own = None;
    let convert = colour_for(icc.as_deref(), limits, &mut own);
    let colour = convert.then(|| own.as_ref().unwrap_or(ToP3::srgb()));
    let mut sink = Sink::new((sw, sh), p.orientation, colour, into, estimate, limits)?;
    for row in data.chunks_exact(line).take(sh as usize) {
        sink.row(layout, row);
    }
    sink.finish()
}

fn jpeg_error(e: jpeg_decoder::Error) -> DecodeError {
    match e {
        jpeg_decoder::Error::Io(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => DecodeError::Truncated,
        jpeg_decoder::Error::Unsupported(_) => DecodeError::Unsupported,
        e => {
            let s = e.to_string();
            if s.contains("buffer size") || s.contains("limit") {
                DecodeError::TooLarge
            } else {
                DecodeError::Corrupt(s)
            }
        }
    }
}

fn gif(bytes: &[u8], into: &mut dyn PixelTarget, limits: &Options) -> Result<bool, DecodeError> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let limit = std::num::NonZeroU64::new(limits.transient as u64).ok_or(DecodeError::TooLarge)?;
    opts.set_memory_limit(gif::MemoryLimit::Bytes(limit));
    let mut d = opts.read_info(Cursor::new(bytes)).map_err(gif_error)?;
    let (sw, sh) = (d.width() as u32, d.height() as u32);
    let global = d.global_palette().map(<[u8]>::to_vec);
    let Some(frame) = d.read_next_frame().map_err(gif_error)? else {
        return Err(DecodeError::Truncated);
    };
    let palette = frame.palette.as_deref().or(global.as_deref()).unwrap_or(&[]);
    let (fx, fy, fw, fh) = (frame.left as u32, frame.top as u32, frame.width as u32, frame.height as u32);
    if frame.buffer.len() < fw as usize * fh as usize {
        return Err(DecodeError::Truncated);
    }
    // The logical screen, transparent where the first frame doesn't cover it.
    let colour = limits.display_p3.then(ToP3::srgb);
    let row_bytes = sw as usize * 4;
    let held = frame.buffer.len() + row_bytes;
    let mut sink = Sink::new((sw, sh), 1, colour, into, held, limits)?;
    let mut row = vec![0u8; row_bytes];
    for y in 0..sh {
        row.fill(0);
        if y >= fy && y < fy + fh {
            let src = &frame.buffer[((y - fy) * fw) as usize..((y - fy + 1) * fw) as usize];
            for (i, &index) in src.iter().enumerate() {
                let x = fx as usize + i;
                if x >= sw as usize {
                    break;
                }
                if Some(index) == frame.transparent {
                    continue;
                }
                let at = index as usize * 3;
                if let Some(c) = palette.get(at..at + 3) {
                    row[x * 4..x * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
        sink.row(Src::Rgba, &row);
    }
    sink.finish()
}

fn gif_error(e: gif::DecodingError) -> DecodeError {
    match e {
        gif::DecodingError::OutOfMemory => DecodeError::TooLarge,
        gif::DecodingError::Io(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => DecodeError::Truncated,
        e => DecodeError::Corrupt(e.to_string()),
    }
}

fn webp(bytes: &[u8], into: &mut dyn PixelTarget, limits: &Options) -> Result<bool, DecodeError> {
    let mut d = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(webp_error)?;
    d.set_memory_limit(limits.transient);
    let (sw, sh) = d.dimensions();
    let alpha = d.has_alpha();
    // image-webp allocates its output and working buffers without checking
    // its limit (which covers metadata chunks only), so check the sum here:
    // the output at 3 or 4 bytes a pixel, plus 4 a pixel for its own copy
    // (lossless decodes RGBA; lossy holds 1.5–2.5 of YUV and alpha), plus
    // 8 more for an animation's canvas and first frame.
    let pixels = sw as u64 * sh as u64;
    let own = if d.is_animated() { 12 } else { 4 };
    let out = d.output_buffer_size().ok_or(DecodeError::TooLarge)?;
    if out as u64 + pixels * own > limits.transient as u64 {
        return Err(DecodeError::TooLarge);
    }
    let icc = d.icc_profile().map_err(webp_error)?;
    let mut buf = vec![0; out];
    d.read_image(&mut buf).map_err(webp_error)?;
    let (layout, px) = if alpha { (Src::Rgba, 4) } else { (Src::Rgb, 3) };
    let mut own = None;
    let convert = colour_for(icc.as_deref(), limits, &mut own);
    let colour = convert.then(|| own.as_ref().unwrap_or(ToP3::srgb()));
    let mut sink = Sink::new((sw, sh), 1, colour, into, out, limits)?;
    for row in buf.chunks_exact(sw as usize * px).take(sh as usize) {
        sink.row(layout, row);
    }
    sink.finish()
}

fn webp_error(e: image_webp::DecodingError) -> DecodeError {
    use image_webp::DecodingError as E;
    match e {
        E::IoError(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => DecodeError::Truncated,
        E::MemoryLimitExceeded | E::ImageTooLarge => DecodeError::TooLarge,
        E::UnsupportedFeature(_) => DecodeError::Unsupported,
        e => DecodeError::Corrupt(e.to_string()),
    }
}

/// The layout of a decoder's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    Gray,
    /// 16-bit big-endian grey.
    Gray16,
    GrayAlpha,
    Rgb,
    Rgba,
    /// Adobe's inverted CMYK, as JPEG decoders hand it over.
    Cmyk,
}

impl Src {
    /// Grey stays grey in any RGB space with the same white: no conversion.
    fn is_colour(self) -> bool {
        matches!(self, Src::Rgb | Src::Rgba | Src::Cmyk)
    }
}

/// Takes source rows in order, scales them to the target by averaging
/// (or repeating, if the target is larger), turns them by the EXIF
/// orientation and writes them.
struct Sink<'a> {
    target: &'a mut dyn PixelTarget,
    /// The colour conversion, applied to each source row.
    colour: Option<&'a ToP3>,
    orientation: u8,
    /// Source rows.
    sh: u32,
    /// The scaled size before the orientation.
    tw: u32,
    th: u32,
    /// For each scaled column, the source columns it averages.
    cols: Vec<(u32, u32)>,
    /// Premultiplied BGRA of the source row being added.
    line: Vec<u8>,
    /// Sums for the scaled row being built: 64-bit, as a span can hold
    /// more than 2³² / 255 source pixels (a 17,000,000 × 1 image).
    acc: Vec<u64>,
    acc_rows: u64,
    /// Next source row, next scaled row.
    sy: u32,
    ty: u32,
    opaque: bool,
}

impl<'a> Sink<'a> {
    /// A sink for a `sw` × `sh` source, or [`DecodeError::TooLarge`] if
    /// its rows and the `held` bytes the decoder keeps for its own would
    /// pass [`Options::transient`]: a source row is 4 bytes a pixel here
    /// whatever the decoder's own size, so a very wide grey image could
    /// otherwise take 4 × the decoder's row past the cap.
    fn new(
        (sw, sh): (u32, u32),
        orientation: u8,
        colour: Option<&'a ToP3>,
        target: &'a mut dyn PixelTarget,
        held: usize,
        limits: &Options,
    ) -> Result<Sink<'a>, DecodeError> {
        let (tw, th) = if orientation >= 5 {
            (target.height(), target.width())
        } else {
            (target.width(), target.height())
        };
        let own = sw as u64 * 4 + tw as u64 * (4 * size_of::<u64>() as u64 + size_of::<(u32, u32)>() as u64);
        if (held as u64).saturating_add(own) > limits.transient as u64 {
            return Err(DecodeError::TooLarge);
        }
        let cols = (0..tw).map(|x| span(x, sw, tw)).collect();
        Ok(Sink {
            target,
            colour,
            orientation,
            sh,
            tw,
            th,
            cols,
            line: vec![0; sw as usize * 4],
            acc: vec![0; tw as usize * 4],
            acc_rows: 0,
            sy: 0,
            ty: 0,
            opaque: true,
        })
    }

    fn row(&mut self, layout: Src, src: &[u8]) {
        if self.sy >= self.sh {
            return;
        }
        if self.orientation <= 1 && self.th == self.sh && self.tw as usize * 4 == self.line.len() {
            // Same size, upright: straight into the target's row.
            let pitch = self.target.row_pitch();
            let at = self.sy as usize * pitch;
            let dst = &mut self.target.bytes_mut()[at..at + self.line.len()];
            self.opaque &= to_bgra(layout, src, dst);
            if let Some(c) = self.colour.filter(|_| layout.is_colour()) {
                c.bgra_premultiplied(dst);
            }
            self.sy += 1;
            self.ty += 1;
            return;
        }
        self.opaque &= to_bgra(layout, src, &mut self.line);
        if let Some(c) = self.colour.filter(|_| layout.is_colour()) {
            c.bgra_premultiplied(&mut self.line);
        }
        let r = self.sy;
        self.sy += 1;
        // Every scaled row whose span holds this source row.
        while self.ty < self.th {
            let (y0, y1) = span(self.ty, self.sh, self.th);
            if r < y0 {
                break;
            }
            self.add_line();
            if r + 1 == y1 {
                self.emit();
                self.ty += 1;
            } else {
                break;
            }
        }
    }

    fn add_line(&mut self) {
        for (x, &(x0, x1)) in self.cols.iter().enumerate() {
            let a = &mut self.acc[x * 4..x * 4 + 4];
            for px in self.line[x0 as usize * 4..x1 as usize * 4].as_chunks::<4>().0 {
                for c in 0..4 {
                    a[c] += px[c] as u64;
                }
            }
        }
        self.acc_rows += 1;
    }

    fn emit(&mut self) {
        let (tw, th, ty) = (self.tw, self.th, self.ty);
        let pitch = self.target.row_pitch();
        let orientation = self.orientation;
        let rows = self.acc_rows.max(1);
        let bytes = self.target.bytes_mut();
        for (x, &(x0, x1)) in self.cols.iter().enumerate() {
            let n = rows * (x1 - x0).max(1) as u64;
            let a = &mut self.acc[x * 4..x * 4 + 4];
            let px = [0, 1, 2, 3].map(|c| ((a[c] + n / 2) / n) as u8);
            a.fill(0);
            let (dx, dy) = orient(orientation, x as u32, ty, tw, th);
            let at = dy as usize * pitch + dx as usize * 4;
            bytes[at..at + 4].copy_from_slice(&px);
        }
        self.acc_rows = 0;
    }

    fn finish(self) -> Result<bool, DecodeError> {
        if self.sy < self.sh || self.ty < self.th {
            Err(DecodeError::Truncated)
        } else {
            Ok(self.opaque)
        }
    }
}

/// The source range `[start, end)` that scaled index `i` of `n` covers in
/// `len` source pixels: at least one pixel, so enlarging repeats them.
fn span(i: u32, len: u32, n: u32) -> (u32, u32) {
    let (i, len, n) = (i as u64, len as u64, n as u64);
    let start = (i * len / n).min(len.saturating_sub(1));
    let end = ((i + 1) * len / n).clamp(start + 1, len.max(1));
    (start as u32, end as u32)
}

/// Where stored pixel (`x`, `y`) of a `w` × `h` image goes once shown with
/// EXIF orientation `o`.
fn orient(o: u8, x: u32, y: u32, w: u32, h: u32) -> (u32, u32) {
    match o {
        2 => (w - 1 - x, y),
        3 => (w - 1 - x, h - 1 - y),
        4 => (x, h - 1 - y),
        5 => (y, x),
        6 => (h - 1 - y, x),
        7 => (h - 1 - y, w - 1 - x),
        8 => (y, w - 1 - x),
        _ => (x, y),
    }
}

/// A source row to premultiplied BGRA; returns whether all of it is opaque.
fn to_bgra(layout: Src, src: &[u8], dst: &mut [u8]) -> bool {
    let out = dst.as_chunks_mut::<4>().0;
    match layout {
        Src::Gray => {
            for (d, &g) in out.iter_mut().zip(src) {
                *d = [g, g, g, 255];
            }
            true
        }
        Src::Gray16 => {
            for (d, g) in out.iter_mut().zip(src.as_chunks::<2>().0) {
                *d = [g[0], g[0], g[0], 255];
            }
            true
        }
        Src::GrayAlpha => {
            let mut all = 255;
            for (d, s) in out.iter_mut().zip(src.as_chunks::<2>().0) {
                let g = premultiply(s[0], s[1]);
                all &= s[1];
                *d = [g, g, g, s[1]];
            }
            all == 255
        }
        Src::Rgb => {
            crate::convert::rgb_row(src, dst);
            true
        }
        Src::Rgba => crate::convert::rgba_row(src, dst),
        Src::Cmyk => {
            for (d, s) in out.iter_mut().zip(src.as_chunks::<4>().0) {
                let k = s[3];
                *d = [premultiply(s[2], k), premultiply(s[1], k), premultiply(s[0], k), 255];
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VecPixels;

    /// No colour conversion, so pixels can be checked exactly.
    fn raw() -> Options {
        Options {
            display_p3: false,
            ..Options::default()
        }
    }

    /// The png crate writes no interlaced PNGs: those are fixtures.
    fn png_bytes(w: u32, h: u32, color: png::ColorType, interlaced: bool, data: &[u8]) -> Vec<u8> {
        if interlaced {
            assert_eq!((w, h, data), (4, 2, &rgba_4x2()[..]));
            return include_bytes!("../tests/fixtures/rgba-4x2-adam7.png").to_vec();
        }
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(color);
            e.set_depth(png::BitDepth::Eight);
            e.write_header().unwrap().write_image_data(data).unwrap();
        }
        out
    }

    /// A 4 × 2 RGBA image: red, green, blue, half-transparent white;
    /// then black, white, grey, transparent.
    fn rgba_4x2() -> Vec<u8> {
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128, //
            0, 0, 0, 255, 255, 255, 255, 255, 128, 128, 128, 255, 9, 9, 9, 0,
        ]
    }

    #[test]
    fn png_at_its_own_size() {
        for interlaced in [false, true] {
            let bytes = png_bytes(4, 2, png::ColorType::Rgba, interlaced, &rgba_4x2());
            let mut t = VecPixels::new(4, 2);
            let d = decode(&bytes, &mut t, &raw()).unwrap();
            assert_eq!(
                d,
                Decoded {
                    opaque: false,
                    source: (4, 2)
                }
            );
            assert_eq!(t.pixel(0, 0), [0, 0, 255, 255]);
            assert_eq!(t.pixel(1, 0), [0, 255, 0, 255]);
            assert_eq!(t.pixel(2, 0), [255, 0, 0, 255]);
            assert_eq!(t.pixel(3, 0), [128, 128, 128, 128]);
            assert_eq!(t.pixel(2, 1), [128, 128, 128, 255]);
            assert_eq!(t.pixel(3, 1), [0, 0, 0, 0]);
        }
    }

    #[test]
    fn converted_to_display_p3() {
        let red = png_bytes(1, 1, png::ColorType::Rgb, false, &[255, 0, 0]);
        let mut t = VecPixels::new(1, 1);
        decode(&red, &mut t, &Options::default()).unwrap();
        let [r, g, b] = ToP3::srgb().rgb([255, 0, 0]);
        assert_eq!(t.pixel(0, 0), [b, g, r, 255]);
        assert_ne!((r, g, b), (255, 0, 0));
        // Grey is left alone.
        let grey = png_bytes(1, 1, png::ColorType::Grayscale, false, &[77]);
        decode(&grey, &mut t, &Options::default()).unwrap();
        assert_eq!(t.pixel(0, 0), [77, 77, 77, 255]);
        // A PNG tagged Display P3 is already right; one tagged sRGB is
        // converted as one with no profile.
        let tagged = |profile: Vec<u8>| {
            let mut info = png::Info::with_size(1, 1);
            info.color_type = png::ColorType::Rgb;
            info.bit_depth = png::BitDepth::Eight;
            info.icc_profile = Some(std::borrow::Cow::Owned(profile));
            let mut out = Vec::new();
            png::Encoder::with_info(&mut out, info)
                .unwrap()
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0])
                .unwrap();
            out
        };
        let p3 = [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)];
        let srgb = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
        decode(&tagged(crate::color::tests::icc(p3)), &mut t, &Options::default()).unwrap();
        assert_eq!(t.pixel(0, 0), [0, 0, 255, 255]);
        decode(&tagged(crate::color::tests::icc(srgb)), &mut t, &Options::default()).unwrap();
        let px = t.pixel(0, 0);
        assert!(px[2].abs_diff(r) <= 1 && px[1].abs_diff(g) <= 1 && px[0].abs_diff(b) <= 1, "{px:?}");
    }

    #[test]
    fn png_colour_types() {
        let mut t = VecPixels::new(2, 1);
        let rgb = png_bytes(2, 1, png::ColorType::Rgb, false, &[10, 20, 30, 40, 50, 60]);
        assert!(decode(&rgb, &mut t, &raw()).unwrap().opaque);
        assert_eq!(t.pixel(1, 0), [60, 50, 40, 255]);
        let grey = png_bytes(2, 1, png::ColorType::Grayscale, false, &[7, 200]);
        decode(&grey, &mut t, &raw()).unwrap();
        assert_eq!(t.pixel(1, 0), [200, 200, 200, 255]);
        let ga = png_bytes(2, 1, png::ColorType::GrayscaleAlpha, false, &[200, 255, 200, 0]);
        assert!(!decode(&ga, &mut t, &raw()).unwrap().opaque);
        assert_eq!(t.pixel(0, 0), [200, 200, 200, 255]);
        assert_eq!(t.pixel(1, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn scaled_down_by_averaging() {
        // 4 × 2 → 2 × 1: each output pixel averages a 2 × 2 block.
        let bytes = png_bytes(4, 2, png::ColorType::Rgba, false, &rgba_4x2());
        let mut t = VecPixels::new(2, 1);
        decode(&bytes, &mut t, &raw()).unwrap();
        // red, green, black, white: (255+0+0+255)/4 etc.
        assert_eq!(t.pixel(0, 0), [64, 128, 128, 255]);
        // blue, half white (128 premultiplied), grey, transparent.
        let avg = |v: u32| ((v + 2) / 4) as u8;
        assert_eq!(
            t.pixel(1, 0),
            [avg(255 + 128 + 128), avg(128 + 128), avg(128 + 128), avg(255 + 128 + 255)]
        );
    }

    #[test]
    fn a_large_image_scales_to_any_size() {
        let (w, h) = (300u32, 200u32);
        let data: Vec<u8> = (0..w * h).flat_map(|i| [(i % w) as u8, (i / w) as u8, 77]).collect();
        let bytes = png_bytes(w, h, png::ColorType::Rgb, false, &data);
        for (tw, th) in [(1, 1), (7, 3), (150, 100), (299, 199), (300, 200), (450, 300)] {
            let mut t = VecPixels::new(tw, th);
            decode(&bytes, &mut t, &raw()).unwrap();
            assert_eq!(t.pixel(0, 0)[0], 77);
            assert_eq!(t.pixel(tw - 1, th - 1)[3], 255);
        }
    }

    #[test]
    fn fit_keeps_the_aspect_ratio() {
        assert_eq!(fit_size((4032, 3024), (1000, 1000)), (1000, 750));
        assert_eq!(fit_size((3024, 4032), (1000, 1000)), (750, 1000));
        assert_eq!(fit_size((100, 50), (1000, 1000)), (100, 50));
        assert_eq!(fit_size((10000, 1), (100, 100)), (100, 1));
        assert_eq!(fit_size((1, 10000), (100, 100)), (1, 100));
        assert_eq!(fit_size((500, 500), (0, 0)), (1, 1));
    }

    #[test]
    fn orientations_turn_the_image() {
        // A 3 × 2 stored image, each pixel a different grey, through each
        // orientation's mapping: every pixel lands somewhere different.
        for o in 1..=8u8 {
            let (w, h) = (3, 2);
            let (dw, dh) = if o >= 5 { (h, w) } else { (w, h) };
            let mut seen = std::collections::HashSet::new();
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = orient(o, x, y, w, h);
                    assert!(dx < dw && dy < dh, "orientation {o}");
                    assert!(seen.insert((dx, dy)));
                }
            }
        }
        // 6: the stored top-left goes to the top-right.
        assert_eq!(orient(6, 0, 0, 3, 2), (1, 0));
        // 8: the stored top-left goes to the bottom-left.
        assert_eq!(orient(8, 0, 0, 3, 2), (0, 2));
    }

    #[test]
    fn errors() {
        let mut t = VecPixels::new(4, 2);
        let l = raw();
        assert_eq!(decode(b"%PDF-1.7", &mut t, &l), Err(DecodeError::Unsupported));
        let bytes = png_bytes(4, 2, png::ColorType::Rgba, false, &rgba_4x2());
        assert!(decode(&bytes[..bytes.len() - 20], &mut t, &l).is_err());
        assert_eq!(decode(&bytes[..20], &mut t, &l), Err(DecodeError::Truncated));
        let mut bad = bytes.clone();
        let n = bad.len();
        bad[n - 30] ^= 0xFF;
        assert!(decode(&bad, &mut t, &l).is_err());
        // A tiny transient limit: an interlaced PNG needs the whole image.
        let big = include_bytes!("../tests/fixtures/gradient-300x300-adam7.png");
        let mut t = VecPixels::new(300, 300);
        assert_eq!(
            decode(
                big,
                &mut t,
                &Options {
                    transient: 100_000,
                    ..raw()
                }
            ),
            Err(DecodeError::TooLarge)
        );
        assert!(decode(big, &mut t, &l).unwrap().opaque);
        assert_eq!(t.pixel(299, 299), [7, 180, 180, 255]);
        let jpeg = include_bytes!("../tests/fixtures/photo-1600x1200.jpg");
        let mut t = VecPixels::new(1600, 1200);
        assert_eq!(
            decode(
                jpeg,
                &mut t,
                &Options {
                    transient: 1 << 20,
                    ..raw()
                }
            ),
            Err(DecodeError::TooLarge)
        );
    }

    /// Red top-left, green top-right, blue bottom-left, white bottom-right.
    fn check_quadrants(t: &VecPixels, tolerance: u8) {
        let (w, h) = (t.width(), t.height());
        let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= tolerance);
        for (x, y, want) in [
            (w / 4, h / 4, [0, 0, 255, 255]),
            (3 * w / 4, h / 4, [0, 255, 0, 255]),
            (w / 4, 3 * h / 4, [255, 0, 0, 255]),
            (3 * w / 4, 3 * h / 4, [255, 255, 255, 255]),
        ] {
            assert!(near(t.pixel(x, y), want), "({x}, {y}) is {:?}, want {want:?}", t.pixel(x, y));
        }
    }

    #[test]
    fn gif_first_frame() {
        let gif = include_bytes!("../tests/fixtures/quadrants-64x48.gif");
        let mut t = VecPixels::new(64, 48);
        let d = decode(gif, &mut t, &raw()).unwrap();
        assert_eq!(d.source, (64, 48));
        check_quadrants(&t, 0);
        let mut t = VecPixels::new(16, 12);
        decode(gif, &mut t, &raw()).unwrap();
        check_quadrants(&t, 0);
    }

    /// A lossless WebP from image-webp's encoder, with an ICC profile if
    /// given (the extended format).
    fn webp_bytes(w: u32, h: u32, rgba: &[u8], icc: Option<Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = image_webp::WebPEncoder::new(&mut out);
        if let Some(icc) = icc {
            e.set_icc_profile(icc);
        }
        e.encode(rgba, w, h, image_webp::ColorType::Rgba8).unwrap();
        out
    }

    #[test]
    fn webp_lossless_at_its_own_size() {
        let bytes = webp_bytes(4, 2, &rgba_4x2(), None);
        let p = probe::probe(&bytes).unwrap();
        assert_eq!((p.format, p.width, p.height), (Format::Webp, 4, 2));
        let mut t = VecPixels::new(4, 2);
        let d = decode(&bytes, &mut t, &raw()).unwrap();
        assert_eq!(
            d,
            Decoded {
                opaque: false,
                source: (4, 2)
            }
        );
        assert_eq!(t.pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(t.pixel(1, 0), [0, 255, 0, 255]);
        assert_eq!(t.pixel(2, 0), [255, 0, 0, 255]);
        assert_eq!(t.pixel(3, 0), [128, 128, 128, 128]);
        assert_eq!(t.pixel(2, 1), [128, 128, 128, 255]);
        assert_eq!(t.pixel(3, 1), [0, 0, 0, 0]);
        // Halved: each pixel averages a 2 × 2 block, as for PNG.
        let mut t = VecPixels::new(2, 1);
        decode(&bytes, &mut t, &raw()).unwrap();
        assert_eq!(t.pixel(0, 0), [64, 128, 128, 255]);
    }

    #[test]
    fn webp_lossy_and_lossless() {
        let lossy = include_bytes!("../tests/fixtures/quadrants-64x48-lossy.webp");
        let lossless = include_bytes!("../tests/fixtures/quadrants-64x48-lossless.webp");
        for (bytes, tolerance) in [(&lossy[..], 40), (&lossless[..], 0)] {
            for (w, h) in [(64, 48), (32, 24), (16, 12), (7, 5), (100, 75)] {
                let mut t = VecPixels::new(w, h);
                let d = decode(bytes, &mut t, &raw()).unwrap();
                assert!(d.opaque);
                assert_eq!(d.source, (64, 48));
                check_quadrants(&t, tolerance);
            }
        }
    }

    /// Opaque red top-left, green at alpha 128 top-right, opaque blue
    /// bottom-left, transparent bottom-right; premultiplied BGRA.
    fn check_alpha_quadrants(t: &VecPixels, tolerance: u8) {
        let (w, h) = (t.width(), t.height());
        let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= tolerance);
        for (x, y, want) in [
            (w / 4, h / 4, [0, 0, 255, 255]),
            (3 * w / 4, h / 4, [0, 128, 0, 128]),
            (w / 4, 3 * h / 4, [255, 0, 0, 255]),
            (3 * w / 4, 3 * h / 4, [0, 0, 0, 0]),
        ] {
            assert!(near(t.pixel(x, y), want), "({x}, {y}) is {:?}, want {want:?}", t.pixel(x, y));
        }
    }

    #[test]
    fn webp_alpha_and_animation() {
        for (name, bytes, tolerance) in [
            ("lossless", &include_bytes!("../tests/fixtures/alpha-64x48-lossless.webp")[..], 0),
            (
                "lossy, ALPH chunk",
                &include_bytes!("../tests/fixtures/alpha-64x48-lossy.webp")[..],
                40,
            ),
            // Its second frame is black: the first is shown, as for GIF.
            ("animated", &include_bytes!("../tests/fixtures/animated-64x48.webp")[..], 40),
        ] {
            for (w, h) in [(64, 48), (16, 12)] {
                let mut t = VecPixels::new(w, h);
                let d = decode(bytes, &mut t, &raw()).unwrap();
                assert!(!d.opaque, "{name}");
                assert_eq!(d.source, (64, 48), "{name}");
                check_alpha_quadrants(&t, tolerance);
            }
        }
    }

    #[test]
    fn webp_converted_to_display_p3() {
        let red = [255, 0, 0, 255];
        let [r, g, b] = ToP3::srgb().rgb([255, 0, 0]);
        let mut t = VecPixels::new(1, 1);
        decode(&webp_bytes(1, 1, &red, None), &mut t, &Options::default()).unwrap();
        assert_eq!(t.pixel(0, 0), [b, g, r, 255]);
        let p3 = [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)];
        let tagged = webp_bytes(1, 1, &red, Some(crate::color::tests::icc(p3)));
        decode(&tagged, &mut t, &Options::default()).unwrap();
        assert_eq!(t.pixel(0, 0), [0, 0, 255, 255]);
        // A lossy photo, through the whole path.
        let photo = include_bytes!("../tests/fixtures/photo-1600x1200.webp");
        let (w, h) = fit(&probe::probe(photo).unwrap(), (400, 400));
        assert_eq!((w, h), (400, 300));
        let mut t = VecPixels::new(w, h);
        assert!(decode(photo, &mut t, &Options::default()).unwrap().opaque);
        let [b, g, r, _] = t.pixel(10, 5);
        assert!(b > r && b > 150, "{b} {g} {r}");
    }

    #[test]
    fn webp_errors_and_limits() {
        let bytes = include_bytes!("../tests/fixtures/alpha-64x48-lossy.webp");
        let mut t = VecPixels::new(64, 48);
        let l = raw();
        assert_eq!(decode(&bytes[..20], &mut t, &l), Err(DecodeError::Truncated));
        // Cut anywhere: no panic, and an error unless only the end of the
        // lossy data is missing (VP8 reads zeros past it, as libwebp does).
        for n in 0..bytes.len() {
            let r = decode(&bytes[..n], &mut t, &l);
            assert!(r.is_err() || n > bytes.len() - 8, "length {n}");
        }
        let photo = include_bytes!("../tests/fixtures/photo-1600x1200.webp");
        let mut t = VecPixels::new(400, 300);
        // 1600 × 1200 × (3 + 4) is 13.44 MB.
        let tight = |transient| Options { transient, ..raw() };
        assert_eq!(decode(photo, &mut t, &tight(13_439_999)), Err(DecodeError::TooLarge));
        assert!(decode(photo, &mut t, &tight(13_440_000)).is_ok());
        let animated = include_bytes!("../tests/fixtures/animated-64x48.webp");
        let mut t = VecPixels::new(64, 48);
        assert_eq!(decode(animated, &mut t, &tight(64 * 48 * 15)), Err(DecodeError::TooLarge));
        assert!(decode(animated, &mut t, &tight(64 * 48 * 16)).is_ok());
    }

    #[test]
    fn jpeg_sizes() {
        let jpeg = include_bytes!("../tests/fixtures/quadrants-64x48.jpg");
        for (w, h) in [(64, 48), (32, 24), (16, 12), (8, 6), (100, 75)] {
            let mut t = VecPixels::new(w, h);
            let d = decode(jpeg, &mut t, &raw()).unwrap();
            assert!(d.opaque);
            assert_eq!(d.source, (64, 48));
            check_quadrants(&t, 40);
        }
    }

    /// The JPEG with an EXIF block saying `orientation`, put after SOI.
    fn with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
        let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        exif.extend_from_slice(&orientation.to_le_bytes());
        exif.extend_from_slice(&[0; 6]);
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        out.extend(exif);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    #[test]
    fn jpeg_orientations() {
        let jpeg = include_bytes!("../tests/fixtures/quadrants-64x48.jpg");
        // Where each stored quadrant (red, green, blue, white: top-left,
        // top-right, bottom-left, bottom-right) is shown.
        let red = [0, 0, 255, 255];
        let green = [0, 255, 0, 255];
        let blue = [255, 0, 0, 255];
        let white = [255, 255, 255, 255];
        let shown: [(u16, [[u8; 4]; 4]); 8] = [
            // (top-left, top-right, bottom-left, bottom-right) once shown.
            (1, [red, green, blue, white]),
            (2, [green, red, white, blue]),
            (3, [white, blue, green, red]),
            (4, [blue, white, red, green]),
            (5, [red, blue, green, white]),
            (6, [blue, red, white, green]),
            (7, [white, green, blue, red]),
            (8, [green, white, red, blue]),
        ];
        for (o, want) in shown {
            let bytes = with_orientation(jpeg, o);
            let p = probe::probe(&bytes).unwrap();
            assert_eq!(p.orientation, o as u8);
            let (w, h) = (p.width, p.height);
            assert_eq!((w, h), if o >= 5 { (48, 64) } else { (64, 48) });
            let mut t = VecPixels::new(w, h);
            decode(&bytes, &mut t, &raw()).unwrap();
            let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 40);
            let at = [(w / 4, h / 4), (3 * w / 4, h / 4), (w / 4, 3 * h / 4), (3 * w / 4, 3 * h / 4)];
            for ((x, y), want) in at.into_iter().zip(want) {
                assert!(
                    near(t.pixel(x, y), want),
                    "orientation {o}: ({x}, {y}) is {:?}, want {want:?}",
                    t.pixel(x, y)
                );
            }
        }
    }

    #[test]
    fn a_photo_decoded_small() {
        let jpeg = include_bytes!("../tests/fixtures/photo-1600x1200.jpg");
        let p = probe::probe(jpeg).unwrap();
        let (w, h) = fit(&p, (400, 400));
        assert_eq!((w, h), (400, 300));
        let mut t = VecPixels::new(w, h);
        decode(jpeg, &mut t, &raw()).unwrap();
        // The sky at the top, bluish; the ground at the bottom, greenish.
        let [b, g, r, _] = t.pixel(10, 5);
        assert!(b > r && b > 150, "{b} {g} {r}");
        let [b, g, r, _] = t.pixel(10, 295);
        assert!(g > r && g > b, "{b} {g} {r}");
    }
}
