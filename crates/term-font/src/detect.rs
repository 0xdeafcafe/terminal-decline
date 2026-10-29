//! Which faces shape whole runs (ligatures design §1).
//!
//! Decided once per face when [`Fonts`](crate::Fonts) is built. OpenType
//! faces are read from their `GSUB` table: the features of the `latn`
//! script's default language system, else `DFLT`'s. Faces with Apple's
//! `morx` table are probed with a line of programming sequences instead
//! (in `lib.rs`), since Core Text prefers `morx` and its features aren't
//! listed by tag.

use crate::Tag;

/// The probe line for `morx` faces: programming sequences that ligature
/// fonts substitute, and Latin ligatures. Checked on macOS 27: Menlo,
/// Monaco and Courier shape every character to its own glyph.
pub(crate) const PROBE: &str = "-> => != == === !== <= >= <=> <- <-- --> :: ::: ... .. ..= /* */ // /// && || |> <| \
    ++ -- ** ## #{ #[ </ /> <!-- ?? ?. ;; ~> <~ :=  =:= <*> www 0x1f fi fl ffi ffl";

/// Features on by default that substitute across characters.
const DEFAULT_ON: [&[u8; 4]; 5] = [b"liga", b"clig", b"calt", b"rlig", b"rclt"];

/// Whether `tag` can substitute across characters. Features that act
/// inside a cluster (already shaped by the cluster path) and features that
/// swap one glyph for another (resolved per character) don't.
pub(crate) fn across(tag: Tag) -> bool {
    let t = &tag.0;
    let inside = [b"ccmp", b"locl", b"mark", b"mkmk", b"kern", b"zero", b"case"];
    if inside.contains(&t) {
        return false;
    }
    let num = |a: u8, b: u8| a.is_ascii_digit() && b.is_ascii_digit();
    let n = |a: u8, b: u8| (a - b'0') * 10 + (b - b'0');
    match t {
        [b's', b's', a, b] if num(*a, *b) => !(1..=20).contains(&n(*a, *b)),
        [b'c', b'v', a, b] if num(*a, *b) => !(1..=99).contains(&n(*a, *b)),
        _ => true,
    }
}

/// The value `user` sets for `tag`, the last setting winning; `None` when
/// it doesn't mention it.
fn setting(user: &[(Tag, u32)], tag: Tag) -> Option<u32> {
    user.iter().rev().find(|(t, _)| *t == tag).map(|&(_, v)| v)
}

/// Whether a face whose `latn`/`DFLT` features are `font` shapes runs
/// with the user's `user` settings: a feature that substitutes across
/// characters is on, either on by default and not turned off, or turned
/// on.
pub(crate) fn gsub_shapes_runs(font: &[Tag], user: &[(Tag, u32)]) -> bool {
    font.iter().any(|&tag| {
        across(tag)
            && match setting(user, tag) {
                Some(v) => v != 0,
                None => DEFAULT_ON.contains(&&tag.0),
            }
    })
}

/// Whether the user turned on a feature that substitutes across
/// characters: for `morx` faces, where Core Text maps OpenType tags onto
/// Apple's features, so `+liga` on Menlo turns on its `fi` ligature.
pub(crate) fn user_turns_on_runs(user: &[(Tag, u32)]) -> bool {
    user.iter()
        .any(|&(tag, _)| across(tag) && setting(user, tag).is_some_and(|v| v != 0))
}

fn u16_at(d: &[u8], at: usize) -> Option<usize> {
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]) as usize)
}

fn tag_at(d: &[u8], at: usize) -> Option<Tag> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(Tag([b[0], b[1], b[2], b[3]]))
}

