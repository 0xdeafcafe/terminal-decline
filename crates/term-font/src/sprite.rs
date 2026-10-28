//! Glyphs drawn in code instead of taken from the font: box drawing
//! (U+2500–257F), block elements (U+2580–259F), braille (U+2800–28FF),
//! Powerline (U+E0A0–E0A3, U+E0B0–E0BF) and the legacy computing sextants,
//! wedges and eighth blocks (U+1FB00–1FB8B).
//!
//! Fonts draw these with their own idea of the cell, so borders show seams
//! or gaps at some sizes. Here they are drawn on the cell's own pixel grid:
//! straight lines are whole pixels and meet exactly at the cell edges, and
//! only curves, diagonals and triangles are anti-aliased. Pure Rust, no
//! Core Text.
//!
//! Each shape has a [`SpriteId`], independent of the character, so the
//! glyph cache can key code-drawn glyphs by (sprite font, sprite id) later
//! and non-character shapes (curly underlines) can join them.

use crate::{Metrics, RasterGlyph};

/// Names one code-drawn shape. Today ids map one-to-one onto characters:
///
/// | ids | characters |
/// |---|---|
/// | 0x000–0x09F | U+2500–259F box drawing and block elements |
/// | 0x100–0x1FF | U+2800–28FF braille |
/// | 0x200–0x21F | U+E0A0–E0BF Powerline (only E0A0–E0A3 and E0B0–E0BF are drawn) |
/// | 0x300–0x38B | U+1FB00–1FB8B legacy computing |
///
/// Ids from 0x1000 up are kept for shapes that aren't characters
/// (underline patterns).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SpriteId(pub u16);

impl SpriteId {
    /// The sprite that draws `ch`, if it is code-drawn.
    pub fn for_char(ch: char) -> Option<SpriteId> {
        let cp = ch as u32;
        let id = match cp {
            0x2500..=0x259F => cp - 0x2500,
            0x2800..=0x28FF => 0x100 + (cp - 0x2800),
            0xE0A0..=0xE0A3 | 0xE0B0..=0xE0BF => 0x200 + (cp - 0xE0A0),
            0x1FB00..=0x1FB8B => 0x300 + (cp - 0x1FB00),
            _ => return None,
        };
        Some(SpriteId(id as u16))
    }

    /// The character this sprite draws.
    pub fn char(self) -> Option<char> {
        let id = self.0 as u32;
        let cp = match id {
            0x000..=0x09F => 0x2500 + id,
            0x100..=0x1FF => 0x2800 + (id - 0x100),
            0x200..=0x21F => 0xE0A0 + (id - 0x200),
            0x300..=0x38B => 0x1FB00 + (id - 0x300),
            _ => return None,
        };
        char::from_u32(cp).filter(|&c| SpriteId::for_char(c) == Some(self))
    }
}

/// Whether `ch` is drawn in code rather than taken from the font.
pub fn is_code_drawn(ch: char) -> bool {
    SpriteId::for_char(ch).is_some()
}

/// Draw `id` for a glyph `cells` cells wide: a coverage bitmap exactly the
/// size of those cells, placed at the cell's top-left corner (`left` 0,
/// `top` the baseline). `None` for shapes with no ink (U+2800, blank
/// braille).
pub fn rasterize(id: SpriteId, m: &Metrics, cells: u32) -> Option<RasterGlyph> {
    let ch = id.char()?;
    let w = (m.cell_width * cells.max(1)).max(1) as i32;
    let h = m.cell_height.max(1) as i32;
    let mut c = Canvas::new(w, h, m.underline_thickness as i32);
    draw(&mut c, ch as u32);
    if c.px.iter().all(|&p| p == 0) {
        return None;
    }
    Some(RasterGlyph {
        width: w as u32,
        height: h as u32,
        left: 0,
        top: m.baseline as i32,
        color: false,
        data: c.px,
    })
}

// ---- canvas ----------------------------------------------------------------

/// Subsamples per pixel side for anti-aliased shapes.
const SS: i32 = 6;

struct Canvas {
    w: i32,
    h: i32,
    /// Light line thickness in pixels; heavy lines are twice this.
    t: i32,
    px: Vec<u8>,
}

type Pt = (f64, f64);

impl Canvas {
    fn new(w: i32, h: i32, underline: i32) -> Canvas {
        // Keep a double line (three light thicknesses) inside the cell.
        let t = underline.clamp(1, (w.min(h) / 5).max(1));
        Canvas {
            w,
            h,
            t,
            px: vec![0; (w * h) as usize],
        }
    }

