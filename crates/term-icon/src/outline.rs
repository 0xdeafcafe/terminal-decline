//! The initial's outline (pins design §8.3): Core Text shapes the initial
//! in the bold system font, its glyph outlines are fitted into the
//! template's box and written as SVG path data, top-down in the 1024 grid.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFArray, CFAttributedString, CFDictionary, CFRange, CFRetained, CFString, CGAffineTransform, CGPoint, CGRect};
use objc2_core_graphics::{CGGlyph, CGMutablePath, CGPath, CGPathElement, CGPathElementType};
use objc2_core_text::{CTFont, CTFontUIFontType, CTLine, CTRun, kCTFontAttributeName};

use crate::IconError;

/// The box the initial's ink is fitted into: 400 × 360 centred at
/// (512, 445), top-down.
pub(crate) const FIT_W: f64 = 400.0;
pub(crate) const FIT_H: f64 = 360.0;
pub(crate) const FIT_CX: f64 = 512.0;
pub(crate) const FIT_CY: f64 = 445.0;

/// Shaping size. Below 28 pt the fitted outline drifts from the template
/// by ~0.6 px (hinting-free, but rounded positions); 100 pt matches it.
const SIZE: f64 = 100.0;

/// One segment of a path, in whatever coordinates it was made in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Seg {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// The outline of `initial`, fitted proportionally into 400 × 360 at
/// (512, 445), as SVG path data, top-down, three decimals.
///
/// Shaped as one line in the bold system font (`.SFNS-Bold`, SF Pro Bold)
/// with a fixed language, so the fallback for scripts SF Pro lacks
/// doesn't depend on the user's locale. A glyph without an outline (a
/// colour emoji) contributes nothing; an initial with no outline at all is
/// [`IconError::NoOutline`].
pub fn outline(initial: &str) -> Result<String, IconError> {
    let segs = shape(initial).ok_or_else(|| IconError::NoOutline(initial.to_owned()))?;
    let (x0, y0, x1, y1) = ink_bounds(&segs).ok_or_else(|| IconError::NoOutline(initial.to_owned()))?;
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return Err(IconError::NoOutline(initial.to_owned()));
    }
    let s = (FIT_W / w).min(FIT_H / h);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    // Core Text is y-up; the grid is top-down.
    let map = |x: f64, y: f64| (FIT_CX + (x - cx) * s, FIT_CY - (y - cy) * s);
    let fitted: Vec<Seg> = segs
        .iter()
        .map(|&seg| match seg {
            Seg::Move(x, y) => {
                let (x, y) = map(x, y);
                Seg::Move(x, y)
            }
            Seg::Line(x, y) => {
                let (x, y) = map(x, y);
                Seg::Line(x, y)
            }
            Seg::Quad(a, b, x, y) => {
                let ((a, b), (x, y)) = (map(a, b), map(x, y));
                Seg::Quad(a, b, x, y)
            }
            Seg::Cubic(a, b, c, d, x, y) => {
                let ((a, b), (c, d), (x, y)) = (map(a, b), map(c, d), map(x, y));
                Seg::Cubic(a, b, c, d, x, y)
            }
            Seg::Close => Seg::Close,
        })
        .collect();
    Ok(format(&fitted))
}

/// The glyph outlines of `text` shaped as one line, y-up, or `None` when
/// no glyph has one.
fn shape(text: &str) -> Option<Vec<Seg>> {
    unsafe {
        let font = CTFont::new_ui_font_for_language(CTFontUIFontType::EmphasizedSystem, SIZE, Some(&CFString::from_str("en")))?;
        let s = CFString::from_str(text);
        let attrs = CFDictionary::<CFString, CTFont>::from_slices(&[kCTFontAttributeName], &[&font]);
        let attributed = CFAttributedString::new(None, Some(&s), Some(attrs.as_opaque()))?;
        let line = CTLine::with_attributed_string(&attributed);
        let runs: CFRetained<CFArray> = line.glyph_runs();
        let path: CFRetained<CGMutablePath> = CGMutablePath::new();
        for r in 0..runs.count() {
            let run: &CTRun = &*(runs.value_at_index(r) as *const CTRun);
            let run_attrs = run.attributes();
            let run_font = run_attrs.value(kCTFontAttributeName as *const CFString as *const c_void) as *const CTFont;
            let run_font: &CTFont = if run_font.is_null() { &font } else { &*run_font };
            let n = run.glyph_count() as usize;
            if n == 0 {
                continue;
            }
            let mut glyphs: Vec<CGGlyph> = vec![0; n];
            let mut positions = vec![CGPoint::default(); n];
            let all = CFRange { location: 0, length: 0 };
            run.glyphs(all, NonNull::new_unchecked(glyphs.as_mut_ptr()));
            run.positions(all, NonNull::new_unchecked(positions.as_mut_ptr()));
            for (g, p) in glyphs.iter().zip(&positions) {
                let at = CGAffineTransform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    tx: p.x,
                    ty: p.y,
                };
                if let Some(glyph_path) = run_font.path_for_glyph(*g, &at) {
                    CGMutablePath::add_path(Some(&path), std::ptr::null(), Some(&glyph_path));
                }
            }
        }
        let segs = segments(&path);
        if segs.iter().any(|s| !matches!(s, Seg::Move(..) | Seg::Close)) {
            Some(segs)
        } else {
            None
        }
    }
}

