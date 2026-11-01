//! The preferences TUI: its state, what each key does, and each frame.
//! Platform-free: the runner in `lib.rs` feeds it events and draws its
//! frames; the screen tests do the same into term-core.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use toml_edit::{Item, Value};

use crate::file::{self, Edit, KeyPath, ListOp};
use crate::host::{Action, Checked, Font, Host, Pin, ThemeInfo};
use crate::keymap::{self, ActionKeys, How, Recorded, Taken};
use crate::layers::{Bind, Bound, Layers, Origin};
use crate::pace::Pace;
use crate::preview::Preview;
use crate::schema::{self, Applies, Editor, SETTINGS, Section, Setting};
use crate::ui::frame::{Frame, str_width};
use crate::ui::input::{Event, Key, Mods, MouseKind};

mod draw;

/// At most this often while a number steps (key repeat); the last value
/// is always written.
pub const WRITE_EVERY: Duration = Duration::from_millis(100);

/// How long the selection in a font list rests before its sample is
/// drawn, so moving quickly through the list draws nothing.
pub const SAMPLE_AFTER: Duration = Duration::from_millis(200);

/// At most this often while the selection moves through a theme list
/// (key repeat); where it stops is always previewed.
pub const PREVIEW_EVERY: Duration = Duration::from_millis(100);

/// The text of a font's sample: letters, the digits and letters that
/// look alike, and code.
pub const SAMPLE_TEXT: &str = "AaBbGg 0O 1lI |{}[]() => != ~/src $";

/// What the runner does for the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Bytes for the terminal (the chord recording request).
    Send(String),
    /// Run the editor on this file, then [`App::reload`].
    EditFile(PathBuf),
    /// Draw the next frame whole.
    Repaint,
    /// Watch these files again (the edited file changed).
    Watch(Vec<PathBuf>),
    Quit,
}

