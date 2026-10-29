//! Safe edits to a config file with `toml_edit`: each edit is applied to
//! the file as it is on disk, keeps comments, order and formatting, and
//! is written atomically. A file that doesn't parse is never written.
//!
//! See docs/design/preferences.md, "Saving".

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Key, TableLike, Value};

/// A key's place in the file: `["font_size"]`, `["colors", "foreground"]`.
pub type KeyPath = Vec<String>;

/// One change to a file.
#[derive(Clone, Debug)]
pub enum Edit {
    /// Sets a value, taking over the old one's decor (its inline comment
    /// and spacing). Missing tables are made as standard `[tables]`.
    Set { path: KeyPath, value: Value },
    /// Removes a key with its comments, so the value comes from the layers
    /// below.
    Remove { path: KeyPath },
    /// Sets (`Some`) or removes (`None`) a `[keybind]` entry, found by its
    /// parsed chord (`chord` is the canonical spelling): an existing entry
    /// is rewritten in place, keeping its spelling; the later one when a
    /// file spells the chord twice. Removing removes every spelling.
    Keybind { chord: String, value: Option<Value> },
    /// Changes one element of a list. A missing list starts as `start` (the
    /// value inherited from the layers below).
    List { path: KeyPath, start: Vec<Value>, op: ListOp },
    /// Puts an entry back as it was, with its key's and value's comments,
    /// at its place among its table's keys (`None`: removes it). Undo is
    /// made of these.
    Restore {
        path: KeyPath,
        entry: Option<(Key, Item)>,
        index: usize,
    },
    /// Sets the comments and blank lines above a top-level table (undo of
    /// a removal whose section comment moved there).
    TablePrefix { key: String, prefix: String },
    /// Sets what follows the last line of the file (as `TablePrefix`).
    Trailing(String),
    /// Sets the file's line endings: CRLF or LF, and whether the last line
    /// ends with one. Undo of an edit that left a file with no line break,
    /// which no longer shows them.
    LineEnds { crlf: bool, final_newline: bool },
    /// Several edits written as one.
    Many(Vec<Edit>),
}

/// A change to one element of a list.
#[derive(Clone, Debug)]
pub enum ListOp {
    /// Replaces element `i`; a shorter list is padded with `""` first (the
    /// palette's "from the theme").
    Set(usize, Value),
    /// Inserts at `i` (at the end if past it).
    Insert(usize, Value),
    Remove(usize),
    /// Moves element `from` to `to`; the list's layout and comments stay
    /// where they are.
    Move(usize, usize),
}

/// An edit applied to a file's text, not yet written.
#[derive(Clone, Debug)]
pub struct Prepared {
    /// The new text, with the file's line endings and BOM.
    pub text: String,
    /// The edit that undoes this one.
    pub undo: Edit,
    /// Whether a comment went with a removed key.
    pub dropped_comment: bool,
}

/// What toml_edit doesn't keep of a file.
struct Shape {
    bom: bool,
    crlf: bool,
    /// The file is not empty and doesn't end with a newline.
    no_final_newline: bool,
}

fn shape(text: &str) -> (Shape, &str) {
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    (
        Shape {
            bom,
            crlf: body.contains("\r\n"),
            no_final_newline: !body.is_empty() && !body.ends_with('\n'),
        },
        body,
    )
}

