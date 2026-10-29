//! Images in a frame (images.md §6.3), without Metal: the pieces of
//! images each frame draws, as instances in three layers, from the
//! snapshot's direct placements and the rows' kitty placeholder cells.
//!
//! Within a layer, pieces are sorted by (z, image, serial), the
//! protocol's order, and neighbours of the same image are one draw.
//! Every instance is clipped to the grid in f32 before it is packed into
//! i16, and its texture coordinates are worked out from the exact
//! (unrounded) fit, so the pieces of one image line up.

use std::ops::Range;
use std::sync::Arc;

use term_core::image::{Fit, ImageKey, ImagePool, Layer, PoolId, SnapImage, SnapPlacement, SnapVirtual, placeholder_key};
use term_core::{ColorKind, Snapshot};

use crate::frame::CellView;

/// One piece of an image: where it is drawn and which texels it shows.
/// Must match `struct Image` in shaders.metal.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageInstance {
    /// x, y, width, height in pixels, relative to the pane.
    pub dst: [i16; 4],
    /// The texels at the left, top, right and bottom edges of `dst`.
    pub uv: [f32; 4],
    /// Samples are kept inside this texel rectangle: the source rectangle
    /// inset by half a texel, so bilinear filtering takes nothing from
    /// outside it.
    pub clamp: [f32; 4],
    /// Texels per pixel, the larger of the two directions: above 2 (drawn
    /// at less than half size) the shader averages four samples.
    pub scale: f32,
    pub _pad: u32,
}

const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<ImageInstance>() == 48);
    assert!(offset_of!(ImageInstance, uv) == 8);
    assert!(offset_of!(ImageInstance, clamp) == 24);
    assert!(offset_of!(ImageInstance, scale) == 40);
};

/// Consecutive instances of one image in one layer: one instanced draw.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageDraw {
    pub layer: Layer,
    /// Index into [`Images::images`].
    pub image: u32,
    pub instances: Range<u32>,
}

/// A run of placeholder cells in one row that shows one strip of an
/// image: next to each other on screen, with consecutive placeholder
/// columns in the same placeholder row of the same image and placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PieceRun {
    /// The first cell's column, and the cells in the run.
    pub col: u16,
    pub len: u16,
    pub image: ImageKey,
    /// The kitty placement id (the underline colour), 0 for any.
    pub placement: u32,
    /// The placeholder row, and the placeholder column of the first cell.
    pub row: u16,
    pub piece_col: u16,
}

/// The placement id a placeholder cell's style names: its underline
/// colour, as the image id is its foreground (cells §7).
fn placement_id(style: &term_core::Style) -> u32 {
    match style.underline_color.kind() {
        ColorKind::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        ColorKind::Indexed(i) => i as u32,
        ColorKind::Default => 0,
    }
}

/// The placeholder runs of one row's cells.
pub(crate) fn pieces_of_row<C: CellView>(cells: impl Iterator<Item = C>, cols: usize, out: &mut Vec<PieceRun>) {
    for (cell, col) in cells.zip(0..cols) {
        let Some((row, piece_col)) = cell.placeholder() else {
            continue;
        };
        let style = cell.style();
        let (image, placement) = (placeholder_key(&style), placement_id(&style));
        let col = col as u16;
        if let Some(last) = out.last_mut()
            && last.col + last.len == col
            && last.image == image
            && last.placement == placement
            && last.row == row
            && last.piece_col + last.len == piece_col
        {
            last.len += 1;
            continue;
        }
        out.push(PieceRun {
            col,
            len: 1,
            image,
            placement,
            row,
            piece_col,
        });
    }
}

/// The grid in pixels, relative to the pane.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Grid {
    pub cw: f32,
    pub ch: f32,
    pub px: f32,
    pub py: f32,
    pub cols: u16,
    pub lines: u16,
}