    /// Fill whole pixels `x0..x1` × `y0..y1` (clipped) with coverage `a`.
    fn rect_a(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, a: u8) {
        let (x0, x1) = (x0.clamp(0, self.w), x1.clamp(0, self.w));
        let (y0, y1) = (y0.clamp(0, self.h), y1.clamp(0, self.h));
        for y in y0..y1 {
            let row = (y * self.w) as usize;
            for p in &mut self.px[row + x0 as usize..row + x1 as usize] {
                *p = (*p).max(a);
            }
        }
    }

    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.rect_a(x0, y0, x1, y1, 255);
    }

    /// Anti-aliased fill of the points `inside` accepts, by supersampling,
    /// within the pixel box `x0..x1` × `y0..y1`.
    fn paint_in(&mut self, (x0, y0, x1, y1): (i32, i32, i32, i32), inside: impl Fn(f64, f64) -> bool) {
        let (x0, x1) = (x0.clamp(0, self.w), x1.clamp(0, self.w));
        let (y0, y1) = (y0.clamp(0, self.h), y1.clamp(0, self.h));
        let step = 1.0 / SS as f64;
        for y in y0..y1 {
            for x in x0..x1 {
                let mut n = 0;
                for sy in 0..SS {
                    let fy = y as f64 + (sy as f64 + 0.5) * step;
                    for sx in 0..SS {
                        let fx = x as f64 + (sx as f64 + 0.5) * step;
                        n += inside(fx, fy) as i32;
                    }
                }
                if n > 0 {
                    let a = ((n * 255 + SS * SS / 2) / (SS * SS)) as u8;
                    let p = &mut self.px[(y * self.w + x) as usize];
                    *p = (*p).max(a);
                }
            }
        }
    }

    fn paint(&mut self, inside: impl Fn(f64, f64) -> bool) {
        self.paint_in((0, 0, self.w, self.h), inside);
    }

    /// An anti-aliased line `width` pixels wide along the polyline `pts`.
    fn stroke(&mut self, pts: &[Pt], width: f64) {
        let r = width / 2.0;
        let r2 = r * r;
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in pts {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let bbox = (
            (x0 - r).floor() as i32,
            (y0 - r).floor() as i32,
            (x1 + r).ceil() as i32 + 1,
            (y1 + r).ceil() as i32 + 1,
        );
        self.paint_in(bbox, |x, y| pts.windows(2).any(|s| seg_dist2((x, y), s[0], s[1]) <= r2));
    }

    fn polygon(&mut self, pts: &[Pt]) {
        self.paint(|x, y| in_polygon(pts, (x, y)));
    }

    /// A filled disc.
    fn disc(&mut self, (cx, cy): Pt, r: f64) {
        let bbox = (
            (cx - r).floor() as i32,
            (cy - r).floor() as i32,
            (cx + r).ceil() as i32 + 1,
            (cy + r).ceil() as i32 + 1,
        );
        self.paint_in(bbox, |x, y| (x - cx).powi(2) + (y - cy).powi(2) <= r * r);
    }

    /// `n` eighths of `len`, rounded: the shared boundaries of block elements.
    fn eighth(n: i32, len: i32) -> i32 {
        (len * n + 4) / 8
    }

    /// Start of a band `thick` pixels thick centred in `len`.
    fn band(len: i32, thick: i32) -> i32 {
        ((len - thick) / 2).max(0)
    }
}

fn seg_dist2(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (qx, qy) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    qx * qx + qy * qy
}