impl Shape {
    fn render(&self, doc: &DocumentMut) -> String {
        let mut out = doc.to_string();
        if self.no_final_newline && out.ends_with('\n') {
            out.pop();
            if out.ends_with('\r') {
                out.pop();
            }
        }
        if self.crlf {
            out = out.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        if self.bom {
            out.insert(0, '\u{feff}');
        }
        out
    }
}

/// Parses a config file's text (a BOM allowed), with toml_edit's message,
/// line and column on failure.
pub fn parse(text: &str) -> Result<DocumentMut, String> {
    let (_, body) = shape(text);
    body.parse::<DocumentMut>().map_err(|e| e.to_string().trim_end().to_string())
}

/// Reads a config file: `None` if it doesn't exist.
pub fn read(path: &Path) -> io::Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not UTF-8 text")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Applies `edit` to `text` (a file's contents, `""` for a missing file).
/// `chord` gives a chord's canonical spelling, `None` if it doesn't parse.
/// Fails, changing nothing, if `text` doesn't parse or the edit can't be
/// made (a value in the way of a table); the result always parses.
pub fn prepare(text: &str, edit: &Edit, chord: &dyn Fn(&str) -> Option<String>) -> Result<Prepared, String> {
    let (mut shape, body) = shape(text);
    let mut doc = body.parse::<DocumentMut>().map_err(|e| e.to_string().trim_end().to_string())?;
    let mut dropped_comment = false;
    let mut undo = apply(&mut doc, edit, chord, &mut dropped_comment)?;
    let old = (shape.crlf, !shape.no_final_newline);
    if let Some((crlf, final_newline)) = line_ends(edit) {
        shape.crlf = crlf;
        shape.no_final_newline = !final_newline;
    }
    let text = shape.render(&doc);
    // A file left with no line break (its only line removed) no longer
    // shows its line endings: the undo puts them back.
    let (new, _) = self::shape(&text);
    if (new.crlf, !new.no_final_newline) != old {
        undo = Edit::Many(vec![
            undo,
            Edit::LineEnds {
                crlf: old.0,
                final_newline: old.1,
            },
        ]);
    }
    parse(&text).map_err(|e| format!("the edit would leave a file that doesn't parse: {e}"))?;
    Ok(Prepared {
        text,
        undo,
        dropped_comment,
    })
}

/// The line endings an edit sets: its last `LineEnds`, as
/// `(crlf, final_newline)`.
fn line_ends(edit: &Edit) -> Option<(bool, bool)> {
    match edit {
        Edit::LineEnds { crlf, final_newline } => Some((*crlf, *final_newline)),
        Edit::Many(edits) => edits.iter().rev().find_map(line_ends),
        _ => None,
    }
}

/// The table an edit works in.
struct Place<'a> {
    table: &'a mut dyn TableLike,
    inline: bool,
    /// Tables made on the way, outermost first.
    made: Vec<KeyPath>,
}

/// `undo`, plus removing the tables the edit made.
fn with_made(made: &[KeyPath], undo: Edit) -> Edit {
    if made.is_empty() {
        return undo;
    }
    let mut all = vec![undo];
    for path in made.iter().rev() {
        all.push(Edit::Restore {
            path: path.clone(),
            entry: None,
            index: 0,
        });
    }
    Edit::Many(all)
}

