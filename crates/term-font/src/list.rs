//! The installed fonts, for pickers and `terminal-decline fonts`: every
//! family Core Text has (its collection of available fonts), each with its
//! faces and whether it is monospaced.

use std::collections::HashMap;
use std::ptr::NonNull;

use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CGSize};
use objc2_core_text::{
    CTFont, CTFontCollection, CTFontDescriptor, CTFontOrientation, CTFontSymbolicTraits, kCTFontDisplayNameAttribute,
    kCTFontFamilyNameAttribute, kCTFontNameAttribute, kCTFontStyleNameAttribute, kCTFontSymbolicTrait, kCTFontTraitsAttribute,
    kCTFontWeightTrait,
};

/// One installed font family.
#[derive(Clone, Debug, PartialEq)]
pub struct FamilyInfo {
    /// "Menlo", "JetBrainsMono Nerd Font Mono".
    pub family: String,
    /// Core Text's monospace flag on one of its faces, or equal advances
    /// for `iMW0.` in its regular face (Ayuthaya has those without the
    /// flag).
    pub monospace: bool,
    /// Its faces, upright before italic, lighter before bolder.
    pub faces: Vec<FaceInfo>,
}

/// One face of a family.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceInfo {
    /// The style name: "Regular", "Bold Italic".
    pub style: String,
    /// "Menlo-BoldItalic".
    pub postscript: String,
    /// "Menlo Bold Italic": what `font_bold` and the others take.
    pub full_name: String,
    pub bold: bool,
    pub italic: bool,
    /// Core Text's weight, -1 (thinnest) to 1 (heaviest); 0 is regular.
    pub weight: f64,
}

impl FamilyInfo {
    /// The face named `name` (full or PostScript name, any case).
    pub fn face(&self, name: &str) -> Option<&FaceInfo> {
        let name = name.trim();
        self.faces
            .iter()
            .find(|f| f.full_name.eq_ignore_ascii_case(name) || f.postscript.eq_ignore_ascii_case(name))
    }
}

/// Every installed family, sorted by name ignoring case. Families whose
/// name starts with "." (the system's private ones) are left out. Takes
/// 100–300 ms: the monospace check opens each family's regular face.
pub fn list_families() -> Vec<FamilyInfo> {
    let collection = unsafe { CTFontCollection::from_available_fonts(None) };
    let Some(descs) = (unsafe { collection.matching_font_descriptors() }) else {
        return Vec::new();
    };
    let mut families: HashMap<String, (FamilyInfo, bool)> = HashMap::new();
    for i in 0..descs.count() {
        let d = unsafe { descs.value_at_index(i) } as *const CTFontDescriptor;
        let Some(d) = (unsafe { d.as_ref() }) else { continue };
        let Some(family) = string_attr(d, unsafe { kCTFontFamilyNameAttribute }) else {
            continue;
        };
        if family.starts_with('.') || family.is_empty() {
            continue;
        }
        let style = string_attr(d, unsafe { kCTFontStyleNameAttribute }).unwrap_or_else(|| "Regular".into());
        let postscript = string_attr(d, unsafe { kCTFontNameAttribute }).unwrap_or_default();
        let full_name = string_attr(d, unsafe { kCTFontDisplayNameAttribute }).unwrap_or_else(|| format!("{family} {style}"));
        let (traits, weight) = traits(d);
        let face = FaceInfo {
            style,
            postscript,
            full_name,
            bold: traits.contains(CTFontSymbolicTraits::BoldTrait),
            italic: traits.contains(CTFontSymbolicTraits::ItalicTrait),
            weight,
        };
        let flagged = traits.contains(CTFontSymbolicTraits::TraitMonoSpace);
        let entry = families.entry(family.clone()).or_insert_with(|| {
            (
                FamilyInfo {
                    family,
                    monospace: false,
                    faces: Vec::new(),
                },
                false,
            )
        });
        entry.1 |= flagged;
        if !entry.0.faces.iter().any(|f| f.postscript == face.postscript) {
            entry.0.faces.push(face);
        }
    }
    let mut out: Vec<FamilyInfo> = families
        .into_values()
        .map(|(mut info, flagged)| {
            info.monospace = flagged || equal_advances(&info.family);
            info.faces.sort_by(|a, b| {
                a.italic
                    .cmp(&b.italic)
                    .then(a.weight.total_cmp(&b.weight))
                    .then(a.style.cmp(&b.style))
            });
            info
        })
        .collect();
    out.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()).then(a.family.cmp(&b.family)));
    out
}

fn string_attr(d: &CTFontDescriptor, key: &CFString) -> Option<String> {
    let v = unsafe { d.attribute(key) }?;
    Some(v.downcast::<CFString>().ok()?.to_string())
}