/// The features of a `GSUB` table's `latn` script, else its `DFLT`
/// script, in their default language system (else the first one), that
/// have lookups; the required feature included. Empty when neither script
/// is there. `None` when the table is malformed. Every read is
/// bounds-checked; offsets of 0 mean absent.
pub(crate) fn gsub_features(d: &[u8]) -> Option<Vec<Tag>> {
    let mut out = Vec::new();
    let (scripts, features) = (u16_at(d, 4)?, u16_at(d, 6)?);
    if scripts == 0 || features == 0 {
        return Some(out);
    }
    let (mut latn, mut dflt) = (None, None);
    for i in 0..u16_at(d, scripts)? {
        let rec = scripts + 2 + i * 6;
        let off = u16_at(d, rec + 4)?;
        if off == 0 {
            continue;
        }
        match &tag_at(d, rec)?.0 {
            b"latn" => latn = Some(scripts + off),
            b"DFLT" => dflt = Some(scripts + off),
            _ => {}
        }
    }
    let Some(script) = latn.or(dflt) else { return Some(out) };
    let mut lang = u16_at(d, script)?;
    if lang == 0 && u16_at(d, script + 2)? > 0 {
        lang = u16_at(d, script + 4 + 4)?;
    }
    if lang == 0 {
        return Some(out);
    }
    let lang = script + lang;
    let required = u16_at(d, lang + 2)?;
    let count = u16_at(d, lang + 4)?;
    let feature_count = u16_at(d, features)?;
    let indices = (0..count).map(|k| u16_at(d, lang + 6 + 2 * k));
    for index in std::iter::once(Some(required)).chain(indices) {
        let index = index?;
        if index == 0xFFFF || index >= feature_count {
            continue;
        }
        let rec = features + 2 + index * 6;
        let tag = tag_at(d, rec)?;
        let table = u16_at(d, rec + 4)?;
        if table != 0 && u16_at(d, features + table + 2)? > 0 && !out.contains(&tag) {
            out.push(tag);
        }
    }
    Some(out)
}

/// The glyphs a face's cross-character lookups can match, from its
/// `GSUB` table (ligatures design §5): every glyph in a coverage table,
/// class definition, ligature or rule sequence of the lookups of the
/// features [`gsub_shapes_runs`] counts as on. A glyph outside the set
/// can't take part in any substitution, so it is never changed, and any
/// such glyph is the same to its neighbours as any other.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LookupGlyphs {
    /// Sorted, no duplicates.
    pub glyphs: Vec<u16>,
    /// A class-based rule takes any glyph at all after its first (class 0
    /// in its input), so the set says nothing: shape whole runs.
    pub any: bool,
}

/// The characters that can take part in a face's substitutions
/// ([`Fonts::ligates`](crate::Fonts::ligates)).
#[derive(Clone, Debug, Default)]
pub(crate) struct Ligating {
    /// Every character can: the face's table couldn't be read for this.
    pub all: bool,
    /// ASCII, by bit.
    pub ascii: u128,
    /// The rest, sorted.
    pub other: Vec<u32>,
    /// A letter the face draws that can't.
    pub plain: Option<char>,
}

impl Ligating {
    pub fn all() -> Ligating {
        Ligating {
            all: true,
            ..Ligating::default()
        }
    }

    #[inline]
    pub fn contains(&self, ch: char) -> bool {
        let c = ch as u32;
        self.all
            || if c < 128 {
                self.ascii >> c & 1 != 0
            } else {
                self.other.binary_search(&c).is_ok()
            }
    }
}

fn u32_at(d: &[u8], at: usize) -> Option<usize> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

