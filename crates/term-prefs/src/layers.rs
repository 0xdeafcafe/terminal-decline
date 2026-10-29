//! The values the TUI shows: the edited file over the layers below it and
//! the defaults, under the layers above it, and where each value comes
//! from.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, TableLike};

use crate::Host;
use crate::file;

/// Where a value comes from, said on its row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Set in the file being edited.
    Here,
    /// From a file below the edited one (a pin's file over the user's).
    Inherited(String),
    /// Set in a file above the edited one (`--config`), which wins.
    Overridden(String),
    /// The app's default.
    Default,
    /// Not set anywhere, and no default value.
    Unset,
}

impl Origin {
    pub fn describe(&self) -> String {
        match self {
            Origin::Here => "set here".into(),
            Origin::Inherited(f) => format!("inherited from {f}"),
            Origin::Overridden(f) => format!("overridden by {f}"),
            Origin::Default => "default".into(),
            Origin::Unset => String::new(),
        }
    }
}

/// The edited file and the layers around it.
pub struct Layers {
    pub paths: Vec<PathBuf>,
    pub edit: usize,
    /// Each layer parsed; `None` if missing or not parsable.
    docs: Vec<Option<DocumentMut>>,
    /// The edited file's text as read (`""` if missing).
    pub text: String,
    /// Why the edited file can't be edited: it doesn't parse (toml_edit's
    /// message, with line and column), or can't be read.
    pub error: Option<String>,
    /// The file exists but can't be written.
    pub read_only: bool,
    defaults: DocumentMut,
}

/// What a shortcut does once the layers are combined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bound {
    Action(String),
    Text(String),
    /// `"none"`.
    Unbound,
}

impl Bound {
    pub fn of(item: &Item) -> Option<Bound> {
        if let Some(s) = item.as_str() {
            return Some(if s == "none" {
                Bound::Unbound
            } else {
                Bound::Action(s.to_string())
            });
        }
        let t = item.as_table_like()?;
        t.get("text").and_then(Item::as_str).map(|s| Bound::Text(s.to_string()))
    }
}

/// One shortcut in effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bind {
    /// Canonical spelling.
    pub chord: String,
    pub bound: Bound,
    /// Its default action, if the chord is one of the menus'.
    pub default: Option<String>,
    pub origin: Origin,
    /// Whether the edited file has an entry for it.
    pub here: bool,
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The item at `path` in a table, through standard, inline and dotted
/// tables.
pub fn get<'a>(table: &'a dyn TableLike, path: &[String]) -> Option<&'a Item> {
    let (first, rest) = path.split_first()?;
    let item = table.get(first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        get(item.as_table_like()?, rest)
    }
}

impl Layers {
    pub fn load(paths: Vec<PathBuf>, edit: usize, defaults: &str) -> Layers {
        let mut docs = Vec::new();
        let mut text = String::new();
        let mut error = None;
        let mut read_only = false;
        for (i, path) in paths.iter().enumerate() {
            let read = file::read(path);
            let doc = match &read {
                Ok(Some(t)) => file::parse(t).ok(),
                Ok(None) => Some(DocumentMut::new()),
                Err(_) => None,
            };
            if i == edit {
                match read {
                    Ok(t) => {
                        text = t.unwrap_or_default();
                        if let Err(e) = file::parse(&text) {
                            error = Some(e);
                        }
                    }
                    Err(e) => error = Some(format!("can't read {}: {e}", path.display())),
                }
                read_only = std::fs::metadata(file::target(path)).is_ok_and(|m| m.permissions().readonly());
            }
            docs.push(doc);
        }
        Layers {
            paths,
            edit,
            docs,
            text,
            error,
            read_only,
            defaults: file::parse(defaults).unwrap_or_default(),
        }
    }

    pub fn edited_path(&self) -> &Path {
        &self.paths[self.edit]
    }

    pub fn edited_name(&self) -> String {
        name_of(self.edited_path())
    }

    /// The edited file's own item at `path`.
    pub fn here(&self, path: &[String]) -> Option<&Item> {
        get(self.docs.get(self.edit)?.as_ref()?.as_table(), path)
    }

    /// The value in effect at `path` and where it comes from.
    pub fn lookup(&self, path: &[String]) -> (Option<&Item>, Origin) {
        for i in (0..self.docs.len()).rev() {
            if let Some(item) = self.docs[i].as_ref().and_then(|d| get(d.as_table(), path)) {
                let name = name_of(&self.paths[i]);
                let origin = match i.cmp(&self.edit) {
                    std::cmp::Ordering::Greater => Origin::Overridden(name),
                    std::cmp::Ordering::Equal => Origin::Here,
                    std::cmp::Ordering::Less => Origin::Inherited(name),
                };
                return (Some(item), origin);
            }
        }
        match get(self.defaults.as_table(), path) {
            Some(item) => (Some(item), Origin::Default),
            None => (None, Origin::Unset),
        }
    }

    /// The value the layers below the edited file give `path`: what a
    /// reset leaves.
    pub fn below(&self, path: &[String]) -> Option<&Item> {
        (0..self.edit)
            .rev()
            .find_map(|i| self.docs[i].as_ref().and_then(|d| get(d.as_table(), path)))
            .or_else(|| get(self.defaults.as_table(), path))
    }

    /// The shortcuts in effect: the defaults with each layer's `[keybind]`
    /// applied in order, chords compared by their parsed form.
    pub fn keybinds(&self, host: &dyn Host) -> Vec<Bind> {
        let mut out: Vec<Bind> = host
            .default_keybinds()
            .into_iter()
            .map(|(chord, action)| Bind {
                chord,
                bound: Bound::Action(action.clone()),
                default: Some(action),
                origin: Origin::Default,
                here: false,
            })
            .collect();
        for (i, doc) in self.docs.iter().enumerate() {
            let Some(table) = doc.as_ref().and_then(|d| d.get("keybind")).and_then(Item::as_table_like) else {
                continue;
            };
            let name = name_of(&self.paths[i]);
            for (key, item) in table.iter() {
                let (Ok(chord), Some(bound)) = (host.parse_chord(key), Bound::of(item)) else {
                    continue;
                };
                let origin = match i.cmp(&self.edit) {
                    std::cmp::Ordering::Greater => Origin::Overridden(name.clone()),
                    std::cmp::Ordering::Equal => Origin::Here,
                    std::cmp::Ordering::Less => Origin::Inherited(name.clone()),
                };
                let here = i == self.edit;
                match out.iter_mut().find(|b| b.chord == chord) {
                    Some(b) => {
                        b.bound = bound;
                        b.here |= here;
                        b.origin = origin;
                    }
                    None => out.push(Bind {
                        chord,
                        bound,
                        default: None,
                        origin,
                        here,
                    }),
                }
            }
        }
        out
    }
}
