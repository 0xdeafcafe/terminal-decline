//! Decline's own images (images.md §7, §8): iTerm2's inline files
//! (`OSC 1337 ; File=` and `MultipartFile=`) and Sixel (`DCS … q`), shown
//! through placeholder cells naming an own image (cells §8.3).
//!
//! - **iTerm2**: a `File=` over 4 KB streams through `osc_put`
//!   ([`Terminal::iterm_osc_large`]); a small one arrives whole through
//!   `osc_dispatch`, its parameters joined back with `;`. The base64 after
//!   the `:` goes into an encoded buffer counted against the budget (a
//!   quarter of it at most, and refused at once when `size=` says it is
//!   larger). At the end the image is sniffed, its cell box worked out
//!   (image pixels count as points, as in iTerm2), and it is decoded at
//!   the size it is drawn, never larger than its own: a phone photo shown
//!   at window size takes a few MB. `MultipartFile=` starts the same
//!   reader, each `FilePart=` appends, `FileEnd` finishes.
//! - **Sixel**: the string streams into term-image's decoder, which
//!   allocates through the host with every growth reserved from the pool
//!   first. At ST the image is ready at once (no job). Mode 80 (DECSDM)
//!   puts it at the top left without scrolling; 8452 leaves the cursor
//!   right of it; 1070 (set by default) gives each image fresh colour
//!   registers, reset carries them over between images of a screen.
//! - **Freeing**: an own image lives while a style of its screen's table
//!   or a history block names it (§4.3). Rather than counting, the
//!   terminal looks again after a style sweep or when blocks are freed
//!   ([`Terminal::collect_own_images`]).
//!
//! Refused images are silent: iTerm2 has no replies, and Sixel none for
//! images.

use std::collections::HashSet;
use std::sync::Arc;

use term_image::base64::Base64;
use term_image::decode::fit_size;
use term_image::iterm::{Args, Dimension};
use term_image::sixel::{Decoder, Limits, Palette};
use term_image::{MAX_SIDE, probe};
use vte::{OscLarge, Params};

use super::Terminal;
use crate::cell::Color;
use crate::image::{Encoded, Fit, Image, ImageJob, ImageKey, ImagePool, ImageState, JobId, PAGE, PixelBuffer, PoolId, VirtualPlacement};
use crate::style::{Attrs, Style};

/// The most rows or columns of placeholder cells an image gets (a cell
/// stores 9 bits of each).
const MAX_CELLS: u16 = 512;

/// Most bytes of iTerm2 arguments before the `:`.
const MAX_HEAD: usize = 16 * 1024;

fn page_round(bytes: usize) -> usize {
    bytes.div_ceil(PAGE).max(1) * PAGE
}

/// What a streamed OSC string's bytes go to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum OscStream {
    #[default]
    None,
    /// The `File=` reader, arguments first.
    File,
    /// A `FilePart=` of the multipart file.
    Part,
}

/// An iTerm2 file being received.
pub(super) struct ItermReader {
    /// The arguments so far, until the `:` (`File=` only).
    head: Option<Vec<u8>>,
    args: Args,
    base64: Base64,
    store: ItermStore,
}

/// The encoded bytes and their accounting, apart from the base64 decoder.
struct ItermStore {
    encoded: Encoded,
    /// The pool entry they count in, released on drop unless taken.
    id: Option<PoolId>,
    pool: Arc<ImagePool>,
    owner: u64,
    counted: usize,
    cap: usize,
    failed: bool,
}

impl Drop for ItermStore {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.pool.release(id);
        }
    }
}

impl ItermStore {
    fn put(&mut self, bytes: &[u8]) {
        if self.failed {
            return;
        }
        let Some(id) = self.id else {
            self.failed = true;
            return;
        };
        if self.encoded.len() + bytes.len() > self.cap {
            self.failed = true;
            return;
        }
        self.encoded.push(bytes);
        let need = self.encoded.len().div_ceil(Encoded::PIECE) * Encoded::PIECE;
        if need > self.counted {
            if self.pool.grow(id, need - self.counted).is_err() {
                self.failed = true;
                return;
            }
            self.counted = need;
        }
    }
}