/// Even-odd rule.
fn in_polygon(pts: &[Pt], (x, y): Pt) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let ((xi, yi), (xj, yj)) = (pts[i], pts[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Points along an arc around `c` from angle `a0` to `a1` (radians, y down).
fn arc(c: Pt, rx: f64, ry: f64, a0: f64, a1: f64, out: &mut Vec<Pt>) {
    const N: usize = 32;
    for i in 0..=N {
        let a = a0 + (a1 - a0) * i as f64 / N as f64;
        out.push((c.0 + rx * a.cos(), c.1 + ry * a.sin()));
    }
}

// ---- dispatch --------------------------------------------------------------

fn draw(c: &mut Canvas, cp: u32) {
    match cp {
        0x2504..=0x250B | 0x254C..=0x254F => dashes(c, cp),
        0x256D..=0x2570 => rounded(c, cp),
        0x2571..=0x2573 => diagonals(c, cp),
        0x2500..=0x257F => {
            if let Some(arms) = arms(cp) {
                lines(c, arms);
            }
        }
        0x2580..=0x259F => blocks(c, cp),
        0x2800..=0x28FF => braille(c, (cp - 0x2800) as u8),
        0xE0A0..=0xE0BF => powerline(c, cp),
        0x1FB00..=0x1FB3B => sextant(c, cp - 0x1FB00),
        0x1FB3C..=0x1FB67 => wedge(c, cp),
        0x1FB68..=0x1FB6F => triangles(c, cp),
        0x1FB70..=0x1FB8B => eighths(c, cp),
        _ => {}
    }
}

// ---- box drawing -----------------------------------------------------------

/// Weight of one arm of a box-drawing character.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum W {
    None,
    Light,
    Heavy,
    Double,
}

/// Arms of a box-drawing character: up, right, down, left.
#[derive(Clone, Copy, Debug)]
struct Arms {
    up: W,
    right: W,
    down: W,
    left: W,
}

/// Arms by codepoint, one string per character: up, right, down, left as
/// `.` none, `L` light, `H` heavy, `D` double.
const ARMS_2500: [&str; 0x4C] = [
    ".L.L", ".H.H", "L.L.", "H.H.", "", "", "", "", "", "", "", "", // 2500–250B (dashes drawn apart)
    ".LL.", ".HL.", ".LH.", ".HH.", "..LL", "..LH", "..HL", "..HH", // 250C–2513
    "LL..", "LH..", "HL..", "HH..", "L..L", "L..H", "H..L", "H..H", // 2514–251B
    "LLL.", "LHL.", "HLL.", "LLH.", "HLH.", "HHL.", "LHH.", "HHH.", // 251C–2523
    "L.LL", "L.LH", "H.LL", "L.HL", "H.HL", "H.LH", "L.HH", "H.HH", // 2524–252B
    ".LLL", ".LLH", ".HLL", ".HLH", ".LHL", ".LHH", ".HHL", ".HHH", // 252C–2533
    "LL.L", "LL.H", "LH.L", "LH.H", "HL.L", "HL.H", "HH.L", "HH.H", // 2534–253B
    "LLLL", "LLLH", "LHLL", "LHLH", "HLLL", "LLHL", "HLHL", "HLLH", // 253C–2543
    "HHLL", "LLHH", "LHHL", "HHLH", "LHHH", "HLHH", "HHHL", "HHHH", // 2544–254B
];
const ARMS_2550: [&str; 0x1D] = [
    ".D.D", "D.D.", ".DL.", ".LD.", ".DD.", "..LD", "..DL", "..DD", // 2550–2557
    "LD..", "DL..", "DD..", "L..D", "D..L", "D..D", "LDL.", "DLD.", // 2558–255F
    "DDD.", "L.LD", "D.DL", "D.DD", ".DLD", ".LDL", ".DDD", "LD.D", // 2560–2567
    "DL.L", "DD.D", "LDLD", "DLDL", "DDDD", // 2568–256C
];
const ARMS_2574: [&str; 0xC] = [
    "...L", "L...", ".L..", "..L.", "...H", "H...", ".H..", "..H.", ".H.L", "L.H.", ".L.H", "H.L.", // 2574–257F
];

fn arms(cp: u32) -> Option<Arms> {
    let s = match cp {
        0x2500..=0x254B => ARMS_2500[(cp - 0x2500) as usize],
        0x2550..=0x256C => ARMS_2550[(cp - 0x2550) as usize],
        0x2574..=0x257F => ARMS_2574[(cp - 0x2574) as usize],
        _ => return None,
    };
    let w = |b: u8| match b {
        b'L' => W::Light,
        b'H' => W::Heavy,
        b'D' => W::Double,
        _ => W::None,
    };
    let b = s.as_bytes();
    (b.len() == 4).then(|| Arms {
        up: w(b[0]),
        right: w(b[1]),
        down: w(b[2]),
        left: w(b[3]),
    })
}

/// Lines from the centre of the cell to its edges. Light and heavy lines
/// are bands of whole pixels centred in the cell, so the same arm in two
/// characters covers the same pixels at the shared edge; a double line is
/// two light strokes a light thickness apart.
///
/// Where arms meet, each reaches just far enough to join the others: a
/// corner is closed, a T or cross has no notch, and the gap between the two
/// strokes of a double line stays open where another line runs into it.
fn lines(c: &mut Canvas, a: Arms) {
    let (w, h, t) = (c.w, c.h, c.t);
    let thick = |x: W| match x {
        W::Heavy => 2 * t,
        _ => t,
    };
    let single = |x: W| matches!(x, W::Light | W::Heavy);
    // Horizontal bands (rows) and vertical bands (columns).
    let hb = |th: i32| Canvas::band(h, th);
    let vb = |tv: i32| Canvas::band(w, tv);
    // Double lines: strokes at d0..d0+t and d0+2t..d0+3t (rows), e0.. (columns).
    let d0 = Canvas::band(h, 3 * t);
    let e0 = Canvas::band(w, 3 * t);
    let hcentre = h / 2;
    let vcentre = w / 2;
    // Thickest light/heavy line across each direction, or 0.
    let th_max = [a.left, a.right]
        .iter()
        .filter(|&&x| single(x))
        .map(|&x| thick(x))
        .max()
        .unwrap_or(0);
    let tv_max = [a.up, a.down].iter().filter(|&&x| single(x)).map(|&x| thick(x)).max().unwrap_or(0);
    let hor_double = a.left == W::Double || a.right == W::Double;
    let ver_double = a.up == W::Double || a.down == W::Double;

    // Where one stroke of a double arm stops, along its own axis: `inner`
    // meets the near stroke of a crossing double line, `outer` its far
    // stroke, `edge` is the far cell edge, `centre` the middle of the cell.
    struct Stop {
        inner: i32,
        outer: i32,
        edge: i32,
        centre: i32,
    }

    // ---- vertical arms ----
    if a.up != W::None {
        if single(a.up) {
            let ta = thick(a.up);
            let end = if hor_double {
                if a.left == W::Double && a.right == W::Double && a.down == W::None {
                    d0 + t
                } else {
                    d0 + 3 * t
                }
            } else {
                let th = if th_max > 0 { th_max } else { ta };
                hb(th) + th
            };
            c.rect(vb(ta), 0, vb(ta) + ta, end);
        } else {
            // Left stroke, then right stroke.
            let stop = Stop {
                inner: d0 + t,
                outer: d0 + 3 * t,
                edge: h,
                centre: hcentre,
            };
            let end = |side: W, other: W| double_stop(side, a.down, other, &stop, |x| hb(thick(x)) + thick(x));
            c.rect(e0, 0, e0 + t, end(a.left, a.right));
            c.rect(e0 + 2 * t, 0, e0 + 3 * t, end(a.right, a.left));
        }
    }
    if a.down != W::None {
        if single(a.down) {
            let ta = thick(a.down);
            let start = if hor_double {
                if a.left == W::Double && a.right == W::Double && a.up == W::None {
                    d0 + 2 * t
                } else {
                    d0
                }
            } else {
                let th = if th_max > 0 { th_max } else { ta };
                hb(th)
            };
            c.rect(vb(ta), start, vb(ta) + ta, h);
        } else {
            let stop = Stop {
                inner: d0 + 2 * t,
                outer: d0,
                edge: 0,
                centre: hcentre,
            };
            let start = |side: W, other: W| double_stop(side, a.up, other, &stop, |x| hb(thick(x)));
            c.rect(e0, start(a.left, a.right), e0 + t, h);
            c.rect(e0 + 2 * t, start(a.right, a.left), e0 + 3 * t, h);
        }
    }

    // ---- horizontal arms ----
    if a.left != W::None {
        if single(a.left) {
            let ta = thick(a.left);
            let end = if ver_double {
                if a.up == W::Double && a.down == W::Double && a.right == W::None {
                    e0 + t
                } else {
                    e0 + 3 * t
                }
            } else {
                let tv = if tv_max > 0 { tv_max } else { ta };
                vb(tv) + tv
            };
            c.rect(0, hb(ta), end, hb(ta) + ta);
        } else {
            let stop = Stop {
                inner: e0 + t,
                outer: e0 + 3 * t,
                edge: w,
                centre: vcentre,
            };
            let end = |side: W, other: W| double_stop(side, a.right, other, &stop, |x| vb(thick(x)) + thick(x));
            c.rect(0, d0, end(a.up, a.down), d0 + t);
            c.rect(0, d0 + 2 * t, end(a.down, a.up), d0 + 3 * t);
        }
    }
    if a.right != W::None {
        if single(a.right) {
            let ta = thick(a.right);
            let start = if ver_double {
                if a.up == W::Double && a.down == W::Double && a.left == W::None {
                    e0 + 2 * t
                } else {
                    e0
                }
            } else {
                let tv = if tv_max > 0 { tv_max } else { ta };
                vb(tv)
            };
            c.rect(start, hb(ta), w, hb(ta) + ta);
        } else {
            let stop = Stop {
                inner: e0 + 2 * t,
                outer: e0,
                edge: 0,
                centre: vcentre,
            };
            let start = |side: W, other: W| double_stop(side, a.left, other, &stop, |x| vb(thick(x)));
            c.rect(start(a.up, a.down), d0, w, d0 + t);
            c.rect(start(a.down, a.up), d0 + 2 * t, w, d0 + 3 * t);
        }
    }

    /// Where one stroke of a double arm stops. `side` is the arm on this
    /// stroke's side, `ahead` the arm opposite this one, `other` the arm on
    /// the far side; `band` gives the stop for a light/heavy `side` arm.
    fn double_stop(side: W, ahead: W, other: W, s: &Stop, band: impl Fn(W) -> i32) -> i32 {
        match side {
            // Inner corner with that arm's near stroke.
            W::Double => s.inner,
            W::Light | W::Heavy => band(side),
            W::None => match (ahead, other) {
                // Runs on into the arm ahead.
                (W::Double | W::Light | W::Heavy, _) => s.edge,
                // Outer corner: turns into the far stroke of the other arm.
                (W::None, W::Double) => s.outer,
                (W::None, W::Light | W::Heavy) => band(other),
                (W::None, W::None) => s.centre,
            },
        }
    }
}

/// ┄┅┆┇┈┉┊┋╌╍╎╏: n dashes per cell with half a gap at each end, so the
/// gaps stay even across cells.
fn dashes(c: &mut Canvas, cp: u32) {
    let (n, heavy, vertical) = match cp {
        0x2504 => (3, false, false),
        0x2505 => (3, true, false),
        0x2506 => (3, false, true),
        0x2507 => (3, true, true),
        0x2508 => (4, false, false),
        0x2509 => (4, true, false),
        0x250A => (4, false, true),
        0x250B => (4, true, true),
        0x254C => (2, false, false),
        0x254D => (2, true, false),
        0x254E => (2, false, true),
        _ => (2, true, true),
    };
    let th = if heavy { 2 * c.t } else { c.t };
    let len = if vertical { c.h } else { c.w };
    let seg = len as f64 / n as f64;
    let gap = (seg / 3.0).round().max(1.0);
    for i in 0..n {
        let a = (i as f64 * seg + gap / 2.0).round() as i32;
        let b = ((i + 1) as f64 * seg - gap / 2.0).round() as i32;
        if vertical {
            let x = Canvas::band(c.w, th);
            c.rect(x, a, x + th, b);
        } else {
            let y = Canvas::band(c.h, th);
            c.rect(a, y, b, y + th);
        }
    }
}

/// ╭╮╯╰: a quarter circle joining the two light lines, with a straight
/// pixel at each cell edge so the ends match '─' and '│' exactly.
fn rounded(c: &mut Canvas, cp: u32) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let (w, h, t) = (c.w as f64, c.h as f64, c.t as f64);
    let cx = Canvas::band(c.w, c.t) as f64 + t / 2.0;
    let cy = Canvas::band(c.h, c.t) as f64 + t / 2.0;
    let r = (cx.min(w - cx).min(cy).min(h - cy) - 1.0).max(t / 2.0);
    let mut pts = Vec::with_capacity(36);
    match cp {
        // ╭ right and down
        0x256D => {
            pts.push((w + t, cy));
            arc((cx + r, cy + r), r, r, -FRAC_PI_2, -PI, &mut pts);
            pts.push((cx, h + t));
        }
        // ╮ left and down
        0x256E => {
            pts.push((-t, cy));
            arc((cx - r, cy + r), r, r, -FRAC_PI_2, 0.0, &mut pts);
            pts.push((cx, h + t));
        }
        // ╯ left and up
        0x256F => {
            pts.push((-t, cy));
            arc((cx - r, cy - r), r, r, FRAC_PI_2, 0.0, &mut pts);
            pts.push((cx, -t));
        }
        // ╰ right and up
        _ => {
            pts.push((w + t, cy));
            arc((cx + r, cy - r), r, r, FRAC_PI_2, PI, &mut pts);
            pts.push((cx, -t));
        }
    }
    c.stroke(&pts, t);
}

