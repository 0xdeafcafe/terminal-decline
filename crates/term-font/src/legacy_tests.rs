//! The `rasterize(char, style)` wrapper against the code it replaced: the
//! function below is `Fonts::rasterize` as it was before glyph keys
//! (`ac2f83b`), with the uncached fallback lookup, kept only to compare
//! bitmaps.

use super::*;

fn old_rasterize(f: &Fonts, ch: char, style: Style) -> Option<RasterGlyph> {
    if let Some(id) = SpriteId::for_char(ch) {
        return sprite::rasterize(id, &f.metrics, 1);
    }
    let base = f.registry.borrow().faces[style as usize].font.clone();
    let (font, glyph) = glyph_for(&base, ch)?;
    let color = unsafe { font.symbolic_traits() }.contains(CTFontSymbolicTraits::ColorGlyphsTrait);

    let mut g = glyph;
    let mut rect = CGRect::default();
    unsafe { font.bounding_rects_for_glyphs(CTFontOrientation::Horizontal, NonNull::from(&mut g), &mut rect, 1) };
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return None;
    }

    let m = f.metrics;
    let scale = if color {
        let fit_w = (m.cell_width * 2) as f64 / rect.size.width;
        let fit_h = m.cell_height as f64 / rect.size.height;
        fit_w.min(fit_h).min(1.0)
    } else {
        1.0
    };

    const PAD: f64 = 1.0;
    let min_x = (rect.origin.x * scale).floor() - PAD;
    let min_y = (rect.origin.y * scale).floor() - PAD;
    let max_x = ((rect.origin.x + rect.size.width) * scale).ceil() + PAD;
    let max_y = ((rect.origin.y + rect.size.height) * scale).ceil() + PAD;
    let width = (max_x - min_x) as usize;
    let height = (max_y - min_y) as usize;
    let bpp = if color { 4 } else { 1 };
    let mut data = vec![0u8; width * height * bpp];

    let owned = unsafe {
        if color {
            let info = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;
            CGBitmapContextCreate(data.as_mut_ptr() as *mut c_void, width, height, 8, width * 4, Some(&f.rgb), info)
        } else {
            CGBitmapContextCreate(
                data.as_mut_ptr() as *mut c_void,
                width,
                height,
                8,
                width,
                None,
                CGImageAlphaInfo::Only.0,
            )
        }
    }?;
    let ctx: &CGContext = &owned;
    CGContext::set_should_antialias(Some(ctx), true);
    CGContext::set_allows_font_smoothing(Some(ctx), true);
    CGContext::set_should_smooth_fonts(Some(ctx), false);
    CGContext::set_allows_font_subpixel_positioning(Some(ctx), true);
    CGContext::set_should_subpixel_position_fonts(Some(ctx), true);
    CGContext::set_gray_fill_color(Some(ctx), 1.0, 1.0);
    CGContext::translate_ctm(Some(ctx), -min_x, -min_y);
    CGContext::scale_ctm(Some(ctx), scale, scale);
    let mut pos = CGPoint::new(0.0, 0.0);
    unsafe { font.draw_glyphs(NonNull::from(&mut g), NonNull::from(&mut pos), 1, ctx) };
    drop(owned);

    Some(RasterGlyph {
        width: width as u32,
        height: height as u32,
        left: min_x as i32,
        top: max_y as i32,
        color,
        data,
    })
}

type Bitmap = (u32, u32, i32, i32, bool, Vec<u8>);

fn parts(g: RasterGlyph) -> Bitmap {
    (g.width, g.height, g.left, g.top, g.color, g.data)
}

#[test]
fn wrapper_matches_the_old_bitmaps() {
    let sample = "AgWjf_|@~ éßλЖ■→∑≠★漢あ한😀🦀👍─│╭█▒⣿\u{E0B0}\u{1FB00}";
    for family in ["Menlo", "Monaco"] {
        for size in [13.0, 26.0, 37.0] {
            let f = Fonts::new(family, size);
            for style in [Style::Regular, Style::Bold, Style::Italic, Style::BoldItalic] {
                for ch in sample.chars() {
                    // Twice: the second goes through the cached lookup.
                    for _ in 0..2 {
                        let new = f.rasterize(ch, style).map(parts);
                        let old = old_rasterize(&f, ch, style).map(parts);
                        assert!(new == old, "{family} {size} {style:?} {ch:?}");
                    }
                }
            }
        }
    }
}