impl ItermReader {
    fn new(pool: Arc<ImagePool>, owner: u64, multipart_args: Option<&[u8]>) -> ItermReader {
        let cap = pool.limit() / 4;
        let mut r = ItermReader {
            head: Some(Vec::new()),
            args: Args::default(),
            base64: Base64::new(),
            store: ItermStore {
                encoded: Encoded::new(),
                id: None,
                pool,
                owner,
                counted: 0,
                cap,
                failed: false,
            },
        };
        if let Some(args) = multipart_args {
            r.head = None;
            r.begin(Args::parse(args));
        }
        r
    }

    /// The arguments are in: refuse downloads and files said to be too big.
    fn begin(&mut self, args: Args) {
        let s = &mut self.store;
        if !args.inline || args.size.is_some_and(|n| n > s.cap as u64) {
            s.failed = true;
        } else {
            match s.pool.reserve(s.owner, 0, None) {
                Ok(id) => s.id = Some(id),
                Err(_) => s.failed = true,
            }
        }
        self.args = args;
    }

    /// Arguments (until the `:`), then base64.
    fn feed(&mut self, mut data: &[u8]) {
        if self.store.failed {
            return;
        }
        if let Some(head) = &mut self.head {
            match memchr::memchr(b':', data) {
                None => {
                    if head.len() + data.len() > MAX_HEAD {
                        self.store.failed = true;
                    } else {
                        head.extend_from_slice(data);
                    }
                    return;
                }
                Some(at) => {
                    head.extend_from_slice(&data[..at]);
                    data = &data[at + 1..];
                    let args = Args::parse(head);
                    self.head = None;
                    self.begin(args);
                }
            }
        }
        let store = &mut self.store;
        if self.base64.feed_to(data, &mut |b| store.put(b)).is_err() {
            store.failed = true;
        }
    }

    /// The end: the arguments, the encoded bytes and their pool entry, or
    /// None if it was refused.
    fn finish(mut self) -> Option<(Args, Encoded, PoolId)> {
        if self.head.is_some() {
            return None;
        }
        let store = &mut self.store;
        if self.base64.finish_to(&mut |b| store.put(b)).is_err() || store.failed || store.encoded.is_empty() {
            return None;
        }
        let id = store.id.take()?;
        let encoded = std::mem::take(&mut store.encoded);
        Some((std::mem::take(&mut self.args), encoded, id))
    }
}

/// A Sixel string being decoded.
pub(super) struct SixelString {
    decoder: Option<Decoder<Box<dyn PixelBuffer>>>,
    id: Option<PoolId>,
    pool: Arc<ImagePool>,
}

impl Drop for SixelString {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.pool.release(id);
        }
    }
}

/// Where the cursor goes after an own image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum After {
    /// iTerm2: one column past the bottom-right corner, or back where it
    /// started (`doNotMoveCursor=1`).
    Iterm { stay: bool },
    /// Sixel scrolling: the row after the image, at its left column.
    SixelBelow,
    /// Sixel with mode 8452: right of the image on its last row.
    SixelRight,
    /// Sixel display mode (80 set): at the top left, the cursor stays.
    SixelFixed,
}

impl Terminal {
    /// `osc_large` with images on: stream iTerm2 files into the reader.
    /// None: the usual size policy applies.
    pub(super) fn iterm_osc_large(&mut self, prefix: &[u8]) -> Option<OscLarge> {
        if !self.images_enabled() {
            return None;
        }
        if let Some(rest) = prefix.strip_prefix(b"1337;File=") {
            let mut r = Box::new(ItermReader::new(self.graphics.pool().clone(), self.terminal_id, None));
            r.feed(rest);
            self.iterm_file = Some(r);
            self.osc_stream = OscStream::File;
            return Some(OscLarge::Stream);
        }
        if let Some(rest) = prefix.strip_prefix(b"1337;FilePart=") {
            let Some(r) = &mut self.iterm_multi else {
                return Some(OscLarge::Drop);
            };
            r.feed(rest);
            self.osc_stream = OscStream::Part;
            return Some(OscLarge::Stream);
        }
        None
    }

