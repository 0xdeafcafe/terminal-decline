//! The config writer on a set of files: comments, dotted keys, inline
//! tables, quoted keys, CRLF, a BOM, no final newline, multi-line arrays,
//! chords spelled several ways; and empty, missing, symlinked, read-only
//! and unparsable files.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use term_prefs::file::{self, Edit, ListOp};
use term_prefs::host::fake_chord;
use toml_edit::Value;

fn chord(c: &str) -> Option<String> {
    fake_chord(c).ok()
}

fn path(key: &str) -> Vec<String> {
    key.split('.').map(String::from).collect()
}

fn set(key: &str, value: impl Into<Value>) -> Edit {
    Edit::Set {
        path: path(key),
        value: value.into(),
    }
}

fn apply(text: &str, edit: &Edit) -> String {
    let p = file::prepare(text, edit, &chord).unwrap_or_else(|e| panic!("{e}"));
    // Undo gives back the text it started from.
    let back = file::prepare(&p.text, &p.undo, &chord).unwrap();
    assert_eq!(
        file::parse(&back.text).unwrap().to_string(),
        file::parse(text).unwrap().to_string(),
        "undo of {edit:?}"
    );
    p.text
}

#[test]
fn values_keep_their_comments_and_integers_stay_integers() {
    let text = "# my settings\nfont_size = 13 # big\npadding = 6\n";
    assert_eq!(
        apply(text, &set("font_size", 14)),
        "# my settings\nfont_size = 14 # big\npadding = 6\n"
    );
    assert_eq!(
        apply(text, &set("font_size", 13.5)),
        "# my settings\nfont_size = 13.5 # big\npadding = 6\n"
    );
}

#[test]
fn new_tables_are_standard_tables() {
    assert_eq!(
        apply("", &set("colors.foreground", "#ffffff")),
        "[colors]\nforeground = \"#ffffff\"\n"
    );
    assert_eq!(
        apply("font_size = 13\n", &set("colors.foreground", "#ffffff")),
        "font_size = 13\n\n[colors]\nforeground = \"#ffffff\"\n"
    );
    // A top-level key goes above the tables.
    assert_eq!(
        apply("[colors]\nforeground = \"#ffffff\"\n", &set("padding", 2)),
        "padding = 2\n[colors]\nforeground = \"#ffffff\"\n"
    );
}

#[test]
fn dotted_keys_inline_tables_and_quoted_keys() {
    assert_eq!(
        apply("colors.foreground = \"#ffffff\"\n", &set("colors.background", "#000000")),
        "colors.foreground = \"#ffffff\"\ncolors.background = \"#000000\"\n"
    );
    assert_eq!(
        apply("colors = { foreground = \"#ffffff\" }\n", &set("colors.background", "#000000")),
        "colors = { foreground = \"#ffffff\", background = \"#000000\" }\n"
    );
    assert_eq!(
        apply(
            "colors = { foreground = \"#ffffff\" } # mine\n",
            &set("colors.foreground", "#000000")
        ),
        "colors = { foreground = \"#000000\" } # mine\n"
    );
    assert_eq!(apply("\"font_size\" = 13\n", &set("font_size", 14)), "\"font_size\" = 14\n");
    let err = file::prepare("colors = 3\n", &set("colors.foreground", "#000000"), &chord).unwrap_err();
    assert!(err.contains("not a table"), "{err}");
}

#[test]
fn crlf_bom_and_no_final_newline() {
    assert_eq!(
        apply("# a\r\nfont_size = 13\r\n", &set("padding", 2)),
        "# a\r\nfont_size = 13\r\npadding = 2\r\n"
    );
    assert_eq!(apply("\u{feff}font_size = 13\n", &set("font_size", 15)), "\u{feff}font_size = 15\n");
    assert_eq!(apply("font_size = 13", &set("font_size", 15)), "font_size = 15");
    let added = apply("[colors]\nforeground = \"#ffffff\"", &set("colors.background", "#000000"));
    assert!(file::parse(&added).is_ok(), "{added:?}");
    assert!(added.starts_with("[colors]\nforeground = \"#ffffff\""), "{added:?}");
}

#[test]
fn removing_takes_the_comment_and_undo_puts_it_back() {
    let text = "# the size I like\nfont_size = 16 # big\npadding = 2\n";
    let p = file::prepare(text, &Edit::Remove { path: path("font_size") }, &chord).unwrap();
    assert_eq!(p.text, "padding = 2\n");
    assert!(p.dropped_comment);
    let back = file::prepare(&p.text, &p.undo, &chord).unwrap();
    assert!(back.text.contains("# the size I like\nfont_size = 16 # big\n"), "{:?}", back.text);
    // Removing what isn't there changes nothing.
    let p = file::prepare(
        text,
        &Edit::Remove {
            path: path("colors.cursor"),
        },
        &chord,
    )
    .unwrap();
    assert_eq!(p.text, text);
    assert!(!p.dropped_comment);
}

