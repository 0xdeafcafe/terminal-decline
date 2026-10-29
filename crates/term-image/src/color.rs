//! Colour conversion to Display P3, done once when an image is decoded
//! (Alex's decision, images.md §12 question 9): the window's layer stays
//! untagged, so the display shows pixel values in its own space (P3 on
//! Macs since 2016); an sRGB image shown as is would look oversaturated.
//!
//! [`ToP3`] maps 8-bit values in a source space to 8-bit Display P3 (which
//! uses the sRGB transfer curve): decode the source curve through a table
//! to 16-bit linear light, apply a 3 × 3 matrix in fixed point, encode
//! through a table. About 3 ns a pixel, no floating point per pixel.
//!
//! Source spaces: sRGB (images without a profile), or an ICC profile of
//! the matrix/TRC kind (`rXYZ`, `gXYZ`, `bXYZ` and `rTRC` as `curv` or
//! `para`), which covers sRGB, Display P3 (no conversion), Adobe RGB and
//! what cameras and phones write. Other profiles (LUT-based, CMYK, grey)
//! are treated as sRGB.

use std::sync::OnceLock;

/// Fixed-point fraction bits of the matrix.
const Q: u32 = 12;
/// Linear values are 0..=65535; the encode table is indexed by `>> 2`.
const ENCODE_SHIFT: u32 = 2;
const ENCODE_LEN: usize = (65536 >> ENCODE_SHIFT) as usize;

/// A conversion from one RGB space to Display P3.
pub struct ToP3 {
    /// Source value → linear light, 0..=65535.
    decode: [u16; 256],
    /// Linear source → linear P3, rows R, G, B, in `Q` fixed point.
    matrix: [[i32; 3]; 3],
    /// Linear P3 `>> ENCODE_SHIFT` → value, the sRGB curve.
    encode: Box<[u8; ENCODE_LEN]>,
    identity: bool,
}

impl std::fmt::Debug for ToP3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToP3")
            .field("matrix", &self.matrix)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

type M3 = [[f64; 3]; 3];

fn mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn mul_v(a: &M3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| (0..3).map(|k| a[i][k] * v[k]).sum())
}

fn inv(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let r = [
        [e * i - f * h, c * h - b * i, b * f - c * e],
        [f * g - d * i, a * i - c * g, c * d - a * f],
        [d * h - e * g, b * g - a * h, a * e - b * d],
    ];
    r.map(|row| row.map(|v| v / det))
}

/// RGB → XYZ for primaries and white given as xy.
fn rgb_to_xyz(prims: [(f64, f64); 3], white: (f64, f64)) -> M3 {
    let xyz = |(x, y): (f64, f64)| [x / y, 1.0, (1.0 - x - y) / y];
    let [r, g, b] = prims.map(xyz);
    let p = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let s = mul_v(&inv(&p), xyz(white));
    p.map(|row| [row[0] * s[0], row[1] * s[1], row[2] * s[2]])
}

const D65: (f64, f64) = (0.3127, 0.3290);
const SRGB: [(f64, f64); 3] = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
const P3: [(f64, f64); 3] = [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)];

/// Bradford adaptation from D65 to D50 (ICC's connection space).
fn d65_to_d50() -> M3 {
    let ma = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    let src = mul_v(&ma, [0.95047, 1.0, 1.08883]);
    let dst = mul_v(&ma, [0.96422, 1.0, 0.82521]);
    let d = [
        [dst[0] / src[0], 0.0, 0.0],
        [0.0, dst[1] / src[1], 0.0],
        [0.0, 0.0, dst[2] / src[2]],
    ];
    mul(&inv(&ma), &mul(&d, &ma))
}

/// The sRGB transfer curve, encoded value 0..=1 → linear.
fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(l: f64) -> f64 {
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// A source transfer curve.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Curve {
    Srgb,
    /// ICC `para`: function type and up to seven parameters.
    Para(u16, [f64; 7]),
}

