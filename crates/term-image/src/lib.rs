//! The parts of terminal images that need no terminal state and no
//! platform code (docs/design/images.md):
//!
//! - [`kitty`]: kitty graphics control keys, parsed into a [`kitty::Command`];
//! - [`base64`]: a streaming base64 decoder (kitty payloads, iTerm2 files);
//! - [`inflate`]: zlib inflate with a cap on the output (kitty `o=z`);
//! - [`probe`]: format sniffing and sizes of PNG, JPEG (with EXIF
//!   orientation) and GIF from their headers;
//! - [`iterm`]: the arguments of iTerm2's `OSC 1337;File=`;
//! - [`sixel`]: a streaming Sixel decoder;
//! - [`convert`]: raw RGB / RGBA pixels to premultiplied BGRA.
//!
//! Pixels are written into a [`PixelTarget`] that someone else allocates
//! (in the terminal, a Metal buffer counted against the image budget), so
//! nothing here allocates image-sized memory of its own except where it
//! says so.

pub mod base64;
pub mod convert;
pub mod inflate;
pub mod iterm;
pub mod kitty;
pub mod probe;
pub mod sixel;

/// Where decoded pixels go: premultiplied BGRA8, `height` rows of
/// `row_pitch` bytes (at least `width * 4`; the bytes past `width * 4` in a
/// row are padding and are never read).
pub trait PixelTarget {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn row_pitch(&self) -> usize;
    /// At least `row_pitch * (height - 1) + width * 4` bytes.
    fn bytes_mut(&mut self) -> &mut [u8];
}

impl<T: PixelTarget + ?Sized> PixelTarget for Box<T> {
    fn width(&self) -> u32 {
        (**self).width()
    }
    fn height(&self) -> u32 {
        (**self).height()
    }
    fn row_pitch(&self) -> usize {
        (**self).row_pitch()
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        (**self).bytes_mut()
    }
}

/// How a decoder asks for (more) room, as `(width, height)`. The caller
/// reserves the bytes from the image budget before allocating, and answers
/// `None` when they don't fit. Decoders are generic over the target type,
/// so a caller can get its own buffer type back; [`Alloc`] is the boxed form.
pub type AllocWith<'a, T> = &'a mut dyn FnMut(u32, u32) -> Option<T>;

/// [`AllocWith`] for boxed targets.
pub type Alloc<'a> = AllocWith<'a, Box<dyn PixelTarget + Send>>;

/// The largest width or height of any image (cells §14).
pub const MAX_SIDE: u32 = 10_000;

/// A [`PixelTarget`] in a `Vec`, for tests, benches and hosts without a GPU.
/// Starts transparent (all zero).
#[derive(Clone, PartialEq, Eq)]
pub struct VecPixels {
    width: u32,
    height: u32,
    pitch: usize,
    bytes: Vec<u8>,
}

impl VecPixels {
    pub fn new(width: u32, height: u32) -> VecPixels {
        VecPixels::with_pitch(width, height, width as usize * 4)
    }

    /// With a row pitch of `pitch` bytes (at least `width * 4`).
    pub fn with_pitch(width: u32, height: u32, pitch: usize) -> VecPixels {
        assert!(pitch >= width as usize * 4);
        VecPixels {
            width,
            height,
            pitch,
            bytes: vec![0; pitch * height as usize],
        }
    }

    /// The pixel at (`x`, `y`) as premultiplied `[b, g, r, a]`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = y as usize * self.pitch + x as usize * 4;
        self.bytes[i..i + 4].try_into().unwrap()
    }

    /// The pixels as straight (not premultiplied) RGBA, row after row with
    /// no padding: what a PNG encoder wants.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for y in 0..self.height {
            for x in 0..self.width {
                let [b, g, r, a] = self.pixel(x, y);
                let un = |c: u8| {
                    if a == 0 {
                        0
                    } else {
                        ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
                    }
                };
                out.extend_from_slice(&[un(r), un(g), un(b), a]);
            }
        }
        out
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl std::fmt::Debug for VecPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VecPixels({}x{})", self.width, self.height)
    }
}

impl PixelTarget for VecPixels {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn row_pitch(&self) -> usize {
        self.pitch
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

/// `c * a / 255`, rounded to nearest, exact for all `c`, `a` in 0..=255.
#[inline(always)]
pub fn premultiply(c: u8, a: u8) -> u8 {
    let t = c as u32 * a as u32 + 128;
    ((t + (t >> 8)) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiply_is_exact() {
        for c in 0..=255u32 {
            for a in 0..=255u32 {
                let want = ((c * a) as f64 / 255.0).round() as u8;
                assert_eq!(premultiply(c as u8, a as u8), want, "{c} {a}");
            }
        }
    }

    #[test]
    fn vec_pixels_round_trip_to_rgba() {
        let mut p = VecPixels::with_pitch(2, 1, 12);
        p.bytes_mut()[..8].copy_from_slice(&[10, 20, 30, 255, 0, 0, 64, 128]);
        assert_eq!(p.to_rgba(), vec![30, 20, 10, 255, 128, 0, 0, 128]);
    }
}