    pub(super) fn iterm_osc_put(&mut self, data: &[u8]) {
        match self.osc_stream {
            OscStream::File => {
                if let Some(r) = &mut self.iterm_file {
                    r.feed(data);
                }
            }
            OscStream::Part => {
                if let Some(r) = &mut self.iterm_multi {
                    r.feed(data);
                }
            }
            OscStream::None => {}
        }
    }

    pub(super) fn iterm_osc_end(&mut self, complete: bool) {
        match std::mem::take(&mut self.osc_stream) {
            OscStream::File => {
                if let Some(r) = self.iterm_file.take()
                    && complete
                {
                    self.iterm_finish(*r);
                }
            }
            OscStream::Part => {
                if !complete {
                    // An abandoned part loses the file.
                    self.iterm_multi = None;
                }
            }
            OscStream::None => {}
        }
    }

    /// `OSC 1337 ; …` arriving whole. True if it was an iTerm2 file.
    pub(super) fn iterm_osc_dispatch(&mut self, params: &[&[u8]]) -> bool {
        let [b"1337", first, rest @ ..] = params else { return false };
        if !self.images_enabled() {
            return first.starts_with(b"File") || first.starts_with(b"MultipartFile");
        }
        // vte keeps 16 parts: a 16th may have been cut short.
        let truncated = params.len() >= 16;
        let joined = || {
            let mut s = first.to_vec();
            for p in rest {
                s.push(b';');
                s.extend_from_slice(p);
            }
            s
        };
        if first.starts_with(b"File=") {
            if !truncated {
                let all = joined();
                let mut r = ItermReader::new(self.graphics.pool().clone(), self.terminal_id, None);
                r.feed(&all[5..]);
                self.iterm_finish(r);
            }
        } else if first.starts_with(b"MultipartFile=") {
            // Another one abandons the first.
            self.iterm_multi = None;
            if !truncated {
                let all = joined();
                let r = ItermReader::new(self.graphics.pool().clone(), self.terminal_id, Some(&all[14..]));
                self.iterm_multi = Some(Box::new(r));
            }
        } else if let Some(data) = first.strip_prefix(b"FilePart=") {
            if let Some(r) = &mut self.iterm_multi {
                r.feed(data);
            }
        } else if *first == b"FileEnd" {
            if let Some(r) = self.iterm_multi.take() {
                self.iterm_finish(*r);
            }
        } else {
            return false;
        }
        true
    }

    /// A whole file is in: size it, place it and queue its decode (§7.2).
    fn iterm_finish(&mut self, r: ItermReader) {
        let pool = self.graphics.pool().clone();
        let Some((args, encoded, id)) = r.finish() else { return };
        let head = encoded.pieces().next().unwrap_or(&[]);
        let p = match probe::probe(head) {
            Ok(p) => Ok(p),
            Err(probe::ProbeError::NeedMore) => probe::probe(&encoded.to_vec()),
            Err(e) => Err(e),
        };
        let Ok(p) = p else {
            pool.release(id);
            return;
        };
        let Some(layout) = self.iterm_layout(&args, (p.width, p.height)) else {
            pool.release(id);
            return;
        };
        let (dw, dh) = layout.decoded;
        if dw > MAX_SIDE || dh > MAX_SIDE || pool.grow(id, page_round(dw as usize * dh as usize * 4)).is_err() {
            // Over the budget even at the size it is drawn.
            pool.release(id);
            return;
        }
        self.collect_own_images();
        let key = self.new_own_key();
        let mut image = Image::new(key, dw, dh);
        image.pool = Some(id);
        let generation = image.generation;
        self.image_jobs.push(ImageJob::Decode {
            job: JobId::next(),
            image: key,
            generation,
            bytes: Arc::new(encoded),
            size: (dw, dh),
        });
        self.place_own(
            image,
            layout.cols,
            layout.rows,
            layout.fit,
            After::Iterm {
                stay: args.do_not_move_cursor,
            },
        );
    }

