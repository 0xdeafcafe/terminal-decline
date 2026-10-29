//! Fonts through Core Text: cell metrics, shaping ([`Fonts::shape`]) and
//! glyph rasterising. Box drawing, blocks, braille and Powerline are drawn
//! in code ([`sprite`]). [`FontConfig`] holds the user's font settings.

mod config;
mod fit;
#[cfg(test)]
mod legacy_tests;
mod registry;
mod shape;
pub mod sprite;

pub use config::{CellWidth, FontConfig};
pub use fit::is_private_use;
pub use registry::{FontId, GlyphKey, SPRITE_FONT};
pub use shape::{Features, RunText, ShapedGlyph};
pub use sprite::{SpriteId, is_code_drawn};

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFArray, CFDictionary, CFRange, CFRetained, CFString, CGFloat, CGPoint, CGRect, CGSize, Type};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGGlyph, CGImageAlphaInfo, CGImageByteOrderInfo};
use objc2_core_text::{CTFont, CTFontDescriptor, CTFontOrientation, CTFontSymbolicTraits, kCTFontCascadeListAttribute};

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
    /// Pixels coverage glyphs move right to stay centred in a cell made
    /// wider or narrower than the advance ([`CellWidth`]); 0 by default.
    x_shift: i32,
    /// Ids of the fonts on the user's fallback list that were found.
    fallback_ids: Vec<FontId>,
    /// Config font names that weren't found.
    missing: Vec<String>,
    rgb: CFRetained<CGColorSpace>,
}

// Core Text fonts are immutable and documented as thread-safe.
unsafe impl Send for Fonts {}

impl Fonts {
    /// `size_px` is the font size in device pixels (points × backing scale).
    /// The same as [`Fonts::with_config`] with only the family and size set,
    /// at scale 1.
    ///
    /// This never fails: Core Text substitutes another font for a family it
    /// can't find, usually Helvetica, which is not monospaced. That includes
    /// "SF Mono": it ships inside Terminal and Xcode rather than as a system
    /// font, so asking for it by name silently gives Helvetica unless the
    /// user installed it. [`Fonts::family`] tells what was actually loaded.
    pub fn new(family: &str, size_px: f64) -> Fonts {
        Fonts::with_config(&FontConfig::new(family, size_px), 1.0)
    }

    /// Fonts as `config` sets them, at `scale` device pixels a point (the
    /// backing scale). Like [`Fonts::new`] this never fails: a family Core
    /// Text can't find is substituted, and fallback fonts or face
    /// overrides it can't find are left out (an override falls back to the
    /// family's own face) and listed by [`Fonts::missing`].
    ///
    /// The fallback list becomes a custom cascade list on each of the four
    /// faces: the listed fonts, then Core Text's default cascade for that
    /// face. So the per-(character, style) lookup (`CTFontCreateForString`
    /// on the face) and CTLine's shaping both try them in order, and the
    /// lookup cache works as before. The face's own glyphs still come
    /// first.
    pub fn with_config(config: &FontConfig, scale: f64) -> Fonts {
        let size = (config.size * scale) as CGFloat;
        let mut missing = Vec::new();
        let regular = unsafe { CTFont::with_name(&CFString::from_str(&config.family), size, std::ptr::null()) };
        let mut face = |name: &Option<String>, traits: CTFontSymbolicTraits| match name {
            None => with_traits(&regular, traits),
            Some(name) => match named(name, size) {
                Some((font, true)) => font,
                Some((font, false)) => with_traits(&font, traits),
                None => {
                    missing.push(name.clone());
                    with_traits(&regular, traits)
                }
            },
        };
        let bold = face(&config.bold, CTFontSymbolicTraits::BoldTrait);
        let italic = face(&config.italic, CTFontSymbolicTraits::ItalicTrait);
        let bold_italic = face(
            &config.bold_italic,
            CTFontSymbolicTraits::BoldTrait | CTFontSymbolicTraits::ItalicTrait,
        );
        let mut fallbacks = Vec::new();
        for name in &config.fallback {
            match named(name, size) {
                Some((font, _)) => fallbacks.push(font),
                None => missing.push(name.clone()),
            }
        }
        let (metrics, x_shift) = metrics(&regular, config, scale);
        let mut faces = [regular, bold, italic, bold_italic];
        if !fallbacks.is_empty() {
            for f in &mut faces {
                *f = with_cascade(f, &fallbacks);
            }
        }
        let mut registry = registry::Registry::new(faces);
        let fallback_ids = fallbacks.iter().filter_map(|f| registry.id(f)).collect();
        Fonts {
            registry: RefCell::new(registry),
            clusters: RefCell::new(shape::ClusterCache::new(shape::ClusterCache::CAP)),
            metrics,
            x_shift,
            fallback_ids,
            missing,
            rgb: CGColorSpace::new_device_rgb().expect("rgb colour space"),
        }
    }

