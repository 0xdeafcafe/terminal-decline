//! A line of text drawn in one font, for the Settings tab's font list:
//! the sample of the font under the selection, shown as a picture since
//! a terminal draws all its text in one font.

use std::ffi::c_void;

use objc2_core_foundation::CGFloat;
use objc2_core_graphics::{CGBitmapContextCreate, CGContext, CGImageAlphaInfo};

use crate::named;
use crate::shape::{ct_line, font_attrs};

/// `text` in font `name` (a family, or a face's full or PostScript name)
/// in `fg` on `bg` (0xRRGGBB), as `width` × `height` RGB pixels, rows top
/// first: one line, as big as fits, centred on the height. `None` when no
/// such font is installed (Core Text would substitute another). A few
/// milliseconds for a line.
pub fn sample(name: &str, text: &str, width: u32, height: u32, fg: u32, bg: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || text.is_empty() {
        return None;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    // Measured at 100 points, then scaled to fit.
    let (font, _) = named(name, 100.0)?;
    let line = ct_line(&units, &font_attrs(&font))?;
    let (mut ascent, mut descent, mut leading): (CGFloat, CGFloat, CGFloat) = (0.0, 0.0, 0.0);
    let advance = unsafe { line.typographic_bounds(&mut ascent, &mut descent, &mut leading) };
    if advance <= 0.0 || ascent + descent <= 0.0 {
        return None;
    }
    let (w, h) = (width as CGFloat, height as CGFloat);
    let margin = (h * 0.15).min(w * 0.05);
    let scale = (h * 0.8 / (ascent + descent)).min((w - 2.0 * margin).max(1.0) / advance);
    let (font, _) = named(name, 100.0 * scale)?;
    let line = ct_line(&units, &font_attrs(&font))?;
    let (ascent, descent) = (ascent * scale, descent * scale);

    // Coverage in an alpha-only bitmap, then blended from bg to fg.
    let (wu, hu) = (width as usize, height as usize);
    let mut alpha = vec![0u8; wu * hu];
    let owned = unsafe { CGBitmapContextCreate(alpha.as_mut_ptr() as *mut c_void, wu, hu, 8, wu, None, CGImageAlphaInfo::Only.0) }?;
    let ctx: &CGContext = &owned;
    CGContext::set_should_antialias(Some(ctx), true);
    CGContext::set_allows_font_smoothing(Some(ctx), true);
    CGContext::set_should_smooth_fonts(Some(ctx), false);
    CGContext::set_gray_fill_color(Some(ctx), 1.0, 1.0);
    let baseline = ((h - (ascent + descent)) / 2.0 + descent).round();
    CGContext::set_text_position(Some(ctx), margin.round(), baseline);
    unsafe { line.draw(ctx) };
    drop(owned);

    let channel = |c: u32, s: u32| (c >> s) & 0xff;
    let mut out = Vec::with_capacity(wu * hu * 3);
    for a in alpha {
        let a = a as u32;
        for s in [16, 8, 0] {
            let (f, b) = (channel(fg, s), channel(bg, s));
            out.push(((f * a + b * (255 - a) + 127) / 255) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menlo_draws_ink_and_a_missing_font_none() {
        let (w, h) = (300, 40);
        let px = sample("Menlo", "AaBb 0O 1lI", w, h, 0xffffff, 0x000000).expect("Menlo ships with macOS");
        assert_eq!(px.len(), (w * h * 3) as usize);
        let lit = px.chunks(3).filter(|p| p[0] > 128).count();
        assert!(lit > 100 && lit < (w * h / 2) as usize, "some ink: {lit}");
        // Background at the corners, in the colour asked for.
        let bg = sample("Menlo", "x", 50, 20, 0xffffff, 0x123456).unwrap();
        assert_eq!(&bg[..3], &[0x12, 0x34, 0x56]);
        // A face by its full name works too.
        assert!(sample("Menlo Bold", "x", 50, 20, 0xffffff, 0).is_some());
        assert_eq!(sample("No Such Font 3141", "x", 50, 20, 0xffffff, 0), None);
    }
}
