//! Raw pixels (kitty `f=24` and `f=32`) to premultiplied BGRA, written into
//! a [`PixelTarget`] as they stream in, so a raw image is ready the moment
//! its last byte arrives (images.md §3.3).

use crate::color::ToP3;
use crate::{PixelTarget, premultiply};

/// The layout of the incoming pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// 3 bytes per pixel, R G B.
    Rgb,
    /// 4 bytes per pixel, R G B A, straight alpha.
    Rgba,
}

impl Layout {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Layout::Rgb => 3,
            Layout::Rgba => 4,
        }
    }
}

/// Converts a stream of raw pixels, split anywhere, into a target of the
/// image's size.
#[derive(Clone, Debug)]
pub struct Converter {
    layout: Layout,
    width: u32,
    height: u32,
    /// The next pixel's position.
    x: u32,
    y: u32,
    /// Bytes of a pixel split across calls.
    partial: [u8; 4],
    partial_len: u8,
    /// Every alpha seen so far was 255.
    opaque: bool,
    /// Bytes past the end of the image, ignored.
    extra: usize,
    colour: Option<&'static ToP3>,
}

/// How a conversion ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Converted {
    /// No pixel had alpha below 255 (always true for RGB).
    pub opaque: bool,
    /// Bytes received past the end of the image (ignored).
    pub extra: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertError {
    /// The data ended before the last pixel; `got` of `want` bytes arrived.
    Short { got: usize, want: usize },
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvertError::Short { got, want } => write!(f, "pixel data short: {got} of {want} bytes"),
        }
    }
}

impl std::error::Error for ConvertError {}

impl Converter {
    /// For an image of `width` × `height`. The target given to
    /// [`Converter::feed`] must be at least that size.
    pub fn new(layout: Layout, width: u32, height: u32) -> Converter {
        Converter {
            layout,
            width,
            height,
            x: 0,
            y: 0,
            partial: [0; 4],
            partial_len: 0,
            opaque: true,
            extra: 0,
            colour: None,
        }
    }

    /// Convert the pixels, taken as sRGB, to Display P3 as they are written.
    pub fn display_p3(mut self) -> Converter {
        self.colour = Some(ToP3::srgb());
        self
    }

    /// The number of bytes the whole image takes.
    pub fn expected_len(&self) -> usize {
        self.width as usize * self.height as usize * self.layout.bytes_per_pixel()
    }

    fn received(&self) -> usize {
        (self.y as usize * self.width as usize + self.x as usize) * self.layout.bytes_per_pixel() + self.partial_len as usize
    }

    fn done(&self) -> bool {
        self.y >= self.height || self.width == 0
    }

    /// Convert `data` into `target`.
    pub fn feed(&mut self, mut data: &[u8], target: &mut dyn PixelTarget) {
        debug_assert!(target.width() >= self.width && target.height() >= self.height);
        let bpp = self.layout.bytes_per_pixel();
        let pitch = target.row_pitch();
        let bytes = target.bytes_mut();
        // Finish a pixel split across calls.
        if self.partial_len > 0 && !self.done() {
            let need = bpp - self.partial_len as usize;
            let take = need.min(data.len());
            self.partial[self.partial_len as usize..self.partial_len as usize + take].copy_from_slice(&data[..take]);
            self.partial_len += take as u8;
            data = &data[take..];
            if self.partial_len as usize == bpp {
                let px = self.partial;
                let at = self.y as usize * pitch + self.x as usize * 4;
                self.opaque &= put(self.layout, &px[..bpp], &mut bytes[at..at + 4]);
                if let Some(c) = self.colour {
                    c.bgra_premultiplied(&mut bytes[at..at + 4]);
                }
                self.partial_len = 0;
                self.advance(1);
            }
        }
        while !data.is_empty() && !self.done() {
            let row_left = (self.width - self.x) as usize;
            let whole = (data.len() / bpp).min(row_left);
            if whole == 0 {
                // Less than a pixel left: keep it.
                self.partial[..data.len()].copy_from_slice(data);
                self.partial_len = data.len() as u8;
                return;
            }
            let at = self.y as usize * pitch + self.x as usize * 4;
            let dst = &mut bytes[at..at + whole * 4];
            let src = &data[..whole * bpp];
            self.opaque &= match self.layout {
                Layout::Rgb => {
                    rgb_row(src, dst);
                    true
                }
                Layout::Rgba => rgba_row(src, dst),
            };
            if let Some(c) = self.colour {
                c.bgra_premultiplied(dst);
            }
            data = &data[whole * bpp..];
            self.advance(whole as u32);
        }
        self.extra += data.len();
    }

    fn advance(&mut self, n: u32) {
        self.x += n;
        if self.x == self.width {
            self.x = 0;
            self.y += 1;
        }
    }