impl Curve {
    fn eval(&self, x: f64) -> f64 {
        match *self {
            Curve::Srgb => srgb_to_linear(x),
            Curve::Para(kind, [g, a, b, c, d, e, f]) => {
                let pow = |v: f64| if v > 0.0 { v.powf(g) } else { 0.0 };
                match kind {
                    0 => pow(x),
                    1 => {
                        if x >= -b / a {
                            pow(a * x + b)
                        } else {
                            0.0
                        }
                    }
                    2 => {
                        if x >= -b / a {
                            pow(a * x + b) + c
                        } else {
                            c
                        }
                    }
                    3 => {
                        if x >= d {
                            pow(a * x + b)
                        } else {
                            c * x
                        }
                    }
                    _ => {
                        if x >= d {
                            pow(a * x + b) + e
                        } else {
                            c * x + f
                        }
                    }
                }
            }
        }
    }
}

impl ToP3 {
    fn new(curve: Curve, matrix: M3) -> ToP3 {
        let decode = std::array::from_fn(|i| {
            let l = curve.eval(i as f64 / 255.0);
            (l.clamp(0.0, 1.0) * 65535.0).round() as u16
        });
        let mut encode = Box::new([0u8; ENCODE_LEN]);
        for (i, v) in encode.iter_mut().enumerate() {
            // The middle of the bucket.
            let l = ((i << ENCODE_SHIFT) as f64 + ((1 << ENCODE_SHIFT) as f64 - 1.0) / 2.0) / 65535.0;
            *v = (linear_to_srgb(l.min(1.0)) * 255.0).round() as u8;
        }
        // Already P3: the matrix is the identity and the curve is sRGB's
        // (within what a profile's fixed-point numbers can say).
        let unit = (0..3).all(|i| (0..3).all(|j| (matrix[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 0.002));
        let srgb_curve = (0..256).all(|i| {
            let s = (srgb_to_linear(i as f64 / 255.0) * 65535.0).round() as i32;
            (decode[i] as i32 - s).abs() <= 32
        });
        let identity = unit && srgb_curve;
        let matrix = matrix.map(|row| row.map(|v| (v * (1 << Q) as f64).round() as i32));
        ToP3 {
            decode,
            matrix,
            encode,
            identity,
        }
    }

    /// sRGB to Display P3 (for images with no profile, kitty's raw pixels
    /// and Sixel colours).
    pub fn srgb() -> &'static ToP3 {
        static SRGB_TO_P3: OnceLock<ToP3> = OnceLock::new();
        SRGB_TO_P3.get_or_init(|| {
            let m = mul(&inv(&rgb_to_xyz(P3, D65)), &rgb_to_xyz(SRGB, D65));
            ToP3::new(Curve::Srgb, m)
        })
    }

    /// From the space an ICC profile describes; `None` if the profile
    /// isn't one this understands (the caller then uses [`ToP3::srgb`]).
    pub fn from_icc(profile: &[u8]) -> Option<ToP3> {
        let icc = Icc::parse(profile)?;
        let m_src = [
            [icc.r[0], icc.g[0], icc.b[0]],
            [icc.r[1], icc.g[1], icc.b[1]],
            [icc.r[2], icc.g[2], icc.b[2]],
        ];
        let p3_d50 = mul(&d65_to_d50(), &rgb_to_xyz(P3, D65));
        let m = mul(&inv(&p3_d50), &m_src);
        if m.iter().flatten().any(|v| !v.is_finite() || v.abs() > 8.0) {
            return None;
        }
        Some(ToP3::new(icc.curve, m))
    }

    /// True if the source is already Display P3: nothing to do.
    pub fn is_identity(&self) -> bool {
        self.identity
    }

    /// One straight (not premultiplied) RGB colour.
    #[inline]
    pub fn rgb(&self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        let lin = [
            self.decode[r as usize] as i32,
            self.decode[g as usize] as i32,
            self.decode[b as usize] as i32,
        ];
        self.matrix.map(|row| {
            let v = (row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2]) >> Q;
            self.encode[(v.clamp(0, 65535) as u32 >> ENCODE_SHIFT) as usize]
        })
    }

