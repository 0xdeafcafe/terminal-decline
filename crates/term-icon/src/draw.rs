//! Drawing a pin icon from its kept record, following
//! `docs/brand/pin-template.svg` (pins design §8.4), and writing it as PNG
//! or `.icns` with ImageIO.
//!
//! Every size is drawn from the vectors (no downscaling), in sRGB, RGBA
//! premultiplied, transparent outside the body. Coordinates are the
//! template's: the 1024 grid, top-down.

use std::ffi::c_void;

use objc2_core_foundation::{
    CFDictionary, CFMutableData, CFNumber, CFRetained, CFString, CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGGradient, CGGradientDrawingOptions, CGImage,
    CGImageAlphaInfo, CGLineCap, CGLineJoin, CGPath, kCGColorSpaceSRGB,
};
use objc2_image_io::{CGImageDestination, kCGImagePropertyDPIHeight, kCGImagePropertyDPIWidth};

use crate::outline::{Seg, ink_bounds, parse, to_cgpath};
use crate::{IconError, PinIcon};

/// The sizes an `.icns` is made from: macOS's 16–512 pt at 1× and 2×.
pub const ICNS_SIZES: [usize; 7] = [16, 32, 64, 128, 256, 512, 1024];

/// The body: 824 × 824 at (100, 100), corner radius 185.
const BODY: (f64, f64, f64, f64, f64) = (100.0, 100.0, 824.0, 824.0, 185.0);
/// The light and rim rect: 818 × 818 at (103, 103), radius 182.
const LIGHT: (f64, f64, f64, f64, f64) = (103.0, 103.0, 818.0, 818.0, 182.0);
/// The Slope maker's mark, `M326 702 L452 814 Q460 822 474 822 H704`,
/// stroked 64 wide with round caps and joins.
const SLOPE: [Seg; 4] = [
    Seg::Move(326.0, 702.0),
    Seg::Line(452.0, 814.0),
    Seg::Quad(460.0, 822.0, 474.0, 822.0),
    Seg::Line(704.0, 822.0),
];
const SLOPE_WIDTH: f64 = 64.0;
/// The Slope's geometry box, which its gradient spans (SVG's
/// objectBoundingBox ignores the stroke).
const SLOPE_BOX: (f64, f64, f64, f64) = (326.0, 702.0, 378.0, 120.0);
/// The foreground ink: `#FFFFFF` → `#B9E5CC`.
const INK: [[f64; 4]; 2] = [[1.0, 1.0, 1.0, 1.0], [185.0 / 255.0, 229.0 / 255.0, 204.0 / 255.0, 1.0]];
const DEPTH_OFFSET: f64 = 8.0;
const DEPTH_OPACITY: f64 = 0.22;
const LIGHT_OPACITY: f64 = 0.06;
const RIM_WIDTH: f64 = 6.0;
const RIM: [(f64, f64); 3] = [(0.0, 0.55), (0.5, 0.06), (1.0, 0.22)];

fn rgb(c: [u8; 3]) -> [f64; 4] {
    [c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0, 1.0]
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect {
        origin: CGPoint { x, y },
        size: CGSize { width: w, height: h },
    }
}

fn rounded((x, y, w, h, r): (f64, f64, f64, f64, f64)) -> CFRetained<CGPath> {
    unsafe { CGPath::with_rounded_rect(rect(x, y, w, h), r, r, std::ptr::null()) }
}

struct Painter {
    ctx: CFRetained<CGContext>,
    srgb: CFRetained<CGColorSpace>,
}

impl Painter {
    fn c(&self) -> Option<&CGContext> {
        Some(&self.ctx)
    }

    fn gradient(&self, stops: &[(f64, [f64; 4])]) -> Result<CFRetained<CGGradient>, IconError> {
        let comps: Vec<CGFloat> = stops.iter().flat_map(|(_, c)| *c).collect();
        let locs: Vec<CGFloat> = stops.iter().map(|(l, _)| *l).collect();
        unsafe { CGGradient::with_color_components(Some(&self.srgb), comps.as_ptr(), locs.as_ptr(), stops.len()) }
            .ok_or(IconError::Graphics("gradient"))
    }

    /// Fills the current clip with a gradient given in a box's unit
    /// square, as SVG's objectBoundingBox gradients are: drawn through the
    /// box's transform, so a non-square box skews it the way SVG does.
    fn bbox_linear(&self, g: &CGGradient, (x, y, w, h): (f64, f64, f64, f64), from: (f64, f64), to: (f64, f64)) {
        let c = self.c();
        CGContext::save_g_state(c);
        CGContext::concat_ctm(
            c,
            CGAffineTransform {
                a: w,
                b: 0.0,
                c: 0.0,
                d: h,
                tx: x,
                ty: y,
            },
        );
        let pad = CGGradientDrawingOptions::DrawsBeforeStartLocation | CGGradientDrawingOptions::DrawsAfterEndLocation;
        CGContext::draw_linear_gradient(c, Some(g), CGPoint { x: from.0, y: from.1 }, CGPoint { x: to.0, y: to.1 }, pad);
        CGContext::restore_g_state(c);
    }