/// ╱╲╳: corner to corner, so diagonals in neighbouring cells meet.
fn diagonals(c: &mut Canvas, cp: u32) {
    let (w, h, t) = (c.w as f64, c.h as f64, c.t as f64);
    // Run past the corners so the round ends fall outside the cell.
    let (ex, ey) = {
        let len = (w * w + h * h).sqrt();
        (w / len * t, h / len * t)
    };
    if cp != 0x2572 {
        c.stroke(&[(w + ex, -ey), (-ex, h + ey)], t);
    }
    if cp != 0x2571 {
        c.stroke(&[(-ex, -ey), (w + ex, h + ey)], t);
    }
}

// ---- block elements ----------------------------------------------------------

fn blocks(c: &mut Canvas, cp: u32) {
    let (w, h) = (c.w, c.h);
    let ex = |n| Canvas::eighth(n, w);
    let ey = |n| Canvas::eighth(n, h);
    let (mx, my) = (ex(4), ey(4));
    match cp {
        0x2580 => c.rect(0, 0, w, my),
        // ▁▂▃▄▅▆▇█: lower n eighths
        0x2581..=0x2588 => {
            let n = (cp - 0x2580) as i32;
            c.rect(0, ey(8 - n), w, h)
        }
        // ▉▊▋▌▍▎▏: left 7..1 eighths
        0x2589..=0x258F => {
            let n = 8 - (cp - 0x2588) as i32;
            c.rect(0, 0, ex(n), h)
        }
        0x2590 => c.rect(mx, 0, w, h),
        // ░▒▓: even coverage, so the shade tiles without seams at any cell
        // size (a pixel pattern can't repeat across an odd-width cell).
        0x2591 => c.rect_a(0, 0, w, h, 0x40),
        0x2592 => c.rect_a(0, 0, w, h, 0x80),
        0x2593 => c.rect_a(0, 0, w, h, 0xC0),
        0x2594 => c.rect(0, 0, w, ey(1)),
        0x2595 => c.rect(ex(7), 0, w, h),
        // Quadrants ▖▗▘▙▚▛▜▝▞▟ as bits: upper left, upper right, lower left, lower right.
        _ => {
            let q = match cp {
                0x2596 => 0b0100,
                0x2597 => 0b1000,
                0x2598 => 0b0001,
                0x2599 => 0b1101,
                0x259A => 0b1001,
                0x259B => 0b0111,
                0x259C => 0b1011,
                0x259D => 0b0010,
                0x259E => 0b0110,
                _ => 0b1110, // 259F
            };
            if q & 1 != 0 {
                c.rect(0, 0, mx, my);
            }
            if q & 2 != 0 {
                c.rect(mx, 0, w, my);
            }
            if q & 4 != 0 {
                c.rect(0, my, mx, h);
            }
            if q & 8 != 0 {
                c.rect(mx, my, w, h);
            }
        }
    }
}