impl Grid {
    fn rect(&self) -> [f32; 4] {
        [
            self.px,
            self.py,
            self.px + self.cols as f32 * self.cw,
            self.py + self.lines as f32 * self.ch,
        ]
    }
}

struct Piece {
    layer: Layer,
    z: i32,
    pool: PoolId,
    serial: u64,
    image: u32,
    instance: ImageInstance,
}

/// A frame's images: the images it draws, their instances and draws, and
/// what it tells the pool.
#[derive(Default)]
pub struct Images {
    /// Each image drawn this frame, once.
    pub images: Vec<SnapImage>,
    pub instances: Vec<ImageInstance>,
    /// In drawing order within each layer.
    pub draws: Vec<ImageDraw>,
    pieces: Vec<Piece>,
    /// Cells covered by images under the text, as (view line, columns),
    /// sorted: highlights over them are tinted, not filled (§6.3 step 4).
    pub(crate) covered: Vec<(u16, Range<u16>)>,
    /// The pool and owner the snapshot came from, and whether the last
    /// frame told it of any image.
    pool: Option<(Arc<ImagePool>, u64)>,
    told: bool,
    drawn: Vec<PoolId>,
}

impl Images {
    /// Remember the pool `term`'s images count against, to tell it which
    /// were drawn.
    pub(crate) fn set_pool(&mut self, pool: &Arc<ImagePool>, owner: u64) {
        if !matches!(&self.pool, Some((p, o)) if Arc::ptr_eq(p, pool) && *o == owner) {
            self.pool = Some((pool.clone(), owner));
        }
    }

    pub(crate) fn clear(&mut self) {
        self.images.clear();
        self.instances.clear();
        self.draws.clear();
        self.covered.clear();
    }

    /// Build this frame's images from `snap` and the placeholder runs of
    /// each view line.
    pub(crate) fn build<'a>(&mut self, snap: &Snapshot, lines: impl Iterator<Item = (usize, &'a [PieceRun])>, g: Grid) {
        self.clear();
        self.pieces.clear();
        let grid = g.rect();
        for p in &snap.placements {
            let Some(instance) = placement_instance(p, &g, grid) else {
                continue;
            };
            let image = self.image_index(&p.image);
            self.pieces.push(Piece {
                layer: p.layer,
                z: p.z,
                pool: p.image.pool,
                serial: p.serial,
                image,
                instance,
            });
        }
        let mut serial = 0;
        for (line, runs) in lines {
            for run in runs {
                let Some(v) = find_virtual(&snap.virtual_placements, run) else {
                    continue;
                };
                let Some(instance) = piece_instance(v, run, line, &g, grid) else {
                    continue;
                };
                let image = self.image_index(&v.image);
                serial += 1;
                self.pieces.push(Piece {
                    layer: Layer::BelowText,
                    z: 0,
                    pool: v.image.pool,
                    serial,
                    image,
                    instance,
                });
            }
        }
        self.pieces.sort_by_key(|p| (p.layer, p.z, p.pool, p.serial));
        for p in &self.pieces {
            let at = self.instances.len() as u32;
            self.instances.push(p.instance);
            match self.draws.last_mut() {
                Some(d) if d.layer == p.layer && d.image == p.image => d.instances.end = at + 1,
                _ => self.draws.push(ImageDraw {
                    layer: p.layer,
                    image: p.image,
                    instances: at..at + 1,
                }),
            }
            if p.layer != Layer::AboveText {
                let d = p.instance.dst;
                let (x0, y0) = ((d[0] as f32 - g.px) / g.cw, (d[1] as f32 - g.py) / g.ch);
                let (x1, y1) = ((d[0] as f32 + d[2] as f32 - g.px) / g.cw, (d[1] as f32 + d[3] as f32 - g.py) / g.ch);
                let cols = x0.floor().max(0.0) as u16..x1.ceil().min(g.cols as f32) as u16;
                for line in y0.floor().max(0.0) as u16..y1.ceil().min(g.lines as f32) as u16 {
                    self.covered.push((line, cols.clone()));
                }
            }
        }
        self.covered.sort_by_key(|(l, c)| (*l, c.start));
    }

    /// Tell the pool which images this frame drew (§5.2): once a frame
    /// while any are drawn, and once more when none are.
    pub(crate) fn tell_pool(&mut self) {
        let Some((pool, owner)) = &self.pool else { return };
        if self.images.is_empty() && !self.told {
            return;
        }
        self.drawn.clear();
        self.drawn.extend(self.images.iter().map(|i| i.pool));
        pool.drawn(*owner, &self.drawn);
        self.told = !self.drawn.is_empty();
    }

    /// The columns of `line` covered by images under the text.
    pub(crate) fn covered_on(&self, line: usize) -> impl Iterator<Item = Range<u16>> + '_ {
        let start = self.covered.partition_point(|(l, _)| (*l as usize) < line);
        self.covered[start..]
            .iter()
            .take_while(move |(l, _)| *l as usize == line)
            .map(|(_, c)| c.clone())
    }

    fn image_index(&mut self, image: &SnapImage) -> u32 {
        if let Some(i) = self
            .images
            .iter()
            .position(|i| i.pool == image.pool && i.generation == image.generation)
        {
            return i as u32;
        }
        self.images.push(image.clone());
        self.images.len() as u32 - 1
    }
}

