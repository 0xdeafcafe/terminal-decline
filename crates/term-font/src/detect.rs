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