    fn clip_to(&self, path: &CGPath) {
        let c = self.c();
        CGContext::begin_path(c);
        CGContext::add_path(c, Some(path));
        CGContext::clip(c);
    }

    /// Clips to `path` stroked like the Slope.
    fn clip_to_stroke(&self, path: &CGPath, width: f64) {
        let c = self.c();
        CGContext::set_line_width(c, width);
        CGContext::set_line_cap(c, CGLineCap::Round);
        CGContext::set_line_join(c, CGLineJoin::Round);
        CGContext::begin_path(c);
        CGContext::add_path(c, Some(path));
        CGContext::replace_path_with_stroked_path(c);
        CGContext::clip(c);
    }
}

/// Draws `icon` at `px` × `px`: RGBA, 8 bits, premultiplied, sRGB, rows
/// top to bottom.
pub fn render_rgba(icon: &PinIcon, px: usize) -> Result<Vec<u8>, IconError> {
    let (_, data) = render(icon, px)?;
    Ok(data)
}

fn render(icon: &PinIcon, px: usize) -> Result<(CFRetained<CGImage>, Vec<u8>), IconError> {
    if px == 0 {
        return Err(IconError::Graphics("zero size"));
    }
    let initial = parse(&icon.outline)?;
    let initial_box = ink_bounds(&initial).ok_or_else(|| IconError::BadOutline("no ink".to_owned()))?;
    let initial_path = to_cgpath(&initial);
    let slope_path = to_cgpath(&SLOPE);

    let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).ok_or(IconError::Graphics("sRGB"))?;
    let mut data = vec![0u8; px * px * 4];
    let ctx = unsafe {
        CGBitmapContextCreate(
            data.as_mut_ptr() as *mut c_void,
            px,
            px,
            8,
            px * 4,
            Some(&srgb),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }
    .ok_or(IconError::Graphics("bitmap context"))?;
    let p = Painter { ctx, srgb };
    let c = p.c();
    CGContext::set_should_antialias(c, true);
    // The template's top-down 1024 grid.
    let k = px as f64 / 1024.0;
    CGContext::concat_ctm(
        c,
        CGAffineTransform {
            a: k,
            b: 0.0,
            c: 0.0,
            d: -k,
            tx: 0.0,
            ty: px as f64,
        },
    );

    // 1. Body: palette gradient, (0,0) → (.7,1) of its box.
    let body = rounded(BODY);
    let body_grad = p.gradient(&[(0.0, rgb(icon.top)), (1.0, rgb(icon.bottom))])?;
    CGContext::save_g_state(c);
    p.clip_to(&body);
    p.bbox_linear(&body_grad, (BODY.0, BODY.1, BODY.2, BODY.3), (0.0, 0.0), (0.7, 1.0));
    CGContext::restore_g_state(c);

    // 2. Depth plane: the foreground in black, 8 px down, at 22% as one
    //    group.
    CGContext::save_g_state(c);
    CGContext::set_alpha(c, DEPTH_OPACITY);
    unsafe { CGContext::begin_transparency_layer(c, None) };
    CGContext::translate_ctm(c, 0.0, DEPTH_OFFSET);
    CGContext::set_rgb_fill_color(c, 0.0, 0.0, 0.0, 1.0);
    CGContext::begin_path(c);
    CGContext::add_path(c, Some(&initial_path));
    CGContext::fill_path(c);
    CGContext::set_rgb_stroke_color(c, 0.0, 0.0, 0.0, 1.0);
    CGContext::set_line_width(c, SLOPE_WIDTH);
    CGContext::set_line_cap(c, CGLineCap::Round);
    CGContext::set_line_join(c, CGLineJoin::Round);
    CGContext::begin_path(c);
    CGContext::add_path(c, Some(&slope_path));
    CGContext::stroke_path(c);
    CGContext::end_transparency_layer(c);
    CGContext::restore_g_state(c);

    // 3. Foreground: the initial, then the Slope, each with the ink
    //    gradient (0,0) → (.3,1) of its own box.
    let ink = p.gradient(&[(0.0, INK[0]), (1.0, INK[1])])?;
    let (x0, y0, x1, y1) = initial_box;
    CGContext::save_g_state(c);
    p.clip_to(&initial_path);
    p.bbox_linear(&ink, (x0, y0, x1 - x0, y1 - y0), (0.0, 0.0), (0.3, 1.0));
    CGContext::restore_g_state(c);
    CGContext::save_g_state(c);
    p.clip_to_stroke(&slope_path, SLOPE_WIDTH);
    p.bbox_linear(&ink, SLOPE_BOX, (0.0, 0.0), (0.3, 1.0));
    CGContext::restore_g_state(c);

    // 4. Light: radial, centre (.25,0), radius 1 of the inner rect, white
    //    6% → 0.
    let light = rounded(LIGHT);
    let light_grad = p.gradient(&[(0.0, [1.0, 1.0, 1.0, LIGHT_OPACITY]), (1.0, [1.0, 1.0, 1.0, 0.0])])?;
    CGContext::save_g_state(c);
    p.clip_to(&light);
    CGContext::concat_ctm(
        c,
        CGAffineTransform {
            a: LIGHT.2,
            b: 0.0,
            c: 0.0,
            d: LIGHT.3,
            tx: LIGHT.0,
            ty: LIGHT.1,
        },
    );
    let centre = CGPoint { x: 0.25, y: 0.0 };
    CGContext::draw_radial_gradient(
        c,
        Some(&light_grad),
        centre,
        0.0,
        centre,
        1.0,
        CGGradientDrawingOptions::DrawsAfterEndLocation,
    );
    CGContext::restore_g_state(c);

    // 5. Rim: that rect stroked 6 wide, (0,0) → (.3,1), white 55/6/22%.
    let rim_grad = p.gradient(&RIM.map(|(at, a)| (at, [1.0, 1.0, 1.0, a])))?;
    CGContext::save_g_state(c);
    p.clip_to_stroke(&light, RIM_WIDTH);
    p.bbox_linear(&rim_grad, (LIGHT.0, LIGHT.1, LIGHT.2, LIGHT.3), (0.0, 0.0), (0.3, 1.0));
    CGContext::restore_g_state(c);

    CGContext::flush(c);
    let image = CGBitmapContextCreateImage(c).ok_or(IconError::Graphics("image"))?;
    drop(p);
    Ok((image, data))
}

