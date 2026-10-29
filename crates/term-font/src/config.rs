//! What the user can set about fonts (roadmap E.4): the family and size,
//! a fallback list tried before Core Text's own, bold and italic faces by
//! name, line height and a cell width adjustment.

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
        }
    }
}

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
    fn applies_to_the_advance() {
        assert_eq!(CellWidth::default().apply(15.6, 2.0), 16);
        assert_eq!(CellWidth::Points(1.0).apply(15.6, 2.0), 18);
        assert_eq!(CellWidth::Points(-1.0).apply(15.6, 2.0), 14);
        assert_eq!(CellWidth::Percent(10.0).apply(15.6, 2.0), 18);
        assert_eq!(CellWidth::Points(-100.0).apply(15.6, 2.0), 1);
    }
}