/// A CGPath's segments.
pub(crate) fn segments(path: &CGPath) -> Vec<Seg> {
    unsafe extern "C-unwind" fn visit(info: *mut c_void, element: NonNull<CGPathElement>) {
        let out = unsafe { &mut *(info as *mut Vec<Seg>) };
        let e = unsafe { element.as_ref() };
        let p = |i: usize| unsafe { *e.points.as_ptr().add(i) };
        out.push(match e.r#type {
            CGPathElementType::MoveToPoint => Seg::Move(p(0).x, p(0).y),
            CGPathElementType::AddLineToPoint => Seg::Line(p(0).x, p(0).y),
            CGPathElementType::AddQuadCurveToPoint => Seg::Quad(p(0).x, p(0).y, p(1).x, p(1).y),
            CGPathElementType::AddCurveToPoint => Seg::Cubic(p(0).x, p(0).y, p(1).x, p(1).y, p(2).x, p(2).y),
            _ => Seg::Close,
        });
    }
    let mut out: Vec<Seg> = Vec::new();
    unsafe { CGPath::apply(Some(path), &mut out as *mut Vec<Seg> as *mut c_void, Some(visit)) };
    out
}

/// The tight bounds of the ink (curves' extremes, not their control
/// points), as CoreGraphics and SVG's bounding box work them out.
pub(crate) fn ink_bounds(segs: &[Seg]) -> Option<(f64, f64, f64, f64)> {
    let path = to_cgpath(segs);
    let r: CGRect = CGPath::path_bounding_box(Some(&path));
    if r.size.width.is_finite() && r.size.height.is_finite() && !r.origin.x.is_infinite() && (r.size.width > 0.0 || r.size.height > 0.0) {
        Some((r.origin.x, r.origin.y, r.origin.x + r.size.width, r.origin.y + r.size.height))
    } else {
        None
    }
}

pub(crate) fn to_cgpath(segs: &[Seg]) -> CFRetained<CGMutablePath> {
    let path = CGMutablePath::new();
    let m = Some(&*path);
    let t = std::ptr::null();
    unsafe {
        for &s in segs {
            match s {
                Seg::Move(x, y) => CGMutablePath::move_to_point(m, t, x, y),
                Seg::Line(x, y) => CGMutablePath::add_line_to_point(m, t, x, y),
                Seg::Quad(a, b, x, y) => CGMutablePath::add_quad_curve_to_point(m, t, a, b, x, y),
                Seg::Cubic(a, b, c, d, x, y) => CGMutablePath::add_curve_to_point(m, t, a, b, c, d, x, y),
                Seg::Close => CGMutablePath::close_subpath(m),
            }
        }
    }
    path
}

/// SVG path data in the template's style: `M367.925 625.000L…Z`.
pub(crate) fn format(segs: &[Seg]) -> String {
    use std::fmt::Write;
    // `+ 0.0` turns -0.0 into 0.0, so no "-0.000".
    let n = |v: f64| (v * 1000.0).round() / 1000.0 + 0.0;
    let mut out = String::new();
    for &s in segs {
        let _ = match s {
            Seg::Move(x, y) => write!(out, "M{:.3} {:.3}", n(x), n(y)),
            Seg::Line(x, y) => write!(out, "L{:.3} {:.3}", n(x), n(y)),
            Seg::Quad(a, b, x, y) => write!(out, "Q{:.3} {:.3} {:.3} {:.3}", n(a), n(b), n(x), n(y)),
            Seg::Cubic(a, b, c, d, x, y) => {
                write!(out, "C{:.3} {:.3} {:.3} {:.3} {:.3} {:.3}", n(a), n(b), n(c), n(d), n(x), n(y))
            }
            Seg::Close => write!(out, "Z"),
        };
    }
    out
}

