//! A streaming Sixel decoder (images.md §8.1).
//!
//! Fed the bytes of `DCS P1 ; P2 ; P3 q <data> ST` after the `q`, in any
//! slices, as vte's DCS passthrough hands them over. Pixels are written as
//! premultiplied BGRA straight into a [`PixelTarget`] the caller allocates
//! (and counts against the image budget) through an [`AllocWith`]:
//!
//! - with raster attributes (`"1;1;W;H`, as chafa and img2sixel send) the
//!   target is allocated once, at that size;
//! - without, it grows by doubling each way as the data reaches past it, up
//!   to [`Limits`]; each growth asks for the whole new size, copies the old
//!   pixels and drops the old target;
//! - a refused allocation stops the decoder: the rest is skipped and the
//!   image is dropped.
//!
//! Pixels past the limits are clipped, and repeats are clamped to the
//! width limit, so no input makes the decoder loop or allocate past them.

use crate::{AllocWith, MAX_SIDE, PixelTarget};

/// Colour registers (cells §14). Higher numbers wrap, as in xterm.
pub const REGISTERS: usize = 1024;

/// The colour registers, as opaque BGRA. 4 KB.
#[derive(Clone, PartialEq, Eq)]
pub struct Palette(pub [[u8; 4]; REGISTERS]);

impl std::fmt::Debug for Palette {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0[..16].iter()).finish_non_exhaustive()
    }
}

/// The VT340's 16 colours (as xterm defines them, in percent), then black.
const VT340: [(u32, u32, u32); 16] = [
    (0, 0, 0),
    (20, 20, 80),
    (80, 13, 13),
    (20, 80, 20),
    (80, 20, 80),
    (20, 80, 80),
    (80, 80, 20),
    (53, 53, 53),
    (26, 26, 26),
    (33, 33, 60),
    (60, 26, 26),
    (33, 60, 33),
    (60, 33, 60),
    (33, 60, 60),
    (60, 60, 33),
    (80, 80, 80),
];

impl Palette {
    /// The initial palette.
    pub fn vt340() -> Box<Palette> {
        let mut p = Box::new(Palette([[0, 0, 0, 255]; REGISTERS]));
        for (i, &(r, g, b)) in VT340.iter().enumerate() {
            p.0[i] = bgra(percent(r), percent(g), percent(b));
        }
        p
    }
}

impl Default for Box<Palette> {
    fn default() -> Box<Palette> {
        Palette::vt340()
    }
}

fn bgra(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}

/// 0–100 to 0–255, rounded.
fn percent(p: u32) -> u8 {
    ((p.min(100) * 255 + 50) / 100) as u8
}

/// DEC's HLS (hue 0° is blue, 120° red, 240° green; lightness and
/// saturation 0–100) to RGB, as xterm converts it.
pub fn hls_to_rgb(h: u32, l: u32, s: u32) -> (u8, u8, u8) {
    let (l, s) = (l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    // The usual HSL, with DEC's hue turned by 240°.
    let h = ((h % 360) + 240) % 360;
    let hp = h as f32 / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match h / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (to(r), to(g), to(b))
}

/// The largest image the decoder makes; data past it is clipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_width: MAX_SIDE,
            max_height: MAX_SIDE,
        }
    }
}