// ---- braille -----------------------------------------------------------------

/// Two columns of four dots, evenly spaced across cells so plots line up.
fn braille(c: &mut Canvas, bits: u8) {
    let (w, h) = (c.w, c.h);
    let xs = [0, Canvas::eighth(4, w), w];
    let ys = [0, Canvas::eighth(2, h), Canvas::eighth(4, h), Canvas::eighth(6, h), h];
    let d = ((w / 2).min(h / 4) / 2).max(1);
    // Dot n (1–8) → bit n-1 at (column, row).
    const DOTS: [(usize, usize); 8] = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)];
    for (bit, &(col, row)) in DOTS.iter().enumerate() {
        if bits & (1 << bit) == 0 {
            continue;
        }
        let (sx0, sx1, sy0, sy1) = (xs[col], xs[col + 1], ys[row], ys[row + 1]);
        if d >= 4 {
            let centre = ((sx0 + sx1) as f64 / 2.0, (sy0 + sy1) as f64 / 2.0);
            c.disc(centre, d as f64 / 2.0);
        } else {
            let x = sx0 + (sx1 - sx0 - d) / 2;
            let y = sy0 + (sy1 - sy0 - d) / 2;
            c.rect(x, y, x + d, y + d);
        }
    }
}

// ---- Powerline ---------------------------------------------------------------