/// The symbolic traits and weight a descriptor says it has.
fn traits(d: &CTFontDescriptor) -> (CTFontSymbolicTraits, f64) {
    let none = (CTFontSymbolicTraits(0), 0.0);
    let Some(v) = (unsafe { d.attribute(kCTFontTraitsAttribute) }) else {
        return none;
    };
    let Ok(dict) = v.downcast::<CFDictionary>() else { return none };
    // The traits dictionary maps trait names to numbers.
    let dict: &CFDictionary<CFString, CFNumber> = unsafe { dict.cast_unchecked() };
    let symbolic = dict.get(unsafe { kCTFontSymbolicTrait }).and_then(|n| n.as_i64()).unwrap_or(0);
    let weight = dict.get(unsafe { kCTFontWeightTrait }).and_then(|n| n.as_f64()).unwrap_or(0.0);
    (CTFontSymbolicTraits(symbolic as u32), weight)
}

/// Whether `family`'s regular face draws `iMW0.` with one advance.
fn equal_advances(family: &str) -> bool {
    let font: CFRetained<CTFont> = unsafe { CTFont::with_name(&CFString::from_str(family), 12.0, std::ptr::null()) };
    let mut chars: Vec<u16> = "iMW0.".encode_utf16().collect();
    let mut glyphs = vec![0u16; chars.len()];
    let mut advances = vec![CGSize::new(0.0, 0.0); chars.len()];
    let n = chars.len() as isize;
    unsafe {
        let found = font.glyphs_for_characters(
            NonNull::new_unchecked(chars.as_mut_ptr()),
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            n,
        );
        if !found {
            return false;
        }
        font.advances_for_glyphs(
            CTFontOrientation::Default,
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            advances.as_mut_ptr(),
            n,
        );
    }
    advances.iter().all(|a| a.width > 0.0 && (a.width - advances[0].width).abs() < 0.01)
}

/// `terminal-decline fonts`' listing of `families`: one family a line,
/// `*` before the monospaced ones (only those with `mono`), and with
/// `styles` each face under its family, style and PostScript name.
pub fn format_families(families: &[FamilyInfo], mono: bool, styles: bool) -> String {
    let mut out = String::new();
    for f in families.iter().filter(|f| !mono || f.monospace) {
        out.push_str(if f.monospace { "* " } else { "  " });
        out.push_str(&f.family);
        out.push('\n');
        if styles {
            let width = f.faces.iter().map(|s| s.style.chars().count()).max().unwrap_or(0);
            for face in &f.faces {
                out.push_str(&format!("      {:width$}  {}\n", face.style, face.postscript));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menlo_is_monospaced_with_four_faces() {
        let all = list_families();
        let menlo = all.iter().find(|f| f.family == "Menlo").expect("Menlo ships with macOS");
        assert!(menlo.monospace);
        let face = |bold, italic| menlo.faces.iter().find(|f| f.bold == bold && f.italic == italic);
        assert_eq!(face(false, false).map(|f| f.full_name.as_str()), Some("Menlo Regular"));
        assert_eq!(face(true, false).map(|f| f.postscript.as_str()), Some("Menlo-Bold"));
        assert_eq!(face(false, true).map(|f| f.style.as_str()), Some("Italic"));
        assert_eq!(face(true, true).map(|f| f.full_name.as_str()), Some("Menlo Bold Italic"));
        assert!(menlo.face("menlo-bold").is_some_and(|f| f.bold && f.weight > 0.0));
        // Regular first: upright before italic, lighter before bolder.
        assert_eq!(menlo.faces[0].style, "Regular");
        let helvetica = all.iter().find(|f| f.family == "Helvetica").expect("Helvetica ships with macOS");
        assert!(!helvetica.monospace);
        assert!(all.iter().all(|f| !f.family.starts_with('.')));
        let lower: Vec<String> = all.iter().map(|f| f.family.to_lowercase()).collect();
        assert!(lower.windows(2).all(|w| w[0] <= w[1]), "sorted ignoring case");
    }

    #[test]
    fn listing_marks_monospaced_families() {
        let face = |style: &str, ps: &str| FaceInfo {
            style: style.into(),
            postscript: ps.into(),
            full_name: String::new(),
            bold: false,
            italic: false,
            weight: 0.0,
        };
        let families = [
            FamilyInfo {
                family: "Helvetica".into(),
                monospace: false,
                faces: vec![face("Regular", "Helvetica")],
            },
            FamilyInfo {
                family: "Menlo".into(),
                monospace: true,
                faces: vec![face("Regular", "Menlo-Regular"), face("Bold Italic", "Menlo-BoldItalic")],
            },
        ];
        assert_eq!(format_families(&families, false, false), "  Helvetica\n* Menlo\n");
        assert_eq!(format_families(&families, true, false), "* Menlo\n");
        assert_eq!(
            format_families(&families, true, true),
            "* Menlo\n      Regular      Menlo-Regular\n      Bold Italic  Menlo-BoldItalic\n"
        );
    }
}