/// The virtual placement a run of placeholder cells names: its own id, or
/// any of the image's when the cells name none.
fn find_virtual<'a>(all: &'a [SnapVirtual], run: &PieceRun) -> Option<&'a SnapVirtual> {
    all.iter()
        .find(|v| v.key.0 == run.image && (run.placement == 0 || v.key.1 == run.placement))
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

/// The instance drawing texels `uv` (left, top, right, bottom) over `dst`
/// (pixels, the same order), the part inside `clip`, kept inside `src`.
fn instance(dst: [f32; 4], uv: [f32; 4], src: [f32; 4], clip: [f32; 4]) -> Option<ImageInstance> {
    let (dw, dh) = (dst[2] - dst[0], dst[3] - dst[1]);
    if dw <= 0.0 || dh <= 0.0 {
        return None;
    }
    // Clipped, then rounded to whole pixels; the texels follow the
    // rounded edges through the exact mapping.
    let c = intersect(dst, clip);
    let (x0, y0, x1, y1) = (c[0].round(), c[1].round(), c[2].round(), c[3].round());
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (su, sv) = ((uv[2] - uv[0]) / dw, (uv[3] - uv[1]) / dh);
    let u = |x: f32| uv[0] + (x - dst[0]) * su;
    let v = |y: f32| uv[1] + (y - dst[1]) * sv;
    let inset = |a: f32, b: f32| {
        if b - a >= 1.0 {
            (a + 0.5, b - 0.5)
        } else {
            ((a + b) / 2.0, (a + b) / 2.0)
        }
    };
    let (cx0, cx1) = inset(src[0], src[2]);
    let (cy0, cy1) = inset(src[1], src[3]);
    let s16 = |v: f32| v.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    Some(ImageInstance {
        dst: [s16(x0), s16(y0), s16(x1 - x0), s16(y1 - y0)],
        uv: [u(x0), v(y0), u(x1), v(y1)],
        clamp: [cx0, cy0, cx1, cy1],
        scale: su.abs().max(sv.abs()),
        _pad: 0,
    })
}

