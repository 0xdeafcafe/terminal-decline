//! Random edits on random valid files: the result still parses, the
//! loader (the fake, which checks types against the schema) finds no
//! errors, the edited value is the one asked for, the lines not edited
//! are the same byte for byte, and undo gives the file back exactly.

use std::path::PathBuf;

use proptest::prelude::*;
use term_prefs::Host;
use term_prefs::file::{self, Edit};
use term_prefs::host::{Fake, fake_chord};
use term_prefs::layers::get;
use toml_edit::Value;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() % xs.len() as u64) as usize]
    }
    fn chance(&mut self, n: u64) -> bool {
        self.next().is_multiple_of(n)
    }
}

const COMMENTS: [&str; 4] = ["", " # mine", "  # from the old Mac", " #"];
const SPELLINGS: [(&str, &[&str]); 4] = [
    ("cmd+k", &["cmd+k", "command+K", "Cmd+k", "super+k"]),
    ("cmd+d", &["cmd+d", "command+d"]),
    ("ctrl+shift+t", &["ctrl+shift+t", "shift+control+T"]),
    ("alt+enter", &["alt+enter", "option+Enter", "opt+enter"]),
];
const ACTIONS: [&str; 5] = ["none", "new_tab", "copy", "find", "clear_scrollback"];

fn hex(r: &mut Rng) -> String {
    format!("#{:06x}", r.next() & 0xffffff)
}

/// A top-level setting with a random valid value, as TOML.
fn scalar(r: &mut Rng, key: &str) -> String {
    match key {
        "font_size" => format!("{}", 6 + r.next() % 60),
        "padding" => format!("{}.5", r.next() % 20),
        "opacity" => format!("0.{}", r.next() % 10),
        "blur" | "copy_on_select" => (r.chance(2)).to_string(),
        "option_as_meta" => format!("\"{}\"", r.pick(&["none", "left", "right", "both"])),
        "font_family" => format!("'{}'", r.pick(&["Menlo", "SF Mono", "Monaco"])),
        "columns" => format!("{}", 10 + r.next() % 200),
        "font_fallback" => "[\n  \"Menlo\", # first\n  \"Monaco\",\n]".into(),
        _ => unreachable!(),
    }
}

const TOP: [&str; 8] = [
    "font_size",
    "padding",
    "opacity",
    "blur",
    "option_as_meta",
    "font_family",
    "columns",
    "font_fallback",
];
const COLOURS: [&str; 4] = ["foreground", "background", "cursor", "selection"];

/// A random valid file, and its line ending.
fn random_file(r: &mut Rng) -> String {
    let mut out = String::new();
    if r.chance(6) {
        out.push('\u{feff}');
    }
    if r.chance(2) {
        out.push_str("# Terminal Decline settings\n\n");
    }
    let mut dotted_colours = false;
    for key in TOP {
        if r.chance(2) {
            continue;
        }
        if r.chance(4) {
            out.push_str("# about it\n");
        }
        let q = if r.chance(8) { format!("\"{key}\"") } else { key.to_string() };
        out.push_str(&format!("{q} = {}{}\n", scalar(r, key), r.pick(&COMMENTS)));
        if r.chance(5) {
            out.push('\n');
        }
    }
    if r.chance(3) {
        dotted_colours = true;
        for c in COLOURS {
            if r.chance(2) {
                out.push_str(&format!("colors.{c} = \"{}\"{}\n", hex(r), r.pick(&COMMENTS)));
            }
        }
    }
    if !dotted_colours && r.chance(2) {
        out.push_str("\n[colors]\n");
        for c in COLOURS {
            if r.chance(2) {
                out.push_str(&format!("{c} = \"{}\"{}\n", hex(r), r.pick(&COMMENTS)));
            }
        }
    }
    if r.chance(2) {
        out.push_str("\n[keybind]\n");
        for (_, spellings) in SPELLINGS {
            if r.chance(2) {
                let s = r.pick(spellings);
                out.push_str(&format!("\"{s}\" = \"{}\"{}\n", r.pick(&ACTIONS), r.pick(&COMMENTS)));
            }
        }
    }
    if r.chance(4) && out.ends_with('\n') {
        out.pop();
    }
    if r.chance(4) {
        out = out.replace('\n', "\r\n");
    }
    out
}

/// A random edit, the key it changes, and the value it should leave.
fn random_edit(r: &mut Rng) -> (Edit, Vec<String>, Option<Value>, Option<&'static str>) {
    let p = |s: &str| s.split('.').map(String::from).collect::<Vec<_>>();
    match r.next() % 5 {
        0 | 1 => {
            let key = *r.pick(&TOP[..7]);
            let text = scalar(r, key);
            let value: Value = text.parse().unwrap();
            (
                Edit::Set {
                    path: p(key),
                    value: value.clone(),
                },
                p(key),
                Some(value),
                None,
            )
        }
        2 => {
            let key = format!("colors.{}", r.pick(&COLOURS));
            let value = Value::from(hex(r));
            (
                Edit::Set {
                    path: p(&key),
                    value: value.clone(),
                },
                p(&key),
                Some(value),
                None,
            )
        }
        3 => {
            let all: Vec<String> = TOP
                .iter()
                .map(|k| k.to_string())
                .chain(COLOURS.iter().map(|c| format!("colors.{c}")))
                .collect();
            let key = r.pick(&all).clone();
            (Edit::Remove { path: p(&key) }, p(&key), None, None)
        }
        _ => {
            let (canon, _) = *r.pick(&SPELLINGS);
            let value = (!r.chance(3)).then(|| Value::from(*r.pick(&ACTIONS)));
            (
                Edit::Keybind {
                    chord: canon.into(),
                    value: value.clone(),
                },
                vec!["keybind".into(), canon.into()],
                value,
                Some(canon),
            )
        }
    }
}