/// The lookup indices of the features of `d`'s `latn` (else `DFLT`)
/// default language system that are on with the user's settings `user`
/// and substitute across characters, as [`gsub_shapes_runs`] decides.
fn lookups_on(d: &[u8], user: &[(Tag, u32)]) -> Option<Vec<usize>> {
    let (scripts, features) = (u16_at(d, 4)?, u16_at(d, 6)?);
    let mut out = Vec::new();
    if scripts == 0 || features == 0 {
        return Some(out);
    }
    let (mut latn, mut dflt) = (None, None);
    for i in 0..u16_at(d, scripts)? {
        let rec = scripts + 2 + i * 6;
        let off = u16_at(d, rec + 4)?;
        if off == 0 {
            continue;
        }
        match &tag_at(d, rec)?.0 {
            b"latn" => latn = Some(scripts + off),
            b"DFLT" => dflt = Some(scripts + off),
            _ => {}
        }
    }
    let Some(script) = latn.or(dflt) else { return Some(out) };
    let mut lang = u16_at(d, script)?;
    if lang == 0 && u16_at(d, script + 2)? > 0 {
        lang = u16_at(d, script + 4 + 4)?;
    }
    if lang == 0 {
        return Some(out);
    }
    let lang = script + lang;
    let required = u16_at(d, lang + 2)?;
    let count = u16_at(d, lang + 4)?;
    let feature_count = u16_at(d, features)?;
    let indices = (0..count).map(|k| u16_at(d, lang + 6 + 2 * k));
    for index in std::iter::once(Some(required)).chain(indices) {
        let index = index?;
        if index == 0xFFFF || index >= feature_count {
            continue;
        }
        let rec = features + 2 + index * 6;
        let tag = tag_at(d, rec)?;
        let table = u16_at(d, rec + 4)?;
        if table == 0 || !gsub_shapes_runs(&[tag], user) {
            continue;
        }
        let table = features + table;
        for k in 0..u16_at(d, table + 2)? {
            out.push(u16_at(d, table + 4 + 2 * k)?);
        }
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// More glyphs than this, duplicates included, and the table is taken as
/// unreadable: a font's ranges could otherwise ask for gigabytes.
const MAX_GLYPHS: usize = 1 << 18;

/// Every glyph of the coverage table at `at`.
fn coverage(d: &[u8], at: usize, out: &mut Vec<u16>) -> Option<()> {
    match u16_at(d, at)? {
        1 => {
            for i in 0..u16_at(d, at + 2)? {
                out.push(u16_at(d, at + 4 + 2 * i)? as u16);
            }
        }
        2 => {
            for i in 0..u16_at(d, at + 2)? {
                let r = at + 4 + 6 * i;
                let (a, b) = (u16_at(d, r)?, u16_at(d, r + 2)?);
                out.extend((a..=b).map(|g| g as u16));
                if out.len() > MAX_GLYPHS {
                    return None;
                }
            }
        }
        _ => return None,
    }
    Some(())
}

/// Every glyph the class definition at `at` puts in a class other than 0.
fn class_def(d: &[u8], at: usize, out: &mut Vec<u16>) -> Option<()> {
    match u16_at(d, at)? {
        1 => {
            let start = u16_at(d, at + 2)?;
            for i in 0..u16_at(d, at + 4)? {
                if u16_at(d, at + 6 + 2 * i)? != 0 {
                    out.push((start + i) as u16);
                }
            }
        }
        2 => {
            for i in 0..u16_at(d, at + 2)? {
                let r = at + 4 + 6 * i;
                let (a, b, class) = (u16_at(d, r)?, u16_at(d, r + 2)?, u16_at(d, r + 4)?);
                if class != 0 {
                    out.extend((a..=b).map(|g| g as u16));
                }
                if out.len() > MAX_GLYPHS {
                    return None;
                }
            }
        }
        _ => return None,
    }
    Some(())
}

/// Each of the `n` u16s from `at`.
fn sequence(d: &[u8], at: usize, n: usize, f: &mut impl FnMut(u16)) -> Option<()> {
    for i in 0..n {
        f(u16_at(d, at + 2 * i)? as u16);
    }
    Some(())
}

/// Every glyph the lookup subtable at `at`, of lookup type `kind`, can
/// match, into `out`.
fn subtable(d: &[u8], at: usize, kind: usize, out: &mut LookupGlyphs, depth: u8) -> Option<()> {
    let format = u16_at(d, at)?;
    let glyphs = &mut out.glyphs;
    match (kind, format) {
        // Single, multiple, alternate.
        (1..=3, _) => coverage(d, at + u16_at(d, at + 2)?, glyphs)?,
        // Ligatures: the first glyphs and every component.
        (4, 1) => {
            coverage(d, at + u16_at(d, at + 2)?, glyphs)?;
            for i in 0..u16_at(d, at + 4)? {
                let set = at + u16_at(d, at + 6 + 2 * i)?;
                for j in 0..u16_at(d, set)? {
                    let lig = set + u16_at(d, set + 2 + 2 * j)?;
                    let n = u16_at(d, lig + 2)?.saturating_sub(1);
                    sequence(d, lig + 4, n, &mut |g| glyphs.push(g))?;
                }
            }
        }
        // Contextual and chained contextual, by glyph sequences.
        (5 | 6, 1) => {
            coverage(d, at + u16_at(d, at + 2)?, glyphs)?;
            for i in 0..u16_at(d, at + 4)? {
                let off = u16_at(d, at + 6 + 2 * i)?;
                if off == 0 {
                    continue;
                }
                let set = at + off;
                for j in 0..u16_at(d, set)? {
                    let mut r = set + u16_at(d, set + 2 + 2 * j)?;
                    if kind == 6 {
                        let n = u16_at(d, r)?;
                        sequence(d, r + 2, n, &mut |g| glyphs.push(g))?;
                        r += 2 + 2 * n;
                    }
                    let n = u16_at(d, r)?.saturating_sub(1);
                    let skip = if kind == 5 { 4 } else { 2 };
                    sequence(d, r + skip, n, &mut |g| glyphs.push(g))?;
                    if kind == 6 {
                        let r = r + 2 + 2 * n;
                        let n = u16_at(d, r)?;
                        sequence(d, r + 2, n, &mut |g| glyphs.push(g))?;
                    }
                }
            }
        }
        // By classes: every classed glyph, and any glyph at all when an
        // input position after the first is class 0.
        (5 | 6, 2) => {
            coverage(d, at + u16_at(d, at + 2)?, glyphs)?;
            let defs = if kind == 5 { 1 } else { 3 };
            for k in 0..defs {
                let off = u16_at(d, at + 4 + 2 * k)?;
                if off != 0 {
                    class_def(d, at + off, glyphs)?;
                }
            }
            let sets = at + 4 + 2 * defs;
            let mut any = false;
            for i in 0..u16_at(d, sets)? {
                let off = u16_at(d, sets + 2 + 2 * i)?;
                if off == 0 {
                    continue;
                }
                let set = at + off;
                for j in 0..u16_at(d, set)? {
                    let mut r = set + u16_at(d, set + 2 + 2 * j)?;
                    if kind == 6 {
                        r += 2 + 2 * u16_at(d, r)?;
                    }
                    let n = u16_at(d, r)?.saturating_sub(1);
                    let skip = if kind == 5 { 4 } else { 2 };
                    sequence(d, r + skip, n, &mut |c| any |= c == 0)?;
                }
            }
            out.any |= any;
        }
        // By coverage tables.
        (5, 3) => {
            for i in 0..u16_at(d, at + 2)? {
                coverage(d, at + u16_at(d, at + 6 + 2 * i)?, glyphs)?;
            }
        }
        (6, 3) | (8, 1) => {
            let mut r = at + 2;
            if kind == 8 {
                coverage(d, at + u16_at(d, at + 2)?, glyphs)?;
                r += 2;
            }
            let lists = if kind == 6 { 3 } else { 2 };
            for _ in 0..lists {
                let n = u16_at(d, r)?;
                for i in 0..n {
                    coverage(d, at + u16_at(d, r + 2 + 2 * i)?, glyphs)?;
                }
                r += 2 + 2 * n;
            }
        }
        // Extension: the real subtable, a 32-bit offset away.
        (7, 1) if depth == 0 => {
            let kind = u16_at(d, at + 2)?;
            subtable(d, at + u32_at(d, at + 4)?, kind, out, 1)?;
        }
        _ => return None,
    }
    Some(())
}

/// The glyphs the lookups of the cross-character features on in `d` (a
/// `GSUB` table), with the user's settings `user`, can match. `None` when
/// the table is malformed or uses a format this doesn't read. Lookups
/// that rules only call (at positions those rules matched) don't widen
/// the set: only the features' own lookups are read.
pub(crate) fn lookup_glyphs(d: &[u8], user: &[(Tag, u32)]) -> Option<LookupGlyphs> {
    let mut out = LookupGlyphs::default();
    let list = u16_at(d, 8)?;
    if list == 0 {
        return Some(out);
    }
    let count = u16_at(d, list)?;
    for index in lookups_on(d, user)? {
        if index >= count {
            continue;
        }
        let lookup = list + u16_at(d, list + 2 + 2 * index)?;
        let kind = u16_at(d, lookup)?;
        for i in 0..u16_at(d, lookup + 4)? {
            subtable(d, lookup + u16_at(d, lookup + 6 + 2 * i)?, kind, &mut out, 0)?;
        }
    }
    out.glyphs.sort_unstable();
    out.glyphs.dedup();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Tag {
        Tag::new(s).unwrap()
    }

    #[test]
    fn which_tags_substitute_across_characters() {
        for tag in ["liga", "calt", "dlig", "clig", "rlig", "salt", "ss21", "cv00"] {
            assert!(across(t(tag)), "{tag}");
        }
        for tag in [
            "ccmp", "locl", "mark", "mkmk", "kern", "zero", "case", "ss01", "ss20", "cv01", "cv99",
        ] {
            assert!(!across(t(tag)), "{tag}");
        }
    }

    #[test]
    fn user_settings_turn_runs_on_and_off() {
        let font = [t("calt"), t("liga"), t("zero"), t("ss01"), t("dlig")];
        assert!(gsub_shapes_runs(&font, &[]));
        assert!(!gsub_shapes_runs(&font, &[(t("calt"), 0), (t("liga"), 0)]));
        assert!(!gsub_shapes_runs(
            &font,
            &[(t("calt"), 0), (t("liga"), 0), (t("zero"), 1), (t("ss01"), 1)]
        ));
        assert!(gsub_shapes_runs(&font, &[(t("calt"), 0), (t("liga"), 0), (t("dlig"), 1)]));
        assert!(
            gsub_shapes_runs(&font, &[(t("calt"), 0), (t("calt"), 1), (t("liga"), 0)]),
            "last wins"
        );
        // Only features the font has count.
        assert!(!gsub_shapes_runs(&[t("kern"), t("zero")], &[(t("salt"), 1)]));
        assert!(!gsub_shapes_runs(&[t("dlig")], &[]), "dlig only when turned on");
        assert!(user_turns_on_runs(&[(t("liga"), 1)]));
        assert!(!user_turns_on_runs(&[(t("liga"), 1), (t("liga"), 0)]));
        assert!(!user_turns_on_runs(&[(t("zero"), 1), (t("calt"), 0)]));
    }

    /// A GSUB table with `scripts` (tag, features by index) and `features`
    /// (tag, lookup count).
    fn table(scripts: &[(&str, &[u16])], features: &[(&str, u16)]) -> Vec<u8> {
        let mut d = vec![0, 1, 0, 0, 0, 10, 0, 0, 0, 0];
        let push = |d: &mut Vec<u8>, v: usize| d.extend_from_slice(&(v as u16).to_be_bytes());
        // ScriptList at 10.
        let list = d.len();
        push(&mut d, scripts.len());
        let records = d.len();
        d.resize(records + scripts.len() * 6, 0);
        for (i, (tag, indices)) in scripts.iter().enumerate() {
            let script = d.len();
            let rec = records + i * 6;
            d[rec..rec + 4].copy_from_slice(tag.as_bytes());
            d[rec + 4..rec + 6].copy_from_slice(&((script - list) as u16).to_be_bytes());
            push(&mut d, 4); // default LangSys right after
            push(&mut d, 0);
            push(&mut d, 0);
            push(&mut d, 0xFFFF);
            push(&mut d, indices.len());
            for &i in indices.iter() {
                push(&mut d, i as usize);
            }
        }
        let flist = d.len();
        d[6..8].copy_from_slice(&(flist as u16).to_be_bytes());
        push(&mut d, features.len());
        let frecs = d.len();
        d.resize(frecs + features.len() * 6, 0);
        for (i, (tag, lookups)) in features.iter().enumerate() {
            let ft = d.len();
            let rec = frecs + i * 6;
            d[rec..rec + 4].copy_from_slice(tag.as_bytes());
            d[rec + 4..rec + 6].copy_from_slice(&((ft - flist) as u16).to_be_bytes());
            push(&mut d, 0);
            push(&mut d, *lookups as usize);
            for l in 0..*lookups {
                push(&mut d, l as usize);
            }
        }
        d
    }

    /// The checked-in JetBrains Mono's `GSUB` table, from its table
    /// directory.
    fn jetbrains_gsub() -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/JetBrainsMono-Regular.ttf");
        let file = std::fs::read(path).unwrap();
        let tables = u16_at(&file, 4).unwrap();
        (0..tables)
            .map(|i| 12 + 16 * i)
            .find(|&rec| &file[rec..rec + 4] == b"GSUB")
            .map(|rec| {
                let (at, len) = (u32_at(&file, rec + 8).unwrap(), u32_at(&file, rec + 12).unwrap());
                file[at..at + len].to_vec()
            })
            .expect("a GSUB table")
    }

    #[test]
    fn reads_the_glyphs_lookups_can_match() {
        let d = jetbrains_gsub();
        let on = lookup_glyphs(&d, &[]).expect("readable");
        assert!(!on.any);
        assert!(
            on.glyphs.len() > 100 && on.glyphs.windows(2).all(|w| w[0] < w[1]),
            "{}",
            on.glyphs.len()
        );
        // Turning the features off leaves nothing to match.
        let off = lookup_glyphs(&d, &[(t("calt"), 0), (t("liga"), 0)]).unwrap();
        assert!(off.glyphs.is_empty(), "{:?}", off.glyphs);
        // Truncated anywhere: never a panic, and never more glyphs.
        for n in (0..d.len()).step_by(97) {
            if let Some(got) = lookup_glyphs(&d[..n], &[]) {
                assert!(got.glyphs.len() <= on.glyphs.len());
            }
        }
    }

    #[test]
    fn reads_latn_else_dflt() {
        let features = [("liga", 1), ("calt", 2), ("kern", 0), ("rlig", 1)];
        // Courier New's case: ligatures for Arabic only.
        let d = table(&[("arab", &[0, 3]), ("latn", &[2])], &features);
        assert_eq!(gsub_features(&d), Some(vec![]), "kern has no lookups");
        let d = table(&[("DFLT", &[0]), ("latn", &[1])], &features);
        assert_eq!(gsub_features(&d), Some(vec![t("calt")]));
        let d = table(&[("DFLT", &[0, 1, 9])], &features);
        assert_eq!(gsub_features(&d), Some(vec![t("liga"), t("calt")]), "bad index skipped");
        let d = table(&[("cyrl", &[0])], &features);
        assert_eq!(gsub_features(&d), Some(vec![]));
        // Truncated anywhere: never a panic.
        let d = table(&[("latn", &[0, 1])], &features);
        for n in 0..d.len() {
            let _ = gsub_features(&d[..n]);
        }
        assert_eq!(gsub_features(&d[..3]), None);
    }
}