#[test]
fn multi_line_arrays_keep_their_comments() {
    let text = "font_fallback = [\n    \"Menlo\", # first\n    \"Monaco\",\n]\n";
    let added = apply(
        text,
        &Edit::List {
            path: path("font_fallback"),
            start: vec![],
            op: ListOp::Insert(2, "SF Mono".into()),
        },
    );
    assert!(added.contains("\"Menlo\", # first"), "{added}");
    assert!(file::parse(&added).is_ok());
    let fallback = file::parse(&added).unwrap()["font_fallback"].as_array().unwrap().len();
    assert_eq!(fallback, 3);
    let removed = apply(
        text,
        &Edit::List {
            path: path("font_fallback"),
            start: vec![],
            op: ListOp::Remove(1),
        },
    );
    assert!(removed.contains("\"Menlo\", # first"), "{removed}");
    assert!(!removed.contains("Monaco"));
    let moved = apply(
        text,
        &Edit::List {
            path: path("font_fallback"),
            start: vec![],
            op: ListOp::Move(1, 0),
        },
    );
    let doc = file::parse(&moved).unwrap();
    let order: Vec<&str> = doc["font_fallback"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(order, ["Monaco", "Menlo"]);
    // A list not in the file starts as the inherited one.
    let made = apply(
        "",
        &Edit::List {
            path: path("command"),
            start: vec!["htop".into()],
            op: ListOp::Insert(1, "-d".into()),
        },
    );
    assert_eq!(made, "command = [\"htop\", \"-d\"]\n");
    // The palette pads with "" (the theme's).
    let palette = apply(
        "[colors]\npalette = [\"#000000\"]\n",
        &Edit::List {
            path: path("colors.palette"),
            start: vec![],
            op: ListOp::Set(2, "#ff0000".into()),
        },
    );
    assert_eq!(palette, "[colors]\npalette = [\"#000000\", \"\", \"#ff0000\"]\n");
}

#[test]
fn chords_are_found_by_their_parsed_form() {
    let text = "[keybind]\n\"command+K\" = \"new_tab\" # mine\n";
    let bind = |c: &str, v: Option<&str>| Edit::Keybind {
        chord: c.into(),
        value: v.map(Value::from),
    };
    assert_eq!(
        apply(text, &bind("cmd+k", Some("clear_scrollback"))),
        "[keybind]\n\"command+K\" = \"clear_scrollback\" # mine\n"
    );
    assert_eq!(
        apply(text, &bind("cmd+d", Some("none"))),
        "[keybind]\n\"command+K\" = \"new_tab\" # mine\n\"cmd+d\" = \"none\"\n"
    );
    // Both spellings: the later one wins, and is the one rewritten.
    let both = "[keybind]\n\"cmd+k\" = \"copy\"\n\"Cmd+K\" = \"paste\"\n";
    assert_eq!(
        apply(both, &bind("cmd+k", Some("find"))),
        "[keybind]\n\"cmd+k\" = \"copy\"\n\"Cmd+K\" = \"find\"\n"
    );
    // Removing removes every spelling.
    assert_eq!(apply(both, &bind("cmd+k", None)), "[keybind]\n");
    // A new [keybind] table is a standard one, never an inline table.
    assert_eq!(apply("", &bind("cmd+d", Some("none"))), "[keybind]\n\"cmd+d\" = \"none\"\n");
}

#[test]
fn unparsable_files_are_never_changed() {
    let text = "font_size = = 3\n";
    let err = file::prepare(text, &set("font_size", 14), &chord).unwrap_err();
    assert!(err.contains("line 1"), "{err}");
}

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("term-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_files_and_folders_are_made() {
    let dir = Dir::new("missing");
    let p = dir.0.join("sub/config.toml");
    assert_eq!(file::read(&p).unwrap(), None);
    file::write(&p, "font_size = 14\n").unwrap();
    assert_eq!(file::read(&p).unwrap().as_deref(), Some("font_size = 14\n"));
}

#[test]
fn symlinks_stay_links_and_modes_are_kept() {
    let dir = Dir::new("symlink");
    let real = dir.0.join("dotfiles.toml");
    std::fs::write(&real, "font_size = 13\n").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.0.join("config.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    file::write(&link, "font_size = 14\n").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "font_size = 14\n");
    assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
    // No temp files left behind.
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn read_only_files_are_not_written() {
    let dir = Dir::new("readonly");
    let p = dir.0.join("config.toml");
    std::fs::write(&p, "font_size = 13\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
    let err = file::write(&p, "font_size = 14\n").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "font_size = 13\n");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn section_comments_stay_when_their_key_goes() {
    let remove = |text: &str, key: &str| apply(text, &Edit::Remove { path: path(key) });
    // The file's header comment belongs to no key.
    assert_eq!(
        remove("# Settings\n\nfont_size = 13\npadding = 2\n", "font_size"),
        "# Settings\n\npadding = 2\n"
    );
    assert_eq!(
        remove("# Settings\n\nfont_size = 13\n\n[colors]\nforeground = \"#ffffff\"\n", "font_size"),
        "# Settings\n\n\n[colors]\nforeground = \"#ffffff\"\n"
    );
    assert_eq!(remove("# Settings\n\nfont_size = 13\n", "font_size"), "# Settings\n\n");
    assert_eq!(
        remove("a = 1\n\ncolors.foreground = \"#72e844\" # x", "colors.foreground"),
        "a = 1\n"
    );
}
