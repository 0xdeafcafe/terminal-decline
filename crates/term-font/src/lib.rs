//! Fonts through Core Text: cell metrics, shaping ([`Fonts::shape`]) and
//! glyph rasterising. Box drawing, blocks, braille and Powerline are drawn
//! in code ([`sprite`]).

#[cfg(test)]
mod legacy_tests;
mod registry;
mod shape;
pub mod sprite;

pub use registry::{FontId, GlyphKey, SPRITE_FONT};
pub use shape::{Features, RunText, ShapedGlyph};
pub use sprite::{SpriteId, is_code_drawn};

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFRange, CFRetained, CFString, CGFloat, CGPoint, CGRect, CGSize, Type};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGGlyph, CGImageAlphaInfo, CGImageByteOrderInfo};
use objc2_core_text::{CTFont, CTFontOrientation, CTFontSymbolicTraits};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Style {
    pub fn new(bold: bool, italic: bool) -> Style {
        match (bold, italic) {
            (false, false) => Style::Regular,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (true, true) => Style::BoldItalic,
        }
    }
}

/// Cell metrics in device pixels.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub cell_width: u32,
    pub cell_height: u32,
    /// Distance from the top of the cell to the baseline.
    pub baseline: u32,
    /// Distance below the baseline to the top of the underline.
    pub underline_offset: u32,
    pub underline_thickness: u32,
    pub strike_offset: u32,
}

/// A rasterised glyph. Coverage glyphs are one byte per pixel; colour glyphs
/// (emoji) are BGRA, premultiplied.
pub struct RasterGlyph {
    pub width: u32,
    pub height: u32,
    /// Offset of the bitmap's left edge from the cell's left edge.
    pub left: i32,
    /// Distance from the baseline up to the bitmap's top edge.
    pub top: i32,
    pub color: bool,
    pub data: Vec<u8>,
}

pub struct Fonts {
    /// The four configured faces (ids 0–3) and every fallback since.
    registry: RefCell<registry::Registry>,
    /// Shaped grapheme clusters, by face and text (§10.2).
    clusters: RefCell<shape::ClusterCache>,
    pub metrics: Metrics,
    rgb: CFRetained<CGColorSpace>,
}

// Core Text fonts are immutable and documented as thread-safe.
unsafe impl Send for Fonts {}

impl Fonts {
    /// `size_px` is the font size in device pixels (points × backing scale).
    ///
    /// This never fails: Core Text substitutes another font for a family it
    /// can't find, usually Helvetica, which is not monospaced. That includes
    /// "SF Mono": it ships inside Terminal and Xcode rather than as a system
    /// font, so asking for it by name silently gives Helvetica unless the
    /// user installed it. [`Fonts::family`] tells what was actually loaded.
    pub fn new(family: &str, size_px: f64) -> Fonts {
        let name = CFString::from_str(family);
        let regular = unsafe { CTFont::with_name(&name, size_px as CGFloat, std::ptr::null()) };
        let with = |traits: CTFontSymbolicTraits| unsafe {
            regular
                .copy_with_symbolic_traits(size_px as CGFloat, std::ptr::null(), traits, traits)
                .unwrap_or_else(|| regular.clone())
        };
        let bold = with(CTFontSymbolicTraits::BoldTrait);
        let italic = with(CTFontSymbolicTraits::ItalicTrait);
        let bold_italic = with(CTFontSymbolicTraits::BoldTrait | CTFontSymbolicTraits::ItalicTrait);
        let metrics = metrics(&regular);
        Fonts {
            registry: RefCell::new(registry::Registry::new([regular, bold, italic, bold_italic])),
            clusters: RefCell::new(shape::ClusterCache::new(shape::ClusterCache::CAP)),
            metrics,
            rgb: CGColorSpace::new_device_rgb().expect("rgb colour space"),
        }
    }

    /// Family name of the regular face actually loaded.
    pub fn family(&self) -> String {
        self.family_of(0).expect("the regular face")
    }

    /// Family name of the font that draws `ch`: the loaded one, or the system
    /// fallback Core Text picks for it.
    pub fn family_for(&self, ch: char, style: Style) -> Option<String> {
        let (id, _) = self.registry.borrow_mut().resolve(ch, style)?;
        self.family_of(id)
    }

    /// Family name of font `id`; `None` for [`SPRITE_FONT`] and ids never
    /// handed out.
    pub fn family_of(&self, id: FontId) -> Option<String> {
        let reg = self.registry.borrow();
        let face = reg.faces.get(id as usize)?;
        Some(unsafe { face.font.family_name() }.to_string())
    }

    /// The font id of a configured face.
    pub fn face_id(style: Style) -> FontId {
        style as FontId
    }

    /// The id for `font`, registering it if it is new: the same face (by
    /// `CFEqual`) always gets the same id. For fonts Core Text picks while
    /// shaping (F2.2). `None` only once 65535 fonts are registered.
    pub fn font_id(&self, font: &CTFont) -> Option<FontId> {
        self.registry.borrow_mut().id(font)
    }