/// Reads back path data [`format`] wrote: absolute `M`, `L`, `Q`, `C` and
/// `Z` with plain decimal numbers. Anything else is refused.
pub(crate) fn parse(data: &str) -> Result<Vec<Seg>, IconError> {
    let bad = |why: &str| IconError::BadOutline(why.to_owned());
    let mut out = Vec::new();
    let mut nums: Vec<f64> = Vec::new();
    let mut cmd: Option<char> = None;
    let flush = |cmd: Option<char>, nums: &mut Vec<f64>, out: &mut Vec<Seg>| -> Result<(), IconError> {
        let Some(c) = cmd else {
            return if nums.is_empty() {
                Ok(())
            } else {
                Err(bad("numbers before a command"))
            };
        };
        let n = nums.as_slice();
        let seg = match (c, n.len()) {
            ('M', 2) => Seg::Move(n[0], n[1]),
            ('L', 2) => Seg::Line(n[0], n[1]),
            ('Q', 4) => Seg::Quad(n[0], n[1], n[2], n[3]),
            ('C', 6) => Seg::Cubic(n[0], n[1], n[2], n[3], n[4], n[5]),
            ('Z', 0) => Seg::Close,
            _ => return Err(bad(&format!("{c} with {} numbers", n.len()))),
        };
        if !matches!(seg, Seg::Move(..)) && out.is_empty() {
            return Err(bad("doesn't start with M"));
        }
        out.push(seg);
        nums.clear();
        Ok(())
    };
    let mut chars = data.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        match c {
            'M' | 'L' | 'Q' | 'C' | 'Z' => {
                flush(cmd, &mut nums, &mut out)?;
                cmd = Some(c);
                chars.next();
            }
            ' ' | ',' => {
                chars.next();
            }
            '-' | '.' | '0'..='9' => {
                let mut end = i;
                while let Some(&(j, d)) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' || (d == '-' && j == i) {
                        end = j + d.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                let v: f64 = data[i..end].parse().map_err(|_| bad(&data[i..end]))?;
                if !v.is_finite() {
                    return Err(bad("not a finite number"));
                }
                nums.push(v);
            }
            other => return Err(bad(&format!("unexpected {other:?}"))),
        }
    }
    flush(cmd, &mut nums, &mut out)?;
    if out.is_empty() {
        return Err(bad("empty"));
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn format_and_parse_round_trip() {
        let segs = vec![
            Seg::Move(1.0, -2.5),
            Seg::Line(3.25, 4.0),
            Seg::Quad(1.0, 2.0, 3.0, 4.0),
            Seg::Cubic(1.0, 2.0, 3.0, 4.0, 5.0, 6.0),
            Seg::Close,
        ];
        let s = format(&segs);
        assert_eq!(
            s,
            "M1.000 -2.500L3.250 4.000Q1.000 2.000 3.000 4.000C1.000 2.000 3.000 4.000 5.000 6.000Z"
        );
        assert_eq!(parse(&s).unwrap(), segs);
    }

    #[test]
    fn parse_refuses_what_it_did_not_write() {
        for bad in ["", "L1 2", "M1", "M1 2 3", "A1 2 3 4 5 6 7", "M1 2<script>", "M1e999 2", "M1 2Z3"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn template_r_parses() {
        let svg = include_str!("../../../docs/brand/pin-template.svg");
        let d = template_initial(svg);
        let segs = parse(d).unwrap();
        let (x0, y0, x1, y1) = ink_bounds(&segs).unwrap();
        assert!((x0 - 367.925).abs() < 0.001 && (x1 - 656.075).abs() < 0.001, "{x0} {x1}");
        assert!((y0 - 265.0).abs() < 0.001 && (y1 - 625.0).abs() < 0.001, "{y0} {y1}");
    }

    /// The first foreground path's data in the template.
    pub(crate) fn template_initial(svg: &str) -> &str {
        let mark = &svg[svg.find("id=\"03-mark\"").unwrap()..];
        let d = &mark[mark.find("<path d=\"").unwrap() + 9..];
        &d[..d.find('"').unwrap()]
    }
}
