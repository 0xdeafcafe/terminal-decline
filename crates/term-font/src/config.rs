//! What the user can set about fonts (roadmap E.4): the family and size,
//! a fallback list tried before Core Text's own, bold and italic faces by
//! name, line height, a cell width adjustment and OpenType features (B.1).

use std::fmt;

/// Font settings for [`Fonts::with_config`](crate::Fonts::with_config).
/// `FontConfig::new(family, size)` with nothing else set gives the same
/// fonts and metrics as [`Fonts::new`](crate::Fonts::new).
#[derive(Clone, Debug, PartialEq)]
pub struct FontConfig {
    /// Family of the regular face, e.g. "Menlo".
    pub family: String,
    /// Size in points; `with_config`'s `scale` turns it into pixels.
    pub size: f64,
    /// Fonts tried, in order, for characters the configured face lacks,
    /// before Core Text's own fallback list. Family, full or PostScript
    /// names; names that aren't installed are skipped (see
    /// [`Fonts::missing`](crate::Fonts::missing)).
    pub fallback: Vec<String>,
    /// The bold face by name, e.g. "JetBrains Mono ExtraBold". `None`:
    /// the family's own bold.
    pub bold: Option<String>,
    /// The italic face by name. `None`: the family's own italic.
    pub italic: Option<String>,
    /// The bold italic face by name. `None`: the family's own bold italic.
    pub bold_italic: Option<String>,
    /// Multiplies the cell height; 1.0 is the font's own line height. The
    /// text stays centred in the cell, so extra height goes half above and
    /// half below.
    pub line_height: f64,
    /// Makes cells wider or narrower than the font's advance.
    pub cell_width: CellWidth,
    /// OpenType features turned on (value 1 or more, the alternate to use)
    /// or off (0) on all four faces, in order: `font_features = ["-calt",
    /// "+ss01", "zero", "cv05=2"]`, each entry parsed by
    /// [`Tag::parse_feature`]. Empty: the font's own defaults.
    pub features: Vec<(Tag, u32)>,
    /// More features for one face, by [`Style`](crate::Style) index, after
    /// `features` (so they win): `font_features_bold = ["-liga"]`.
    pub style_features: [Vec<(Tag, u32)>; 4],
    /// Variable font axes per face, by [`Style`](crate::Style) index:
    /// `wght=450`, `wdth=90`. Axes the font doesn't have are ignored by
    /// Core Text. Empty: the face's own instance.
    pub variations: [Vec<(Tag, f64)>; 4],
    /// Stem darkening: glyphs drawn with a stroke this many points wide
    /// (at most 4) around their outline as well as filled, 0 for none (the
    /// default). Colour glyphs and code-drawn shapes aren't thickened.
    pub thicken: f64,
    /// Draw bold by thickening the regular face when the family has no
    /// bold face (the default); off, such bold text is drawn regular.
    pub synthetic_bold: bool,
    /// Draw italic by slanting the regular face when the family has no
    /// italic face (the default); off, such italic text is drawn upright.
    pub synthetic_italic: bool,
    /// Added to the cell height after `line_height`: points, or percent of
    /// the height. The text stays centred.
    pub cell_height: Adjust,
    /// Moves the text up (positive) or down within the cell: points, or
    /// percent of the cell height.
    pub baseline: Adjust,
    /// Moves underlines down (positive) or up: points, or percent of the
    /// cell height.
    pub underline_position: Adjust,
    /// Added to the underline thickness: points, or percent of it.
    pub underline_thickness: Adjust,
    /// Moves the strikethrough down (positive) or up: points, or percent
    /// of the cell height.
    pub strike_position: Adjust,
    /// Added to the strikethrough thickness (by default the underline's):
    /// points, or percent of it.
    pub strike_thickness: Adjust,
    /// Added to the bar and underline cursor's thickness: points, or
    /// percent of it.
    pub cursor_thickness: Adjust,
    /// Code point ranges drawn from a font of their own before the face
    /// and the fallbacks are tried, for Nerd Font icons and the like:
    /// `(0xE000..=0xF8FF, "Symbols Nerd Font Mono")`. Names that aren't
    /// installed are skipped (see [`Fonts::missing`](crate::Fonts::missing)).
    pub symbol_map: Vec<(std::ops::RangeInclusive<u32>, String)>,
    /// The font colour glyphs (emoji) are drawn from when it has them,
    /// in place of the colour font Core Text picks. `None`: Core Text's
    /// (Apple Color Emoji).
    pub emoji_font: Option<String>,
}