    /// Horizontal advance of `ch`, in device pixels.
    pub fn advance(&self, ch: char, style: Style) -> Option<f64> {
        let (id, mut g) = self.registry.borrow_mut().resolve(ch, style)?;
        let reg = self.registry.borrow();
        let font = &reg.faces[id as usize].font;
        let mut adv = CGSize::default();
        unsafe { font.advances_for_glyphs(CTFontOrientation::Horizontal, NonNull::from(&mut g), &mut adv, 1) };
        Some(adv.width)
    }

    /// The key for `ch` drawn alone in `style`: a sprite key for code-drawn
    /// characters ([`is_code_drawn`]), otherwise the glyph in the face or in
    /// the fallback Core Text picks, with colour glyphs covering 2 cells.
    /// `None` when no font has the character. Lookups are cached per
    /// character and style.
    pub fn glyph_key(&self, ch: char, style: Style) -> Option<GlyphKey> {
        if let Some(id) = SpriteId::for_char(ch) {
            return Some(GlyphKey::sprite(id, 1));
        }
        let (font, glyph) = self.registry.borrow_mut().resolve(ch, style)?;
        let color = self.registry.borrow().faces[font as usize].color;
        Some(GlyphKey {
            font,
            glyph,
            cells: if color { 2 } else { 1 },
            variant: 0,
        })
    }

    /// The glyph for `ch`: a coverage bitmap (or BGRA for colour glyphs),
    /// `None` for glyphs with no ink. Code-drawn characters
    /// ([`is_code_drawn`]) come from [`sprite`], sized exactly to the cell
    /// and the same in every style. The same as
    /// `rasterize_key(glyph_key(ch, style)?)`.
    pub fn rasterize(&self, ch: char, style: Style) -> Option<RasterGlyph> {
        self.rasterize_key(self.glyph_key(ch, style)?)
    }

    /// The bitmap `key` names; `None` for glyphs with no ink and for font
    /// ids never handed out. Colour glyphs are scaled down to fit
    /// `key.cells` cells; sprites are drawn across them.
    pub fn rasterize_key(&self, key: GlyphKey) -> Option<RasterGlyph> {
        if let Some(id) = key.sprite_id() {
            return sprite::rasterize(id, &self.metrics, key.cells as u32);
        }
        let reg = self.registry.borrow();
        let face = reg.faces.get(key.font as usize)?;
        let font = &face.font;
        let color = face.color;

        let mut g = key.glyph;
        let mut rect = CGRect::default();
        unsafe { font.bounding_rects_for_glyphs(CTFontOrientation::Horizontal, NonNull::from(&mut g), &mut rect, 1) };
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return None; // spaces and other invisible glyphs
        }

        // Colour glyphs (emoji) are drawn at a size that fits their cells.
        let m = self.metrics;
        let scale = if color {
            let fit_w = (m.cell_width * key.cells.max(1) as u32) as f64 / rect.size.width;
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
                CGBitmapContextCreate(data.as_mut_ptr() as *mut c_void, width, height, 8, width * 4, Some(&self.rgb), info)
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
}

/// The glyph for `ch` in `font`, or in the system fallback font for it.
pub(crate) fn glyph_for(font: &CTFont, ch: char) -> Option<(CFRetained<CTFont>, CGGlyph)> {
    let mut utf16 = [0u16; 2];
    let units = ch.encode_utf16(&mut utf16).len();
    let mut glyphs = [0 as CGGlyph; 2];
    let mut lookup = |f: &CTFont, glyphs: &mut [CGGlyph; 2]| unsafe {
        f.glyphs_for_characters(NonNull::from(&mut utf16[0]), NonNull::from(&mut glyphs[0]), units as isize) && glyphs[0] != 0
    };
    if lookup(font, &mut glyphs) {
        return Some((font.retain(), glyphs[0]));
    }
    let s = CFString::from_str(ch.encode_utf8(&mut [0; 4]));
    let fallback = unsafe {
        font.for_string(
            &s,
            CFRange {
                location: 0,
                length: units as isize,
            },
        )
    };
    if lookup(&fallback, &mut glyphs) {
        Some((fallback, glyphs[0]))
    } else {
        None
    }
}

fn metrics(font: &CTFont) -> Metrics {
    unsafe {
        let ascent = font.ascent();
        let descent = font.descent();
        let leading = font.leading();
        let mut g = [0 as CGGlyph; 1];
        let mut m = [b'M' as u16];
        font.glyphs_for_characters(NonNull::from(&mut m[0]), NonNull::from(&mut g[0]), 1);
        let mut adv = CGSize::default();
        font.advances_for_glyphs(CTFontOrientation::Horizontal, NonNull::from(&mut g[0]), &mut adv, 1);
        let cell_width = adv.width.round().max(1.0) as u32;
        let cell_height = (ascent + descent + leading).ceil().max(1.0) as u32;
        // Centre any extra leading around the text.
        let baseline = ((cell_height as f64 - (ascent + descent)) / 2.0 + ascent).round() as u32;
        let thickness = font.underline_thickness().round().max(1.0) as u32;
        let underline_offset = (-font.underline_position()).round().max(1.0) as u32;
        let strike_offset = (font.x_height() / 2.0).round() as u32;
        Metrics {
            cell_width,
            cell_height,
            baseline,
            underline_offset,
            underline_thickness: thickness,
            strike_offset,
        }
    }
}