/// The vertical size of a sixel pixel for the introducer's `P1` (the VT340
/// table): omitted, 0, 1, 5, 6 → 2:1; 2 → 5:1; 3, 4 → 3:1; 7–9 → 1:1.
pub fn aspect_for(p1: u16) -> u32 {
    match p1 {
        2 => 5,
        3 | 4 => 3,
        7..=9 => 1,
        _ => 2,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Repeat,
    Color,
    Raster,
}

/// A finished image: the top-left `width` × `height` of `pixels`.
#[derive(Debug)]
pub struct Sixel<T> {
    pub pixels: T,
    pub width: u32,
    pub height: u32,
}

/// What [`Decoder::finish`] gives back.
#[derive(Debug)]
pub struct Finished<T> {
    /// `None` if nothing was painted and no raster size given, or the
    /// decoder stopped on a refused allocation.
    pub image: Option<Sixel<T>>,
    /// The registers as the string left them, for terminals that carry
    /// them over between images (mode 1070 reset).
    pub palette: Box<Palette>,
}

/// The decoder for one Sixel string.
pub struct Decoder<T: PixelTarget = Box<dyn PixelTarget + Send>> {
    target: Option<T>,
    palette: Box<Palette>,
    color: [u8; 4],
    /// P2 ≠ 1: unset pixels take colour 0 at the end.
    fill_background: bool,
    /// Device pixels per sixel pixel, vertically.
    scale: u32,
    x: u32,
    band: u32,
    /// The painted extent.
    width: u32,
    height: u32,
    raster: (u32, u32),
    painted: bool,
    state: State,
    params: [u32; 5],
    nparams: usize,
    failed: bool,
    limits: Limits,
}

impl<T: PixelTarget> std::fmt::Debug for Decoder<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("size", &(self.width, self.height))
            .field("raster", &self.raster)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl<T: PixelTarget> Decoder<T> {
    /// A decoder for the introducer `DCS P1 ; P2 ; P3 q`. `palette` is the
    /// initial palette ([`Palette::vt340`], or the previous image's when
    /// registers carry over).
    pub fn new(p1: u16, p2: u16, palette: Box<Palette>, limits: Limits) -> Decoder<T> {
        Decoder {
            target: None,
            color: palette.0[0],
            palette,
            // P2 = 1: unset pixels stay transparent; otherwise they take
            // colour 0, as register 0 is at the end of the string.
            fill_background: p2 != 1,
            scale: aspect_for(p1),
            x: 0,
            band: 0,
            width: 0,
            height: 0,
            raster: (0, 0),
            painted: false,
            state: State::Ground,
            params: [0; 5],
            nparams: 0,
            failed: false,
            limits,
        }
    }

    /// True once an allocation was refused; the rest is skipped.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// The painted size so far (or the raster size if larger).
    pub fn size(&self) -> (u32, u32) {
        (self.width.max(self.raster.0), self.height.max(self.raster.1))
    }

    /// Decode `data`.
    pub fn put_slice(&mut self, data: &[u8], alloc: AllocWith<'_, T>) {
        let mut i = 0;
        while i < data.len() && !self.failed {
            let b = data[i];
            match self.state {
                State::Ground => {
                    if let b'?'..=b'~' = b {
                        let n = self.paint_run(&data[i..]);
                        if n > 0 {
                            i += n;
                            continue;
                        }
                    }
                    i += 1;
                    match b {
                        b'?'..=b'~' => self.paint(b - b'?', 1, alloc),
                        b'!' => self.start(State::Repeat),
                        b'#' => self.start(State::Color),
                        b'"' => self.start(State::Raster),
                        b'$' => self.x = 0,
                        b'-' => {
                            self.x = 0;
                            self.band = self.band.saturating_add(1);
                        }
                        _ => {}
                    }
                }
                _ => match b {
                    b'0'..=b'9' => {
                        i += 1;
                        let p = &mut self.params[self.nparams.max(1) - 1];
                        *p = p.saturating_mul(10).saturating_add((b - b'0') as u32);
                        self.nparams = self.nparams.max(1);
                    }
                    b';' => {
                        i += 1;
                        self.nparams = (self.nparams.max(1) + 1).min(self.params.len());
                    }
                    _ => {
                        // The command ends; the byte is handled in Ground
                        // (a repeat takes it as its sixel).
                        let state = std::mem::replace(&mut self.state, State::Ground);
                        match state {
                            State::Repeat => {
                                if let b'?'..=b'~' = b {
                                    i += 1;
                                    let n = self.params[0].max(1);
                                    self.paint(b - b'?', n, alloc);
                                }
                            }
                            State::Color => self.color_command(),
                            State::Raster => self.raster_command(alloc),
                            State::Ground => unreachable!(),
                        }
                    }
                },
            }
        }
    }

    fn start(&mut self, state: State) {
        self.state = state;
        self.params = [0; 5];
        self.nparams = 0;
    }

    fn color_command(&mut self) {
        let [pc, pu, px, py, pz] = self.params;
        let reg = pc as usize % REGISTERS;
        if self.nparams >= 2 {
            let rgb = match pu {
                1 => Some(hls_to_rgb(px, py, pz)),
                2 => Some((percent(px), percent(py), percent(pz))),
                _ => None,
            };
            if let Some((r, g, b)) = rgb {
                self.palette.0[reg] = bgra(r, g, b);
            }
        }
        self.color = self.palette.0[reg];
    }

    fn raster_command(&mut self, alloc: AllocWith<'_, T>) {
        // Only before the first pixel, as xterm.
        if self.painted {
            return;
        }
        let [pan, pad, ph, pv, _] = self.params;
        if self.nparams >= 2 {
            let pad = pad.max(1);
            // Rounded to a whole scale 1–5.
            self.scale = ((pan.min(1000) + pad / 2) / pad).clamp(1, 5);
        }
        if self.nparams >= 4 {
            self.raster = (ph.min(self.limits.max_width), pv.min(self.limits.max_height));
            let (w, h) = self.raster;
            if w > 0 && h > 0 && self.target.is_none() {
                self.allocate(w, h, alloc);
            }
        }
    }

    /// The fast path for a run of single sixels at 1:1 inside the target
    /// (most of what chafa and img2sixel send): paints them and returns how
    /// many bytes it used, or 0 if the slow path must handle the first.
    #[inline]
    fn paint_run(&mut self, data: &[u8]) -> usize {
        let Some(target) = self.target.as_mut() else { return 0 };
        let top = self.band.saturating_mul(6);
        if self.scale != 1 || top.saturating_add(6) > target.height() || self.x >= target.width() {
            return 0;
        }
        let room = (target.width() - self.x) as usize;
        let pitch = target.row_pitch();
        let color = u32::from_ne_bytes(self.color);
        let x0 = self.x as usize;
        let bytes = target.bytes_mut();
        let base = top as usize * pitch;
        let band = &mut bytes[base..base + 5 * pitch + (x0 + room) * 4];
        let mut n = 0;
        let mut any = 0u8;
        let mut last = 0;
        for &b in data.iter().take(room) {
            if !(b'?'..=b'~').contains(&b) {
                break;
            }
            let bits = b - b'?';
            any |= bits;
            if bits != 0 {
                last = n + 1;
            }
            let x = (x0 + n) * 4;
            // All six rows, choosing old or new without branches, since
            // the bits are as good as random.
            for row in 0..6 {
                let at = row * pitch + x;
                let px: &mut [u8; 4] = (&mut band[at..at + 4]).try_into().unwrap();
                let old = u32::from_ne_bytes(*px);
                let set = 0u32.wrapping_sub((bits as u32 >> row) & 1);
                *px = ((color & set) | (old & !set)).to_ne_bytes();
            }
            n += 1;
        }
        if any != 0 {
            self.painted = true;
            self.width = self.width.max((x0 + last) as u32);
            let highest = 7 - any.leading_zeros();
            self.height = self.height.max(top + highest + 1);
        }
        self.x += n as u32;
        n
    }

    /// Paint `bits` (bit 0 at the top) in `count` columns from the cursor.
    #[inline]
    fn paint(&mut self, bits: u8, count: u32, alloc: AllocWith<'_, T>) {
        let x = self.x;
        let count = count.min(self.limits.max_width);
        self.x = x.saturating_add(count);
        if bits == 0 {
            return;
        }
        self.painted = true;
        let (max_w, max_h) = (self.limits.max_width, self.limits.max_height);
        if x >= max_w {
            return;
        }
        let x_end = (x + count).min(max_w);
        let top = self.band.saturating_mul(6 * self.scale);
        if top >= max_h {
            return;
        }
        let highest = 7 - bits.leading_zeros(); // 0..=5
        let bottom = top.saturating_add((highest + 1) * self.scale).min(max_h);
        if !self.ensure(x_end, bottom, alloc) {
            return;
        }
        let target = self.target.as_mut().unwrap();
        let pitch = target.row_pitch();
        let band_fits = self.scale == 1 && top + 6 <= target.height();
        let bytes = target.bytes_mut();
        let color = self.color;
        let (x0, x1) = (x as usize * 4, x_end as usize * 4);
        if count == 1 && band_fits {
            // The common case (one sixel at 1:1, as chafa and img2sixel
            // send): all six rows, choosing old or new without branches,
            // since the bits are as good as random.
            let color = u32::from_ne_bytes(color);
            let base = top as usize * pitch + x0;
            let band = &mut bytes[base..base + 5 * pitch + 4];
            for row in 0..6 {
                let px: &mut [u8; 4] = (&mut band[row * pitch..row * pitch + 4]).try_into().unwrap();
                let old = u32::from_ne_bytes(*px);
                let set = 0u32.wrapping_sub((bits as u32 >> row) & 1);
                *px = ((color & set) | (old & !set)).to_ne_bytes();
            }
            self.width = self.width.max(x_end);
            self.height = self.height.max(bottom);
            return;
        }
        for bit in 0..=highest {
            if bits & (1 << bit) == 0 {
                continue;
            }
            let row0 = top + bit * self.scale;
            for row in row0..(row0 + self.scale).min(bottom) {
                let at = row as usize * pitch;
                let dst = &mut bytes[at + x0..at + x1];
                if count == 1 {
                    dst.copy_from_slice(&color);
                } else {
                    for px in dst.as_chunks_mut::<4>().0 {
                        *px = color;
                    }
                }
            }
        }
        self.width = self.width.max(x_end);
        self.height = self.height.max(bottom);
    }

    /// Make the target at least `w` × `h`; false if refused.
    #[inline]
    fn ensure(&mut self, w: u32, h: u32, alloc: AllocWith<'_, T>) -> bool {
        let (cw, ch) = match &self.target {
            Some(t) => (t.width(), t.height()),
            None => (0, 0),
        };
        if w <= cw && h <= ch {
            return true;
        }
        self.grow(cw, ch, w, h, alloc)
    }

    #[cold]
    fn grow(&mut self, cw: u32, ch: u32, w: u32, h: u32, alloc: AllocWith<'_, T>) -> bool {
        let next = |cur: u32, need: u32, max: u32| {
            if need <= cur {
                cur
            } else {
                need.max(cur.saturating_mul(2)).max(64).min(max)
            }
        };
        let nw = next(cw, w, self.limits.max_width);
        let nh = next(ch, h, self.limits.max_height);
        self.allocate(nw, nh, alloc)
    }

    /// Replace the target with a new one of `w` × `h`, keeping the pixels.
    fn allocate(&mut self, w: u32, h: u32, alloc: AllocWith<'_, T>) -> bool {
        let Some(mut new) = alloc(w, h) else {
            self.failed = true;
            self.target = None;
            return false;
        };
        if new.width() < w || new.height() < h {
            // A broken allocator: treat as refused.
            self.failed = true;
            self.target = None;
            return false;
        }
        let pitch = new.row_pitch();
        let (nw, nh) = (new.width() as usize, new.height() as usize);
        {
            // Transparent; painted pixels are always opaque, so at the end
            // alpha 0 means unset.
            let bytes = new.bytes_mut();
            for y in 0..nh {
                bytes[y * pitch..y * pitch + nw * 4].fill(0);
            }
        }
        if let Some(mut old) = self.target.take() {
            let (w, h) = (old.width(), old.height());
            copy_rect(&mut old, &mut new, w, h);
        }
        self.target = Some(new);
        true
    }

    /// The string ended with ST: the image, cropped to the area painted
    /// (or the raster size if larger). If the target is larger than that,
    /// it is copied into one of the exact size when `alloc` allows.
    pub fn finish(mut self, alloc: AllocWith<'_, T>) -> Finished<T> {
        // A command at the very end (`#1` then ST) still counts.
        match std::mem::replace(&mut self.state, State::Ground) {
            State::Color => self.color_command(),
            State::Raster if !self.failed => self.raster_command(alloc),
            _ => {}
        }
        let palette = std::mem::take(&mut self.palette);
        if self.failed {
            return Finished { image: None, palette };
        }
        let Some(mut target) = self.target.take() else {
            return Finished { image: None, palette };
        };
        let (w, h) = self.size();
        let (w, h) = (w.min(target.width()), h.min(target.height()));
        if (target.width(), target.height()) != (w, h)
            && let Some(mut exact) = alloc(w, h)
            && exact.width() >= w
            && exact.height() >= h
        {
            copy_rect(&mut target, &mut exact, w, h);
            target = exact;
        }
        if self.fill_background {
            let bg = palette.0[0];
            let pitch = target.row_pitch();
            let bytes = target.bytes_mut();
            for y in 0..h as usize {
                for px in bytes[y * pitch..y * pitch + w as usize * 4].as_chunks_mut::<4>().0 {
                    if px[3] == 0 {
                        *px = bg;
                    }
                }
            }
        }
        Finished {
            image: Some(Sixel {
                pixels: target,
                width: w,
                height: h,
            }),
            palette,
        }
    }

    /// The string was abandoned (CAN, SUB): the image is dropped. Returns
    /// the registers.
    pub fn abandon(mut self) -> Box<Palette> {
        std::mem::take(&mut self.palette)
    }
}

/// Copy the top-left `w` × `h` of `from` into `to`.
fn copy_rect(from: &mut impl PixelTarget, to: &mut impl PixelTarget, w: u32, h: u32) {
    let (fp, tp) = (from.row_pitch(), to.row_pitch());
    let row = w as usize * 4;
    let src = from.bytes_mut();
    let dst = to.bytes_mut();
    for y in 0..h as usize {
        dst[y * tp..y * tp + row].copy_from_slice(&src[y * fp..y * fp + row]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VecPixels;

    /// Decode `data` (after the `q`) into a `VecPixels`, with an allocator
    /// that allows `budget` bytes per allocation.
    pub(crate) fn decode_with(p1: u16, p2: u16, data: &[u8], budget: usize) -> Option<(VecPixels, u32, u32)> {
        let mut d = Decoder::<VecPixels>::new(p1, p2, Palette::vt340(), Limits::default());
        let mut alloc = |w: u32, h: u32| (w as usize * h as usize * 4 <= budget).then(|| VecPixels::new(w, h));
        d.put_slice(data, &mut alloc);
        let f = d.finish(&mut alloc);
        f.image.map(|s| (s.pixels, s.width, s.height))
    }

    fn decode(p1: u16, p2: u16, data: &[u8]) -> (VecPixels, u32, u32) {
        decode_with(p1, p2, data, 1 << 30).expect("an image")
    }

    const RED: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [255, 0, 0, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];

    #[test]
    fn one_column_one_colour() {
        // `~` is all six pixels; aspect 1:1 from the raster attributes.
        let (p, w, h) = decode(0, 1, b"\"1;1#1;2;100;0;0#1~");
        assert_eq!((w, h), (1, 6));
        for y in 0..6 {
            assert_eq!(p.pixel(0, y), RED);
        }
    }

    #[test]
    fn default_aspect_is_two_to_one() {
        // P1 omitted and no raster attributes: each sixel pixel is two tall.
        let (p, w, h) = decode(0, 1, b"#1;2;100;0;0#1A");
        // `A` = 0b000010: the second sixel pixel, rows 2 and 3.
        assert_eq!((w, h), (1, 4));
        assert_eq!(p.pixel(0, 0), CLEAR);
        assert_eq!(p.pixel(0, 1), CLEAR);
        assert_eq!(p.pixel(0, 2), RED);
        assert_eq!(p.pixel(0, 3), RED);
        for (p1, scale) in [
            (0, 2),
            (1, 2),
            (2, 5),
            (3, 3),
            (4, 3),
            (5, 2),
            (6, 2),
            (7, 1),
            (8, 1),
            (9, 1),
            (42, 2),
        ] {
            let (_, _, h) = decode(p1, 1, b"#1~");
            assert_eq!(h, 6 * scale, "P1 = {p1}");
        }
    }

    #[test]
    fn repeats_and_newlines() {
        // Two bands: red 3 wide, then `$` back and green over its first
        // pixel row; `-` to the next band, blue 2 wide.
        let (p, w, h) = decode(0, 1, b"\"1;1#1;2;100;0;0#2;2;0;100;0#3;2;0;0;100#1!3~$#2@-#3!2~");
        assert_eq!((w, h), (3, 12));
        assert_eq!(p.pixel(0, 0), GREEN);
        assert_eq!(p.pixel(1, 0), RED);
        assert_eq!(p.pixel(2, 5), RED);
        assert_eq!(p.pixel(0, 6), BLUE);
        assert_eq!(p.pixel(1, 11), BLUE);
        assert_eq!(p.pixel(2, 6), CLEAR);
        // `!0` is one, as in xterm; a repeat with no sixel after it is dropped.
        let (_, w, _) = decode(7, 1, b"#1!0~");
        assert_eq!(w, 1);
        let (_, w, _) = decode(7, 1, b"#1!5$~");
        assert_eq!(w, 1);
    }

    #[test]
    fn hls_hue_zero_is_blue() {
        assert_eq!(hls_to_rgb(0, 50, 100), (0, 0, 255));
        assert_eq!(hls_to_rgb(120, 50, 100), (255, 0, 0));
        assert_eq!(hls_to_rgb(240, 50, 100), (0, 255, 0));
        assert_eq!(hls_to_rgb(360, 50, 100), (0, 0, 255));
        assert_eq!(hls_to_rgb(0, 100, 100), (255, 255, 255));
        assert_eq!(hls_to_rgb(77, 0, 50), (0, 0, 0));
        assert_eq!(hls_to_rgb(77, 50, 0), (128, 128, 128));
        let (p, _, _) = decode(7, 1, b"#5;1;0;50;100#5~");
        assert_eq!(p.pixel(0, 0), BLUE);
    }

    #[test]
    fn rgb_percentages() {
        let (p, _, _) = decode(7, 1, b"#9;2;100;50;0~");
        assert_eq!(p.pixel(0, 0), [0, 128, 255, 255]);
        // Over 100 is 100.
        let (p, _, _) = decode(7, 1, b"#9;2;250;0;0~");
        assert_eq!(p.pixel(0, 0), RED);
    }

    #[test]
    fn transparent_or_background() {
        // P2 = 1: unset pixels stay transparent.
        // `D`: sixel pixels 0 and 2.
        let (p, _, _) = decode(7, 1, b"#1;2;100;0;0#1D");
        assert_eq!(p.pixel(0, 0), RED);
        assert_eq!(p.pixel(0, 1), CLEAR);
        // P2 = 0 (or 2): they take colour 0 (black in the VT340 palette).
        for p2 in [0, 2] {
            let (p, _, _) = decode(7, p2, b"#1;2;100;0;0#1D");
            assert_eq!(p.pixel(0, 0), RED);
            assert_eq!(p.pixel(0, 1), [0, 0, 0, 255]);
        }
    }

    #[test]
    fn raster_attributes() {
        // The size, even where nothing is painted.
        let (p, w, h) = decode(0, 1, b"\"1;1;10;20#1;2;100;0;0#1@");
        assert_eq!((w, h), (10, 20));
        assert_eq!((p.width(), p.height()), (10, 20));
        assert_eq!(p.pixel(9, 19), CLEAR);
        // Pan/Pad rounded to a whole scale; Pad = 0 taken as 1.
        for (attrs, scale) in [
            (&b"\"2;1"[..], 2),
            (b"\"5;2", 3),
            (b"\"9;1", 5),
            (b"\"3;0", 3),
            (b"\"0;1", 1),
            (b"\"1;3", 1),
        ] {
            let mut data = attrs.to_vec();
            data.extend_from_slice(b"#1~");
            let (_, _, h) = decode(0, 1, &data);
            assert_eq!(h, 6 * scale, "{:?}", attrs.escape_ascii());
        }
        // After the first pixel they are ignored.
        let (_, w, h) = decode(7, 1, b"#1~\"1;1;50;50");
        assert_eq!((w, h), (1, 6));
        // Painting past the raster size grows the image.
        let (p, w, h) = decode(0, 1, b"\"1;1;2;2#1;2;100;0;0#1!4~");
        assert_eq!((w, h), (4, 6));
        assert_eq!(p.pixel(3, 5), RED);
    }

    #[test]
    fn growth_without_raster_attributes() {
        // 300 columns and five bands: past the first 64 × 64 both ways.
        let mut data = b"#1;2;100;0;0#1".to_vec();
        for _ in 0..15 {
            data.extend_from_slice(b"!300~-");
        }
        data.extend_from_slice(b"#2;2;0;100;0!299?~");
        let (p, w, h) = decode(7, 1, &data);
        assert_eq!((w, h), (300, 96));
        assert_eq!((p.width(), p.height()), (300, 96), "cropped to the painted area");
        assert_eq!(p.pixel(0, 0), RED);
        assert_eq!(p.pixel(299, 89), RED);
        assert_eq!(p.pixel(299, 90), GREEN);
        assert_eq!(p.pixel(298, 90), CLEAR);
    }

    #[test]
    fn redefining_a_register_doesnt_recolour() {
        let (p, _, _) = decode(7, 1, b"#1;2;100;0;0#1~#1;2;0;0;100~");
        assert_eq!(p.pixel(0, 0), RED);
        assert_eq!(p.pixel(1, 0), BLUE);
        // Registers wrap modulo 1024.
        let (p, _, _) = decode(7, 1, b"#1025;2;0;100;0#1~");
        assert_eq!(p.pixel(0, 0), GREEN);
    }

    #[test]
    fn initial_palette() {
        let pal = Palette::vt340();
        assert_eq!(pal.0[0], [0, 0, 0, 255]);
        assert_eq!(pal.0[1], [204, 51, 51, 255]); // blue: 20, 20, 80
        assert_eq!(pal.0[2], [33, 33, 204, 255]); // red: 80, 13, 13
        assert_eq!(pal.0[15], [204, 204, 204, 255]);
        assert_eq!(pal.0[16], [0, 0, 0, 255]);
        // Selecting a register without defining it uses the palette.
        let (p, _, _) = decode(7, 1, b"#2~");
        assert_eq!(p.pixel(0, 0), pal.0[2]);
    }

    #[test]
    fn palette_carries_over() {
        let mut d = Decoder::<VecPixels>::new(7, 1, Palette::vt340(), Limits::default());
        let mut alloc = |w, h| Some(VecPixels::new(w, h));
        d.put_slice(b"#3;2;1;2;3~", &mut alloc);
        let f = d.finish(&mut alloc);
        assert_eq!(f.palette.0[3], [8, 5, 3, 255]);
        let d = Decoder::<VecPixels>::new(7, 1, f.palette, Limits::default());
        assert_eq!(d.abandon().0[3], [8, 5, 3, 255]);
    }

    #[test]
    fn any_split_gives_the_same_image() {
        let data = b"\"1;1;8;12#0;2;0;0;0#1;2;100;20;0#12;1;120;50;70#1!7~$#12A?_-#1!3N!2]";
        let (whole, w, h) = decode(0, 1, data);
        for piece in 1..data.len() {
            let mut d = Decoder::<VecPixels>::new(0, 1, Palette::vt340(), Limits::default());
            let mut alloc = |w, h| Some(VecPixels::new(w, h));
            for p in data.chunks(piece) {
                d.put_slice(p, &mut alloc);
            }
            let s = d.finish(&mut alloc).image.unwrap();
            assert_eq!((s.width, s.height), (w, h));
            assert_eq!(s.pixels, whole, "pieces of {piece}");
        }
    }

    #[test]
    fn caps() {
        // A huge raster with little data: refused at once, nothing allocated.
        let mut sizes = Vec::new();
        let mut d = Decoder::<VecPixels>::new(0, 1, Palette::vt340(), Limits::default());
        let mut alloc = |w: u32, h: u32| {
            sizes.push((w, h));
            (w as usize * h as usize * 4 <= 1 << 20).then(|| VecPixels::new(w, h))
        };
        d.put_slice(b"\"1;1;10000;10000#1~~~", &mut alloc);
        assert!(d.failed());
        assert!(d.finish(&mut alloc).image.is_none());
        assert_eq!(sizes, vec![(10_000, 10_000)]);

        // Raster sizes past the limits are clamped to them.
        let limits = Limits {
            max_width: 100,
            max_height: 50,
        };
        let mut d = Decoder::<VecPixels>::new(0, 1, Palette::vt340(), limits);
        let mut alloc = |w, h| Some(VecPixels::new(w, h));
        d.put_slice(b"\"1;1;99999;99999#1", &mut alloc);
        assert_eq!(d.size(), (100, 50));

        // Repeats are clamped and pixels past the limits clipped.
        let mut d = Decoder::<VecPixels>::new(7, 1, Palette::vt340(), limits);
        d.put_slice(b"#1!4000000000~", &mut alloc);
        for _ in 0..20 {
            d.put_slice(b"-~", &mut alloc);
        }
        d.put_slice(b"$!99~", &mut alloc);
        let s = d.finish(&mut alloc).image.unwrap();
        assert_eq!((s.width, s.height), (100, 50));

        // Growth refused part way: the image is dropped.
        // (The first allocation is 100 × 64; the twelfth band needs 128 rows.)
        let bands = |n: usize| [&b"#1"[..], &b"!100~-".repeat(n)].concat();
        assert!(decode_with(7, 1, &bands(10), 100 * 64 * 4).is_some());
        assert!(decode_with(7, 1, &bands(12), 100 * 64 * 4).is_none());
    }

    #[test]
    fn nothing_painted() {
        assert!(decode_with(0, 1, b"", 1 << 20).is_none());
        assert!(decode_with(0, 1, b"#1;2;1;2;3$-???", 1 << 20).is_none());
        // Garbage is skipped.
        let (_, w, h) = decode(7, 1, b"\x00\x01 hello\r\n\xff#1~");
        assert!(w >= 1 && h == 6);
    }

    #[test]
    fn a_command_at_the_end() {
        // A colour command cut off by ST still defines the register.
        let mut d = Decoder::<VecPixels>::new(7, 1, Palette::vt340(), Limits::default());
        let mut alloc = |w, h| Some(VecPixels::new(w, h));
        d.put_slice(b"#1~#4;2;100;100;100", &mut alloc);
        assert_eq!(d.finish(&mut alloc).palette.0[4], [255, 255, 255, 255]);
    }
}