/// Writes `images` with ImageIO as `kind`, each with its DPI: 72 for 1×,
/// 144 for 2× (the `.icns` writer files a 2× image under its point size).
fn encode(kind: &str, images: &[(&CGImage, f64)]) -> Result<Vec<u8>, IconError> {
    let out = CFMutableData::new(None, 0).ok_or(IconError::Graphics("data"))?;
    unsafe {
        let dest = CGImageDestination::with_data(&out, &CFString::from_str(kind), images.len(), None)
            .ok_or(IconError::Graphics("image destination"))?;
        for &(image, dpi) in images {
            let dpi = CFNumber::new_cgfloat(dpi);
            let props =
                CFDictionary::<CFString, CFNumber>::from_slices(&[kCGImagePropertyDPIWidth, kCGImagePropertyDPIHeight], &[&dpi, &dpi]);
            dest.add_image(image, Some(props.as_opaque()));
        }
        if !dest.finalize() {
            return Err(IconError::Graphics("ImageIO couldn't write the image"));
        }
        Ok(std::slice::from_raw_parts(out.byte_ptr(), out.length() as usize).to_vec())
    }
}

/// `icon` at `px` × `px` as a PNG.
pub fn render_png(icon: &PinIcon, px: usize) -> Result<Vec<u8>, IconError> {
    let (image, _) = render(icon, px)?;
    encode("public.png", &[(&image, 72.0)])
}

/// The `.icns` entries: point size and scale, as `iconutil` names them
/// (`icon_16x16.png`, `icon_16x16@2x.png`, …).
pub const ICNS_ENTRIES: [(usize, usize); 10] = [
    (16, 1),
    (16, 2),
    (32, 1),
    (32, 2),
    (128, 1),
    (128, 2),
    (256, 1),
    (256, 2),
    (512, 1),
    (512, 2),
];

/// `icon` as an `.icns`, by ImageIO's own writer, from renders at each of
/// [`ICNS_SIZES`], each drawn once and used for every entry of its size.
pub fn render_icns(icon: &PinIcon) -> Result<Vec<u8>, IconError> {
    let rendered = ICNS_SIZES
        .iter()
        .map(|&px| render(icon, px).map(|(i, _)| (px, i)))
        .collect::<Result<Vec<_>, _>>()?;
    let image = |px: usize| -> &CGImage { &rendered.iter().find(|(p, _)| *p == px).expect("a rendered size").1 };
    let entries: Vec<(&CGImage, f64)> = ICNS_ENTRIES
        .iter()
        .map(|&(pt, scale)| (image(pt * scale), 72.0 * scale as f64))
        .collect();
    encode("com.apple.icns", &entries)
}
