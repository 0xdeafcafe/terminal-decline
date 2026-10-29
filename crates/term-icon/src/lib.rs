//! Generated Dock pin icons by the Slope rule, `slope-pins-2`
//! (docs/brand/README.md, "Generation contract"; docs/design/pins.md §8).
//!
//! [`plan`] works out the canonical name, hash, palette entry and initial
//! (steps 1–3); [`outline`] turns the initial into a path with Core Text;
//! [`generate`] does both, with the fallbacks for initials that have no
//! outline. What they produce is what a pin keeps ([`PinIcon`]), so an
//! icon is drawn again from the kept record ([`render_icns`],
//! [`render_png`]) and never depends on the fonts of a later macOS.

mod draw;
mod outline;

pub use draw::{ICNS_ENTRIES, ICNS_SIZES, render_icns, render_png, render_rgba};
pub use outline::outline;

use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};
use unicode_segmentation::UnicodeSegmentation;

/// The generator version kept with every pin.
pub const GENERATOR: &str = "slope-pins-2";

/// One palette entry: the body gradient's top and bottom colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hue {
    pub name: &'static str,
    /// HSL hue in degrees; describes the entry, never used to draw.
    pub degrees: u16,
    pub top: [u8; 3],
    pub bottom: [u8; 3],
}

const fn hue(name: &'static str, degrees: u16, top: u32, bottom: u32) -> Hue {
    Hue {
        name,
        degrees,
        top: [(top >> 16) as u8, (top >> 8) as u8, top as u8],
        bottom: [(bottom >> 16) as u8, (bottom >> 8) as u8, bottom as u8],
    }
}

/// The ten-entry palette, in the brand doc's order (the order is part of
/// the format: `index = hash % 10` picks from it).
pub const PALETTE: [Hue; 10] = [
    hue("Cyan", 180, 0x205B5B, 0x102D2D),
    hue("Azure", 200, 0x20475B, 0x10232D),
    hue("Amber", 40, 0x5B4720, 0x2D2310),
    hue("Violet", 270, 0x3D205B, 0x1F102D),
    hue("Blue", 220, 0x20335B, 0x101A2D),
    hue("Teal", 170, 0x205B51, 0x102D28),
    hue("Indigo", 235, 0x20255B, 0x10122D),
    hue("Periwinkle", 250, 0x2A205B, 0x15102D),
    hue("Orchid", 290, 0x51205B, 0x28102D),
    hue("Plum", 315, 0x5B204C, 0x2D1026),
];

/// Everything a generated pin icon is drawn from, kept in the pin's
/// record so it is never worked out again (contract step 5).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PinIcon {
    /// [`GENERATOR`].
    pub generator: &'static str,
    /// Step 1: NFC, White_Space trimmed, ASCII folded to lower case.
    pub canonical: String,
    /// Step 2: FNV-1a 32 of the canonical name's UTF-8.
    pub hash: u32,
    /// `hash % 10`, into [`PALETTE`].
    pub index: u8,
    /// The palette entry's colours, kept so a later palette change can't
    /// move an existing pin.
    pub top: [u8; 3],
    pub bottom: [u8; 3],
    /// Step 3: the uppercased first cluster with a Letter or Number, or
    /// `#`. After [`generate`], what was actually outlined.
    pub initial: String,
    /// The initial's outline fitted into the 1024 grid, top-down, as SVG
    /// path data (`M`, `L`, `Q`, `C`, `Z` and numbers only). Empty after
    /// [`plan`]; filled in by [`generate`].
    pub outline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconError {
    /// No font draws the initial as an outline (a colour emoji, say).
    NoOutline(String),
    /// The kept outline isn't path data this crate wrote.
    BadOutline(String),
    /// CoreGraphics or ImageIO refused.
    Graphics(&'static str),
}

impl std::fmt::Display for IconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IconError::NoOutline(s) => write!(f, "no font has an outline for {s:?}"),
            IconError::BadOutline(s) => write!(f, "not an outline: {s}"),
            IconError::Graphics(s) => write!(f, "drawing the icon failed: {s}"),
        }
    }
}

impl std::error::Error for IconError {}

/// Unicode White_Space, the code points the contract lists (and only
/// those), trimmed from both ends of a name.
pub const WHITE_SPACE: [char; 25] = [
    '\u{0009}', '\u{000A}', '\u{000B}', '\u{000C}', '\u{000D}', '\u{0020}', '\u{0085}', '\u{00A0}', '\u{1680}', '\u{2000}', '\u{2001}',
    '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200A}', '\u{2028}', '\u{2029}',
    '\u{202F}', '\u{205F}', '\u{3000}',
];

/// The display name in NFC with White_Space trimmed: what the initial is
/// taken from.
fn trimmed(display_name: &str) -> String {
    let nfc: String = display_name.nfc().collect();
    nfc.trim_matches(|c| WHITE_SPACE.contains(&c)).to_owned()
}

/// Step 1: the canonical name. Locale-independent: only ASCII A–Z fold.
pub fn canonical(display_name: &str) -> String {
    trimmed(display_name).chars().map(|c| c.to_ascii_lowercase()).collect()
}

/// Step 2: FNV-1a 32.
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(2_166_136_261u32, |h, &b| (h ^ b as u32).wrapping_mul(16_777_619))
}

fn letter_or_number(c: char) -> bool {
    matches!(
        c.general_category_group(),
        GeneralCategoryGroup::Letter | GeneralCategoryGroup::Number
    )
}

/// Step 3: the visible initial, from the NFC, trimmed display name.
pub fn initial(display_name: &str) -> String {
    let name = trimmed(display_name);
    match name.graphemes(true).find(|g| g.chars().any(letter_or_number)) {
        Some(cluster) => cluster.to_uppercase(),
        None => "#".to_owned(),
    }
}

/// Steps 1–3, without the outline.
pub fn plan(display_name: &str) -> PinIcon {
    let canonical = canonical(display_name);
    let hash = fnv1a32(canonical.as_bytes());
    let index = (hash % 10) as u8;
    let entry = PALETTE[index as usize];
    PinIcon {
        generator: GENERATOR,
        canonical,
        hash,
        index,
        top: entry.top,
        bottom: entry.bottom,
        initial: initial(display_name),
        outline: String::new(),
    }
}

/// A complete icon for a new pin: [`plan`], then the initial's
/// [`outline`]. An initial no font outlines (a keycap emoji: its cluster
/// holds a Number, but Apple Color Emoji has no outlines) falls back to
/// the cluster's first Letter or Number alone, then to `#`; `initial`
/// then says what was drawn.
pub fn generate(display_name: &str) -> Result<PinIcon, IconError> {
    let mut icon = plan(display_name);
    let mut tries = vec![icon.initial.clone()];
    if let Some(c) = icon.initial.chars().find(|&c| letter_or_number(c)) {
        tries.push(c.to_uppercase().collect());
    }
    tries.push("#".to_owned());
    let mut last = None;
    for t in tries {
        match outline(&t) {
            Ok(o) => {
                icon.initial = t;
                icon.outline = o;
                return Ok(icon);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.expect("at least one try"))
}

#[cfg(test)]
mod tests;