    /// The cell box, how the image fills it and the size to decode it at,
    /// for an image of `(iw, ih)` pixels (§7.2).
    fn iterm_layout(&self, args: &Args, (iw, ih): (u32, u32)) -> Option<ItermLayout> {
        if iw == 0 || ih == 0 {
            return None;
        }
        let (cw, ch) = (self.cell_pixels.0.max(1) as f64, self.cell_pixels.1.max(1) as f64);
        let (sw, sh) = (self.cols as f64 * cw, self.lines as f64 * ch);
        let scale = self.image_scale.max(0.1) as f64;
        let (iwf, ihf) = (iw as f64, ih as f64);
        let want = |d: Dimension, cell: f64, screen: f64| match d {
            Dimension::Auto | Dimension::Cells(0) | Dimension::Pixels(0) | Dimension::Percent(0) => None,
            Dimension::Cells(n) => Some(n as f64 * cell),
            Dimension::Pixels(n) => Some(n as f64 * scale),
            Dimension::Percent(p) => Some(screen * p.min(100) as f64 / 100.0),
        };
        let (mut bw, mut bh) = match (want(args.width, cw, sw), want(args.height, ch, sh)) {
            (None, None) => (iwf * scale, ihf * scale),
            (Some(w), None) => (w, w * ihf / iwf),
            (None, Some(h)) => (h * iwf / ihf, h),
            (Some(w), Some(h)) => (w, h),
        };
        // No wider than the screen (iTerm2 scales `auto` images down).
        if bw > sw {
            if args.preserve_aspect_ratio || matches!(args.height, Dimension::Auto) {
                bh *= sw / bw;
            }
            bw = sw;
        }
        let cells = |px: f64, cell: f64| ((px / cell).ceil() as u64).clamp(1, MAX_CELLS as u64) as u16;
        let (cols, rows) = (cells(bw, cw), cells(bh, ch));
        // At most the box the cells hold.
        let (bw, bh) = (bw.min(cols as f64 * cw), bh.min(rows as f64 * ch));
        let (decoded, fit) = if args.preserve_aspect_ratio {
            let s = (bw / iwf).min(bh / ihf);
            let drawn = ((iwf * s).ceil().max(1.0) as u32, (ihf * s).ceil().max(1.0) as u32);
            (fit_size((iw, ih), drawn), Fit::ContainTopLeft)
        } else {
            ((iw.min(bw.ceil().max(1.0) as u32), ih.min(bh.ceil().max(1.0) as u32)), Fit::Stretch)
        };
        let decoded = fit_size(decoded, (MAX_SIDE, MAX_SIDE));
        Some(ItermLayout { cols, rows, fit, decoded })
    }

    /// `DCS P1 ; P2 ; P3 q`: a Sixel image starts.
    pub(super) fn sixel_start(&mut self, params: &Params) {
        let mut it = params.iter();
        let p1 = it.next().map_or(0, |p| p[0]);
        let p2 = it.next().map_or(0, |p| p[0]);
        let pool = self.graphics.pool().clone();
        let id = pool.reserve(self.terminal_id, 0, None).ok();
        let screen = self.alt_active as usize;
        let palette = if self.modes.sixel_private_colors {
            Palette::vt340()
        } else {
            self.sixel_palettes[screen].take().unwrap_or_else(Palette::vt340)
        };
        let limits = if self.modes.sixel_display {
            // Display mode: clipped at the screen's edges.
            Limits {
                max_width: (self.cols as u32 * self.cell_pixels.0.max(1) as u32).min(MAX_SIDE),
                max_height: (self.lines as u32 * self.cell_pixels.1.max(1) as u32).min(MAX_SIDE),
            }
        } else {
            Limits {
                max_width: (self.cols as u32 * self.cell_pixels.0.max(1) as u32).min(MAX_SIDE),
                max_height: MAX_SIDE,
            }
        };
        let decoder = Decoder::new(p1, p2, palette, limits).display_p3();
        self.sixel = Some(Box::new(SixelString {
            decoder: Some(decoder),
            id,
            pool,
        }));
    }