fn apply(doc: &mut DocumentMut, edit: &Edit, chord: &dyn Fn(&str) -> Option<String>, dropped: &mut bool) -> Result<Edit, String> {
    match edit {
        Edit::Set { path, value } => {
            let (parents, leaf) = split(path)?;
            let place = descend(doc, parents, true)?.expect("made");
            let undo = set(place.table, place.inline, path, leaf, value.clone());
            Ok(with_made(&place.made, undo))
        }
        Edit::Remove { path } => {
            let (parents, leaf) = split(path)?;
            let Some(place) = descend(doc, parents, false)? else {
                return Ok(Edit::Many(Vec::new()));
            };
            let (undo, kept) = remove(place.table, place.inline, path, leaf, dropped);
            let undo = settle(doc, parents, &kept, undo);
            // An empty dotted table shows nothing, and would hide the
            // comments above it: it goes too.
            let empty_dotted = match parents {
                [top] => matches!(doc.get(top), Some(Item::Table(t)) if t.is_dotted() && t.is_empty()),
                _ => false,
            };
            if !empty_dotted {
                return Ok(undo);
            }
            let (outer, kept) = remove(doc.as_table_mut(), false, parents, &parents[0], dropped);
            let outer = settle(doc, &[], &kept, outer);
            Ok(Edit::Many(vec![outer, undo]))
        }
        Edit::Keybind { chord: want, value } => {
            let parents = ["keybind".to_string()];
            let Some(place) = descend(doc, &parents, value.is_some())? else {
                return Ok(Edit::Many(Vec::new()));
            };
            let spellings: Vec<String> = place
                .table
                .iter()
                .map(|(k, _)| k.to_string())
                .filter(|k| chord(k).as_deref() == Some(want.as_str()))
                .collect();
            let path_of = |k: &str| vec!["keybind".to_string(), k.to_string()];
            match value {
                Some(v) => {
                    let key = spellings.last().cloned().unwrap_or_else(|| want.clone());
                    let undo = set(place.table, place.inline, &path_of(&key), &key, v.clone());
                    Ok(with_made(&place.made, undo))
                }
                None => {
                    let mut undo = Vec::new();
                    let mut kept = String::new();
                    for k in spellings.iter().rev() {
                        let (u, k) = remove(place.table, place.inline, &path_of(k), k, dropped);
                        undo.push(u);
                        kept.insert_str(0, &k);
                    }
                    undo.reverse();
                    Ok(settle(doc, &parents, &kept, Edit::Many(undo)))
                }
            }
        }
        Edit::List { path, start, op } => {
            let (parents, leaf) = split(path)?;
            let place = descend(doc, parents, true)?.expect("made");
            let table = place.table;
            let undo = restore_of(table, path, leaf);
            if table.get(leaf).is_none() {
                let mut array = toml_edit::Array::new();
                for v in start {
                    array.push(strip(v.clone()));
                }
                insert(table, place.inline, leaf, Value::Array(array));
            }
            let array = table
                .get_mut(leaf)
                .and_then(Item::as_array_mut)
                .ok_or_else(|| format!("{} isn't a list", path.join(".")))?;
            list_op(array, op)?;
            Ok(with_made(&place.made, undo))
        }
        Edit::Restore { path, entry, index } => {
            let (parents, leaf) = split(path)?;
            let Some(place) = descend(doc, parents, entry.is_some())? else {
                return Ok(Edit::Many(Vec::new()));
            };
            let table = place.table;
            let undo = restore_of(table, path, leaf);
            match entry {
                Some((key, item)) if table.contains_key(leaf) => {
                    // In place: the key keeps its position.
                    *table.get_mut(leaf).expect("present") = item.clone();
                    if let Some(mut k) = table.key_mut(leaf) {
                        *k.leaf_decor_mut() = key.leaf_decor().clone();
                    }
                }
                Some((key, item)) => {
                    if let toml_edit::Entry::Vacant(v) = table.entry_format(key) {
                        v.insert(item.clone());
                    }
                    // Back to its place: move the keys after it to the end.
                    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
                    for k in keys.iter().skip(*index).filter(|k| *k != leaf) {
                        let key = table.key(k).expect("present").clone();
                        let item = table.remove(k).expect("present");
                        if let toml_edit::Entry::Vacant(v) = table.entry_format(&key) {
                            v.insert(item);
                        }
                    }
                }
                None => take_out(table, place.inline, leaf),
            }
            Ok(with_made(&place.made, undo))
        }
        Edit::TablePrefix { key, prefix } => {
            let Some(Item::Table(t)) = doc.as_table_mut().get_mut(key) else {
                return Ok(Edit::Many(Vec::new()));
            };
            let old = raw(t.decor().prefix());
            t.decor_mut().set_prefix(prefix.as_str());
            Ok(Edit::TablePrefix {
                key: key.clone(),
                prefix: old,
            })
        }
        Edit::Trailing(text) => {
            let old = raw(Some(doc.trailing()));
            doc.set_trailing(text.as_str());
            Ok(Edit::Trailing(old))
        }
        // Applied to the file's shape by `prepare`, which makes its undo.
        Edit::LineEnds { .. } => Ok(Edit::Many(Vec::new())),
        Edit::Many(edits) => {
            let mut undo = Vec::new();
            for e in edits {
                undo.push(apply(doc, e, chord, dropped)?);
            }
            undo.reverse();
            Ok(Edit::Many(undo))
        }
    }
}

fn split(path: &[String]) -> Result<(&[String], &str), String> {
    match path.split_last() {
        Some((leaf, parents)) => Ok((parents, leaf)),
        None => Err("no key to edit".into()),
    }
}

