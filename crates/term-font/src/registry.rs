//! The font registry and glyph keys (cells design §10.2).
//!
//! Core Text picks fallback fonts itself (CJK, emoji, symbols), so a glyph
//! id alone doesn't say which font to draw it from. The registry gives every
//! font face a glyph has come from a small [`FontId`]: the four configured
//! faces first, then each fallback as it is first seen. A [`GlyphKey`] is a
//! font id, a glyph id and the cells the glyph covers, which is everything
//! needed to rasterise it again.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use objc2_core_foundation::{CFRetained, CFType, Type};
use objc2_core_graphics::CGGlyph;
use objc2_core_text::{CTFont, CTFontSymbolicTraits};

use crate::{SpriteId, Style};

/// Names one font face in a [`Fonts`](crate::Fonts)' registry. Ids 0–3 are
/// the configured faces in [`Style`] order; fallbacks follow.
pub type FontId = u16;

/// The reserved font id for code-drawn glyphs ([`crate::sprite`]): a
/// [`GlyphKey`] with this font has a [`SpriteId`] as its glyph.
pub const SPRITE_FONT: FontId = u16::MAX;

/// Names one rasterised bitmap.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    /// [`SPRITE_FONT`] for code-drawn glyphs.
    pub font: FontId,
    /// A `CGGlyph` in that font, or a [`SpriteId`].
    pub glyph: u16,
    /// Cells the glyph covers: 1, or 2 for wide glyphs. Colour glyphs are
    /// fitted to these cells and sprites drawn across them; coverage glyphs
    /// are drawn at the font's size whatever this says.
    pub cells: u8,
    /// Flags for how the glyph is drawn: [`GlyphKey::FITTED`] for icons;
    /// the other bits are spare (synthetic bold, a later subpixel phase).
    pub variant: u8,
}

impl GlyphKey {
    /// `variant` flag: an icon, scaled down and centred to fit `cells`
    /// cells and never drawn past them (E.4; see `term_font::fit`).
    pub const FITTED: u8 = 1;

    /// The key for a code-drawn shape across `cells` cells.
    pub fn sprite(id: SpriteId, cells: u8) -> GlyphKey {
        GlyphKey {
            font: SPRITE_FONT,
            glyph: id.0,
            cells,
            variant: 0,
        }
    }

    /// The sprite this key names, if it is a sprite key.
    pub fn sprite_id(self) -> Option<SpriteId> {
        (self.font == SPRITE_FONT).then_some(SpriteId(self.glyph))
    }
}

pub(crate) struct Face {
    pub font: CFRetained<CTFont>,
    /// The font has colour glyphs (emoji): bitmaps are BGRA.
    pub color: bool,
}

/// A CTFont compared by `CFEqual` and hashed by `CFHash`, so the same face
/// reached from different lookups gets one id.
struct CfKey(CFRetained<CTFont>);

impl CfKey {
    fn cf(&self) -> &CFType {
        &self.0
    }
}

impl PartialEq for CfKey {
    fn eq(&self, other: &CfKey) -> bool {
        self.cf() == other.cf()
    }
}

impl Eq for CfKey {}

impl Hash for CfKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.cf().hash(state)
    }
}

/// What makes two CTFonts draw the same glyphs: PostScript name, size and
/// variation axes. `CFEqual` is stricter than this: the font
/// `CTFontCreateForString` returns for an emoji and the one CTLine puts in
/// its run are different objects that aren't `CFEqual`, though their
/// descriptors are identical, and without this they got two ids and every
/// glyph two bitmaps.
struct FaceKey {
    name: String,
    size: u64,
    variation: Option<CFRetained<CFType>>,
}

impl FaceKey {
    fn of(font: &CTFont) -> FaceKey {
        unsafe {
            FaceKey {
                name: font.post_script_name().to_string(),
                size: font.size().to_bits(),
                variation: font.variation().map(|v| CFRetained::<CFType>::from(&*v)),
            }
        }
    }
}

impl PartialEq for FaceKey {
    fn eq(&self, other: &FaceKey) -> bool {
        self.name == other.name && self.size == other.size && self.variation == other.variation
    }
}

impl Eq for FaceKey {}

impl Hash for FaceKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (&self.name, self.size).hash(state)
    }
}

/// What a character resolves to in one style: a font id and glyph, or
/// nothing (no font has it, or the registry is full).
pub(crate) type Resolved = Option<(FontId, CGGlyph)>;

pub(crate) struct Registry {
    pub faces: Vec<Face>,
    /// Every font object seen, by `CFEqual`: the fast lookup.
    by_font: HashMap<CfKey, FontId>,
    /// One per face, by name, size and variation, for font objects
    /// `by_font` hasn't seen yet.
    by_face: HashMap<FaceKey, FontId>,
    /// ASCII lookups per style skip the hash map.
    ascii: [[Option<Resolved>; 128]; 4],
    /// Every other character looked up, per style: the cmap lookup and,
    /// when the face lacks it, Core Text's fallback search happen once.
    other: HashMap<(char, Style), Resolved>,
}

impl Registry {
    pub fn new(faces: [CFRetained<CTFont>; 4]) -> Registry {
        let mut reg = Registry {
            faces: Vec::with_capacity(8),
            by_font: HashMap::new(),
            by_face: HashMap::new(),
            ascii: [[None; 128]; 4],
            other: HashMap::new(),
        };
        // Ids 0–3 are the four faces even when two are the same font (a
        // family with no bold); lookups by font find the first.
        for font in faces {
            let id = reg.faces.len() as FontId;
            reg.by_font.entry(CfKey(font.clone())).or_insert(id);
            reg.by_face.entry(FaceKey::of(&font)).or_insert(id);
            reg.faces.push(face(font));
        }
        reg
    }

    /// The id for `font`, registering it if it is new. `None` once every id
    /// below [`SPRITE_FONT`] is taken.
    pub fn id(&mut self, font: &CTFont) -> Option<FontId> {
        let key = CfKey(font.retain());
        if let Some(&id) = self.by_font.get(&key) {
            return Some(id);
        }
        // Another object for a face already registered: remember it too, so
        // the next lookup is the cheap one. Core Text hands out a stable
        // object per face and lookup path, so this adds a few entries, not
        // one per line.
        let face_key = FaceKey::of(font);
        if let Some(&id) = self.by_face.get(&face_key) {
            self.by_font.insert(key, id);
            return Some(id);
        }
        if self.faces.len() >= SPRITE_FONT as usize {
            return None;
        }
        let id = self.faces.len() as FontId;
        self.faces.push(face(key.0.clone()));
        self.by_font.insert(key, id);
        self.by_face.insert(face_key, id);
        Some(id)
    }

    /// `ch` in `style`, through the per-character cache.
    pub fn resolve(&mut self, ch: char, style: Style) -> Resolved {
        let cached = if ch.is_ascii() {
            self.ascii[style as usize][ch as usize]
        } else {
            self.other.get(&(ch, style)).copied()
        };
        if let Some(r) = cached {
            return r;
        }
        let r = crate::glyph_for(&self.faces[style as usize].font, ch).and_then(|(font, glyph)| Some((self.id(&font)?, glyph)));
        if ch.is_ascii() {
            self.ascii[style as usize][ch as usize] = Some(r);
        } else {
            self.other.insert((ch, style), r);
        }
        r
    }
}

fn face(font: CFRetained<CTFont>) -> Face {
    let color = unsafe { font.symbolic_traits() }.contains(CTFontSymbolicTraits::ColorGlyphsTrait);
    Face { font, color }
}