impl FontConfig {
    /// `family` at `size` points, everything else as the font has it.
    pub fn new(family: &str, size: f64) -> FontConfig {
        FontConfig {
            family: family.to_string(),
            size,
            fallback: Vec::new(),
            bold: None,
            italic: None,
            bold_italic: None,
            line_height: 1.0,
            cell_width: CellWidth::default(),
            features: Vec::new(),
            style_features: Default::default(),
            variations: Default::default(),
            thicken: 0.0,
            synthetic_bold: true,
            synthetic_italic: true,
            cell_height: Adjust::default(),
            baseline: Adjust::default(),
            underline_position: Adjust::default(),
            underline_thickness: Adjust::default(),
            strike_position: Adjust::default(),
            strike_thickness: Adjust::default(),
            cursor_thickness: Adjust::default(),
            symbol_map: Vec::new(),
            emoji_font: None,
        }
    }

    /// The features face `style` is drawn with: `features`, then its own.
    pub fn features_of(&self, style: crate::Style) -> Vec<(Tag, u32)> {
        let mut f = self.features.clone();
        f.extend_from_slice(&self.style_features[style as usize]);
        f
    }
}

/// Parse one `font_variations` entry: `tag=value`, e.g. `wght=450` or
/// `wdth = 87.5`. `None` for anything else.
pub fn parse_variation(s: &str) -> Option<(Tag, f64)> {
    let (t, v) = s.split_once('=')?;
    let v: f64 = v.trim().parse().ok()?;
    v.is_finite().then_some((Tag::new(t.trim())?, v))
}

/// Parse one `symbol_map` key: code points and ranges separated by commas,
/// `U+E0A0-U+E0D4,U+F000` (the `U+` is optional, the digits hex). `None`
/// for anything else, an empty list, or a range that runs backwards.
pub fn parse_ranges(s: &str) -> Option<Vec<std::ops::RangeInclusive<u32>>> {
    let point = |t: &str| {
        let t = t.trim();
        let t = t.strip_prefix("U+").or_else(|| t.strip_prefix("u+")).unwrap_or(t);
        if t.is_empty() || t.len() > 6 {
            return None;
        }
        u32::from_str_radix(t, 16).ok().filter(|&v| v <= 0x10FFFF)
    };
    let mut out = Vec::new();
    for part in s.split(',') {
        let r = match part.split_once('-') {
            Some((a, b)) => point(a)?..=point(b)?,
            None => {
                let p = point(part)?;
                p..=p
            }
        };
        if r.start() > r.end() {
            return None;
        }
        out.push(r);
    }
    (!out.is_empty()).then_some(out)
}

/// An OpenType feature tag: four ASCII characters, e.g. `liga`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tag(pub [u8; 4]);

impl Tag {
    /// The tag `s` names: exactly four printable ASCII characters.
    pub fn new(s: &str) -> Option<Tag> {
        let b: [u8; 4] = s.as_bytes().try_into().ok()?;
        b.iter().all(|c| c.is_ascii_graphic()).then_some(Tag(b))
    }

    /// The tag as a string.
    pub fn as_str(&self) -> &str {
        // Only ever built from ASCII.
        std::str::from_utf8(&self.0).unwrap_or("????")
    }