#[derive(Clone, Debug)]
enum RowKind {
    /// A group's name (the shortcut list's menus): not selectable.
    Header,
    Setting(&'static Setting),
    /// A menu action and its shortcuts.
    Action(ActionKeys),
    ThemeSide(&'static str),
    Palette(usize),
    Bind(Bind),
    AddBind,
    /// An entry of a table of strings (`env`, `symbol_map`), by key and name.
    Env(&'static str, String),
    AddEnv(&'static str),
    Pin(Pin),
    Info,
}

#[derive(Clone, Debug)]
struct Row {
    kind: RowKind,
    label: String,
    value: String,
    note: String,
    swatch: Option<u32>,
    description: String,
    /// More text search matches (an action's name and shortcuts).
    keywords: String,
}

impl Row {
    fn new(kind: RowKind, label: impl Into<String>) -> Row {
        Row {
            kind,
            label: label.into(),
            value: String::new(),
            note: String::new(),
            swatch: None,
            description: String::new(),
            keywords: String::new(),
        }
    }

    fn selectable(&self) -> bool {
        !matches!(self.kind, RowKind::Header)
    }
}

/// What a committed value does.
#[derive(Clone, Debug)]
enum Then {
    Set(KeyPath),
    ThemeSide(&'static str),
    Palette(usize),
    Font(KeyPath),
    BindText(String),
    /// A new chord for the binding at `from`, or for a new key that
    /// sends text.
    BindChord(Option<String>),
    /// A new shortcut for an action, replacing its shortcuts or added to
    /// them.
    ActionChord(String, How),
    EnvName(&'static str),
    EnvValue(&'static str, String),
    /// An element of a list: replace `Some(i)`, or add at the end.
    ListItem(KeyPath, Option<usize>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextKind {
    Plain,
    Colour,
    Width,
    Chord,
}

#[derive(Clone, Debug)]
enum Confirmed {
    Edit(Box<Edit>, String),
    RemovePin(String),
    /// A new text key's chord is in use: type its text anyway.
    TextFor(String),
    /// An action's new shortcut is another action's (`other`) or sends
    /// text (`None`): replace it, or swap with the other action when it
    /// can take this one's old shortcuts (`swap`).
    Clash {
        action: String,
        chord: String,
        how: How,
        other: Option<String>,
        swap: bool,
    },
}

/// One at a time, so its size doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum Mode {
    Browse,
    Help,
    Choice {
        title: String,
        /// Value and label.
        items: Vec<(String, String)>,
        filter: String,
        sel: usize,
        then: Then,
        /// A font picker: a name typed that isn't listed is taken as is.
        fonts: bool,
    },
    Number {
        path: KeyPath,
        value: f64,
        typed: Option<String>,
        session: u64,
        min: f64,
        max: f64,
        step: f64,
        integer: bool,
    },
    Text {
        title: String,
        buf: String,
        /// In chars.
        cursor: usize,
        kind: TextKind,
        then: Then,
    },
    List {
        path: KeyPath,
        fonts: bool,
        sel: usize,
    },
    Recording(Then),
    Confirm(String, Confirmed),
}

struct Search {
    query: String,
}

#[derive(Clone, Copy, Default)]
struct Hit {
    /// Sections pane: x range and the y of the first section.
    sections: Option<(usize, usize, usize)>,
    /// Rows: x range, first y, first row index, count.
    rows: (usize, usize, usize, usize, usize),
    /// A popup's items: x range, first y, first index, count.
    popup: Option<(usize, usize, usize, usize, usize)>,
}

struct Status {
    text: String,
    error: bool,
}

/// The preferences program.
pub struct App<'h> {
    host: &'h dyn Host,
    in_app: bool,
    layers: Layers,
    pin_file: bool,
    /// The layers to go back to after a pin's file opened from Pins.
    parent: Option<(Vec<PathBuf>, usize)>,
    section: usize,
    row: usize,
    scroll: usize,
    mode: Mode,
    search: Option<Search>,
    status: Option<Status>,
    checked: Checked,
    undo: Vec<(Edit, Option<u64>)>,
    sessions: u64,
    pending: Option<(Edit, Option<u64>)>,
    last_write: Option<Instant>,
    last_written: Option<String>,
    fonts: Option<Vec<Font>>,
    width: usize,
    height: usize,
    /// A cell's size in pixels, if the terminal says (for the sample).
    cell_px: (u32, u32),
    hit: Hit,
    effects: Vec<Effect>,
    /// The theme every window shows while a theme list is open (the live
    /// preview), and the colours windows show with it (`[colors]` on
    /// top), for drawing this tab.
    previewed: Option<(String, Checked)>,
    /// The theme to preview next while the list moves faster than
    /// [`PREVIEW_EVERY`].
    theme_pace: Pace<String>,
    /// The font under a font list's selection, until it has rested for
    /// [`SAMPLE_AFTER`]; then `sample` names it.
    sample_pace: Pace<String>,
    /// The font the open font list shows a sample of.
    sample: Option<String>,
    /// Samples drawn, newest last: font, cells, pixels and colours, and
    /// the pixels (`None`: not installed, or the host can't draw).
    samples: RefCell<Vec<(SampleKey, Pixels)>>,
    /// Themes' titles and colours, read once each until the files are
    /// read again.
    theme_infos: RefCell<HashMap<String, Option<ThemeInfo>>>,
    /// The menus' actions.
    actions: Vec<Action>,
    /// The pins, read once until the files are read again.
    pins: RefCell<Option<Vec<Pin>>>,
    /// Bumped when what the rows show may have changed (the files were
    /// read again).
    generation: Cell<u64>,
    /// The rows last worked out, and for what.
    rows_cache: RefCell<Option<(RowsKey, Rc<Vec<Row>>)>>,
    /// For a list with group headers: the item on each row shown (`None`
    /// for a header). Empty for a list without.
    hit_rows: Vec<Option<usize>>,
}

/// What the rows are worked out for: the files' generation, the section,
/// the search and whether a pin's file is edited.
type RowsKey = (u64, usize, Option<String>, bool);

/// A sample's font, size in cells and pixels, and colours.
type SampleKey = (String, usize, usize, u32, u32, u32, u32);

/// A sample's RGB pixels; `None` if the font isn't installed or the host
/// can't draw.
type Pixels = Option<Rc<Vec<u8>>>;

fn path_of(key: &str) -> KeyPath {
    key.split('.').map(str::to_string).collect()
}

fn format_number(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        let s = format!("{:.4}", f);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn format_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.value().clone(),
        Value::Integer(i) => i.value().to_string(),
        Value::Float(f) => format_number(*f.value()),
        Value::Boolean(b) => if *b.value() { "on" } else { "off" }.into(),
        Value::Array(a) if a.is_empty() => "none".into(),
        Value::Array(a) => a.iter().map(format_value).collect::<Vec<_>>().join(", "),
        Value::InlineTable(t) => t
            .iter()
            .map(|(k, v)| format!("{k} = {}", format_value(v)))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Datetime(d) => d.value().to_string(),
    }
}

fn format_item(item: &Item) -> String {
    match item {
        Item::Value(v) => format_value(v),
        Item::Table(t) if t.is_empty() => "none".into(),
        Item::Table(t) => t
            .iter()
            .map(|(k, v)| format!("{k} = {}", format_item(v)))
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn number_of(item: Option<&Item>) -> Option<f64> {
    let v = item?.as_value()?;
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
}

/// A number as TOML: whole numbers as integers (`13` → `14`, not `14.0`).
fn number_value(f: f64, integer: bool) -> Value {
    if integer || f.fract() == 0.0 {
        Value::from(f.round() as i64)
    } else {
        Value::from((f * 10_000.0).round() / 10_000.0)
    }
}

fn luminance(rgb: u32) -> f64 {
    let c = |s: u32| ((rgb >> s) & 0xff) as f64 / 255.0;
    0.2126 * c(16) + 0.7152 * c(8) + 0.0722 * c(0)
}

impl<'h> App<'h> {
    pub fn new(host: &'h dyn Host, layers: Vec<PathBuf>, edit: usize, in_app: bool) -> App<'h> {
        let mut app = App {
            host,
            in_app,
            layers: Layers::load(Vec::new(), 0, ""),
            pin_file: false,
            parent: None,
            section: 0,
            row: 0,
            scroll: 0,
            mode: Mode::Browse,
            search: None,
            status: None,
            checked: Checked::default(),
            undo: Vec::new(),
            sessions: 0,
            pending: None,
            last_write: None,
            last_written: None,
            fonts: None,
            width: 80,
            height: 24,
            cell_px: (0, 0),
            hit: Hit::default(),
            effects: Vec::new(),
            previewed: None,
            theme_pace: Pace::throttle(PREVIEW_EVERY),
            sample_pace: Pace::debounce(SAMPLE_AFTER),
            sample: None,
            samples: RefCell::new(Vec::new()),
            theme_infos: RefCell::new(HashMap::new()),
            actions: host.actions(),
            pins: RefCell::new(None),
            generation: Cell::new(0),
            rows_cache: RefCell::new(None),
            hit_rows: Vec::new(),
        };
        app.open(layers, edit);
        app
    }

    /// The `editor` setting in effect (empty when unset).
    pub fn editor_setting(&self) -> String {
        let (item, _) = self.layers.lookup(&["editor".to_string()]);
        item.and_then(|i| i.as_str()).unwrap_or("").to_string()
    }

    fn open(&mut self, layers: Vec<PathBuf>, edit: usize) {
        self.pin_file = edit > 0 && self.host.is_pin_file(&layers[edit]);
        self.layers = Layers::load(layers, edit, &self.host.defaults());
        self.touch();
        self.undo.clear();
        self.section = 0;
        self.row = 0;
        self.scroll = 0;
        self.recheck();
        self.effects.push(Effect::Watch(self.layers.paths.clone()));
    }

    fn recheck(&mut self) {
        self.checked = self.host.check(&self.layers.paths, self.layers.edit, &self.layers.text);
    }

    /// Reads the files again (a watched file changed, or `$EDITOR` exited).
    /// A change that is only this program's last write says nothing.
    pub fn reload(&mut self) {
        let before = self.layers.text.clone();
        self.layers = Layers::load(self.layers.paths.clone(), self.layers.edit, &self.host.defaults());
        self.touch();
        self.recheck();
        let ours = self.last_written.as_deref() == Some(self.layers.text.as_str());
        if self.layers.text != before && !ours {
            self.say(format!("{} changed on disk: reloaded", self.layers.edited_name()));
        }
        self.clamp();
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.effects.push(Effect::Repaint);
    }

    /// The size of a cell in pixels, as the terminal reports it (0 if it
    /// doesn't): font samples are drawn to fit their cells.
    pub fn set_cell_pixels(&mut self, width: u32, height: u32) {
        self.cell_px = (width, height);
    }

    /// The files were read again: what the rows show may have changed,
    /// and so may the pins and themes.
    fn touch(&self) {
        self.generation.set(self.generation.get() + 1);
        *self.pins.borrow_mut() = None;
        self.theme_infos.borrow_mut().clear();
    }

    /// The pins, read once until the files are read again.
    fn pins(&self) -> Vec<Pin> {
        self.pins.borrow_mut().get_or_insert_with(|| self.host.pins()).clone()
    }

    /// Theme `name`'s title and colours, read once until the files are
    /// read again.
    fn theme_info(&self, name: &str) -> Option<ThemeInfo> {
        if name.is_empty() {
            return None;
        }
        if let Some(info) = self.theme_infos.borrow().get(name) {
            return info.clone();
        }
        let info = self.host.theme_info(name);
        self.theme_infos.borrow_mut().insert(name.to_string(), info.clone());
        info
    }

    fn action_label(&self, name: &str) -> String {
        self.actions
            .iter()
            .find(|a| a.name == name)
            .map_or_else(|| name.to_string(), |a| a.label.clone())
    }

    /// What the runner should do now.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// When something held back is due, if anything is: a write, a theme
    /// preview or a font sample.
    pub fn deadline(&self) -> Option<Instant> {
        let write = self
            .pending
            .as_ref()
            .map(|_| self.last_write.map_or_else(Instant::now, |t| t + WRITE_EVERY));
        [write, self.theme_pace.deadline(), self.sample_pace.deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    /// Does what is held back and due at `now` ([`App::deadline`]).
    pub fn tick(&mut self, now: Instant) {
        let write_due = self.pending.as_ref().map(|_| self.last_write.map_or(now, |t| t + WRITE_EVERY));
        if write_due.is_some_and(|d| now >= d) {
            self.flush(now);
        }
        if let Some(name) = self.theme_pace.poll(now) {
            self.preview(&name);
        }
        if let Some(name) = self.sample_pace.poll(now) {
            self.sample = Some(name);
        }
    }

    /// Writes a held-back value now (before quitting, or any other edit).
    pub fn flush(&mut self, now: Instant) {
        if let Some((edit, session)) = self.pending.take() {
            self.write(edit, session, now);
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: false,
        });
    }

    fn fail(&mut self, text: impl AsRef<str>) {
        self.status = Some(Status {
            text: format!("error: {}", text.as_ref()),
            error: true,
        });
    }

    /// Applies `edit` to the file as it is now, checks it with the app's
    /// loader and writes it. Returns whether it was written.
    fn write(&mut self, edit: Edit, session: Option<u64>, now: Instant) -> Option<file::Prepared> {
        let name = self.layers.edited_name();
        if let Some(e) = &self.layers.error {
            let e = e.clone();
            self.fail(format!("{name} can't be edited here ({e}): press e to fix it in your editor"));
            return None;
        }
        if self.layers.read_only {
            self.fail(format!("{name} is read-only"));
            return None;
        }
        let path = self.layers.edited_path().to_path_buf();
        let text = match file::read(&path) {
            Ok(t) => t.unwrap_or_default(),
            Err(e) => {
                self.fail(format!("can't read {name}: {e}"));
                return None;
            }
        };
        let host = self.host;
        let prepared = match file::prepare(&text, &edit, &|c: &str| host.parse_chord(c).ok()) {
            Ok(p) => p,
            Err(e) => {
                self.fail(e);
                return None;
            }
        };
        let paths = &self.layers.paths;
        let before = self.host.check(paths, self.layers.edit, &text);
        let after = self.host.check(paths, self.layers.edit, &prepared.text);
        if let Some(new) = after.notices.iter().find(|n| !before.notices.contains(n)) {
            let new = new.clone();
            self.fail(new.strip_prefix("config: ").unwrap_or(&new));
            return None;
        }
        if let Err(e) = file::write(&path, &prepared.text) {
            self.fail(format!("can't write {name}: {e}"));
            return None;
        }
        self.last_write = Some(now);
        self.last_written = Some(prepared.text.clone());
        let coalesce = session.is_some() && self.undo.last().is_some_and(|(_, s)| *s == session);
        if !coalesce {
            self.undo.push((prepared.undo.clone(), session));
        }
        self.layers = Layers::load(self.layers.paths.clone(), self.layers.edit, &self.host.defaults());
        self.touch();
        self.checked = after;
        self.status = None;
        Some(prepared)
    }

    /// As [`App::write`], held back while values step faster than
    /// [`WRITE_EVERY`].
    fn write_soon(&mut self, edit: Edit, session: u64, now: Instant) {
        let due = self.last_write.is_none_or(|t| now.duration_since(t) >= WRITE_EVERY);
        if due {
            self.pending = None;
            self.write(edit, Some(session), now);
        } else {
            self.pending = Some((edit, Some(session)));
        }
    }

    fn sections(&self) -> Vec<Section> {
        Section::ALL.into_iter().filter(|s| *s != Section::Pins || !self.pin_file).collect()
    }

    fn current_section(&self) -> Section {
        let s = self.sections();
        s[self.section.min(s.len() - 1)]
    }

    fn note(&self, origin: &Origin, applies: Applies) -> String {
        let mut n = origin.describe();
        if applies == Applies::NewWindows {
            if !n.is_empty() {
                n.push_str(" · ");
            }
            n.push_str("new windows");
        }
        n
    }

    fn setting_row(&self, s: &'static Setting) -> Row {
        let (item, origin) = self.layers.lookup(&s.path());
        let value = match item {
            Some(i) => format_item(i),
            None if !s.unset.is_empty() => s.unset.to_string(),
            None => "unset".into(),
        };
        let swatch = (s.editor == Editor::Colour)
            .then(|| item.and_then(Item::as_str).and_then(schema::parse_hex))
            .flatten();
        Row {
            kind: RowKind::Setting(s),
            label: s.label.into(),
            value,
            note: self.note(&origin, s.applies),
            swatch,
            description: s.description.into(),
            keywords: String::new(),
        }
    }

    fn theme_pair(&self) -> Option<(String, String)> {
        let item = self.layers.lookup(&["theme".into()]).0?;
        let t = item.as_table_like()?;
        let side = |k| t.get(k).and_then(Item::as_str).unwrap_or("").to_string();
        Some((side("light"), side("dark")))
    }

    /// The light and dark themes: a pair's sides, or one theme twice.
    fn theme_sides(&self) -> Option<(String, String)> {
        if let Some(pair) = self.theme_pair() {
            return Some(pair);
        }
        let one = self.layers.lookup(&path_of("theme")).0?.as_str()?.to_string();
        Some((one.clone(), one))
    }

    /// The edit that makes `side`'s theme `value` ("" for none). A single
    /// theme becomes a pair with the old one on the other side; with none
    /// before, the other side is the theme's own partner (Rosé Pine for
    /// Rosé Pine Dawn) if it has one.
    fn theme_side_edit(&self, side: &str, value: &str) -> Edit {
        let theme = path_of("theme");
        let here_table = self.layers.here(&theme).is_some_and(|i| i.as_table_like().is_some());
        let sides = self.theme_sides();
        let other = |(l, d): &(String, String)| if side == "light" { d.clone() } else { l.clone() };
        if value.is_empty() {
            return match sides.filter(|(l, d)| l != d) {
                // The other side's theme for both.
                Some(pair) => Edit::Set {
                    path: theme,
                    value: Value::from(other(&pair)),
                },
                None => Edit::Remove { path: theme },
            };
        }
        if here_table {
            return Edit::Set {
                path: vec!["theme".into(), side.into()],
                value: Value::from(value),
            };
        }
        let other = match &sides {
            Some(pair) => other(pair),
            None => self
                .host
                .theme_info(value)
                .and_then(|i| i.partner)
                .unwrap_or_else(|| value.to_string()),
        };
        if other == value {
            return Edit::Set {
                path: theme,
                value: Value::from(value),
            };
        }
        let (l, d) = if side == "light" {
            (value.to_string(), other)
        } else {
            (other, value.to_string())
        };
        let mut t = toml_edit::InlineTable::new();
        t.insert("light", Value::from(l));
        t.insert("dark", Value::from(d));
        Edit::Set {
            path: theme,
            value: Value::InlineTable(t),
        }
    }

    fn rows_for(&self, section: Section) -> Vec<Row> {
        let mut rows = Vec::new();
        if section == Section::Pins {
            let pins = self.pins();
            if pins.is_empty() {
                rows.push(Row {
                    kind: RowKind::Info,
                    label: "No pins".into(),
                    value: "make one with terminal-decline pin".into(),
                    note: String::new(),
                    swatch: None,
                    keywords: String::new(),
                    description: "Pins are apps in the Dock that open a terminal with their own settings".into(),
                });
            }
            for p in pins {
                rows.push(Row {
                    label: p.name.clone(),
                    value: p.command.clone(),
                    note: p.icon.clone().map_or_else(String::new, |_| "icon".into()),
                    swatch: None,
                    keywords: String::new(),
                    description: "enter: edit its settings · d: remove it".into(),
                    kind: RowKind::Pin(p),
                });
            }
            return rows;
        }
        for s in SETTINGS.iter().filter(|s| s.section == section && !s.hidden) {
            if s.pin_only && !self.pin_file {
                continue;
            }
            match s.editor {
                Editor::Theme => {
                    // One row for each side of the pair: a single theme is
                    // both.
                    let (item, origin) = self.layers.lookup(&s.path());
                    let sides = self.theme_sides();
                    let mode = self
                        .layers
                        .lookup(&path_of("appearance"))
                        .0
                        .and_then(Item::as_str)
                        .unwrap_or("system")
                        .to_string();
                    for side in ["light", "dark"] {
                        let name = sides.as_ref().map(|(l, d)| if side == "light" { l } else { d }.clone());
                        let info = name.as_deref().and_then(|n| self.theme_info(n));
                        let mut note = if item.is_some() {
                            self.note(&origin, s.applies)
                        } else {
                            String::new()
                        };
                        if mode != "system" && mode != side {
                            note = format!("not used: always {mode}");
                        }
                        rows.push(Row {
                            kind: RowKind::ThemeSide(side),
                            label: format!("{} theme", if side == "light" { "Light" } else { "Dark" }),
                            value: match (&name, &info) {
                                (_, Some(i)) => i.title.clone(),
                                (Some(n), None) => n.clone(),
                                (None, _) => s.unset.to_string(),
                            },
                            note,
                            swatch: info.as_ref().map(|i| i.background),
                            keywords: String::new(),
                            description: format!(
                                "The theme while windows are {side}: moving through the list previews it, Esc puts back the old one"
                            ),
                        });
                    }
                }
                Editor::Palette => {
                    let (item, origin) = self.layers.lookup(&s.path());
                    let array = item.and_then(Item::as_array);
                    for (i, name) in schema::PALETTE_NAMES.iter().enumerate() {
                        let v = array.and_then(|a| a.get(i)).and_then(Value::as_str).unwrap_or("");
                        rows.push(Row {
                            kind: RowKind::Palette(i),
                            label: format!("Colour {i}"),
                            value: if v.is_empty() {
                                format!("{name}, from the theme")
                            } else {
                                v.to_string()
                            },
                            note: if v.is_empty() {
                                String::new()
                            } else {
                                self.note(&origin, s.applies)
                            },
                            swatch: schema::parse_hex(v),
                            keywords: String::new(),
                            description: format!("ANSI colour {i} ({name}), over the theme's"),
                        });
                    }
                }
                Editor::Keybinds => {
                    // Every action with its shortcuts, under its menu's
                    // name; then the keys that send text.
                    let binds = self.layers.keybinds(self.host);
                    let mut group: Option<String> = None;
                    for k in keymap::action_keys(&self.actions, &binds) {
                        if group.as_deref() != Some(k.menu.as_str()) {
                            rows.push(Row::new(RowKind::Header, k.menu.clone()));
                            group = Some(k.menu.clone());
                        }
                        let shown: Vec<String> = k.chords.iter().map(|c| keymap::symbols(c)).collect();
                        let defaults: Vec<String> = k.defaults.iter().map(|c| keymap::symbols(c)).collect();
                        let mut row = Row::new(RowKind::Action(k.clone()), k.label.clone());
                        row.value = if shown.is_empty() { "none".into() } else { shown.join("  ") };
                        row.note = k.origin.describe();
                        row.description = format!(
                            "{} · {}",
                            k.name,
                            if defaults.is_empty() {
                                "no shortcut by default".to_string()
                            } else {
                                format!("by default {}", defaults.join(" "))
                            }
                        );
                        row.keywords = format!("{} {} {}", k.name, k.chords.join(" "), shown.join(" "));
                        rows.push(row);
                    }
                    rows.push(Row::new(RowKind::Header, "Keys that send text"));
                    for b in binds.into_iter().filter(|b| matches!(b.bound, Bound::Text(_))) {
                        let Bound::Text(t) = &b.bound else { continue };
                        let mut row = Row::new(RowKind::Bind(b.clone()), keymap::symbols(&b.chord));
                        row.value = format!("sends {t:?}");
                        row.note = b.origin.describe();
                        row.description = "enter: change the text · c: change the key · d: remove · r: reset".into();
                        row.keywords = b.chord.clone();
                        rows.push(row);
                    }
                    let mut add = Row::new(RowKind::AddBind, "+ add a key that sends text");
                    add.description = "A key that types text into the program, such as Shift+Enter sending a new line".into();
                    rows.push(add);
                }
                Editor::Map => {
                    for name in self.env_names(s.key) {
                        let (item, origin) = self.layers.lookup(&[s.key.into(), name.clone()]);
                        rows.push(Row {
                            label: format!("{} {name}", s.label),
                            value: item.map(format_item).unwrap_or_default(),
                            note: self.note(&origin, s.applies),
                            swatch: None,
                            keywords: String::new(),
                            description: "enter: change the value · d: remove · r: reset".into(),
                            kind: RowKind::Env(s.key, name),
                        });
                    }
                    rows.push(Row {
                        kind: RowKind::AddEnv(s.key),
                        label: if s.key == "env" { "+ add a variable" } else { "+ add a range" }.into(),
                        value: String::new(),
                        note: if s.applies == schema::Applies::NewWindows {
                            "new windows"
                        } else {
                            ""
                        }
                        .into(),
                        swatch: None,
                        keywords: String::new(),
                        description: s.description.into(),
                    });
                }
                _ => rows.push(self.setting_row(s)),
            }
        }
        rows
    }

    fn env_names(&self, key: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        if let (Some(item), _) = self.layers.lookup(&[key.into()])
            && let Some(t) = item.as_table_like()
        {
            names.extend(t.iter().map(|(k, _)| k.to_string()));
        }
        if let Some(t) = self.layers.below(&[key.into()]).and_then(Item::as_table_like) {
            for (k, _) in t.iter() {
                if !names.iter().any(|n| n == k) {
                    names.push(k.to_string());
                }
            }
        }
        if let Some(t) = self.layers.here(&[key.into()]).and_then(Item::as_table_like) {
            for (k, _) in t.iter() {
                if !names.iter().any(|n| n == k) {
                    names.push(k.to_string());
                }
            }
        }
        names
    }

    /// The rows shown: the section's, or every section's that match the
    /// search. Worked out once until the files are read again or the
    /// section or search changes, since every key and scroll step asks.
    fn rows(&self) -> Rc<Vec<Row>> {
        let key: RowsKey = (
            self.generation.get(),
            self.section,
            self.search.as_ref().map(|s| s.query.clone()),
            self.pin_file,
        );
        if let Some((k, rows)) = self.rows_cache.borrow().as_ref()
            && *k == key
        {
            return rows.clone();
        }
        let rows = Rc::new(self.work_out_rows());
        *self.rows_cache.borrow_mut() = Some((key, rows.clone()));
        rows
    }

    fn work_out_rows(&self) -> Vec<Row> {
        match &self.search {
            Some(s) => {
                let q = s.query.to_lowercase();
                self.sections()
                    .into_iter()
                    .flat_map(|sec| {
                        self.rows_for(sec).into_iter().map(move |mut r| {
                            r.label = format!("{} › {}", sec.name(), r.label.trim_start());
                            r
                        })
                    })
                    .filter(|r| {
                        let key = match &r.kind {
                            RowKind::Setting(s) => s.key,
                            _ => "",
                        };
                        r.selectable()
                            && (q.is_empty()
                                || r.label.to_lowercase().contains(&q)
                                || r.description.to_lowercase().contains(&q)
                                || key.contains(&q)
                                || r.keywords.to_lowercase().contains(&q))
                    })
                    .collect()
            }
            None => self.rows_for(self.current_section()),
        }
    }

    /// The nearest row to `row` that can be selected (not a header),
    /// looking first in direction `dir` (1 or -1).
    fn selectable_near(rows: &[Row], row: usize, dir: isize) -> usize {
        let n = rows.len() as isize;
        for d in [dir, -dir] {
            let mut r = row as isize;
            while r >= 0 && r < n {
                if rows[r as usize].selectable() {
                    return r as usize;
                }
                r += d;
            }
        }
        row
    }

    fn clamp(&mut self) {
        let rows = self.rows();
        self.row = Self::selectable_near(&rows, self.row.min(rows.len().saturating_sub(1)), 1);
    }

    /// Handles one event; `now` is for holding back fast writes.
    pub fn handle(&mut self, event: Event, now: Instant) {
        let stepping = matches!((&self.mode, &event), (Mode::Number { .. }, Event::Key(Key::Left | Key::Right, _)));
        if !stepping {
            self.flush(now);
        }
        match event {
            Event::Key(Key::Char('l'), m) if m.ctrl => {
                self.effects.push(Effect::Repaint);
                return;
            }
            Event::Key(Key::Char('c'), m) if m.ctrl => {
                self.flush(now);
                self.effects.push(Effect::Quit);
                return;
            }
            Event::Mouse { kind, x, y } => return self.mouse(kind, x, y, now),
            _ => {}
        }
        let mode = std::mem::replace(&mut self.mode, Mode::Browse);
        self.mode = match mode {
            Mode::Browse => {
                self.browse(event, now);
                return;
            }
            Mode::Help => Mode::Browse,
            Mode::Choice { .. } => return self.choice(mode, event, now),
            Mode::Number { .. } => return self.number(mode, event, now),
            Mode::Text { .. } => return self.text(mode, event, now),
            Mode::List { .. } => return self.list(mode, event, now),
            Mode::Recording(then) => match event {
                Event::Chord(c) => {
                    self.recorded(then, &c, now);
                    return;
                }
                Event::ChordCancelled | Event::Key(Key::Esc, _) => {
                    self.say("cancelled");
                    Mode::Browse
                }
                Event::Key(Key::Char('t'), m) if m.is_empty() => {
                    self.status = None;
                    Mode::Text {
                        title: "Shortcut".into(),
                        buf: String::new(),
                        cursor: 0,
                        kind: TextKind::Chord,
                        then,
                    }
                }
                _ => Mode::Recording(then),
            },
            Mode::Confirm(
                q,
                Confirmed::Clash {
                    action,
                    chord,
                    how,
                    other,
                    swap,
                },
            ) => match event {
                Event::Key(Key::Char(c @ ('y' | 'r' | 's')), _) if c != 's' || swap => {
                    self.status = None;
                    let how = if c == 's' { How::Swap } else { how };
                    self.put_on_action(&action, &chord, how, other.as_deref(), now);
                    self.clamp();
                    Mode::Browse
                }
                Event::Key(Key::Char('n') | Key::Esc, _) => {
                    self.say("cancelled");
                    Mode::Browse
                }
                _ => Mode::Confirm(
                    q,
                    Confirmed::Clash {
                        action,
                        chord,
                        how,
                        other,
                        swap,
                    },
                ),
            },
            Mode::Confirm(q, action) => match event {
                Event::Key(Key::Char('y'), _) => {
                    self.status = None;
                    match action {
                        Confirmed::Edit(edit, done) => {
                            if self.write(*edit, None, now).is_some() {
                                self.say(done);
                            }
                        }
                        Confirmed::RemovePin(name) => {
                            match self.host.remove_pin(&name) {
                                Ok(msg) => self.say(msg),
                                Err(e) => self.fail(e),
                            }
                            self.touch();
                        }
                        Confirmed::TextFor(chord) => {
                            self.text_for(&chord);
                            return;
                        }
                        Confirmed::Clash { .. } => unreachable!("handled above"),
                    }
                    self.clamp();
                    Mode::Browse
                }
                Event::Key(Key::Char('n') | Key::Esc, _) => {
                    self.say("cancelled");
                    Mode::Browse
                }
                _ => Mode::Confirm(q, action),
            },
        };
    }

    fn move_row(&mut self, delta: isize) {
        let rows = self.rows();
        let n = rows.len();
        if n == 0 {
            return;
        }
        let to = (self.row as isize + delta).clamp(0, n as isize - 1) as usize;
        self.row = Self::selectable_near(&rows, to, if delta < 0 { -1 } else { 1 });
        self.status = None;
    }

    fn move_section(&mut self, delta: isize) {
        let n = self.sections().len() as isize;
        self.section = ((self.section as isize + delta).rem_euclid(n)) as usize;
        self.row = 0;
        self.scroll = 0;
        self.status = None;
    }

    fn browse(&mut self, event: Event, now: Instant) {
        if let Some(search) = &mut self.search {
            match event {
                Event::Key(Key::Esc, _) => {
                    self.search = None;
                    self.row = 0;
                    self.scroll = 0;
                }
                Event::Key(Key::Backspace, _) => {
                    search.query.pop();
                    self.row = 0;
                }
                Event::Key(Key::Char(c), m) if !m.ctrl && !m.alt && !m.cmd => {
                    search.query.push(c);
                    self.row = 0;
                    self.scroll = 0;
                }
                Event::Paste(t) => {
                    search.query.push_str(&t);
                    self.row = 0;
                }
                Event::Key(Key::Up, _) => self.move_row(-1),
                Event::Key(Key::Down, _) => self.move_row(1),
                Event::Key(Key::Enter, _) => self.activate(now),
                _ => {}
            }
            return;
        }
        let Event::Key(key, m) = event else { return };
        let plain = !m.ctrl && !m.alt && !m.cmd;
        match key {
            Key::Up | Key::Char('k') if plain => self.move_row(-1),
            Key::Down | Key::Char('j') if plain => self.move_row(1),
            Key::PageUp => self.move_row(-(self.page() as isize)),
            Key::PageDown => self.move_row(self.page() as isize),
            Key::Home => self.move_row(isize::MIN / 2),
            Key::End => self.move_row(isize::MAX / 2),
            Key::Left | Key::Char('h') | Key::BackTab if plain => self.move_section(-1),
            Key::Right | Key::Char('l') | Key::Tab if plain => self.move_section(1),
            Key::Enter | Key::Char(' ') if plain => self.activate(now),
            Key::Char('/') => {
                self.search = Some(Search { query: String::new() });
                self.row = 0;
                self.scroll = 0;
                self.status = None;
            }
            Key::Char('?') => self.mode = Mode::Help,
            Key::Char('r') if plain => self.reset(now),
            Key::Char('u') if plain => self.undo(now),
            Key::Char('e') if plain => self.effects.push(Effect::EditFile(self.layers.edited_path().to_path_buf())),
            Key::Char('d') if plain => self.remove(now),
            // ⌫ clears a shortcut, as in macOS's key recorders.
            Key::Backspace | Key::Delete if matches!(self.selected().map(|r| r.kind), Some(RowKind::Action(_) | RowKind::Bind(_))) => {
                self.remove(now)
            }
            Key::Char('c') if plain => match self.selected().map(|r| r.kind) {
                Some(RowKind::Bind(b)) => self.start_chord(Then::BindChord(Some(b.chord))),
                Some(RowKind::Action(k)) => self.start_chord(Then::ActionChord(k.name, How::Replace)),
                _ => {}
            },
            Key::Char('a') if plain => match (self.current_section(), self.selected().map(|r| r.kind)) {
                (_, Some(RowKind::Action(k))) => self.start_chord(Then::ActionChord(k.name, How::Add)),
                (Section::Keys, _) => self.start_chord(Then::BindChord(None)),
                (Section::Advanced, _) => self.edit_text("Variable name", String::new(), TextKind::Plain, Then::EnvName("env")),
                _ => {}
            },
            Key::Char('q') if plain => self.effects.push(Effect::Quit),
            Key::Esc => match self.parent.take() {
                Some((layers, edit)) => {
                    self.open(layers, edit);
                    self.say("back to your own settings");
                }
                None => self.effects.push(Effect::Quit),
            },
            _ => {}
        }
    }

    fn page(&self) -> usize {
        self.hit.rows.4.max(1)
    }

    fn selected(&self) -> Option<Row> {
        self.rows().get(self.row).cloned()
    }

    fn edit_text(&mut self, title: &str, value: String, kind: TextKind, then: Then) {
        self.status = None;
        self.mode = Mode::Text {
            title: title.into(),
            cursor: value.chars().count(),
            buf: value,
            kind,
            then,
        };
    }

    fn start_chord(&mut self, then: Then) {
        if self.in_app {
            self.effects.push(Effect::Send("\x1b]7771;record-chord\x1b\\".into()));
            let clear = if matches!(then, Then::ActionChord(..)) {
                " · ⌫ clears it"
            } else {
                ""
            };
            self.say(format!(
                "press the shortcut · Esc cancels{clear} · t types it instead (macOS keeps ⌘Tab, ⌘Space)"
            ));
            self.mode = Mode::Recording(then);
        } else {
            self.edit_text("Shortcut", String::new(), TextKind::Chord, then);
            self.say("type the shortcut, such as cmd+shift+k");
        }
    }

    fn choose(&mut self, title: &str, items: Vec<(String, String)>, current: &str, then: Then, fonts: bool) {
        let sel = items.iter().position(|(v, _)| v == current).unwrap_or(0);
        self.status = None;
        self.mode = Mode::Choice {
            title: title.into(),
            items,
            filter: String::new(),
            sel,
            then,
            fonts,
        };
    }

    /// Reads the installed fonts again: each time a font list opens, so a
    /// font installed while Settings is open is listed (Core Text is
    /// asked afresh; nothing keeps the list in the app).
    fn refresh_fonts(&mut self) {
        self.fonts = Some(self.host.fonts());
        self.sample = None;
        self.sample_pace.cancel();
        self.samples.borrow_mut().clear();
    }

    /// Every installed family: the monospaced ones, then the others (the
    /// list draws a header before each group), as last read.
    fn font_items(&mut self) -> Vec<(String, String)> {
        let fonts = self.fonts.get_or_insert_with(|| self.host.fonts());
        let (mono, other): (Vec<&Font>, Vec<&Font>) = fonts.iter().partition(|f| f.monospace);
        mono.into_iter()
            .chain(other)
            .map(|f| (f.family.clone(), f.family.clone()))
            .collect()
    }

    /// Whether `family` is installed and monospaced, for the font list's
    /// headers.
    pub(crate) fn is_monospace(&self, family: &str) -> Option<bool> {
        self.fonts.as_ref()?.iter().find(|f| f.family == family).map(|f| f.monospace)
    }

    /// Whether the family or face `name` is monospaced, if it is installed.
    pub(crate) fn font_is_monospace(&self, name: &str) -> Option<bool> {
        self.fonts
            .as_ref()?
            .iter()
            .find(|f| f.family.eq_ignore_ascii_case(name) || f.faces.iter().any(|x| x.eq_ignore_ascii_case(name)))
            .map(|f| f.monospace)
    }

    /// The font family in effect.
    fn family(&self) -> String {
        let (item, _) = self.layers.lookup(&path_of("font_family"));
        item.and_then(Item::as_str).unwrap_or("").to_string()
    }

    /// The selection in a font list moved to `value` (a family, a face,
    /// or "" for the family's own face): its sample is drawn once it has
    /// rested for [`SAMPLE_AFTER`]. Nothing else changes until Enter.
    fn offer_sample(&mut self, value: &str, now: Instant) {
        let name = if value.is_empty() { self.family() } else { value.to_string() };
        if self.sample.as_deref() == Some(name.as_str()) {
            self.sample_pace.cancel();
            return;
        }
        let _ = self.sample_pace.offer(name, now);
    }

    /// A font list closed: no sample.
    fn end_sample(&mut self) {
        self.sample = None;
        self.sample_pace.cancel();
    }

    /// The faces of the family in effect, by full name, after "unset".
    fn face_items(&mut self, unset: &str) -> Vec<(String, String)> {
        let (item, _) = self.layers.lookup(&path_of("font_family"));
        let family = item.and_then(Item::as_str).unwrap_or("").to_string();
        let fonts = self.fonts.get_or_insert_with(|| self.host.fonts());
        let mut items = vec![(String::new(), unset.to_string())];
        if let Some(f) = fonts.iter().find(|f| f.family.eq_ignore_ascii_case(&family)) {
            items.extend(f.faces.iter().map(|face| (face.clone(), face.clone())));
        }
        items
    }

    fn activate(&mut self, now: Instant) {
        let Some(row) = self.selected() else { return };
        match row.kind {
            RowKind::Setting(s) => {
                let path = s.path();
                let (item, _) = self.layers.lookup(&path);
                let current = item.map(format_item).unwrap_or_default();
                match s.editor {
                    Editor::Toggle => {
                        let on = item.and_then(Item::as_bool).unwrap_or(false);
                        self.commit(
                            Edit::Set {
                                path,
                                value: Value::from(!on),
                            },
                            now,
                        );
                    }
                    Editor::Choice(choices) => {
                        let items = choices.iter().map(|c| (c.to_string(), c.to_string())).collect();
                        self.choose(s.label, items, &current, Then::Set(path), false);
                    }
                    Editor::Theme => {}
                    Editor::Font => {
                        self.refresh_fonts();
                        let items = self.font_items();
                        let title = if s.key == "font_family" { "Font family" } else { s.label };
                        self.choose(title, items, &current, Then::Font(path), true);
                        self.say("type to search · the sample shows the font under the selection · Enter uses it");
                        self.offer_sample(&current, now);
                    }
                    Editor::Face => {
                        self.refresh_fonts();
                        let items = self.face_items(s.unset);
                        self.choose(s.label, items, &current, Then::Set(path), true);
                        self.say("type to search, or any face's full name · Enter uses it");
                        self.offer_sample(&current, now);
                    }
                    Editor::Number { min, max, step, integer } => {
                        self.sessions += 1;
                        self.status = None;
                        self.mode = Mode::Number {
                            path,
                            value: number_of(item).unwrap_or(min),
                            typed: None,
                            session: self.sessions,
                            min,
                            max,
                            step,
                            integer,
                        };
                    }
                    Editor::Text => {
                        let v = item.and_then(Item::as_str).unwrap_or("").to_string();
                        self.edit_text(s.label, v, TextKind::Plain, Then::Set(path));
                    }
                    Editor::Width => self.edit_text(s.label, current, TextKind::Width, Then::Set(path)),
                    Editor::Colour => {
                        let v = item.and_then(Item::as_str).unwrap_or("").to_string();
                        self.edit_text(s.label, v, TextKind::Colour, Then::Set(path));
                    }
                    Editor::List(kind) => {
                        self.status = None;
                        self.mode = Mode::List {
                            path,
                            fonts: kind == schema::Item::Font,
                            sel: 0,
                        };
                    }
                    Editor::Palette | Editor::Map | Editor::Keybinds => {}
                }
            }
            RowKind::ThemeSide(side) => {
                let current = self
                    .theme_sides()
                    .map(|(l, d)| if side == "light" { l } else { d })
                    .unwrap_or_default();
                let mut items = vec![("".to_string(), "none (the built-in colours)".to_string())];
                // Grouped by background: the side's own group first, the
                // order inside a group kept. The list draws the headers;
                // they are not items.
                let (mut own, mut other) = (Vec::new(), Vec::new());
                for t in self.host.themes() {
                    let info = self.theme_info(&t);
                    let dark = info.as_ref().is_none_or(ThemeInfo::is_dark);
                    let title = info.map_or_else(|| t.clone(), |i| i.title);
                    if dark == (side == "dark") {
                        own.push((t, title));
                    } else {
                        other.push((t, title));
                    }
                }
                items.extend(own);
                items.extend(other);
                let title = if side == "light" { "Light theme" } else { "Dark theme" };
                self.choose(title, items, &current, Then::ThemeSide(side), false);
            }
            RowKind::Palette(i) => {
                let v = if row.swatch.is_some() { row.value } else { String::new() };
                self.edit_text(&row.label, v, TextKind::Colour, Then::Palette(i));
            }
            RowKind::Bind(b) => {
                let text = match &b.bound {
                    Bound::Text(t) => escape(t),
                    _ => String::new(),
                };
                let sym = keymap::symbols(&b.chord);
                self.edit_text(&format!("{sym} sends"), text, TextKind::Plain, Then::BindText(b.chord));
                self.say(format!(
                    "the text {sym} sends; escapes such as \\e and \\u001b are written as typed"
                ));
            }
            RowKind::Action(k) => self.start_chord(Then::ActionChord(k.name, How::Replace)),
            RowKind::AddBind => self.start_chord(Then::BindChord(None)),
            RowKind::Env(key, name) => {
                let v = self
                    .layers
                    .lookup(&[key.into(), name.clone()])
                    .0
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string();
                self.edit_text(&name, v, TextKind::Plain, Then::EnvValue(key, name.clone()));
            }
            RowKind::AddEnv(key) => {
                let what = if key == "env" {
                    "Variable name"
                } else {
                    "Code points (U+E000-U+F8FF)"
                };
                self.edit_text(what, String::new(), TextKind::Plain, Then::EnvName(key))
            }
            RowKind::Pin(p) => {
                let user = self.host.user_config();
                let back = (self.layers.paths.clone(), self.layers.edit);
                self.open(vec![user, p.config.clone()], 1);
                self.parent = Some(back);
                self.say(format!("editing {}'s settings, over yours · Esc goes back", p.name));
            }
            RowKind::Info | RowKind::Header => {}
        }
    }

    /// Writes an edit at once and reports it.
    fn commit(&mut self, edit: Edit, now: Instant) -> bool {
        self.write(edit, None, now).is_some()
    }

    fn reset(&mut self, now: Instant) {
        let Some(row) = self.selected() else { return };
        if let RowKind::Action(k) = &row.kind {
            let plan = keymap::plan_reset(&self.layers.keybinds(self.host), &self.below_binds(), &k.name);
            let shown = keymap::chords_of(&self.below_binds(), &k.name)
                .iter()
                .map(|c| keymap::symbols(c))
                .collect::<Vec<_>>()
                .join(" ");
            let to = if shown.is_empty() { "no shortcut".to_string() } else { shown };
            self.apply_binds(plan, format!("{} reset to {to} · u undoes", k.label), now);
            self.clamp();
            return;
        }
        let (edit, what) = match &row.kind {
            RowKind::Setting(s) => {
                if s.keep_in_pin && self.pin_file {
                    self.fail(format!(
                        "{} can't be reset in a pin: a pin with no command opens a login shell",
                        s.key
                    ));
                    return;
                }
                (Edit::Remove { path: s.path() }, s.key.to_string())
            }
            RowKind::ThemeSide(_) => (Edit::Remove { path: path_of("theme") }, "theme".into()),
            RowKind::Palette(i) => (self.palette_edit(*i, ""), format!("colors.palette[{i}]")),
            RowKind::Bind(b) => (
                Edit::Keybind {
                    chord: b.chord.clone(),
                    value: None,
                },
                format!("keybind {}", b.chord),
            ),
            RowKind::Env(key, n) => (
                Edit::Remove {
                    path: vec![key.to_string(), n.clone()],
                },
                format!("{key}.{n}"),
            ),
            _ => return,
        };
        if let Some(p) = self.write(edit, None, now) {
            let comment = if p.dropped_comment { "; its comment went with it" } else { "" };
            let from = match &row.kind {
                RowKind::Setting(s) => match self.layers.lookup(&s.path()).1 {
                    Origin::Unset => String::new(),
                    o => format!(" ({})", o.describe()),
                },
                _ => String::new(),
            };
            self.say(format!("{what} reset{from}{comment} · u undoes"));
        }
        self.clamp();
    }

    fn undo(&mut self, now: Instant) {
        let Some((edit, _)) = self.undo.pop() else {
            self.say("nothing to undo");
            return;
        };
        let depth = self.undo.len();
        if self.write(edit, None, now).is_some() {
            self.undo.truncate(depth);
            self.say("undone");
        }
        self.clamp();
    }

    /// The shortcuts the layers below the edited file give (the defaults
    /// for the user's own file).
    fn below_binds(&self) -> Vec<Bind> {
        let below = Layers::load(self.layers.paths[..self.layers.edit].to_vec(), usize::MAX, "");
        below.keybinds(self.host)
    }

    /// The binding the layers below the edited file give `chord`.
    fn bound_below(&self, chord: &str) -> Bound {
        self.below_binds()
            .into_iter()
            .find(|b| b.chord == chord)
            .map_or(Bound::Unbound, |b| b.bound)
    }

    /// Writes the bindings `plan` asks for (each chord and what it should
    /// do) as one edit, one undo, and says `done`.
    fn apply_binds(&mut self, plan: Vec<(String, Bound)>, done: String, now: Instant) {
        let edits: Vec<Edit> = plan.into_iter().map(|(c, want)| self.bind_edit(&c, want)).collect();
        if edits.is_empty() {
            self.say(done);
            return;
        }
        if self.commit(Edit::Many(edits), now) {
            self.say(done);
        }
    }

    /// The edit that makes `chord` do `want`: removing this file's entry if
    /// the layers below already do that ("restoring one deletes the
    /// entry"), else setting it.
    fn bind_edit(&self, chord: &str, want: Bound) -> Edit {
        let value = if self.bound_below(chord) == want {
            None
        } else {
            Some(match want {
                Bound::Action(a) => Value::from(a),
                Bound::Unbound => Value::from("none"),
                Bound::Text(t) => {
                    let mut it = toml_edit::InlineTable::new();
                    it.insert("text", Value::from(t));
                    Value::InlineTable(it)
                }
            })
        };
        Edit::Keybind {
            chord: chord.to_string(),
            value,
        }
    }

    fn remove(&mut self, now: Instant) {
        let Some(row) = self.selected() else { return };
        match row.kind {
            RowKind::Action(k) => {
                let plan = keymap::plan_clear(&self.layers.keybinds(self.host), &k.name);
                if plan.is_empty() {
                    self.say(format!("{} has no shortcut", k.label));
                } else {
                    self.apply_binds(
                        plan,
                        format!("{} has no shortcut now · r gives back the default · u undoes", k.label),
                        now,
                    );
                }
            }
            RowKind::Bind(b) => {
                let edit = self.bind_edit(&b.chord, Bound::Unbound);
                if self.commit(edit, now) {
                    self.say(format!("{} removed · u undoes", keymap::symbols(&b.chord)));
                }
            }
            RowKind::Env(key, n) => {
                if self.commit(
                    Edit::Remove {
                        path: vec![key.into(), n.clone()],
                    },
                    now,
                ) {
                    self.say(format!("{n} removed · u undoes"));
                }
            }
            RowKind::Pin(p) => {
                self.mode = Mode::Confirm(
                    format!("Remove the pin {}? Its Dock tile stays. (y/n)", p.name),
                    Confirmed::RemovePin(p.name),
                );
            }
            _ => {}
        }
        self.clamp();
    }

    fn palette_edit(&self, i: usize, hex: &str) -> Edit {
        let start = self
            .layers
            .lookup(&path_of("colors.palette"))
            .0
            .and_then(Item::as_array)
            .map(|a| a.iter().cloned().collect())
            .unwrap_or_default();
        Edit::List {
            path: path_of("colors.palette"),
            start,
            op: ListOp::Set(i, Value::from(hex)),
        }
    }

    fn list_items(&self, path: &[String]) -> Vec<String> {
        self.layers
            .lookup(path)
            .0
            .and_then(Item::as_array)
            .map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect())
            .unwrap_or_default()
    }

    fn list_edit(&self, path: &[String], op: ListOp) -> Edit {
        Edit::List {
            path: path.to_vec(),
            start: self.list_items(path).into_iter().map(Value::from).collect(),
            op,
        }
    }

    /// A value chosen or typed: what `then` says to do with it.
    fn committed(&mut self, then: Then, value: String, now: Instant) {
        let edit = match then {
            Then::Set(path) => {
                if value.is_empty() {
                    Edit::Remove { path }
                } else {
                    Edit::Set {
                        path,
                        value: Value::from(value),
                    }
                }
            }
            Then::ThemeSide(side) => self.theme_side_edit(side, &value),
            Then::Palette(i) => self.palette_edit(i, &value),
            Then::Font(path) => {
                let fonts = self.fonts.get_or_insert_with(|| self.host.fonts());
                match fonts.iter().find(|f| f.family.eq_ignore_ascii_case(&value)) {
                    Some(f) => Edit::Set {
                        path,
                        value: Value::from(f.family.clone()),
                    },
                    None => {
                        self.fail(format!("no font named {value:?} is installed"));
                        return;
                    }
                }
            }
            Then::BindText(chord) => self.bind_edit(&chord, Bound::Text(unescape(&value))),
            Then::BindChord(_) | Then::ActionChord(..) => return self.chord_entered(then, &value, now),
            Then::EnvName(key) => {
                if value.is_empty() {
                    return;
                }
                self.edit_text(&value, String::new(), TextKind::Plain, Then::EnvValue(key, value.clone()));
                return;
            }
            Then::EnvValue(key, name) => Edit::Set {
                path: vec![key.into(), name],
                value: Value::from(value),
            },
            Then::ListItem(path, index) => {
                let op = match index {
                    Some(i) => ListOp::Set(i, Value::from(value)),
                    None => ListOp::Insert(usize::MAX, Value::from(value)),
                };
                let edit = self.list_edit(&path, op);
                self.commit(edit, now);
                self.mode = Mode::List {
                    fonts: schema::get(&path.join(".")).is_some_and(|s| s.editor == Editor::List(schema::Item::Font)),
                    sel: index.unwrap_or_else(|| self.list_items(&path).len().saturating_sub(1)),
                    path,
                };
                return;
            }
        };
        self.commit(edit, now);
        self.clamp();
    }

    /// A key the app recorded: a shortcut, or ⌫ to clear, `t` to type
    /// one instead, or a plain key, which can't be an action's shortcut
    /// (it types), so recording goes on.
    fn recorded(&mut self, then: Then, chord: &str, now: Instant) {
        let action = matches!(then, Then::ActionChord(..));
        match keymap::recorded(chord) {
            Recorded::Clear => {
                if let Then::ActionChord(name, _) = &then {
                    let plan = keymap::plan_clear(&self.layers.keybinds(self.host), name);
                    let label = self.action_label(name);
                    self.apply_binds(
                        plan,
                        format!("{label} has no shortcut now · r gives back the default · u undoes"),
                        now,
                    );
                } else {
                    self.say("cancelled");
                }
            }
            Recorded::TypeIt => {
                self.edit_text("Shortcut", String::new(), TextKind::Chord, then);
                self.say("type the shortcut, such as cmd+shift+k");
            }
            Recorded::Plain(c) if action => {
                self.start_chord(then);
                self.fail(format!(
                    "{} types: a shortcut needs ⌘, ⌃ or ⌥ (or is an F-key) · press another, Esc cancels",
                    keymap::symbols(&c)
                ));
            }
            Recorded::Plain(c) | Recorded::Chord(c) => self.chord_entered(then, &c, now),
        }
    }

    /// What taking `chord` from the menu item that has it fixed says, if
    /// one does (Quit, Hide, Cmd+1…9).
    fn takes_fixed(&self, chord: &str) -> String {
        self.host
            .fixed_chords()
            .into_iter()
            .find(|(c, _)| self.host.parse_chord(c).ok().as_deref() == Some(chord))
            .map(|(_, item)| format!(" (it takes {} from {item})", keymap::symbols(chord)))
            .unwrap_or_default()
    }

    /// A chord recorded or typed, for an action's new shortcut or a text
    /// key.
    fn chord_entered(&mut self, then: Then, text: &str, now: Instant) {
        let chord = match self.host.parse_chord(text) {
            Ok(c) => c,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        let binds = self.layers.keybinds(self.host);
        let sym = keymap::symbols(&chord);
        let fixed = self.takes_fixed(&chord);
        let from = match then {
            Then::ActionChord(name, how) => {
                if !keymap::usable(&chord) {
                    self.fail(format!("{sym} types: a shortcut needs ⌘, ⌃ or ⌥ (or is an F-key)"));
                    return;
                }
                let label = self.action_label(&name);
                match keymap::taken(&binds, &chord, &name) {
                    Taken::Free | Taken::Same => self.put_on_action(&name, &chord, how, None, now),
                    Taken::Action(other) => {
                        let other_label = self.action_label(&other);
                        let old = keymap::chords_of(&binds, &name);
                        // A swap gives the other action this one's old
                        // shortcuts: only when replacing, and it has some.
                        let swap = how == How::Replace && !old.is_empty();
                        let old_shown = old.iter().map(|c| keymap::symbols(c)).collect::<Vec<_>>().join(" ");
                        let q = if swap {
                            format!("{sym} is {other_label}'s: swap (it gets {old_shown}), or replace (it loses it)?")
                        } else {
                            format!("{sym} is {other_label}'s: give it to {label} instead?")
                        };
                        self.mode = Mode::Confirm(
                            q,
                            Confirmed::Clash {
                                action: name,
                                chord,
                                how,
                                other: Some(other),
                                swap,
                            },
                        );
                    }
                    Taken::Text(t) => {
                        self.mode = Mode::Confirm(
                            format!("{sym} sends {t:?}: give it to {label} instead?"),
                            Confirmed::Clash {
                                action: name,
                                chord,
                                how,
                                other: None,
                                swap: false,
                            },
                        );
                    }
                }
                return;
            }
            Then::BindChord(from) => from,
            _ => return,
        };
        let actions = &self.actions;
        let label = |b: &Bound| match b {
            Bound::Action(a) => actions.iter().find(|x| &x.name == a).map_or(a.clone(), |x| x.label.clone()),
            Bound::Text(t) => format!("sending {t:?}"),
            Bound::Unbound => "nothing".into(),
        };
        match from {
            None => {
                // A new text key: its text next, after asking if the chord
                // is in use.
                if let Some(b) = binds.iter().find(|b| b.chord == chord && b.bound != Bound::Unbound) {
                    self.mode = Mode::Confirm(
                        format!("{sym} is {}: replace?{fixed} (y/n)", label(&b.bound)),
                        Confirmed::TextFor(chord),
                    );
                    return;
                }
                self.text_for(&chord);
                if !fixed.is_empty() {
                    self.say(format!("the text {sym} sends{fixed}"));
                }
            }
            Some(old) => {
                if old == chord {
                    return;
                }
                let Some(moving) = binds.iter().find(|b| b.chord == old).map(|b| b.bound.clone()) else {
                    return;
                };
                let done = format!("{} is now on {sym}{fixed} · u undoes", label(&moving));
                let in_use = binds
                    .iter()
                    .find(|b| b.chord == chord && b.bound != Bound::Unbound)
                    .map(|b| label(&b.bound));
                let edit = Edit::Many(vec![self.bind_edit(&chord, moving), self.bind_edit(&old, Bound::Unbound)]);
                match in_use {
                    Some(what) => {
                        self.mode = Mode::Confirm(format!("{sym} is {what}: replace? (y/n)"), Confirmed::Edit(Box::new(edit), done));
                    }
                    None => {
                        if self.commit(edit, now) {
                            self.say(done);
                        }
                    }
                }
            }
        }
    }

    /// Puts `chord` on action `name`, `how`; `other` is the action that
    /// had it, if any, for the message.
    fn put_on_action(&mut self, name: &str, chord: &str, how: How, other: Option<&str>, now: Instant) {
        let binds = self.layers.keybinds(self.host);
        let plan = keymap::plan(&binds, name, chord, how);
        let (label, sym) = (self.action_label(name), keymap::symbols(chord));
        let fixed = self.takes_fixed(chord);
        if plan.len() == 1 && keymap::taken(&binds, chord, name) == Taken::Same {
            self.say(format!("{sym} is {label}'s shortcut already"));
            return;
        }
        let shortcuts = keymap::chords_of(&binds, name);
        let mut done = match how {
            How::Add if !shortcuts.is_empty() => format!("{label} is on {sym} too{fixed}"),
            _ => format!("{label} is now on {sym}{fixed}"),
        };
        if let Some(other) = other {
            let other_label = self.action_label(other);
            done.push_str(&match how {
                How::Swap => {
                    let old: Vec<String> = shortcuts.iter().filter(|c| *c != chord).map(|c| keymap::symbols(c)).collect();
                    format!(", {other_label} on {}", old.join(" "))
                }
                _ => format!(", {other_label} lost it"),
            });
        }
        done.push_str(" · u undoes");
        self.apply_binds(plan, done, now);
    }

    /// The text editor for a new text key on `chord`.
    fn text_for(&mut self, chord: &str) {
        let sym = keymap::symbols(chord);
        self.edit_text(
            &format!("{sym} sends"),
            String::new(),
            TextKind::Plain,
            Then::BindText(chord.to_string()),
        );
        self.say(format!(
            "the text {sym} sends; escapes such as \\e and \\u001b are written as typed"
        ));
    }

    fn choice(&mut self, mode: Mode, event: Event, now: Instant) {
        let Mode::Choice {
            title,
            items,
            mut filter,
            mut sel,
            then,
            fonts,
        } = mode
        else {
            unreachable!()
        };
        let shown = |items: &[(String, String)], filter: &str| -> Vec<usize> {
            let f = filter.to_lowercase();
            (0..items.len())
                .filter(|&i| f.is_empty() || items[i].1.to_lowercase().contains(&f) || items[i].0.to_lowercase().contains(&f))
                .collect()
        };
        let theme_list = matches!(then, Then::ThemeSide(_));
        match event {
            Event::Key(Key::Esc, _) => {
                // Nothing was written or restyled for a font; a theme
                // preview goes back to the config's.
                self.end_preview(false);
                self.end_sample();
                self.say("cancelled");
                if let Then::ListItem(path, _) = then {
                    self.mode = Mode::List { path, fonts, sel: 0 };
                }
                return;
            }
            Event::Key(Key::Enter, _) => {
                let list = shown(&items, &filter);
                let value = match list.get(sel) {
                    Some(&i) => items[i].0.clone(),
                    None if fonts && !filter.is_empty() => filter.clone(),
                    None => {
                        self.mode = Mode::Choice {
                            title,
                            items,
                            filter,
                            sel,
                            then,
                            fonts,
                        };
                        return;
                    }
                };
                let undo = self.undo.len();
                self.end_sample();
                self.committed(then, value, now);
                // Kept only if it was written; else back to the old one.
                self.end_preview(self.undo.len() > undo);
                return;
            }
            Event::Key(Key::Up, _) => sel = sel.saturating_sub(1),
            Event::Key(Key::Down, _) => sel += 1,
            Event::Key(Key::PageUp, _) => sel = sel.saturating_sub(10),
            Event::Key(Key::PageDown, _) => sel += 10,
            Event::Key(Key::Backspace, _) => {
                filter.pop();
                sel = 0;
            }
            Event::Key(Key::Char(c), m) if !m.ctrl && !m.alt && !m.cmd => {
                filter.push(c);
                sel = 0;
            }
            Event::Paste(t) => {
                filter.push_str(&t);
                sel = 0;
            }
            _ => {}
        }
        let list = shown(&items, &filter);
        sel = sel.min(list.len().saturating_sub(1));
        if let Some(&i) = list.get(sel) {
            let value = items[i].0.clone();
            if theme_list {
                self.offer_preview(value, now);
            } else if fonts {
                self.offer_sample(&value, now);
            }
        }
        self.mode = Mode::Choice {
            title,
            items,
            filter,
            sel,
            then,
            fonts,
        };
    }

    /// The selection in a list moved by mouse: a theme list previews the
    /// theme under it, a font list shows its sample.
    fn choice_moved(&mut self, now: Instant) {
        let Mode::Choice {
            items,
            filter,
            sel,
            then,
            fonts,
            ..
        } = &self.mode
        else {
            return;
        };
        let f = filter.to_lowercase();
        let value = items
            .iter()
            .filter(|(v, l)| f.is_empty() || l.to_lowercase().contains(&f) || v.to_lowercase().contains(&f))
            .nth(*sel)
            .map(|(v, _)| v.clone());
        let (theme_list, fonts) = (matches!(then, Then::ThemeSide(_)), *fonts);
        if let Some(value) = value {
            if theme_list {
                self.offer_preview(value, now);
            } else if fonts {
                self.offer_sample(&value, now);
            }
        }
    }

    /// Theme `name` is under the selection: previewed at once, or, while
    /// the selection moves faster than [`PREVIEW_EVERY`], when it is up.
    fn offer_preview(&mut self, name: String, now: Instant) {
        if let Some(name) = self.theme_pace.offer(name, now) {
            self.preview(&name);
        }
    }

    /// Shows theme `name` in every window (the app's live preview) and in
    /// this tab, if it isn't already.
    fn preview(&mut self, name: &str) {
        if self.previewed.as_ref().is_some_and(|(n, _)| n == name) {
            return;
        }
        if self.in_app {
            self.effects.push(Effect::Send(Preview::Show(name.to_string()).osc()));
        }
        let colours = self
            .host
            .check_preview(&self.layers.paths, self.layers.edit, &self.layers.text, name);
        self.previewed = Some((name.to_string(), colours));
    }

    /// The theme list closed: what was written is kept (`keep`), else the
    /// windows go back to the config's.
    fn end_preview(&mut self, keep: bool) {
        self.theme_pace.cancel();
        if self.previewed.take().is_some() && self.in_app {
            let ask = if keep { Preview::Keep } else { Preview::Cancel };
            self.effects.push(Effect::Send(ask.osc()));
        }
    }

    fn number(&mut self, mode: Mode, event: Event, now: Instant) {
        let Mode::Number {
            path,
            mut value,
            mut typed,
            session,
            min,
            max,
            step,
            integer,
        } = mode
        else {
            unreachable!()
        };
        let set = |v: f64| Edit::Set {
            path: path.clone(),
            value: number_value(v, integer),
        };
        match event {
            Event::Key(Key::Left | Key::Right, _) => {
                let dir = if matches!(event, Event::Key(Key::Right, _)) { 1.0 } else { -1.0 };
                let next = ((value + dir * step) / step).round() * step;
                let next = (next * 10_000.0).round() / 10_000.0;
                let next = next.clamp(min, max);
                typed = None;
                if next != value {
                    value = next;
                    self.write_soon(set(value), session, now);
                }
            }
            Event::Key(Key::Char(c), _) if c.is_ascii_digit() || c == '.' || c == '-' => {
                typed.get_or_insert_with(String::new).push(c);
            }
            Event::Key(Key::Backspace, _) => {
                if let Some(t) = &mut typed {
                    t.pop();
                }
            }
            Event::Key(Key::Enter, _) => {
                if let Some(t) = typed.take() {
                    match t.parse::<f64>() {
                        Ok(v) if v.is_finite() && (min..=max).contains(&v) && (!integer || v.fract() == 0.0) => {
                            self.write(set(v), Some(session), now);
                        }
                        _ => {
                            self.fail(format!(
                                "{t:?} isn't {} from {} to {}",
                                if integer { "a whole number" } else { "a number" },
                                format_number(min),
                                format_number(max)
                            ));
                            self.mode = Mode::Number {
                                path,
                                value,
                                typed: Some(t),
                                session,
                                min,
                                max,
                                step,
                                integer,
                            };
                            return;
                        }
                    }
                }
                self.flush(now);
                return;
            }
            Event::Key(Key::Esc, _) => {
                // Cancel: put back what was there before this edit began.
                self.pending = None;
                if self.undo.last().is_some_and(|(_, s)| *s == Some(session)) {
                    let (edit, _) = self.undo.pop().expect("checked");
                    let depth = self.undo.len();
                    self.write(edit, None, now);
                    self.undo.truncate(depth);
                }
                self.say("cancelled");
                return;
            }
            _ => {}
        }
        self.mode = Mode::Number {
            path,
            value,
            typed,
            session,
            min,
            max,
            step,
            integer,
        };
    }

    fn text(&mut self, mode: Mode, event: Event, now: Instant) {
        let Mode::Text {
            title,
            mut buf,
            mut cursor,
            kind,
            then,
        } = mode
        else {
            unreachable!()
        };
        let byte = |buf: &str, c: usize| buf.char_indices().nth(c).map_or(buf.len(), |(i, _)| i);
        match event {
            Event::Key(Key::Esc, _) => {
                self.say("cancelled");
                if let Then::ListItem(path, i) = then {
                    self.mode = Mode::List {
                        fonts: false,
                        sel: i.unwrap_or(0),
                        path,
                    };
                }
                return;
            }
            Event::Key(Key::Enter, _) => {
                let value = buf.trim().to_string();
                let bad = match kind {
                    TextKind::Colour if !value.is_empty() && schema::parse_hex(&value).is_none() => {
                        Some(format!("{value:?} is not a colour: use #rrggbb, or nothing for the theme's"))
                    }
                    TextKind::Width if !value.is_empty() && !schema::valid_width(&value) => {
                        Some(format!("{value:?} is not a width: use points (\"1.5\") or a percentage (\"10%\")"))
                    }
                    _ => None,
                };
                if let Some(bad) = bad {
                    self.fail(bad);
                } else {
                    let value = if kind == TextKind::Colour && !value.is_empty() && !value.starts_with('#') {
                        format!("#{value}")
                    } else {
                        value
                    };
                    if kind == TextKind::Chord {
                        self.status = None;
                        return self.chord_entered(then, &value, now);
                    }
                    return self.committed(then, value, now);
                }
            }
            Event::Key(Key::Backspace, _) if cursor > 0 => {
                let at = byte(&buf, cursor - 1);
                buf.remove(at);
                cursor -= 1;
            }
            Event::Key(Key::Delete, _) if cursor < buf.chars().count() => {
                let at = byte(&buf, cursor);
                buf.remove(at);
            }
            Event::Key(Key::Left, _) => cursor = cursor.saturating_sub(1),
            Event::Key(Key::Right, _) => cursor = (cursor + 1).min(buf.chars().count()),
            Event::Key(Key::Home, _) => cursor = 0,
            Event::Key(Key::Char('a'), m) if m.ctrl => cursor = 0,
            Event::Key(Key::End, _) => cursor = buf.chars().count(),
            Event::Key(Key::Char('e'), m) if m.ctrl => cursor = buf.chars().count(),
            Event::Key(Key::Char('u'), m) if m.ctrl => {
                buf.clear();
                cursor = 0;
            }
            Event::Key(Key::Char(c), m) if !m.ctrl && !m.alt && !m.cmd => {
                let at = byte(&buf, cursor);
                buf.insert(at, c);
                cursor += 1;
            }
            Event::Paste(t) => {
                let t: String = t.chars().filter(|c| !c.is_control()).collect();
                let at = byte(&buf, cursor);
                buf.insert_str(at, &t);
                cursor += t.chars().count();
            }
            _ => {}
        }
        self.mode = Mode::Text {
            title,
            buf,
            cursor,
            kind,
            then,
        };
    }

    fn list(&mut self, mode: Mode, event: Event, now: Instant) {
        let Mode::List { path, fonts, mut sel } = mode else {
            unreachable!()
        };
        let items = self.list_items(&path);
        let n = items.len();
        let Event::Key(key, m) = event else {
            self.mode = Mode::List { path, fonts, sel };
            return;
        };
        match key {
            Key::Esc | Key::Char('q') => {
                self.status = None;
                return;
            }
            Key::Up if m.shift => {
                if sel > 0 {
                    let edit = self.list_edit(&path, ListOp::Move(sel, sel - 1));
                    if self.commit(edit, now) {
                        sel -= 1;
                    }
                }
            }
            Key::Down if m.shift => {
                if sel + 1 < n {
                    let edit = self.list_edit(&path, ListOp::Move(sel, sel + 1));
                    if self.commit(edit, now) {
                        sel += 1;
                    }
                }
            }
            Key::Char('K') => {
                return self.list(
                    Mode::List { path, fonts, sel },
                    Event::Key(Key::Up, Mods { shift: true, ..Mods::NONE }),
                    now,
                );
            }
            Key::Char('J') => {
                return self.list(
                    Mode::List { path, fonts, sel },
                    Event::Key(Key::Down, Mods { shift: true, ..Mods::NONE }),
                    now,
                );
            }
            Key::Up | Key::Char('k') => sel = sel.saturating_sub(1),
            Key::Down | Key::Char('j') => sel = (sel + 1).min(n.saturating_sub(1)),
            Key::Char('d') | Key::Delete | Key::Backspace if n > 0 => {
                let edit = self.list_edit(&path, ListOp::Remove(sel));
                if self.commit(edit, now) {
                    self.say(format!("removed {:?} · u undoes", items[sel]));
                    sel = sel.min(n.saturating_sub(2));
                }
            }
            Key::Char('a') => {
                if fonts {
                    self.refresh_fonts();
                    let items = self.font_items();
                    let first = items.first().map(|(v, _)| v.clone()).unwrap_or_default();
                    self.choose("Add a fallback font", items, "", Then::ListItem(path, None), true);
                    self.offer_sample(&first, now);
                } else {
                    self.edit_text("Add", String::new(), TextKind::Plain, Then::ListItem(path, None));
                }
                return;
            }
            Key::Enter if n > 0 => {
                self.edit_text("Change", items[sel].clone(), TextKind::Plain, Then::ListItem(path, Some(sel)));
                return;
            }
            _ => {}
        }
        self.mode = Mode::List { path, fonts, sel };
    }

    fn mouse(&mut self, kind: MouseKind, x: usize, y: usize, now: Instant) {
        let hit = self.hit;
        match kind {
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let key = if kind == MouseKind::ScrollUp { Key::Up } else { Key::Down };
                match self.mode {
                    Mode::Browse | Mode::Choice { .. } | Mode::List { .. } => self.handle(Event::Key(key, Mods::NONE), now),
                    _ => {}
                }
            }
            MouseKind::Press(0) => {
                if let Some((x0, x1, y0, first, count)) = hit.popup {
                    if (x0..x1).contains(&x) && y >= y0 && y < y0 + count {
                        // A header row isn't an item.
                        let i = match self.hit_rows.get(y - y0) {
                            Some(Some(i)) => *i,
                            Some(None) => return,
                            None => first + (y - y0),
                        };
                        match &mut self.mode {
                            Mode::Choice { sel, .. } | Mode::List { sel, .. } => {
                                let again = *sel == i;
                                *sel = i;
                                if again {
                                    self.handle(Event::Key(Key::Enter, Mods::NONE), now);
                                } else {
                                    self.choice_moved(now);
                                }
                            }
                            _ => {}
                        }
                    }
                    return;
                }
                if !matches!(self.mode, Mode::Browse) {
                    return;
                }
                if let Some((x0, x1, y0)) = hit.sections
                    && (x0..x1).contains(&x)
                    && y >= y0
                    && y < y0 + self.sections().len()
                {
                    self.search = None;
                    self.section = y - y0;
                    self.row = 0;
                    self.scroll = 0;
                    return;
                }
                let (x0, x1, y0, first, count) = hit.rows;
                if (x0..x1).contains(&x) && y >= y0 && y < y0 + count {
                    let i = first + (y - y0);
                    if self.rows().get(i).is_some_and(Row::selectable) {
                        if self.row == i {
                            self.activate(now);
                        } else {
                            self.row = i;
                            self.status = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Text to send as it is typed: controls as `\e`, `\n`, `\r`, `\t` or
/// `\u001b`, and `\` doubled ([`unescape`] reads it back).
fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\x1b' => out.push_str("\\e"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// `\u001b`, `\e`, `\n`, `\t` and `\\` in typed text.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('e') => out.push('\x1b'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => {
                        out.push_str("\\u");
                        out.push_str(&hex);
                    }
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_escapes() {
        assert_eq!(format_number(13.0), "13");
        assert_eq!(format_number(0.05), "0.05");
        assert_eq!(number_value(14.0, false).to_string(), "14");
        assert_eq!(number_value(1.25, false).to_string(), "1.25");
        assert_eq!(unescape("\\u001b[1;5A\\e"), "\x1b[1;5A\x1b");
        for text in ["\x1b[116;5u", "a\nb\\c\t", "\x07bell"] {
            assert_eq!(unescape(&escape(text)), text, "{text:?}");
        }
        assert_eq!(escape("\x1b[A\n"), "\\e[A\\n");
        assert!(luminance(0xffffff) > 0.9 && luminance(0x16181c) < 0.1);
    }
}