    /// The data has ended.
    pub fn finish(&self) -> Result<Converted, ConvertError> {
        if self.done() {
            Ok(Converted {
                opaque: self.opaque,
                extra: self.extra,
            })
        } else {
            Err(ConvertError::Short {
                got: self.received(),
                want: self.expected_len(),
            })
        }
    }
}

/// One pixel; returns whether it is opaque.
#[inline(always)]
fn put(layout: Layout, src: &[u8], dst: &mut [u8]) -> bool {
    match layout {
        Layout::Rgb => {
            dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
            true
        }
        Layout::Rgba => {
            let a = src[3];
            dst.copy_from_slice(&[premultiply(src[2], a), premultiply(src[1], a), premultiply(src[0], a), a]);
            a == 255
        }
    }
}

/// RGB to BGRA, opaque.
pub fn rgb_row(src: &[u8], dst: &mut [u8]) {
    for (s, d) in src.as_chunks::<3>().0.iter().zip(dst.as_chunks_mut::<4>().0) {
        *d = [s[2], s[1], s[0], 255];
    }
}

/// Straight RGBA to premultiplied BGRA; returns whether every pixel was
/// opaque.
pub fn rgba_row(src: &[u8], dst: &mut [u8]) -> bool {
    let mut all = 255u8;
    for (s, d) in src.as_chunks::<4>().0.iter().zip(dst.as_chunks_mut::<4>().0) {
        let a = s[3];
        all &= a;
        *d = if a == 255 {
            [s[2], s[1], s[0], 255]
        } else {
            [premultiply(s[2], a), premultiply(s[1], a), premultiply(s[0], a), a]
        };
    }
    all == 255
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VecPixels;

    #[test]
    fn rgb_and_rgba() {
        let mut t = VecPixels::with_pitch(2, 2, 16);
        let mut c = Converter::new(Layout::Rgb, 2, 2);
        c.feed(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], &mut t);
        assert_eq!(c.finish(), Ok(Converted { opaque: true, extra: 0 }));
        assert_eq!(t.pixel(0, 0), [3, 2, 1, 255]);
        assert_eq!(t.pixel(1, 0), [6, 5, 4, 255]);
        assert_eq!(t.pixel(0, 1), [9, 8, 7, 255]);
        assert_eq!(t.pixel(1, 1), [12, 11, 10, 255]);

        let mut t = VecPixels::new(2, 1);
        let mut c = Converter::new(Layout::Rgba, 2, 1);
        c.feed(&[200, 100, 50, 255, 200, 100, 50, 128], &mut t);
        assert_eq!(c.finish(), Ok(Converted { opaque: false, extra: 0 }));
        assert_eq!(t.pixel(0, 0), [50, 100, 200, 255]);
        assert_eq!(t.pixel(1, 0), [25, 50, 100, 128]);
    }

    #[test]
    fn any_split_gives_the_same_pixels() {
        let (w, h) = (7u32, 5u32);
        for layout in [Layout::Rgb, Layout::Rgba] {
            let n = (w * h) as usize * layout.bytes_per_pixel();
            let data: Vec<u8> = (0..n as u32).map(|i| (i * 37 % 256) as u8).collect();
            let mut whole = VecPixels::with_pitch(w, h, 32);
            let mut c = Converter::new(layout, w, h);
            c.feed(&data, &mut whole);
            let want = c.finish().unwrap();
            for piece in 1..=9 {
                let mut t = VecPixels::with_pitch(w, h, 32);
                let mut c = Converter::new(layout, w, h);
                for p in data.chunks(piece) {
                    c.feed(p, &mut t);
                }
                assert_eq!(c.finish().unwrap(), want);
                assert_eq!(t, whole, "{layout:?} in pieces of {piece}");
            }
        }
    }

    #[test]
    fn display_p3() {
        let mut t = VecPixels::new(2, 1);
        let mut c = Converter::new(Layout::Rgb, 2, 1).display_p3();
        c.feed(&[255, 0, 0, 9], &mut t);
        c.feed(&[9, 9], &mut t);
        c.finish().unwrap();
        let [r, g, b] = ToP3::srgb().rgb([255, 0, 0]);
        assert_eq!(t.pixel(0, 0), [b, g, r, 255]);
        assert_eq!(t.pixel(1, 0), [9, 9, 9, 255]);
    }

    #[test]
    fn short_and_extra_data() {
        let mut t = VecPixels::new(2, 2);
        let mut c = Converter::new(Layout::Rgba, 2, 2);
        c.feed(&[0; 10], &mut t);
        assert_eq!(c.finish(), Err(ConvertError::Short { got: 10, want: 16 }));
        c.feed(&[0; 10], &mut t);
        assert_eq!(c.finish(), Ok(Converted { opaque: false, extra: 4 }));
        // A zero-sized image is done at once.
        assert!(Converter::new(Layout::Rgb, 0, 5).finish().is_ok());
    }
}