    pub(super) fn sixel_put(&mut self, data: &[u8]) {
        let host = self.image_host.clone();
        let Some(s) = self.sixel.as_deref_mut() else { return };
        let (Some(d), Some(id)) = (&mut s.decoder, s.id) else { return };
        let pool = &s.pool;
        d.put_slice(data, &mut |w, h| {
            // The whole new size, before the old buffer goes.
            pool.grow(id, page_round(w as usize * h as usize * 4)).ok()?;
            host.alloc(w, h)
        });
    }

    pub(super) fn sixel_end(&mut self, complete: bool) {
        let Some(mut s) = self.sixel.take() else { return };
        let Some(decoder) = s.decoder.take() else { return };
        let screen = self.alt_active as usize;
        let keep = !self.modes.sixel_private_colors;
        let Some(id) = s.id else {
            if keep {
                self.sixel_palettes[screen] = Some(decoder.abandon());
            }
            return;
        };
        if !complete {
            let palette = decoder.abandon();
            if keep {
                self.sixel_palettes[screen] = Some(palette);
            }
            return;
        }
        let host = self.image_host.clone();
        let pool = s.pool.clone();
        let done = decoder.finish(&mut |w, h| {
            pool.grow(id, page_round(w as usize * h as usize * 4)).ok()?;
            host.alloc(w, h)
        });
        if keep {
            self.sixel_palettes[screen] = Some(done.palette);
        }
        let Some(img) = done.image else { return };
        if img.width == 0 || img.height == 0 {
            return;
        }
        // The entry is the image's now.
        s.id = None;
        let pixels: Arc<dyn PixelBuffer> = Arc::from(img.pixels);
        pool.set_pixels(id, pixels);
        pool.set_busy(id, false);
        self.collect_own_images();
        let key = self.new_own_key();
        let mut image = Image::new(key, img.width, img.height);
        image.pool = Some(id);
        image.state = ImageState::Ready;
        let (cw, ch) = (self.cell_pixels.0.max(1) as u32, self.cell_pixels.1.max(1) as u32);
        let cols = img.width.div_ceil(cw).clamp(1, MAX_CELLS as u32) as u16;
        let rows = img.height.div_ceil(ch).clamp(1, MAX_CELLS as u32) as u16;
        let after = if self.modes.sixel_display {
            After::SixelFixed
        } else if self.modes.sixel_cursor_right {
            After::SixelRight
        } else {
            After::SixelBelow
        };
        self.place_own(image, cols, rows, Fit::Actual, after);
    }

    fn new_own_key(&mut self) -> ImageKey {
        loop {
            let key = self.graphics.new_key(ImageKey::Own);
            if self.graphics.image(key).is_none() {
                return key;
            }
        }
    }