    /// Fallback fonts and face overrides from the config that weren't
    /// found, for the app to warn about.
    pub fn missing(&self) -> &[String] {
        &self.missing
    }

    /// PostScript name of font `id`, e.g. "Menlo-Bold": which face an
    /// override or a fallback actually loaded.
    pub fn postscript_name_of(&self, id: FontId) -> Option<String> {
        let reg = self.registry.borrow();
        let face = reg.faces.get(id as usize)?;
        Some(unsafe { face.font.post_script_name() }.to_string())
    }

    /// An icon, fitted to its cells ([`fit`]): a Private Use Area
    /// character, or a symbol drawn from a font on the fallback list.
    pub(crate) fn is_icon(&self, ch: char, font: FontId) -> bool {
        fit::is_private_use(ch) || (fit::is_symbol(ch) && self.fallback_ids.contains(&font))
    }

    /// Whether glyph `glyph` of font `font` has ink wider than one cell.
    pub(crate) fn wider_than_cell(&self, font: FontId, glyph: CGGlyph) -> bool {
        let reg = self.registry.borrow();
        let Some(face) = reg.faces.get(font as usize) else { return false };
        let mut g = glyph;
        let mut rect = CGRect::default();
        unsafe {
            face.font
                .bounding_rects_for_glyphs(CTFontOrientation::Horizontal, NonNull::from(&mut g), &mut rect, 1)
        };
        rect.size.width > self.metrics.cell_width as f64
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
        let fitted = !color && self.is_icon(ch, font);
        Some(GlyphKey {
            font,
            glyph,
            cells: if color { 2 } else { 1 },
            variant: if fitted { GlyphKey::FITTED } else { 0 },
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
    /// `key.cells` cells; sprites are drawn across them; icons
    /// ([`GlyphKey::FITTED`]) are scaled down and centred in them and never
    /// reach past them.
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
        if !color && key.variant & GlyphKey::FITTED != 0 {
            return self.rasterize_fitted(font, key, rect);
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
            left: min_x as i32 + if color { 0 } else { self.x_shift },
            top: max_y as i32,
            color,
            data,
        })
    }

    /// An icon scaled down and centred in its cells ([`fit`]), drawn into a
    /// bitmap the size of those cells and then trimmed to its ink, so it
    /// never reaches into a neighbour.
    fn rasterize_fitted(&self, font: &CTFont, key: GlyphKey, ink: CGRect) -> Option<RasterGlyph> {
        let m = self.metrics;
        let (w, h) = ((m.cell_width * key.cells.max(1) as u32) as usize, m.cell_height as usize);
        let (scale, x, y) = fit::fit((ink.origin.x, ink.origin.y, ink.size.width, ink.size.height), w as f64, h as f64);
        let mut data = vec![0u8; w * h];
        let owned = unsafe { CGBitmapContextCreate(data.as_mut_ptr() as *mut c_void, w, h, 8, w, None, CGImageAlphaInfo::Only.0) }?;
        let ctx: &CGContext = &owned;
        CGContext::set_should_antialias(Some(ctx), true);
        CGContext::set_allows_font_smoothing(Some(ctx), true);
        CGContext::set_should_smooth_fonts(Some(ctx), false);
        CGContext::set_allows_font_subpixel_positioning(Some(ctx), true);
        CGContext::set_should_subpixel_position_fonts(Some(ctx), true);
        CGContext::set_gray_fill_color(Some(ctx), 1.0, 1.0);
        CGContext::translate_ctm(Some(ctx), x, y);
        CGContext::scale_ctm(Some(ctx), scale, scale);
        let mut g = key.glyph;
        let mut pos = CGPoint::new(0.0, 0.0);
        unsafe { font.draw_glyphs(NonNull::from(&mut g), NonNull::from(&mut pos), 1, ctx) };
        drop(owned);

        // Trim to the ink; row 0 is the cell's top.
        let rows = |r: usize| data[r * w..][..w].iter().any(|&b| b != 0);
        let cols = |c: usize| (0..h).any(|r| data[r * w + c] != 0);
        let top = (0..h).find(|&r| rows(r))?;
        let bottom = (0..h).rev().find(|&r| rows(r))?;
        let left = (0..w).find(|&c| cols(c))?;
        let right = (0..w).rev().find(|&c| cols(c))?;
        let (tw, th) = (right - left + 1, bottom - top + 1);
        let mut out = Vec::with_capacity(tw * th);
        for r in top..=bottom {
            out.extend_from_slice(&data[r * w + left..][..tw]);
        }
        Some(RasterGlyph {
            width: tw as u32,
            height: th as u32,
            left: left as i32,
            top: m.baseline as i32 - top as i32,
            color: false,
            data: out,
        })
    }
}

/// `font` with `traits` (bold, italic), or `font` itself when its family
/// has no such face.
fn with_traits(font: &CTFont, traits: CTFontSymbolicTraits) -> CFRetained<CTFont> {
    unsafe {
        font.copy_with_symbolic_traits(0.0, std::ptr::null(), traits, traits)
            .unwrap_or_else(|| font.retain())
    }
}

/// The font `name` names at `size`, and whether it names one face (a full
/// or PostScript name, "Menlo Bold" or "Menlo-Bold") rather than a family
/// ("Menlo"). `None` when it isn't installed: Core Text substituted
/// another font.
fn named(name: &str, size: CGFloat) -> Option<(CFRetained<CTFont>, bool)> {
    let font = unsafe { CTFont::with_name(&CFString::from_str(name), size, std::ptr::null()) };
    let is = |s: CFRetained<CFString>| s.to_string().eq_ignore_ascii_case(name.trim());
    let family = is(unsafe { font.family_name() });
    let face = is(unsafe { font.post_script_name() }) || is(unsafe { font.full_name() }) || is(unsafe { font.display_name() });
    match (face, family) {
        (_, true) => Some((font, false)),
        (true, false) => Some((font, true)),
        (false, false) => None,
    }
}

/// `face` with a cascade list of `fallbacks` and then Core Text's default
/// cascade for it, so fallback lookups and CTLine try them in that order.
fn with_cascade(face: &CTFont, fallbacks: &[CFRetained<CTFont>]) -> CFRetained<CTFont> {
    unsafe {
        let mut list: Vec<CFRetained<CTFontDescriptor>> = fallbacks.iter().map(|f| f.font_descriptor()).collect();
        if let Some(default) = face.default_cascade_list_for_languages(None) {
            for i in 0..default.count() {
                let d = default.value_at_index(i) as *const CTFontDescriptor;
                if let Some(d) = d.as_ref() {
                    list.push(d.retain());
                }
            }
        }
        let list = CFArray::from_retained_objects(&list);
        let attrs = CFDictionary::<CFString, CFArray<CTFontDescriptor>>::from_slices(&[kCTFontCascadeListAttribute], &[&list]);
        let desc = CTFontDescriptor::with_attributes(attrs.as_opaque());
        face.copy_with_attributes(0.0, std::ptr::null(), Some(&desc))
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

/// Cell metrics for `font` with the config's line height and cell width,
/// and how far glyphs move right to stay centred in the cell.
fn metrics(font: &CTFont, config: &FontConfig, scale: f64) -> (Metrics, i32) {
    let natural = natural_metrics(font);
    let mut m = natural;
    let line_height = config.line_height;
    if line_height.is_finite() && line_height > 0.0 && line_height != 1.0 {
        let (ascent, descent) = unsafe { (font.ascent(), font.descent()) };
        m.cell_height = (natural.cell_height as f64 * line_height).round().max(1.0) as u32;
        // The same centring as the font's own line height: half the extra
        // height above the text, half below.
        let baseline = ((m.cell_height as f64 - (ascent + descent)) / 2.0 + ascent).round();
        m.baseline = (baseline.max(1.0) as u32).min(m.cell_height.saturating_sub(1).max(1));
        if m.cell_height < natural.cell_height {
            // Keep the underline and strike inside a shorter cell.
            let room = m.cell_height.saturating_sub(m.baseline);
            m.underline_thickness = m.underline_thickness.min(room.max(1));
            m.underline_offset = m.underline_offset.min(room.saturating_sub(m.underline_thickness)).max(1);
            m.strike_offset = m.strike_offset.min(m.baseline);
        }
    }
    let advance = unsafe { advance_of_m(font) };
    m.cell_width = config.cell_width.apply(advance, scale);
    let x_shift = (m.cell_width as i32 - natural.cell_width as i32).div_euclid(2);
    (m, x_shift)
}

/// The advance of 'M', in pixels.
unsafe fn advance_of_m(font: &CTFont) -> f64 {
    let mut g = [0 as CGGlyph; 1];
    let mut m = [b'M' as u16];
    let mut adv = CGSize::default();
    unsafe {
        font.glyphs_for_characters(NonNull::from(&mut m[0]), NonNull::from(&mut g[0]), 1);
        font.advances_for_glyphs(CTFontOrientation::Horizontal, NonNull::from(&mut g[0]), &mut adv, 1);
    }
    adv.width
}

/// The font's own cell metrics, before the config adjusts them.
fn natural_metrics(font: &CTFont) -> Metrics {
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