    /// The tag as a big-endian number, as font tables store it.
    pub fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    /// Parse one `font_features` entry, the syntax of Ghostty's
    /// `font-feature` and kitty's: `tag` or `+tag` turns it on (1), `-tag`
    /// off (0), `tag=n` sets value `n` (an alternate's number). `None` for
    /// anything else.
    pub fn parse_feature(s: &str) -> Option<(Tag, u32)> {
        let s = s.trim();
        if let Some(t) = s.strip_prefix('-') {
            return Some((Tag::new(t)?, 0));
        }
        if let Some(t) = s.strip_prefix('+') {
            return Some((Tag::new(t)?, 1));
        }
        match s.split_once('=') {
            Some((t, n)) => Some((Tag::new(t.trim())?, n.trim().parse().ok()?)),
            None => Some((Tag::new(s)?, 1)),
        }
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tag({:?})", self.as_str())
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An adjustment to a metric: points (scaled to pixels like the font
/// size) or a percentage. The same syntax as [`CellWidth`].
pub type Adjust = CellWidth;

/// An adjustment to the cell width. Glyphs are centred in the cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CellWidth {
    /// Add this many points (scaled to pixels like the font size); may be
    /// negative.
    Points(f64),
    /// Add this percentage of the font's advance; may be negative.
    Percent(f64),
}

impl Default for CellWidth {
    fn default() -> CellWidth {
        CellWidth::Points(0.0)
    }
}

impl CellWidth {
    /// Parse a config value: `"1"`, `"-0.5"` (points) or `"10%"`, `"-5%"`.
    pub fn parse(s: &str) -> Option<CellWidth> {
        let s = s.trim();
        let num = |t: &str| t.trim().parse::<f64>().ok().filter(|v| v.is_finite());
        match s.strip_suffix('%') {
            Some(p) => num(p).map(CellWidth::Percent),
            None => num(s).map(CellWidth::Points),
        }
    }

    /// The change in pixels to a value: points at `scale` pixels a point,
    /// or the percentage of `of` (pixels).
    pub(crate) fn delta(self, of: f64, scale: f64) -> f64 {
        match self {
            CellWidth::Points(p) => p * scale,
            CellWidth::Percent(p) => of * p / 100.0,
        }
    }

    /// Whether it changes nothing.
    pub fn is_zero(self) -> bool {
        matches!(self, CellWidth::Points(0.0) | CellWidth::Percent(0.0))
    }

    /// The cell width for a font whose advance is `advance` pixels, at
    /// `scale` pixels a point: never under 1 pixel.
    pub(crate) fn apply(self, advance: f64, scale: f64) -> u32 {
        let natural = advance.round().max(1.0);
        let delta = match self {
            CellWidth::Points(p) => (p * scale).round(),
            CellWidth::Percent(p) => (advance * p / 100.0).round(),
        };
        (natural + delta).max(1.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_points_and_percent() {
        assert_eq!(CellWidth::parse("2"), Some(CellWidth::Points(2.0)));
        assert_eq!(CellWidth::parse(" -0.5 "), Some(CellWidth::Points(-0.5)));
        assert_eq!(CellWidth::parse("10%"), Some(CellWidth::Percent(10.0)));
        assert_eq!(CellWidth::parse("-5 %"), Some(CellWidth::Percent(-5.0)));
        assert_eq!(CellWidth::parse("wide"), None);
        assert_eq!(CellWidth::parse("inf"), None);
    }

    #[test]
    fn parses_features() {
        let t = |s| Tag::new(s).unwrap();
        assert_eq!(Tag::parse_feature("zero"), Some((t("zero"), 1)));
        assert_eq!(Tag::parse_feature("+ss01"), Some((t("ss01"), 1)));
        assert_eq!(Tag::parse_feature(" -calt "), Some((t("calt"), 0)));
        assert_eq!(Tag::parse_feature("cv05=2"), Some((t("cv05"), 2)));
        assert_eq!(Tag::parse_feature("cv05 = 0"), Some((t("cv05"), 0)));
        for bad in [
            "",
            "lig",
            "ligat",
            "+-liga",
            "li a",
            "liga=",
            "liga=x",
            "liga=-1",
            "+liga=2",
            "caf\u{e9}",
        ] {
            assert_eq!(Tag::parse_feature(bad), None, "{bad:?}");
        }
        assert_eq!(t("liga").to_u32(), 0x6c69_6761);
        assert_eq!(t("liga").to_string(), "liga");
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a list of one range is what's meant
    fn parses_variations_and_ranges() {
        let t = |s| Tag::new(s).unwrap();
        assert_eq!(parse_variation("wght=450"), Some((t("wght"), 450.0)));
        assert_eq!(parse_variation(" wdth = 87.5 "), Some((t("wdth"), 87.5)));
        for bad in ["wght", "wght=", "wght=x", "wgh=4", "wght=inf", "=4"] {
            assert_eq!(parse_variation(bad), None, "{bad:?}");
        }
        assert_eq!(parse_ranges("U+E0A0-U+E0D4,U+F000"), Some(vec![0xE0A0..=0xE0D4, 0xF000..=0xF000]));
        assert_eq!(parse_ranges("e000 - f8ff"), Some(vec![0xE000..=0xF8FF]));
        for bad in ["", "U+", "U+G000", "U+F000-U+E000", "U+110000", "U+E000,", "U+E000-"] {
            assert_eq!(parse_ranges(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn deltas() {
        assert_eq!(Adjust::Points(1.5).delta(10.0, 2.0), 3.0);
        assert_eq!(Adjust::Percent(-50.0).delta(10.0, 2.0), -5.0);
        assert!(Adjust::default().is_zero() && Adjust::Percent(0.0).is_zero());
        assert!(!Adjust::Points(0.1).is_zero());
    }

    #[test]
    fn applies_to_the_advance() {
        assert_eq!(CellWidth::default().apply(15.6, 2.0), 16);
        assert_eq!(CellWidth::Points(1.0).apply(15.6, 2.0), 18);
        assert_eq!(CellWidth::Points(-1.0).apply(15.6, 2.0), 14);
        assert_eq!(CellWidth::Percent(10.0).apply(15.6, 2.0), 18);
        assert_eq!(CellWidth::Points(-100.0).apply(15.6, 2.0), 1);
    }
}