/// The table at `parents`, made if missing and `create` (as a standard
/// `[table]` at the top, else of the same kind as its parent).
fn descend<'a>(doc: &'a mut DocumentMut, parents: &[String], create: bool) -> Result<Option<Place<'a>>, String> {
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    let mut inline = false;
    let mut made = Vec::new();
    for (depth, k) in parents.iter().enumerate() {
        if table.get(k).is_none() {
            if !create {
                return Ok(None);
            }
            let new = if inline {
                Item::Value(Value::InlineTable(toml_edit::InlineTable::new()))
            } else if depth == 0 || !table.is_dotted() {
                toml_edit::table()
            } else {
                let mut t = toml_edit::Table::new();
                t.set_dotted(true);
                Item::Table(t)
            };
            table.insert(k, new);
            made.push(parents[..=depth].to_vec());
        }
        let item = table.get_mut(k).expect("present");
        inline = inline || item.is_inline_table();
        let what = parents[..=depth].join(".");
        table = item
            .as_table_like_mut()
            .ok_or_else(|| format!("{what} is set to a value, not a table: change it in the file"))?;
    }
    Ok(Some(Place { table, inline, made }))
}

/// The edit that puts `leaf` back as it is now.
fn restore_of(table: &dyn TableLike, path: &[String], leaf: &str) -> Edit {
    let index = table.iter().position(|(k, _)| k == leaf).unwrap_or(usize::MAX);
    let entry = table.key(leaf).cloned().zip(table.get(leaf).cloned());
    Edit::Restore {
        path: path.to_vec(),
        entry,
        index,
    }
}

/// A value without the decor it came with, for a place of its own.
fn strip(mut v: Value) -> Value {
    v.decor_mut().clear();
    v
}

/// Adds a key at the end of a table. In an inline table the closing
/// space moves to the new last value: `{ a = 1, b = 2 }`.
fn insert(table: &mut dyn TableLike, inline: bool, leaf: &str, value: Value) {
    let mut value = value;
    if inline {
        let last = table.iter().last().map(|(k, _)| k.to_string());
        if let Some(last) = last
            && let Some(Item::Value(v)) = table.get_mut(&last)
        {
            let suffix = v.decor().suffix().cloned().unwrap_or_default();
            v.decor_mut().set_suffix("");
            value.decor_mut().set_prefix(" ");
            value.decor_mut().set_suffix(suffix);
        } else {
            value.decor_mut().set_prefix(" ");
            value.decor_mut().set_suffix(" ");
        }
    }
    table.insert(leaf, Item::Value(value));
}

fn set(table: &mut dyn TableLike, inline: bool, path: &[String], leaf: &str, value: Value) -> Edit {
    let undo = restore_of(table, path, leaf);
    let mut value = strip(value);
    match table.get_mut(leaf) {
        Some(Item::Value(old)) => {
            *value.decor_mut() = old.decor().clone();
            *old = value;
        }
        Some(other) => *other = Item::Value(value),
        None => insert(table, inline, leaf, value),
    }
    undo
}

/// Removes a key. In an inline table the closing space goes to the new
/// last value.
fn take_out(table: &mut dyn TableLike, inline: bool, leaf: &str) {
    let last = table.iter().last().map(|(k, _)| k.to_string());
    let Some(Item::Value(old)) = table.remove(leaf) else {
        return;
    };
    if inline && last.as_deref() == Some(leaf) {
        let prev = table.iter().last().map(|(k, _)| k.to_string());
        if let Some(prev) = prev
            && let Some(Item::Value(v)) = table.get_mut(&prev)
        {
            v.decor_mut().set_suffix(old.decor().suffix().cloned().unwrap_or_default());
        }
    }
}

fn has_comment(d: &toml_edit::Decor) -> bool {
    [d.prefix(), d.suffix()]
        .into_iter()
        .flatten()
        .any(|r| r.as_str().is_some_and(|s| s.contains('#')))
}