    /// Convert premultiplied BGRA pixels in place.
    pub fn bgra_premultiplied(&self, pixels: &mut [u8]) {
        if self.identity {
            return;
        }
        for px in pixels.as_chunks_mut::<4>().0 {
            let a = px[3];
            if a == 255 {
                let [r, g, b] = self.rgb([px[2], px[1], px[0]]);
                *px = [b, g, r, 255];
            } else if a != 0 {
                let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                let [r, g, b] = self.rgb([un(px[2]), un(px[1]), un(px[0])]);
                let pre = crate::premultiply;
                *px = [pre(b, a), pre(g, a), pre(r, a), a];
            }
        }
    }

    /// Convert an opaque BGRA colour (a Sixel register).
    pub fn bgra(&self, [b, g, r, a]: [u8; 4]) -> [u8; 4] {
        let [r, g, b] = self.rgb([r, g, b]);
        [b, g, r, a]
    }
}

/// The sRGB curve as ICC `para` type 3 parameters (for test profiles).
#[cfg(test)]
fn srgb_para() -> [f64; 7] {
    [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045, 0.0, 0.0]
}

/// What [`ToP3::from_icc`] reads from a profile.
struct Icc {
    r: [f64; 3],
    g: [f64; 3],
    b: [f64; 3],
    curve: Curve,
}