fn powerline(c: &mut Canvas, cp: u32) {
    use std::f64::consts::FRAC_PI_2;
    let (w, h, t) = (c.w as f64, c.h as f64, c.t as f64);
    let mid = h / 2.0;
    let half = t / 2.0;
    match cp {
        0xE0A0 => {
            // Version-control branch: a trunk and a branch curving into it.
            let x = w * 0.3;
            c.stroke(&[(x, h * 0.12), (x, h * 0.88)], t * 1.5);
            let node = (w * 0.72, h * 0.3);
            c.disc(node, (w * 0.14).max(t));
            let mut curve = Vec::with_capacity(17);
            for i in 0..=16 {
                // Quadratic Bézier from the node down into the trunk.
                let s = i as f64 / 16.0;
                let (p0, p1, p2) = (node, (w * 0.72, h * 0.6), (x, h * 0.72));
                let u = 1.0 - s;
                curve.push((
                    u * u * p0.0 + 2.0 * u * s * p1.0 + s * s * p2.0,
                    u * u * p0.1 + 2.0 * u * s * p1.1 + s * s * p2.1,
                ));
            }
            c.stroke(&curve, t * 1.5);
        }
        0xE0A1 | 0xE0A3 => {
            // "LN" (line number) and "CN" (column number), small letters.
            let (y0, y1) = (h * 0.25, h * 0.75);
            if cp == 0xE0A1 {
                c.stroke(&[(w * 0.12, y0), (w * 0.12, y1), (w * 0.42, y1)], t);
            } else {
                let mut letter = Vec::with_capacity(33);
                let r = w * 0.17;
                arc((w * 0.3, mid), r, (y1 - y0) / 2.0, -FRAC_PI_2 * 0.6, -FRAC_PI_2 * 3.4, &mut letter);
                c.stroke(&letter, t);
            }
            c.stroke(&[(w * 0.58, y1), (w * 0.58, y0), (w * 0.88, y1), (w * 0.88, y0)], t);
        }
        0xE0A2 => {
            // Padlock: a body and a shackle.
            let (x0, x1, y0, y1) = (w * 0.15, w * 0.85, h * 0.45, h * 0.85);
            c.polygon(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
            let r = w * 0.22;
            let mut shackle = vec![(w * 0.5 - r, y0)];
            arc(
                (w * 0.5, h * 0.3),
                r,
                r,
                std::f64::consts::PI,
                2.0 * std::f64::consts::PI,
                &mut shackle,
            );
            shackle.push((w * 0.5 + r, y0));
            c.stroke(&shackle, t * 1.5);
        }
        //  solid right-pointing triangle
        0xE0B0 => c.polygon(&[(0.0, 0.0), (w, mid), (0.0, h)]),
        //  thin right-pointing chevron
        0xE0B1 => c.stroke(&[(0.0, 0.0), (w - half, mid), (0.0, h)], t),
        //  solid left-pointing triangle
        0xE0B2 => c.polygon(&[(w, 0.0), (0.0, mid), (w, h)]),
        //  thin left-pointing chevron
        0xE0B3 => c.stroke(&[(w, 0.0), (half, mid), (w, h)], t),
        //  solid right half circle
        0xE0B4 => c.paint(|x, y| (x / w).powi(2) + ((y - mid) / mid).powi(2) <= 1.0),
        //  thin right half circle
        0xE0B5 => {
            let mut pts = Vec::with_capacity(33);
            arc((0.0, mid), w - half, mid - half, -FRAC_PI_2, FRAC_PI_2, &mut pts);
            c.stroke(&pts, t);
        }
        //  solid left half circle
        0xE0B6 => c.paint(|x, y| ((w - x) / w).powi(2) + ((y - mid) / mid).powi(2) <= 1.0),
        //  thin left half circle
        0xE0B7 => {
            let mut pts = Vec::with_capacity(33);
            arc((w, mid), w - half, mid - half, FRAC_PI_2 * 3.0, FRAC_PI_2, &mut pts);
            c.stroke(&pts, t);
        }
        //  lower left triangle
        0xE0B8 => c.polygon(&[(0.0, 0.0), (w, h), (0.0, h)]),
        //  lower right triangle
        0xE0BA => c.polygon(&[(w, 0.0), (w, h), (0.0, h)]),
        //  upper left triangle
        0xE0BC => c.polygon(&[(0.0, 0.0), (w, 0.0), (0.0, h)]),
        //  upper right triangle
        0xE0BE => c.polygon(&[(0.0, 0.0), (w, 0.0), (w, h)]),
        //   backslash separators
        0xE0B9 | 0xE0BF => c.stroke(&[(0.0, 0.0), (w, h)], t),
        //   forward slash separators
        0xE0BB | 0xE0BD => c.stroke(&[(0.0, h), (w, 0.0)], t),
        _ => {}
    }
}

// ---- legacy computing ----------------------------------------------------------

/// 🬀–🬻: 2×3 blocks. Index n skips the four patterns Unicode has elsewhere
/// (empty, left half, right half, full).
fn sextant(c: &mut Canvas, n: u32) {
    let mut bits = n + 1;
    if bits >= 21 {
        bits += 1;
    }
    if bits >= 42 {
        bits += 1;
    }
    let (w, h) = (c.w, c.h);
    let xs = [0, Canvas::eighth(4, w), w];
    let third = |k: i32| (2 * h * k + 3) / 6;
    let ys = [0, third(1), third(2), h];
    for i in 0..6 {
        if bits & (1 << i) != 0 {
            let (col, row) = ((i % 2) as usize, (i / 2) as usize);
            c.rect(xs[col], ys[row], xs[col + 1], ys[row + 1]);
        }
    }
}

/// 🬼–🭧: the part of the cell on one side of a line between points on a
/// grid of halves across and thirds down.
fn wedge(c: &mut Canvas, cp: u32) {
    // Points: x in halves, y in thirds.
    type P = (u8, u8);
    const UL: (u8, u8) = (0, 0);
    const UC: (u8, u8) = (1, 0);
    const UR: (u8, u8) = (2, 0);
    const UML: (u8, u8) = (0, 1);
    const UMR: (u8, u8) = (2, 1);
    const LML: (u8, u8) = (0, 2);
    const LMR: (u8, u8) = (2, 2);
    const LL: (u8, u8) = (0, 3);
    const LC: (u8, u8) = (1, 3);
    const LR: (u8, u8) = (2, 3);
    // (filled corner, from, to)
    const WEDGES: [(P, P, P); 44] = [
        (LL, LML, LC),  // 1FB3C
        (LL, LML, LR),  // 1FB3D
        (LL, UML, LC),  // 1FB3E
        (LL, UML, LR),  // 1FB3F
        (LL, UL, LC),   // 1FB40
        (LR, UML, UC),  // 1FB41
        (LR, UML, UR),  // 1FB42
        (LR, LML, UC),  // 1FB43
        (LR, LML, UR),  // 1FB44
        (LR, LL, UC),   // 1FB45
        (LR, LML, UMR), // 1FB46
        (LR, LC, LMR),  // 1FB47
        (LR, LL, LMR),  // 1FB48
        (LR, LC, UMR),  // 1FB49
        (LR, LL, UMR),  // 1FB4A
        (LR, LC, UR),   // 1FB4B
        (LL, UC, UMR),  // 1FB4C
        (LL, UL, UMR),  // 1FB4D
        (LL, UC, LMR),  // 1FB4E
        (LL, UL, LMR),  // 1FB4F
        (LL, UC, LR),   // 1FB50
        (LL, UML, LMR), // 1FB51
        (UR, LML, LC),  // 1FB52
        (UR, LML, LR),  // 1FB53
        (UR, UML, LC),  // 1FB54
        (UR, UML, LR),  // 1FB55
        (UR, UL, LC),   // 1FB56
        (UL, UML, UC),  // 1FB57
        (UL, UML, UR),  // 1FB58
        (UL, LML, UC),  // 1FB59
        (UL, LML, UR),  // 1FB5A
        (UL, LL, UC),   // 1FB5B
        (UL, LML, UMR), // 1FB5C
        (UL, LC, LMR),  // 1FB5D
        (UL, LL, LMR),  // 1FB5E
        (UL, LC, UMR),  // 1FB5F
        (UL, LL, UMR),  // 1FB60
        (UL, LC, UR),   // 1FB61
        (UR, UC, UMR),  // 1FB62
        (UR, UL, UMR),  // 1FB63
        (UR, UC, LMR),  // 1FB64
        (UR, UL, LMR),  // 1FB65
        (UR, UC, LR),   // 1FB66
        (UR, UML, LMR), // 1FB67
    ];
    let (corner, a, b) = WEDGES[(cp - 0x1FB3C) as usize];
    let (w, h) = (c.w as f64, c.h as f64);
    let p = |(x, y): (u8, u8)| (x as f64 * w / 2.0, y as f64 * h / 3.0);
    let (corner, a, b) = (p(corner), p(a), p(b));
    let side = |(x, y): Pt| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
    let want = side(corner) > 0.0;
    c.paint(|x, y| (side((x, y)) > 0.0) == want);
}

/// 🭨–🭯: triangles meeting at the centre of the cell, or the cell without one.
fn triangles(c: &mut Canvas, cp: u32) {
    let (w, h) = (c.w as f64, c.h as f64);
    let centre = (w / 2.0, h / 2.0);
    let tri = |i: u32| match i {
        0 => [(0.0, 0.0), centre, (0.0, h)], // left
        1 => [(0.0, 0.0), (w, 0.0), centre], // upper
        2 => [(w, 0.0), (w, h), centre],     // right
        _ => [(0.0, h), (w, h), centre],     // lower
    };
    if cp >= 0x1FB6C {
        let t = tri(cp - 0x1FB6C);
        c.polygon(&t);
    } else {
        let t = tri(cp - 0x1FB68);
        c.paint(|x, y| !in_polygon(&t, (x, y)));
    }
}

/// 🭰–🮋: one-eighth bars and the upper/right blocks block elements lack.
fn eighths(c: &mut Canvas, cp: u32) {
    let (w, h) = (c.w, c.h);
    let ex = |n| Canvas::eighth(n, w);
    let ey = |n| Canvas::eighth(n, h);
    match cp {
        // Vertical one-eighth block 2–7.
        0x1FB70..=0x1FB75 => {
            let n = (cp - 0x1FB70) as i32 + 1;
            c.rect(ex(n), 0, ex(n + 1).max(ex(n) + 1), h);
        }
        // Horizontal one-eighth block 2–7.
        0x1FB76..=0x1FB7B => {
            let n = (cp - 0x1FB76) as i32 + 1;
            c.rect(0, ey(n), w, ey(n + 1).max(ey(n) + 1));
        }
        0x1FB7C => {
            c.rect(0, 0, ex(1), h);
            c.rect(0, ey(7), w, h);
        }
        0x1FB7D => {
            c.rect(0, 0, ex(1), h);
            c.rect(0, 0, w, ey(1));
        }
        0x1FB7E => {
            c.rect(ex(7), 0, w, h);
            c.rect(0, 0, w, ey(1));
        }
        0x1FB7F => {
            c.rect(ex(7), 0, w, h);
            c.rect(0, ey(7), w, h);
        }
        0x1FB80 => {
            c.rect(0, 0, w, ey(1));
            c.rect(0, ey(7), w, h);
        }
        0x1FB81 => {
            for n in [0, 2, 4, 7] {
                c.rect(0, ey(n), w, ey(n + 1).max(ey(n) + 1));
            }
        }
        // Upper 2, 3, 5, 6, 7 eighths.
        0x1FB82..=0x1FB86 => {
            let n = [2, 3, 5, 6, 7][(cp - 0x1FB82) as usize];
            c.rect(0, 0, w, ey(n));
        }
        // Right 2, 3, 5, 6, 7 eighths.
        _ => {
            let n = [2, 3, 5, 6, 7][(cp - 0x1FB87) as usize];
            c.rect(ex(8 - n), 0, w, h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(ch: char, w: i32, h: i32, t: i32) -> Canvas {
        let mut c = Canvas::new(w, h, t);
        draw(&mut c, ch as u32);
        c
    }

    fn col(c: &Canvas, x: i32) -> Vec<u8> {
        (0..c.h).map(|y| c.px[(y * c.w + x) as usize]).collect()
    }

    fn row(c: &Canvas, y: i32) -> Vec<u8> {
        c.px[(y * c.w) as usize..((y + 1) * c.w) as usize].to_vec()
    }

    const SIZES: [(i32, i32, i32); 6] = [(7, 15, 1), (8, 17, 1), (16, 34, 2), (19, 41, 2), (10, 21, 3), (9, 20, 2)];

    /// Every arm of every line character ends at the cell edge with exactly
    /// the pixels of the straight line of its weight, so any two neighbours
    /// join without a seam.
    #[test]
    fn arms_match_straight_lines_at_every_edge() {
        let straight = |wt: W, vertical: bool| match (wt, vertical) {
            (W::Light, false) => '─',
            (W::Heavy, false) => '━',
            (W::Double, false) => '═',
            (W::Light, true) => '│',
            (W::Heavy, true) => '┃',
            (W::Double, true) => '║',
            (W::None, _) => ' ',
        };
        for (w, h, t) in SIZES {
            let lines: Vec<u32> = (0x2500..=0x257F).filter(|&cp| arms(cp).is_some()).chain(0x256D..=0x2570).collect();
            for cp in lines {
                let ch = char::from_u32(cp).unwrap();
                let a = arms(cp).unwrap_or(match cp {
                    0x256D => Arms {
                        up: W::None,
                        right: W::Light,
                        down: W::Light,
                        left: W::None,
                    },
                    0x256E => Arms {
                        up: W::None,
                        right: W::None,
                        down: W::Light,
                        left: W::Light,
                    },
                    0x256F => Arms {
                        up: W::Light,
                        right: W::None,
                        down: W::None,
                        left: W::Light,
                    },
                    _ => Arms {
                        up: W::Light,
                        right: W::Light,
                        down: W::None,
                        left: W::None,
                    },
                });
                let c = canvas(ch, w, h, t);
                let edge = |wt: W, vertical: bool, f: &dyn Fn(&Canvas) -> Vec<u8>| {
                    if wt == W::None {
                        vec![0; f(&c).len()]
                    } else {
                        f(&canvas(straight(wt, vertical), w, h, t))
                    }
                };
                assert_eq!(col(&c, 0), edge(a.left, false, &|s| col(s, 0)), "{ch} left edge at {w}x{h}");
                assert_eq!(
                    col(&c, w - 1),
                    edge(a.right, false, &|s| col(s, w - 1)),
                    "{ch} right edge at {w}x{h}"
                );
                assert_eq!(row(&c, 0), edge(a.up, true, &|s| row(s, 0)), "{ch} top edge at {w}x{h}");
                assert_eq!(
                    row(&c, h - 1),
                    edge(a.down, true, &|s| row(s, h - 1)),
                    "{ch} bottom edge at {w}x{h}"
                );
            }
        }
    }

    /// The gap between the strokes of a double line stays open where a line
    /// runs into it from the side, and closes where the line crosses it.
    #[test]
    fn double_line_junctions() {
        let (w, h, t) = (16, 34, 2);
        let d0 = Canvas::band(h, 3 * t);
        let e0 = Canvas::band(w, 3 * t);
        let at = |c: &Canvas, x: i32, y: i32| c.px[(y * w + x) as usize];
        // ╠: the right stroke breaks for the arm; the gap between the
        // horizontal strokes is open all the way to the left stroke.
        let c = canvas('╠', w, h, t);
        assert_eq!(at(&c, e0 + 2 * t, d0 + t), 0, "╠ right stroke open at the gap");
        assert_eq!(at(&c, e0 + t, d0 + t), 0, "╠ gap open between the strokes");
        assert_eq!(at(&c, e0, d0 + t), 255, "╠ left stroke continuous");
        // ╬: the centre is empty.
        let c = canvas('╬', w, h, t);
        assert_eq!(at(&c, e0 + t, d0 + t), 0);
        // ╪: the single vertical line crosses the double horizontal one.
        let c = canvas('╪', w, h, t);
        let x = Canvas::band(w, t);
        assert!((0..h).all(|y| at(&c, x, y) == 255), "╪ vertical continuous");
    }
}