/// Removes a key with its own comments. Comments above it that a blank
/// line separates from it belong to the section, not the key: they go to
/// the next key, or are returned when there is none.
fn remove(table: &mut dyn TableLike, inline: bool, path: &[String], leaf: &str, dropped: &mut bool) -> (Edit, String) {
    let undo = restore_of(table, path, leaf);
    let Edit::Restore {
        entry: Some((key, item)),
        index,
        ..
    } = &undo
    else {
        return (undo, String::new());
    };
    let prefix = raw(key.leaf_decor().prefix());
    let (kept, own) = split_section(&prefix);
    let item_comment = match item {
        Item::Value(v) => has_comment(v.decor()),
        Item::Table(t) => has_comment(t.decor()) || t.to_string().contains('#'),
        _ => false,
    };
    *dropped |= own.contains('#') || raw(key.leaf_decor().suffix()).contains('#') || item_comment;
    let index = *index;
    let kept = kept.to_string();
    take_out(table, inline, leaf);
    if kept.is_empty() || inline {
        return (undo, String::new());
    }
    // The next line drawn: the next value, or the first of a dotted table
    // (`colors.foreground = …`), which are drawn with the values.
    let mut through = None;
    let mut next = None;
    for (k, v) in table.iter().skip(index) {
        if v.is_value() {
            next = Some(k.to_string());
            break;
        }
        if let Item::Table(t) = v
            && t.is_dotted()
            && let Some((first, _)) = t.iter().find(|(_, v)| v.is_value())
        {
            through = Some(k.to_string());
            next = Some(first.to_string());
            break;
        }
    }
    let Some(next) = next else {
        return (undo, kept);
    };
    let mut next_path = path[..path.len() - 1].to_vec();
    let target: &mut dyn TableLike = match &through {
        Some(t) => {
            next_path.push(t.clone());
            table.get_mut(t).and_then(Item::as_table_like_mut).expect("found above")
        }
        None => table,
    };
    next_path.push(next.clone());
    let snapshot = restore_of(target, &next_path, &next);
    if let Some(mut k) = target.key_mut(&next) {
        let p = raw(k.leaf_decor().prefix());
        k.leaf_decor_mut().set_prefix(format!("{kept}{p}"));
    }
    (Edit::Many(vec![undo, snapshot]), String::new())
}

/// A key's prefix split after its last blank line: the section's part and
/// the key's own.
fn split_section(prefix: &str) -> (&str, &str) {
    let mut end = 0;
    let mut at = 0;
    for line in prefix.split_inclusive('\n') {
        at += line.len();
        if line.ends_with('\n') && line.trim().is_empty() {
            end = at;
        }
    }
    prefix.split_at(end)
}

/// Puts section comments left by removing a table's last key (`kept`)
/// where they still show: above the next table, or at the end of the
/// file. `undo` is the removal's undo; the result undoes both.
fn settle(doc: &mut DocumentMut, parents: &[String], kept: &str, undo: Edit) -> Edit {
    if kept.is_empty() {
        return undo;
    }
    // Keys of the top level and of dotted tables come before every table.
    let after = match parents.first().and_then(|k| doc.get(k)) {
        Some(Item::Table(t)) if !t.is_dotted() => Some(t.position().unwrap_or(isize::MAX)),
        _ => None,
    };
    Edit::Many(vec![undo, keep_before_table(doc, kept, after)])
}

/// `kept` above the first top-level table after position `after`, or at
/// the end of the file.
fn keep_before_table(doc: &mut DocumentMut, kept: &str, after: Option<isize>) -> Edit {
    let first = doc
        .as_table()
        .iter()
        .filter_map(|(k, v)| {
            v.as_table()
                .filter(|t| !t.is_dotted())
                .map(|t| (t.position().unwrap_or(isize::MAX), k.to_string()))
        })
        .filter(|(p, _)| after.is_none_or(|a| *p > a))
        .min();
    match first {
        Some((_, key)) => {
            let Some(Item::Table(t)) = doc.as_table_mut().get_mut(&key) else {
                unreachable!("found above")
            };
            let old = raw(t.decor().prefix());
            t.decor_mut().set_prefix(format!("{kept}{old}"));
            Edit::TablePrefix { key, prefix: old }
        }
        None => {
            let old = raw(Some(doc.trailing()));
            doc.set_trailing(format!("{kept}{old}"));
            Edit::Trailing(old)
        }
    }
}