impl Icc {
    fn parse(p: &[u8]) -> Option<Icc> {
        let be32 = |at: usize| -> Option<u32> { Some(u32::from_be_bytes(p.get(at..at + 4)?.try_into().ok()?)) };
        let s15 = |at: usize| -> Option<f64> { Some(be32(at)? as i32 as f64 / 65536.0) };
        if p.get(16..20)? != b"RGB " || p.get(36..40)? != b"acsp" {
            return None;
        }
        let count = be32(128)? as usize;
        if count > 100 {
            return None;
        }
        let tag = |sig: &[u8; 4]| -> Option<(usize, usize)> {
            (0..count).find_map(|n| {
                let at = 132 + n * 12;
                (p.get(at..at + 4)? == sig).then(|| Some((be32(at + 4)? as usize, be32(at + 8)? as usize)))?
            })
        };
        let xyz = |sig: &[u8; 4]| -> Option<[f64; 3]> {
            let (at, len) = tag(sig)?;
            if len < 20 || p.get(at..at + 4)? != b"XYZ " {
                return None;
            }
            Some([s15(at + 8)?, s15(at + 12)?, s15(at + 16)?])
        };
        let (r, g, b) = (xyz(b"rXYZ")?, xyz(b"gXYZ")?, xyz(b"bXYZ")?);
        let curve = match tag(b"rTRC") {
            Some((at, _)) => match p.get(at..at + 4)? {
                b"curv" => match be32(at + 8)? {
                    0 => Curve::Para(0, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
                    1 => {
                        let g = u16::from_be_bytes(p.get(at + 12..at + 14)?.try_into().ok()?) as f64 / 256.0;
                        Curve::Para(0, [g, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
                    }
                    // A table: taken as the sRGB curve, which such tables
                    // nearly always are.
                    _ => Curve::Srgb,
                },
                b"para" => {
                    let kind = u16::from_be_bytes(p.get(at + 8..at + 10)?.try_into().ok()?);
                    let n = [1, 3, 4, 5, 7].get(kind as usize).copied()?;
                    let mut params = [0.0; 7];
                    for (i, v) in params.iter_mut().enumerate().take(n) {
                        *v = s15(at + 12 + i * 4)?;
                    }
                    if kind > 0 && params[1] == 0.0 {
                        return None;
                    }
                    Curve::Para(kind, params)
                }
                _ => return None,
            },
            None => return None,
        };
        Some(Icc { r, g, b, curve })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal matrix/TRC ICC profile for primaries `prims` (D65 white,
    /// adapted to D50 as ICC wants) with the sRGB curve as `para` type 3.
    pub fn icc(prims: [(f64, f64); 3]) -> Vec<u8> {
        let m = mul(&d65_to_d50(), &rgb_to_xyz(prims, D65));
        let mut p = vec![0u8; 128];
        p[16..20].copy_from_slice(b"RGB ");
        p[20..24].copy_from_slice(b"XYZ ");
        p[36..40].copy_from_slice(b"acsp");
        let tags: [&[u8; 4]; 4] = [b"rXYZ", b"gXYZ", b"bXYZ", b"rTRC"];
        p.extend_from_slice(&(tags.len() as u32).to_be_bytes());
        let data_at = 128 + 4 + tags.len() * 12;
        let mut data = Vec::new();
        let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
        for (n, sig) in tags.iter().enumerate() {
            let at = data_at + data.len();
            let start = data.len();
            if n < 3 {
                data.extend_from_slice(b"XYZ \0\0\0\0");
                for row in &m {
                    data.extend_from_slice(&fixed(row[n]));
                }
            } else {
                data.extend_from_slice(b"para\0\0\0\0\0\x03\0\0");
                for v in srgb_para().iter().take(5) {
                    data.extend_from_slice(&fixed(*v));
                }
            }
            p.extend_from_slice(*sig);
            p.extend_from_slice(&(at as u32).to_be_bytes());
            p.extend_from_slice(&((data.len() - start) as u32).to_be_bytes());
        }
        p.extend(data);
        p
    }

    #[test]
    fn srgb_to_p3_known_values() {
        let t = ToP3::srgb();
        assert!(!t.is_identity());
        // Neutral stays neutral.
        for v in [0, 1, 17, 128, 200, 255] {
            let [r, g, b] = t.rgb([v, v, v]);
            assert!(r.abs_diff(v) <= 1 && g.abs_diff(v) <= 1 && b.abs_diff(v) <= 1, "{v}: {r} {g} {b}");
        }
        // sRGB primaries inside P3 (reference: 0.9175 0.2003 0.1386, etc.).
        let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 1);
        assert!(near(t.rgb([255, 0, 0]), [234, 51, 35]), "{:?}", t.rgb([255, 0, 0]));
        assert!(near(t.rgb([0, 255, 0]), [117, 251, 76]), "{:?}", t.rgb([0, 255, 0]));
        assert!(near(t.rgb([0, 0, 255]), [0, 0, 245]), "{:?}", t.rgb([0, 0, 255]));
    }

    #[test]
    fn premultiplied_pixels() {
        let t = ToP3::srgb();
        let mut px = [0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 128, 128];
        t.bgra_premultiplied(&mut px);
        let [r, g, b] = t.rgb([255, 0, 0]);
        assert_eq!(&px[..4], &[b, g, r, 255]);
        assert_eq!(&px[4..8], &[0, 0, 0, 0]);
        assert_eq!(px[11], 128);
        assert!(px[10].abs_diff(r / 2) <= 1);
    }

    #[test]
    fn icc_profiles() {
        // A Display P3 profile: nothing to do.
        let p3 = ToP3::from_icc(&icc(P3)).unwrap();
        assert!(p3.is_identity());
        // An sRGB profile: the same as no profile.
        let s = ToP3::from_icc(&icc(SRGB)).unwrap();
        assert!(!s.is_identity());
        for c in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [10, 100, 200]] {
            let (a, b) = (s.rgb(c), ToP3::srgb().rgb(c));
            assert!(a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 1), "{c:?}: {a:?} {b:?}");
        }
        // Garbage and truncation.
        assert!(ToP3::from_icc(b"").is_none());
        assert!(ToP3::from_icc(&[0; 200]).is_none());
        let good = icc(SRGB);
        for n in 0..good.len() {
            let _ = ToP3::from_icc(&good[..n]);
        }
    }
}