fn chord(c: &str) -> Option<String> {
    fake_chord(c).ok()
}

/// Whether a line is the edited key's.
fn mentions(line: &str, path: &[String], chord_of: Option<&str>) -> bool {
    let t = line.trim_start_matches('\u{feff}').trim_start();
    let key = t.split('=').next().unwrap_or("").trim().trim_matches('"');
    match chord_of {
        Some(c) => fake_chord(key).ok().as_deref() == Some(c),
        None => {
            let leaf = path.last().unwrap();
            key == path.join(".") || (path.len() == 1 && key == leaf) || (path.len() == 2 && key == leaf)
        }
    }
}

/// One random file and edit, from `seed`.
fn check(seed: u64) -> Result<(), TestCaseError> {
    let mut r = Rng(seed | 1);
    let text = random_file(&mut r);
    let fake = Fake::new(PathBuf::from("/nonexistent/config.toml"));
    let layers = [fake.user_config()];
    prop_assert_eq!(fake.check(&layers, 0, &text).notices, Vec::<String>::new(), "{:?}", text);
    let (edit, path, want, chord_of) = random_edit(&mut r);
    let p = file::prepare(&text, &edit, &chord).map_err(|e| TestCaseError::fail(format!("{e}\n{text:?}\n{edit:?}")))?;
    let doc = file::parse(&p.text).map_err(TestCaseError::fail)?;
    prop_assert_eq!(fake.check(&layers, 0, &p.text).notices, Vec::<String>::new(), "{:?}", p.text);

    // The value is the one asked for.
    let got = match chord_of {
        Some(c) => doc.get("keybind").and_then(|t| t.as_table_like()).and_then(|t| {
            t.iter()
                .filter(|(k, _)| fake_chord(k).ok().as_deref() == Some(c))
                .last()
                .map(|(_, v)| v.clone())
        }),
        None => get(doc.as_table(), &path).cloned(),
    };
    let got = got.and_then(|i| i.as_value().cloned()).map(|mut v| {
        v.decor_mut().clear();
        v.to_string()
    });
    let want = want.map(|v| {
        let mut v = v;
        v.decor_mut().clear();
        v.to_string()
    });
    let removing = want.is_none();
    prop_assert_eq!(got, want, "{:?} -> {:?}", text, p.text);

    // The other lines are untouched, in order.
    let crlf = text.contains("\r\n");
    if crlf {
        prop_assert!(!p.text.replace("\r\n", "").contains('\n'), "LF in a CRLF file: {:?}", p.text);
    }
    prop_assert_eq!(p.text.starts_with('\u{feff}'), text.starts_with('\u{feff}'));
    let new_lines: Vec<&str> = p.text.trim_start_matches('\u{feff}').split('\n').collect();
    let mut at = 0;
    // The edited key's lines: its own, and the rest of a multi-line list.
    // A removed key takes the comment lines right above it.
    let lines: Vec<&str> = text.trim_start_matches('\u{feff}').split('\n').collect();
    let mut gone = vec![false; lines.len()];
    let mut in_list = false;
    for (i, l) in lines.iter().enumerate() {
        if in_list {
            gone[i] = true;
            in_list = !l.starts_with(']');
        } else if mentions(l, &path, chord_of) {
            gone[i] = true;
            in_list = l.trim_end().ends_with('[');
            let mut j = i;
            while removing && j > 0 && lines[j - 1].trim_start_matches('\u{feff}').trim_start().starts_with('#') {
                j -= 1;
                gone[j] = true;
            }
        }
    }
    let kept: Vec<&str> = lines.iter().zip(&gone).filter(|(_, g)| !**g).map(|(l, _)| *l).collect();
    for line in kept {
        match new_lines[at..]
            .iter()
            .position(|n| n.trim_end_matches('\r') == line.trim_end_matches('\r'))
        {
            Some(i) => at += i + 1,
            None => prop_assert!(false, "line {line:?} changed or moved:\n{text:?}\n->\n{:?}", p.text),
        }
    }

    // Undo gives the file back byte for byte, and redo the edited file.
    let back = file::prepare(&p.text, &p.undo, &chord).map_err(TestCaseError::fail)?;
    prop_assert_eq!(&back.text, &text, "undo of {:?}", edit);
    let redo = file::prepare(&back.text, &back.undo, &chord).map_err(TestCaseError::fail)?;
    prop_assert_eq!(&redo.text, &p.text, "redo of {:?}", edit);
    Ok(())
}

/// Seeds that found bugs, run every time (the regressions file only
/// keeps the shrunk inputs proptest persisted itself).
const KNOWN_SEEDS: &[u64] = &[8796317076710775082];

#[test]
fn known_seeds() {
    for &seed in KNOWN_SEEDS {
        if let Err(e) = check(seed) {
            panic!("seed {seed}: {e}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn random_edits_on_random_files(seed in any::<u64>()) {
        check(seed)?;
    }
}