fn raw(r: Option<&toml_edit::RawString>) -> String {
    r.and_then(|r| r.as_str()).unwrap_or("").to_string()
}

/// The prefix for a new element next to `near`: the same indent on a line
/// of its own in a multi-line list, else a space. Comments stay behind.
fn fresh_prefix(near: Option<&Value>) -> String {
    let p = raw(near.and_then(|v| v.decor().prefix()));
    match p.rfind('\n') {
        Some(i) => format!("\n{}", &p[i + 1..]),
        None => " ".into(),
    }
}

fn list_op(array: &mut toml_edit::Array, op: &ListOp) -> Result<(), String> {
    match op {
        ListOp::Set(i, v) => {
            while array.len() <= *i {
                array.push("");
            }
            array.replace(*i, strip(v.clone()));
        }
        ListOp::Insert(i, v) => {
            let i = (*i).min(array.len());
            let mut v = strip(v.clone());
            if array.is_empty() {
                array.insert_formatted(0, v);
                // Default decor: `["a"]`.
                array.fmt();
                return Ok(());
            }
            let near = array.get(i.min(array.len() - 1)).or_else(|| array.get(i.saturating_sub(1)));
            let prefix = fresh_prefix(near);
            if i == 0 {
                // The new first takes the old first's prefix; the old one
                // now follows a comma.
                let old = raw(array.get(0).and_then(|v| v.decor().prefix()));
                v.decor_mut().set_prefix(old);
                array.get_mut(0).expect("not empty").decor_mut().set_prefix(prefix);
            } else {
                v.decor_mut().set_prefix(prefix);
            }
            v.decor_mut().set_suffix("");
            array.insert_formatted(i, v);
        }
        ListOp::Remove(i) => {
            if *i >= array.len() {
                return Err("no such element".into());
            }
            let removed = array.remove(*i);
            let prefix = raw(removed.decor().prefix());
            if *i == 0 {
                if let Some(next) = array.get_mut(0) {
                    next.decor_mut().set_prefix(prefix);
                }
            } else {
                // A comment after the previous element's comma is that
                // element's: it stays.
                let kept = prefix.find('\n').map_or("", |n| &prefix[..n]).to_string();
                if !kept.trim().is_empty() {
                    match array.get_mut(*i) {
                        Some(next) => {
                            let p = raw(next.decor().prefix());
                            next.decor_mut().set_prefix(format!("{kept}{p}"));
                        }
                        None => {
                            let t = raw(Some(array.trailing()));
                            array.set_trailing(format!("{kept}{t}"));
                        }
                    }
                }
            }
        }
        ListOp::Move(from, to) => {
            if *from >= array.len() || *to >= array.len() {
                return Err("no such element".into());
            }
            // The values move; each slot keeps its decor.
            let mut values: Vec<Value> = array.iter().map(|v| strip(v.clone())).collect();
            let v = values.remove(*from);
            values.insert(*to, v);
            for (i, v) in values.into_iter().enumerate() {
                array.replace(i, v);
            }
        }
    }
    Ok(())
}

/// Where a write to `path` goes: the file a symlink points at, so the link
/// stays a link.
pub fn target(path: &Path) -> PathBuf {
    if let Ok(p) = fs::canonicalize(path) {
        return p;
    }
    // A link to a file not made yet.
    match fs::read_link(path) {
        Ok(link) if link.is_absolute() => link,
        Ok(link) => path.parent().unwrap_or(Path::new(".")).join(link),
        Err(_) => path.to_path_buf(),
    }
}

/// Writes `text` to `path` atomically: a temp file in the same folder,
/// renamed over the target (a symlink's target), keeping its permissions.
/// A read-only file isn't written.
pub fn write(path: &Path, text: &str) -> io::Result<()> {
    let target = target(path);
    let dir = target.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let old = match fs::metadata(&target) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if old.as_ref().is_some_and(|m| m.permissions().readonly()) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the file is read-only"));
    }
    let name = target.file_name().map_or("config.toml".into(), |n| n.to_string_lossy());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let _ = fs::remove_file(&tmp);
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        if let Some(m) = &old {
            fs::set_permissions(&tmp, m.permissions())?;
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