    /// Store an own image with its virtual placement and write its
    /// placeholder cells at the cursor, row by row, scrolling as text does
    /// (cells §8.3).
    fn place_own(&mut self, image: Image, cols: u16, rows: u16, fit: Fit, after: After) {
        self.clear_grapheme();
        let key = image.key;
        let n = match key {
            ImageKey::Own(n) => n,
            _ => unreachable!("own images only"),
        };
        if let Some(id) = image.pool {
            self.graphics.pool().set_placed(id, true);
        }
        self.graphics.make_room_for_image();
        self.graphics.insert(image);
        self.graphics.set_virtual(VirtualPlacement {
            image: key,
            id: 0,
            cols,
            rows,
            fit,
        });
        let style = Style {
            fg: Color::rgb((n >> 16) as u8, (n >> 8) as u8, n as u8),
            image_id_high: (n >> 24) as u8,
            attrs: Attrs::OWN_IMAGE,
            ..Style::DEFAULT
        };
        let fixed = after == After::SixelFixed;
        let saved = (self.cursor.row, self.cursor.col, self.cursor.pending_wrap);
        if fixed {
            self.cursor.row = 0;
            self.cursor.col = 0;
        } else if self.cursor.pending_wrap && self.modes.autowrap {
            self.screen[self.cursor.row].wrapped = true;
            self.cursor.col = 0;
            self.linefeed();
        }
        self.cursor.pending_wrap = false;
        let c0 = self.cursor.col;
        let width = (cols as usize).min(self.cols - c0);
        let mut row = self.cursor.row;
        let mut scrolled = 0;
        for r in 0..rows {
            if r > 0 {
                if fixed {
                    row += 1;
                    if row >= self.lines {
                        break;
                    }
                } else {
                    if self.cursor.row == self.scroll_bottom {
                        scrolled += 1;
                    }
                    self.linefeed();
                    self.cursor.col = c0;
                    row = self.cursor.row;
                }
            }
            self.erase_cells(row, c0..c0 + width);
            // After the erase: interning may renumber.
            let template = self.intern(style);
            let line = &mut self.screen[row];
            for c in 0..width {
                line.cells[c0 + c] = template.with_placeholder(r, c as u16);
            }
            self.touch_row(row);
        }
        self.placeholders_written = true;
        match after {
            After::SixelFixed => {
                (self.cursor.row, self.cursor.col, self.cursor.pending_wrap) = saved;
            }
            After::Iterm { stay: true } => {
                self.cursor.row = saved.0.saturating_sub(scrolled);
                self.cursor.col = c0;
            }
            After::Iterm { stay: false } | After::SixelRight => {
                let to = c0 + width;
                if to >= self.cols {
                    self.cursor.col = self.cols - 1;
                    self.cursor.pending_wrap = self.modes.autowrap;
                } else {
                    self.cursor.col = to;
                }
            }
            After::SixelBelow => {
                self.linefeed();
                self.cursor.col = c0;
            }
        }
        self.touch();
    }

    /// Free own images nothing names any more (§4.3): after a style sweep,
    /// or when history blocks were freed.
    pub(super) fn collect_own_images(&mut self) {
        let mut freed = Vec::new();
        self.scrollback.take_freed_images(&mut freed);
        if !self.own_gc_due && freed.is_empty() {
            return;
        }
        self.own_gc_due = false;
        let alt = self.alt_active;
        for shown in [true, false] {
            let (g, styles) = if shown {
                (&mut self.graphics, &self.styles)
            } else {
                (&mut self.other_graphics, &self.other_styles)
            };
            if !g.has_own() {
                continue;
            }
            let mut live = HashSet::new();
            for (i, s) in styles.styles().iter().enumerate() {
                if s.attrs.contains(Attrs::OWN_IMAGE) && styles.is_live(i as u32) {
                    live.insert(crate::image::placeholder_key(s));
                }
            }
            // The primary screen's lines go to history.
            if shown != alt {
                self.scrollback.listed_images(&mut live);
            }
            if g.retain_own(|k| live.contains(&k)) > 0 {
                self.generation = self.generation.wrapping_add(1);
            }
        }
    }

    /// XTSMGRAPHICS `CSI ? Pi ; Pa ; Pv S` (§8.3).
    pub(super) fn xtsmgraphics(&mut self, args: &[u16]) {
        let pi = args.first().copied().unwrap_or(0);
        let pa = args.get(1).copied().unwrap_or(0);
        let reply = match (pi, pa) {
            (1, 1..=4) => format!("\x1b[?1;0;{}S", term_image::sixel::REGISTERS),
            (2, 1..=3) => {
                let (cw, ch) = self.cell_pixels;
                let w = (self.cols * cw as usize).min(MAX_SIDE as usize);
                let h = (self.lines * ch as usize).min(MAX_SIDE as usize);
                format!("\x1b[?2;0;{w};{h}S")
            }
            (2, 4) => format!("\x1b[?2;0;{MAX_SIDE};{MAX_SIDE}S"),
            (1 | 2, _) => format!("\x1b[?{pi};2;0S"),
            _ => format!("\x1b[?{pi};1;0S"),
        };
        self.replies.extend_from_slice(reply.as_bytes());
    }
}

struct ItermLayout {
    cols: u16,
    rows: u16,
    fit: Fit,
    decoded: (u32, u32),
}