/// A direct placement: its source rectangle (width or height 0: to the
/// image's edge) over its cells, stretched, from its pixel offset; with no
/// cells given, one image pixel a device pixel, and with one of the two,
/// the other keeps the aspect.
fn placement_instance(p: &SnapPlacement, g: &Grid, grid: [f32; 4]) -> Option<ImageInstance> {
    let (iw, ih) = (p.image.width as f32, p.image.height as f32);
    let sx = (p.src[0] as f32).min(iw);
    let sy = (p.src[1] as f32).min(ih);
    let sw = if p.src[2] == 0 { iw - sx } else { (p.src[2] as f32).min(iw - sx) };
    let sh = if p.src[3] == 0 { ih - sy } else { (p.src[3] as f32).min(ih - sy) };
    if sw <= 0.0 || sh <= 0.0 {
        return None;
    }
    let (w, h) = match (p.cols as f32 * g.cw, p.rows as f32 * g.ch) {
        (0.0, 0.0) => (sw, sh),
        (w, 0.0) => (w, w * sh / sw),
        (0.0, h) => (h * sw / sh, h),
        (w, h) => (w, h),
    };
    let x = g.px + p.col as f32 * g.cw + p.offset.0 as f32;
    let y = g.py + p.view_row as f32 * g.ch + p.offset.1 as f32;
    let src = [sx, sy, sx + sw, sy + sh];
    instance([x, y, x + w, y + h], src, src, grid)
}

/// The strip of a virtual placement's image a run of placeholder cells on
/// view line `line` shows, fitted to the placement's cell box.
fn piece_instance(v: &SnapVirtual, run: &PieceRun, line: usize, g: &Grid, grid: [f32; 4]) -> Option<ImageInstance> {
    let (iw, ih) = (v.image.width as f32, v.image.height as f32);
    if iw <= 0.0 || ih <= 0.0 {
        return None;
    }
    let (bw, bh) = (v.cols as f32 * g.cw, v.rows as f32 * g.ch);
    let (sx, sy) = match v.fit {
        Fit::Contain | Fit::ContainTopLeft => {
            let s = (bw / iw).min(bh / ih);
            (s, s)
        }
        Fit::Stretch => (bw / iw, bh / ih),
        Fit::Actual => (1.0, 1.0),
    };
    let (ox, oy) = match v.fit {
        Fit::Contain => ((bw - iw * sx) / 2.0, (bh - ih * sy) / 2.0),
        _ => (0.0, 0.0),
    };
    // The box's top-left corner on screen, where this run's cells say it is.
    let bx = g.px + (run.col as f32 - run.piece_col as f32) * g.cw;
    let by = g.py + (line as f32 - run.row as f32) * g.ch;
    let dst = [bx + ox, by + oy, bx + ox + iw * sx, by + oy + ih * sy];
    let x0 = g.px + run.col as f32 * g.cw;
    let y0 = g.py + line as f32 * g.ch;
    let cells = [x0, y0, x0 + run.len as f32 * g.cw, y0 + g.ch];
    let in_box = [bx, by, bx + bw, by + bh];
    let clip = intersect(intersect(cells, in_box), grid);
    instance(dst, [0.0, 0.0, iw, ih], [0.0, 0.0, iw, ih], clip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_are_clipped_with_their_texels() {
        // A 10×10 image over 100×100 pixels, half of it left of the clip.
        let i = instance(
            [-50.0, 0.0, 50.0, 100.0],
            [0.0, 0.0, 10.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [0.0, 0.0, 200.0, 200.0],
        )
        .unwrap();
        assert_eq!(i.dst, [0, 0, 50, 100]);
        assert_eq!(i.uv, [5.0, 0.0, 10.0, 10.0]);
        assert_eq!(i.clamp, [0.5, 0.5, 9.5, 9.5]);
        assert!((i.scale - 0.1).abs() < 1e-6);
        assert!(instance([0.0, 0.0, 10.0, 10.0], [0.0; 4], [0.0; 4], [20.0, 0.0, 30.0, 10.0]).is_none());
    }

    #[test]
    fn far_placements_dont_overflow() {
        let i = instance(
            [0.0, -1e9, 10.0, 1e9],
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 100.0, 100.0],
        )
        .unwrap();
        assert_eq!(i.dst, [0, 0, 10, 100]);
        assert!(i.uv[1] > 0.49 && i.uv[3] < 0.51);
    }
}
