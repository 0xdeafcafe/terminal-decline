//! Fonts through Core Text: cell metrics, shaping ([`Fonts::shape`]) and
//! glyph rasterising. Box drawing, blocks, braille and Powerline are drawn
//! in code ([`sprite`]). [`FontConfig`] holds the user's font settings.

mod config;
mod detect;
mod fit;
#[cfg(test)]
mod legacy_tests;
mod registry;
mod shape;
pub mod sprite;

pub use config::{CellWidth, FontConfig, Tag};
pub use fit::is_private_use;
pub use registry::{FontId, GlyphKey, SPRITE_FONT};
pub use shape::{Features, RunText, ShapedGlyph};
pub use sprite::{SpriteId, is_code_drawn};

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use detect::Ligating;
use objc2_core_foundation::{CFArray, CFDictionary, CFIndex, CFRange, CFRetained, CFString, CGFloat, CGPoint, CGRect, CGSize, Type};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGGlyph, CGImageAlphaInfo, CGImageByteOrderInfo};
use objc2_core_text::{
    CTFont, CTFontDescriptor, CTFontOrientation, CTFontSymbolicTraits, CTFontTableOptions, kCTFontCascadeListAttribute, kCTFontTableGSUB,
    kCTFontTableMorx,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Style {
    /// The four styles, in face order.
    pub const ALL: [Style; 4] = [Style::Regular, Style::Bold, Style::Italic, Style::BoldItalic];

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
    /// Per face, the registry's id for its font: a cell is the face's to
    /// shape in a run when its character resolves to this id. Not
    /// [`Fonts::face_id`]: in a family with no bold, the bold face is the
    /// regular font, id 0.
    face_ids: [FontId; 4],
    /// Per face, whether it has ligatures or contextual alternates on
    /// (ligatures design §1), worked out at construction.
    runs: [bool; 4],
    /// Per face that shapes runs, the characters that can take part in its
    /// substitutions ([`Fonts::ligates`]).
    ligating: [Ligating; 4],
    /// Whether [`Fonts::features`] reports `runs`: off until the renderer
    /// draws ligatures (B1.2).
    allow_runs: bool,
    /// The regular face's advance in pixels, which [`GlyphKey::SCALED`]
    /// glyphs are stretched from to the cell width.
    advance: f64,
    /// Per face, the CTLine attributes: its font.
    line_attrs: [CFRetained<CFDictionary<CFString, CTFont>>; 4],
    /// Buffers `shape` reuses, so shaping a run allocates nothing.
    scratch: RefCell<shape::Scratch>,
    /// Whether a split glyph needs [`GlyphKey::SCALED`], by font and glyph.
    scaled: RefCell<HashMap<(FontId, CGGlyph), bool>>,
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
        let advance = unsafe { advance_of_m(&regular) };
        let mut faces = [regular, bold, italic, bold_italic];
        if !config.features.is_empty() {
            for f in &mut faces {
                *f = with_features(f, &config.features);
            }
        }
        if !fallbacks.is_empty() {
            for f in &mut faces {
                *f = with_cascade(f, &fallbacks);
            }
        }
        let line_attrs = faces.each_ref().map(|f| shape::font_attrs(f));
        let face_fonts = faces.clone();
        let mut registry = registry::Registry::new(faces);
        registry.line_lookup = [!config.features.is_empty(); 4];
        let fallback_ids = fallbacks.iter().filter_map(|f| registry.id(f)).collect();
        let face_ids = Style::ALL.map(|s| registry.id(&face_fonts[s as usize]).unwrap_or(s as FontId));
        let mut fonts = Fonts {
            registry: RefCell::new(registry),
            clusters: RefCell::new(shape::ClusterCache::new(shape::ClusterCache::CAP)),
            metrics,
            x_shift,
            fallback_ids,
            missing,
            rgb: CGColorSpace::new_device_rgb().expect("rgb colour space"),
            face_ids,
            runs: [false; 4],
            ligating: Default::default(),
            allow_runs: false,
            advance,
            line_attrs,
            scratch: RefCell::new(shape::Scratch::default()),
            scaled: RefCell::new(HashMap::new()),
        };
        fonts.runs = fonts.detect_runs(&face_fonts, &config.features);
        fonts.ligating = fonts.detect_ligating(&face_fonts, &config.features);
        fonts
    }

    /// Per face that shapes runs, which characters can take part in its
    /// substitutions: those whose glyph is in the `GSUB` lookups of its
    /// cross-character features ([`detect::lookup_glyphs`]). Every
    /// character for `morx` faces, tables this can't read, and rules that
    /// take any glyph.
    fn detect_ligating(&self, faces: &[CFRetained<CTFont>; 4], user: &[(Tag, u32)]) -> [Ligating; 4] {
        let mut out: [Ligating; 4] = Default::default();
        for style in Style::ALL {
            let i = style as usize;
            if !self.runs[i] {
                continue;
            }
            if let Some(j) = (0..i).find(|&j| self.face_ids[j] == self.face_ids[i]) {
                out[i] = out[j].clone();
                continue;
            }
            let font = &faces[i];
            let morx = unsafe { font.table(kCTFontTableMorx, CTFontTableOptions(0)) }.is_some();
            let gsub = unsafe { font.table(kCTFontTableGSUB, CTFontTableOptions(0)) };
            let set = match gsub {
                Some(gsub) if !morx => detect::lookup_glyphs(unsafe { gsub.as_bytes_unchecked() }, user),
                _ => None,
            };
            out[i] = match set {
                Some(set) if !set.any => self.ligating_chars(font, style, &set.glyphs),
                _ => Ligating::all(),
            };
        }
        out
    }

    /// The characters of the Basic Multilingual Plane whose glyph in
    /// `font` is in `glyphs` (sorted): by the font's character map, and for
    /// ASCII also by the glyph the per-character lookup picks (which
    /// single-glyph features like `zero` change).
    fn ligating_chars(&self, font: &CTFont, style: Style, glyphs: &[u16]) -> Ligating {
        let chars: Vec<u16> = (0x20..=0xFFFFu32)
            .filter(|c| !(0xD800..0xE000).contains(c))
            .map(|c| c as u16)
            .collect();
        let mut mapped = vec![0 as CGGlyph; chars.len()];
        // About 2 ms for the whole plane, at construction.
        unsafe {
            font.glyphs_for_characters(
                NonNull::new_unchecked(chars.as_ptr() as *mut u16),
                NonNull::new_unchecked(mapped.as_mut_ptr()),
                chars.len() as CFIndex,
            )
        };
        let in_set = |g: CGGlyph| g != 0 && glyphs.binary_search(&g).is_ok();
        // With the user's features, ASCII may resolve to other glyphs.
        let featured = self.registry.borrow().line_lookup[style as usize];
        let face_id = self.face_ids[style as usize];
        let resolved =
            |ch: char| featured && ch.is_ascii_graphic() && self.glyph_key(ch, style).is_some_and(|k| k.font == face_id && in_set(k.glyph));
        let mut l = Ligating::default();
        for (&c, &g) in chars.iter().zip(&mapped) {
            if in_set(g) || resolved(char::from_u32(c as u32).unwrap_or(' ')) {
                if c < 128 {
                    l.ascii |= 1 << c;
                } else {
                    l.other.push(c as u32);
                }
            }
        }
        let has = |ch: char| mapped[ch as usize - 0x20] != 0;
        l.plain = ('a'..='z').chain('A'..='Z').find(|&ch| has(ch) && !l.contains(ch));
        l
    }

    /// Whether `ch` can take part in `face`'s ligatures and contextual
    /// alternates: its glyph is in a lookup of a cross-character feature
    /// that is on (ligatures design §5). A character that can't is never
    /// substituted, and no rule tells it from any other such character, so
    /// a renderer shapes only the stretches of characters that can, each
    /// with a [`plain_char`](Fonts::plain_char) for a neighbour that can't.
    /// True for every character of a face whose table can't be read this
    /// way (`morx` faces, rules taking any glyph); false in faces that
    /// don't shape runs.
    pub fn ligates(&self, ch: char, face: Style) -> bool {
        self.ligating[face as usize].contains(ch)
    }

    /// A letter `face` draws that can't take part in its substitutions
    /// ([`Fonts::ligates`]), to stand for any such neighbour of a stretch
    /// that can. `None` when there is none.
    pub fn plain_char(&self, face: Style) -> Option<char> {
        self.ligating[face as usize].plain
    }

    /// Whether each face shapes whole runs (ligatures design §1). OpenType
    /// faces: a feature that substitutes across characters is on in their
    /// `latn` (else `DFLT`) features, after the user's settings. Faces with
    /// a `morx` table: a line of programming sequences shapes differently
    /// from the per-character glyphs, or the user turned such a feature
    /// on. Faces that are one font are decided once.
    fn detect_runs(&self, faces: &[CFRetained<CTFont>; 4], user: &[(Tag, u32)]) -> [bool; 4] {
        let mut runs = [false; 4];
        for style in Style::ALL {
            let i = style as usize;
            if let Some(j) = (0..i).find(|&j| self.face_ids[j] == self.face_ids[i]) {
                runs[i] = runs[j];
                continue;
            }
            let font = &faces[i];
            let morx = unsafe { font.table(kCTFontTableMorx, CTFontTableOptions(0)) }.is_some();
            let gsub = unsafe { font.table(kCTFontTableGSUB, CTFontTableOptions(0)) };
            runs[i] = if morx {
                detect::user_turns_on_runs(user) || self.probe_differs(style)
            } else if let Some(gsub) = gsub {
                // The bytes live as long as `gsub`.
                match detect::gsub_features(unsafe { gsub.as_bytes_unchecked() }) {
                    Some(tags) => detect::gsub_shapes_runs(&tags, user),
                    None => self.probe_differs(style),
                }
            } else {
                false
            };
        }
        runs
    }

    /// [`Fonts::detect_runs`] again, for measuring what it costs.
    #[doc(hidden)]
    pub fn redetect_runs(&self, features: &[(Tag, u32)]) -> [bool; 4] {
        let faces = {
            let reg = self.registry.borrow();
            [0, 1, 2, 3].map(|i| reg.faces[i].font.clone())
        };
        self.detect_runs(&faces, features)
    }

    /// Whether `style`'s face shapes [`detect::PROBE`] into anything but
    /// the per-character glyphs: a ligature, an alternate, a deleted or
    /// added glyph.
    fn probe_differs(&self, style: Style) -> bool {
        let text: Vec<u16> = detect::PROBE.encode_utf16().collect();
        let Some(line) = self.line(&text, style) else { return false };
        let mut got = Vec::new();
        self.walk(&line, &mut shape::LineBufs::default(), |i, font, glyph, _, _| {
            got.push((i, font, glyph))
        });
        let mut seen = vec![0u32; text.len()];
        for &(i, font, glyph) in &got {
            let Some(&unit) = text.get(i) else { return true };
            if unit == b' ' as u16 {
                continue;
            }
            seen[i] += 1;
            let ch = char::from_u32(unit as u32).unwrap_or(' ');
            match self.glyph_key(ch, style) {
                Some(k) if k.font == font && k.glyph == glyph => {}
                _ => return true,
            }
        }
        text.iter().zip(&seen).any(|(&u, &n)| u != b' ' as u16 && n != 1)
    }

    /// How runs in `face` are to be shaped: [`Features::ligatures`] when the
    /// face has ligatures or contextual alternates on (ligatures design
    /// §1) and runs are allowed ([`Fonts::allow_runs`]).
    pub fn features(&self, face: Style) -> Features {
        Features {
            ligatures: self.allow_runs && self.runs[face as usize],
        }
    }

    /// Let [`Fonts::features`] report whole-run shaping for faces that
    /// have ligatures. Off by default, so nothing draws ligatures until the
    /// renderer can draw a glyph in pieces (B1.2 turns it on).
    pub fn allow_runs(&mut self, allow: bool) {
        self.allow_runs = allow;
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
        // Ligature glyphs stretched from the font's advance to the cell's,
        // about their origin, so their ink lands on the grid.
        let scaled = !color && key.variant & GlyphKey::SCALED != 0;
        let sx = if scaled { self.x_scale() } else { 1.0 } * scale;

        const PAD: f64 = 1.0;
        let min_x = (rect.origin.x * sx).floor() - PAD;
        let min_y = (rect.origin.y * scale).floor() - PAD;
        let max_x = ((rect.origin.x + rect.size.width) * sx).ceil() + PAD;
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
        CGContext::scale_ctm(Some(ctx), sx, scale);
        let mut pos = CGPoint::new(0.0, 0.0);
        unsafe { font.draw_glyphs(NonNull::from(&mut g), NonNull::from(&mut pos), 1, ctx) };
        drop(owned);

        Some(RasterGlyph {
            width: width as u32,
            height: height as u32,
            left: min_x as i32 + if color || scaled { 0 } else { self.x_shift },
            top: max_y as i32,
            color,
            data,
        })
    }

    /// Cell width / advance: how much [`GlyphKey::SCALED`] glyphs are
    /// stretched.
    fn x_scale(&self) -> f64 {
        self.metrics.cell_width as f64 / self.advance
    }

    /// Whether split glyph `glyph` of font `font`, drawn at its cell's
    /// origin, would have ink 1 pixel or more off where the cell grid puts
    /// it: x at the font's size lands at x + the centring shift, and
    /// belongs at x × cell width / advance (ligatures design §3). Glyphs
    /// whose ink stays in their own cell are within that whenever plain
    /// glyphs are. Cached per glyph.
    pub(crate) fn needs_scaling(&self, font: FontId, glyph: CGGlyph) -> bool {
        let k = self.x_scale();
        if (k - 1.0).abs() < 1e-9 {
            return false;
        }
        if let Some(&s) = self.scaled.borrow().get(&(font, glyph)) {
            return s;
        }
        let rect = {
            let reg = self.registry.borrow();
            let Some(face) = reg.faces.get(font as usize) else { return false };
            let mut g = glyph;
            let mut rect = CGRect::default();
            unsafe {
                face.font
                    .bounding_rects_for_glyphs(CTFontOrientation::Horizontal, NonNull::from(&mut g), &mut rect, 1)
            };
            rect
        };
        let off = |x: f64| (x + self.x_shift as f64 - x * k).abs();
        let s = rect.size.width > 0.0 && off(rect.origin.x).max(off(rect.origin.x + rect.size.width)) >= 1.0;
        self.scaled.borrow_mut().insert((font, glyph), s);
        s
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

/// Make the font file at `path` available to this process only
/// (`CTFontManagerRegisterFontsForURL`, process scope), as if installed:
/// for tests and benchmarks with a font macOS doesn't ship. `false` when
/// it couldn't be registered; registering the same file again is `true`.
pub fn register_font_file(path: &std::path::Path) -> bool {
    use objc2_core_foundation::CFURL;
    use objc2_core_text::{CTFontManagerRegisterFontsForURL, CTFontManagerScope};
    let Some(url) = CFURL::from_file_path(path) else { return false };
    let mut error = std::ptr::null_mut();
    let ok = unsafe { CTFontManagerRegisterFontsForURL(&url, CTFontManagerScope::Process, &mut error) };
    if !error.is_null() {
        // Already registered (kCTFontManagerErrorAlreadyRegistered, 105)
        // is fine; release the error either way.
        let error = unsafe { CFRetained::from_raw(NonNull::new_unchecked(error)) };
        return ok || error.code() == 105;
    }
    ok
}

/// The glyph a one-character CTLine gives `ch` in `font`, with the font's
/// feature settings applied (`zero`, `ss01`): `None` when it isn't exactly
/// one glyph.
pub(crate) fn glyph_by_line(font: &CTFont, ch: char) -> Option<(CFRetained<CTFont>, CGGlyph)> {
    let mut utf16 = [0u16; 2];
    let units = ch.encode_utf16(&mut utf16);
    let line = shape::ct_line(units, &shape::font_attrs(font))?;
    let runs = unsafe { line.glyph_runs() };
    if runs.count() != 1 {
        return None;
    }
    let run: &objc2_core_text::CTRun = unsafe { &*(runs.value_at_index(0) as *const objc2_core_text::CTRun) };
    if unsafe { run.glyph_count() } != 1 {
        return None;
    }
    let found = shape::run_font(run)?;
    let mut g: CGGlyph = 0;
    unsafe { run.glyphs(CFRange { location: 0, length: 1 }, NonNull::from(&mut g)) };
    (g != 0 && g != 0xFFFF).then(|| (found.retain(), g))
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

/// `face` with OpenType `features` set (`kCTFontFeatureSettingsAttribute`):
/// CTLines in it, and fallback lookups through it, use them.
fn with_features(face: &CTFont, features: &[(Tag, u32)]) -> CFRetained<CTFont> {
    use objc2_core_foundation::{CFNumber, CFType};
    use objc2_core_text::{kCTFontFeatureSettingsAttribute, kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue};
    let settings: Vec<CFRetained<CFDictionary<CFString, CFType>>> = features
        .iter()
        .map(|&(tag, value)| {
            let tag = CFString::from_str(tag.as_str());
            let value = CFNumber::new_i64(value as i64);
            let keys = unsafe { [kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue] };
            CFDictionary::from_slices(&keys, &[&**tag as &CFType, &**value as &CFType])
        })
        .collect();
    let settings = CFArray::from_retained_objects(&settings);
    unsafe {
        let attrs = CFDictionary::<CFString, CFArray<CFDictionary<CFString, CFType>>>::from_slices(
            &[kCTFontFeatureSettingsAttribute],
            &[&settings],
        );
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
