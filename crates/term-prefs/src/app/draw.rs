//! Each frame of the preferences: a header, the sections and the settings
//! side by side, a detail pane for the selected setting, and a line of key
//! hints; one column of settings on small screens.
//!
//! Colours come from the terminal's own: the default foreground and
//! background, faint (SGR 2) for what is secondary, and one of the
//! theme's ANSI colours as the accent (the first of blue, bright blue,
//! cyan, … that is readable on the background). So the tab follows the
//! theme, a preview and a theme transition with every other window. Only
//! if the theme's own foreground is unreadable on its background does it
//! draw in colours it picks itself.

use super::*;
use crate::ui::frame::{Color, Image, Style, truncate, wrap2};

/// The font list's sample: a rule, the font's name, two rows of picture.
const SAMPLE_ROWS: usize = 4;

/// The font list's header over the fonts that aren't monospaced.
const NOT_MONOSPACED: &str = "Not monospaced (uneven in a terminal)";

/// The ANSI colours tried for the accent, calmest first.
const ACCENTS: [u8; 7] = [4, 12, 6, 14, 5, 13, 2];

/// WCAG's contrast ratio of two 0xRRGGBB colours (1 to 21).
pub(super) fn contrast(a: u32, b: u32) -> f64 {
    let lum = |rgb: u32| {
        let c = |s: u32| {
            let v = ((rgb >> s) & 0xff) as f64 / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * c(16) + 0.7152 * c(8) + 0.0722 * c(0)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The narrowest description column worth showing.
const MIN_DESCRIPTION: usize = 24;

/// Where a list's columns start: the label after the `▸ ` marker, the
/// value, and the description if there is room for it beside the value
/// and the note (x and width).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Columns {
    pub label: usize,
    pub value: usize,
    pub desc: Option<(usize, usize)>,
}

/// The columns of a list drawn from `x`, `w` cells wide, whose longest
/// label, value and note are that wide. Labels get what they need, up to
/// about half the width (a long one is cut with `…`); values what they
/// need, up to a third of the rest, before a description is shown in
/// what is left once the note has its room. Narrow, there is no
/// description column (the detail line under the list still has it).
pub(super) fn columns(x: usize, w: usize, label_max: usize, value_max: usize, note_max: usize) -> Columns {
    let label = x + 3;
    // The marker before, one cell of margin after.
    let inner = w.saturating_sub(4);
    let cap = (inner * 45 / 100).max(w.min(24) / 2);
    let label_w = (label_max + 2).min(cap);
    let value = label + label_w;
    let rest = (x + w).saturating_sub(value + 1);
    let note_w = if note_max > 0 { note_max + 3 } else { 0 };
    let value_w = (value_max + 3).min((rest / 3).max(16));
    let desc = (rest >= value_w + note_w + MIN_DESCRIPTION).then(|| (value + value_w, rest - value_w - note_w));
    Columns { label, value, desc }
}

/// The styles a frame is drawn with.
#[derive(Clone, Copy, Debug)]
pub(super) struct Look {
    pub base: Style,
    /// Secondary text, borders and hints.
    pub dim: Style,
    /// The accent: titles, the current section, prompts, keys.
    pub accent: Style,
    /// The selection bar: the accent behind the background's colour.
    pub sel: Style,
    pub error: Style,
}

impl Look {
    /// For a terminal whose default colours are `fg` on `bg` with ANSI
    /// colours `palette` (each unknown when `None` or empty).
    pub fn new(fg: Option<u32>, bg: Option<u32>, palette: &[u32]) -> Look {
        let mut base = Style::default();
        if let Some(bg) = bg {
            let readable = fg.is_some_and(|fg| contrast(fg, bg) >= 3.0);
            if !readable {
                // foreground = background, say: pick our own.
                let fg = if luminance(bg) < 0.5 { 0xe8e8e8 } else { 0x1a1a1a };
                base = base.fg(Color::Rgb(fg)).bg(Color::Rgb(bg));
            }
        }
        let pick = |want: f64| {
            ACCENTS
                .iter()
                .find(|&&i| match (palette.get(i as usize), bg) {
                    (Some(&c), Some(bg)) => contrast(c, bg) >= want,
                    _ => palette.is_empty(),
                })
                .map(|&i| Color::Ansi(i))
        };
        let accent = pick(3.0).or_else(|| pick(2.0)).unwrap_or(base.fg);
        let red = match (palette.get(1), bg) {
            (Some(&c), Some(bg)) if contrast(c, bg) < 3.0 => base.fg,
            _ => Color::Ansi(1),
        };
        Look {
            base,
            dim: base.dim(),
            accent: base.fg(accent),
            sel: Style {
                fg: accent,
                bg: base.bg,
                reverse: true,
                ..Style::default()
            },
            error: base.fg(red).bold(),
        }
    }
}

impl App<'_> {
    /// The look for the colours windows show now: a previewed theme's
    /// while a theme list is open, with `[colors]` on top as windows show
    /// it.
    fn look(&self) -> Look {
        let c = self.previewed.as_ref().map_or(&self.checked, |(_, c)| c);
        Look::new(c.foreground, c.background, &c.palette)
    }

    /// The key hints for the mode: key and what it does.
    fn hints(&self) -> &'static [(&'static str, &'static str)] {
        match &self.mode {
            Mode::Browse if self.search.is_some() => &[("type", "filter"), ("↑↓", "move"), ("enter", "edit"), ("esc", "done")],
            Mode::Browse if matches!(self.selected().map(|r| r.kind), Some(RowKind::Action(_))) => &[
                ("↑↓", "move"),
                ("enter", "record"),
                ("a", "add"),
                ("⌫", "clear"),
                ("r", "default"),
                ("/", "search"),
                ("?", "help"),
                ("q", "quit"),
            ],
            Mode::Browse => &[
                ("↑↓", "move"),
                ("←→", "section"),
                ("enter", "edit"),
                ("/", "search"),
                ("r", "reset"),
                ("u", "undo"),
                ("?", "help"),
                ("q", "quit"),
            ],
            Mode::Help => &[("any key", "closes")],
            // Theme lists preview what is under the selection in every
            // window; font lists show a sample, and change nothing until
            // Enter.
            Mode::Choice {
                then: Then::ThemeSide(_), ..
            } => &[("↑↓", "preview"), ("type", "filter"), ("enter", "keep"), ("esc", "put back")],
            Mode::Choice { fonts: true, .. } => &[("↑↓", "sample"), ("type", "filter"), ("enter", "use it"), ("esc", "cancel")],
            Mode::Choice { .. } => &[("type", "filter"), ("↑↓", "move"), ("enter", "pick"), ("esc", "cancel")],
            Mode::Number { .. } => &[("←→", "step"), ("0-9", "type"), ("enter", "done"), ("esc", "cancel")],
            Mode::Text { .. } => &[("enter", "save"), ("esc", "cancel")],
            Mode::List { .. } => &[
                ("↑↓", "move"),
                ("a", "add"),
                ("enter", "change"),
                ("d", "remove"),
                ("shift+↑↓", "reorder"),
                ("esc", "done"),
            ],
            Mode::Recording(Then::ActionChord(..)) => &[("press", "the shortcut"), ("⌫", "clear"), ("t", "type it"), ("esc", "cancel")],
            Mode::Recording(_) => &[("press", "the shortcut"), ("t", "type it"), ("esc", "cancel")],
            Mode::Confirm(_, Confirmed::Clash { swap: true, .. }) => &[("s", "swap"), ("r", "replace"), ("esc", "cancel")],
            Mode::Confirm(_, Confirmed::Clash { .. }) => &[("r", "replace"), ("esc", "cancel")],
            Mode::Confirm(..) => &[("y", "yes"), ("n", "no")],
        }
    }

    /// The key hints at `y`, keys in the accent. When they don't all fit,
    /// those before the last two (help and quit, the ones to keep) go
    /// first, from the end.
    fn draw_hints(&self, f: &mut Frame, y: usize, look: &Look) {
        let w = f.width;
        let mut hints: Vec<(&str, &str)> = self.hints().to_vec();
        let width = |hs: &[(&str, &str)]| -> usize { hs.iter().map(|(k, v)| str_width(k) + 1 + str_width(v) + 2).sum::<usize>() };
        let keep = if hints.last().is_some_and(|(k, _)| *k == "q") { 2 } else { 0 };
        while hints.len() > 1 && width(&hints) > w {
            let i = hints.len().saturating_sub(keep + 1);
            hints.remove(i);
        }
        let mut x = 1;
        for (i, (key, what)) in hints.iter().enumerate() {
            if i > 0 {
                x = f.text(x, y, w, "  ", look.dim);
            }
            x = f.text(x, y, w, key, look.accent);
            x = f.text(x, y, w, " ", look.dim);
            x = f.text(x, y, w, what, look.dim);
        }
    }

    /// The line under the settings: a question, the status, the file's
    /// first error, or the selected row's description; and whether it is
    /// an error.
    fn message(&self, rows: &[Row]) -> (String, bool) {
        match (&self.mode, &self.status) {
            (Mode::Confirm(q, _), _) => (q.clone(), false),
            (_, Some(s)) => (s.text.clone(), s.error),
            _ if !self.checked.notices.is_empty() => {
                let n = &self.checked.notices;
                let more = if n.len() > 1 {
                    format!(" (+{} more)", n.len() - 1)
                } else {
                    String::new()
                };
                (format!("error: {}{more}", n[0].strip_prefix("config: ").unwrap_or(&n[0])), true)
            }
            _ => (rows.get(self.row).map(|r| r.description.clone()).unwrap_or_default(), false),
        }
    }

    /// The banner for a file that can't be edited.
    fn banner(&self) -> Option<String> {
        if let Some(e) = &self.layers.error {
            // toml_edit's first line names the line and column.
            let at = e.lines().next().unwrap_or("").trim_start_matches("TOML parse error at ");
            Some(format!(
                "error: {} doesn't parse ({at}): read-only · e fixes it",
                self.layers.edited_name()
            ))
        } else if self.layers.read_only {
            Some(format!("{} is read-only: changes can't be saved", self.layers.edited_name()))
        } else {
            None
        }
    }

    /// What the selected row's value line says beside its value: where it
    /// comes from, and what a reset would give.
    fn provenance(&self, row: &Row) -> String {
        let mut out = row.note.clone();
        if let RowKind::Setting(s) = &row.kind {
            let path = s.path();
            let (_, origin) = self.layers.lookup(&path);
            if matches!(origin, Origin::Here | Origin::Overridden(_)) {
                let below = self.layers.below(&path).map(format_item);
                let below = match below {
                    Some(v) if !v.is_empty() => v,
                    _ if !s.unset.is_empty() => s.unset.to_string(),
                    _ => "unset".into(),
                };
                let what = if self.layers.edit == 0 { "default" } else { "reset gives" };
                if !out.is_empty() {
                    out.push_str(" · ");
                }
                out.push_str(&format!("{what} {below}"));
            }
        }
        out
    }

    /// Draws the frame for the current state at the current size.
    pub fn draw(&mut self) -> Frame {
        let (w, h) = (self.width, self.height);
        let look = self.look();
        let mut f = Frame::new(w, h, look.base);
        self.hit = Hit::default();
        if w < 12 || h < 4 {
            f.text(0, 0, w, "too small", look.base);
            return f;
        }
        let rows = self.rows();
        self.row = Self::selectable_near(&rows, self.row.min(rows.len().saturating_sub(1)), 1);
        self.draw_hints(&mut f, h - 1, &look);
        let (line, error) = self.message(&rows);
        let roomy = w >= 60 && h >= 12;
        let sections = self.sections();
        let mut top = 0;
        if roomy {
            // The header: what this is and which file it edits.
            let end = f.text(1, 0, w, "Settings", look.accent.bold());
            let end = f.text(end, 0, w, "  ", look.dim);
            let mut file = self.layers.edited_name();
            if self.pin_file {
                file.push_str(" (a pin's own settings)");
            }
            f.text(end, 0, w.saturating_sub(1), &file, look.dim);
            let n = self.checked.notices.len();
            if n > 0 {
                let t = if n == 1 { "1 problem".to_string() } else { format!("{n} problems") };
                let x = w.saturating_sub(str_width(&t) + 1);
                f.text(x, 0, w, &t, look.error);
            }
            top = 1;
        }
        if let Some(b) = self.banner() {
            f.text(1, top, w, &truncate(&b, w.saturating_sub(2)), look.error);
            top += 1;
        }
        if roomy {
            // The detail pane's lines, with the rule over them.
            let detail_h = if h >= 20 { 4 } else { 3 };
            let main_h = (h - 1).saturating_sub(detail_h + top);
            if main_h < 3 {
                return f;
            }
            // One card: the sections beside the settings, and the selected
            // setting under them.
            let sw = 16;
            let bottom = top + main_h + detail_h - 1;
            f.boxed(0, top, w, main_h + detail_h, "", look.dim, look.accent);
            let mid = top + main_h - 1;
            f.text(0, mid, 1, "├", look.dim);
            f.rule(1, mid, w - 1, look.dim);
            f.text(w - 1, mid, w, "┤", look.dim);
            f.text(sw, top, sw + 1, "┬", look.dim);
            for y in top + 1..mid {
                f.text(sw, y, sw + 1, "│", look.dim);
            }
            f.text(sw, mid, sw + 1, "┴", look.dim);
            let _ = bottom;
            for (i, s) in sections.iter().enumerate() {
                let y = top + 1 + i;
                if y >= mid {
                    break;
                }
                let current = i == self.section && self.search.is_none();
                if current {
                    f.text(2, y, sw, "▸ ", look.accent);
                    f.text(4, y, sw, s.name(), look.accent.bold());
                } else {
                    f.text(4, y, sw, s.name(), look.base);
                }
            }
            self.hit.sections = Some((1, sw, top + 1));
            // Settings, titled with their section.
            let rx = sw + 1;
            let rend = w - 1;
            let title = match &self.search {
                Some(_) => "Search".to_string(),
                None => self.current_section().name().to_string(),
            };
            let tx = f.text(rx + 1, top, rend, " ", look.dim);
            let tx = f.text(tx, top, rend, &title, look.accent.bold());
            f.text(tx, top, rend, " ", look.dim);
            let mut y0 = top + 1;
            if let Some(s) = &self.search {
                self.draw_search(&mut f, rx + 2, y0, rend - 1, &s.query, rows.len(), &look);
                y0 += 1;
            }
            let visible = mid.saturating_sub(y0);
            self.draw_rows(&mut f, rx, y0, rend - rx, visible, &rows, &look);
            // The selected setting: its name, value and where it comes
            // from; its description (or the status); the sample.
            let dy = mid + 1;
            let end = w - 2;
            if let Some(r) = rows.get(self.row) {
                let x = f.text(2, dy, end, r.label.trim_start(), look.base.bold());
                let mut x = x + 2;
                if let Some(rgb) = r.swatch
                    && x + 3 < end
                {
                    f.text(x, dy, x + 2, "  ", Style::default().bg(Color::Rgb(rgb)));
                    x += 3;
                }
                let x = f.text(x, dy, end, &truncate(&r.value, end.saturating_sub(x)), look.accent);
                let p = self.provenance(r);
                if !p.is_empty() && x + 6 < end {
                    let x = f.text(x, dy, end, "  ·  ", look.dim);
                    f.text(x, dy, end, &truncate(&p, end - x), look.dim);
                }
            }
            let style = if error { look.error } else { look.base };
            // The colour sample where colours are set; elsewhere a long
            // description takes its line too.
            let colours = self.search.is_none() && self.current_section() == Section::Appearance
                || rows.get(self.row).is_some_and(|r| r.swatch.is_some());
            if detail_h == 4 && !colours {
                let (one, two) = wrap2(&line, end - 2);
                f.text(2, dy + 1, end, &one, style);
                f.text(2, dy + 2, end, &two, style);
            } else {
                f.text(2, dy + 1, end, &truncate(&line, end - 2), style);
                if detail_h == 4 {
                    self.draw_sample(&mut f, 2, dy + 2, end, &look);
                }
            }
        } else {
            // One column: the section, its settings, the message.
            let end = match &self.search {
                Some(s) => {
                    let q = s.query.clone();
                    self.draw_search(&mut f, 1, top, w - 1, &q, rows.len(), &look);
                    w
                }
                None => {
                    let x = f.text(1, top, w, self.current_section().name(), look.accent.bold());
                    f.text(x + 1, top, w, &format!("{}/{}", self.section + 1, sections.len()), look.dim)
                }
            };
            if self.pin_file && end + 3 < w {
                f.text(end + 2, top, w, &self.layers.edited_name(), look.dim);
            }
            let y0 = top + 1;
            let visible = (h - 2).saturating_sub(y0);
            self.draw_rows(&mut f, 0, y0, w, visible, &rows, &look);
            let style = if error { look.error } else { look.dim };
            f.text(1, h - 2, w, &truncate(&line, w.saturating_sub(2)), style);
        }
        self.draw_popups(&mut f, &look);
        f
    }

    /// The search field: `/`, the query and how many settings match.
    #[allow(clippy::too_many_arguments)]
    fn draw_search(&self, f: &mut Frame, x: usize, y: usize, end: usize, query: &str, found: usize, look: &Look) {
        let x = f.text(x, y, end, "/ ", look.accent.bold());
        let x = f.text(x, y, end, query, look.base);
        let n = match found {
            0 => "  nothing matches".to_string(),
            1 => "  1 setting".to_string(),
            n => format!("  {n} settings"),
        };
        f.text(x, y, end, &n, look.dim);
    }

    /// Sample text in the default colours and the 16 ANSI colours: the
    /// live preview of the colours set.
    fn draw_sample(&self, f: &mut Frame, x: usize, y: usize, end: usize, look: &Look) {
        let x = f.text(x, y, end, "Sample text ", look.base);
        let x = f.text(x, y, end, "dim ", look.dim);
        let x = f.text(x, y, end, "bold", look.base.bold());
        let mut x = x + 2;
        for i in 0..16u8 {
            if x + 2 > end {
                break;
            }
            f.text(x, y, x + 2, "  ", Style::default().bg(Color::Ansi(i)));
            x += 2;
        }
    }

    /// The rows from `y`, `visible` of them, scrolled to the selection:
    /// the marker, label, value (or the editor in it), the description
    /// when there is room for it, and where the value comes from. The
    /// columns are as wide as the list's longest label and value need,
    /// within the width ([`columns`]); a group's header spans the row.
    #[allow(clippy::too_many_arguments)]
    fn draw_rows(&mut self, f: &mut Frame, x: usize, y: usize, w: usize, visible: usize, rows: &[Row], look: &Look) {
        if visible == 0 {
            return;
        }
        if self.row < self.scroll {
            self.scroll = self.row;
        }
        if self.row >= self.scroll + visible {
            self.scroll = self.row + 1 - visible;
        }
        // A header just above the selection stays in view with it.
        if visible > 1 && self.scroll > 0 && self.scroll == self.row && rows.get(self.row - 1).is_some_and(|r| !r.selectable()) {
            self.scroll -= 1;
        }
        self.hit.rows = (x, x + w, y, self.scroll, visible.min(rows.len().saturating_sub(self.scroll)));
        // Rows with no value ("+ add a variable") don't widen the labels:
        // theirs run on into the value's room.
        let widest = |what: fn(&Row) -> &str| {
            rows.iter()
                .filter(|r| r.selectable() && !r.value.is_empty())
                .map(|r| str_width(what(r)))
                .max()
                .unwrap_or(0)
        };
        let cols = columns(
            x,
            w,
            widest(|r| r.label.as_str()),
            widest(|r| r.value.as_str()),
            widest(|r| r.note.as_str()),
        );
        let end = x + w;
        let mut cursor = None;
        for (i, r) in rows.iter().enumerate().skip(self.scroll).take(visible) {
            let yy = y + i - self.scroll;
            if !r.selectable() {
                f.text(cols.label.saturating_sub(1), yy, end, &r.label, look.accent);
                continue;
            }
            let selected = i == self.row;
            let style = if selected { look.sel } else { look.base };
            let faint = if selected { look.sel } else { look.dim };
            if selected {
                f.fill(x, yy, w, 1, style);
            }
            f.text(x + 1, yy, end, if selected { "▸ " } else { "  " }, style);
            let vx = cols.value;
            let editing = selected && matches!(self.mode, Mode::Text { .. } | Mode::Number { .. } | Mode::Recording(_));
            let label_end = if r.value.is_empty() && !editing {
                end.saturating_sub(1)
            } else {
                vx.min(end)
            };
            let label_after = f.text(
                cols.label,
                yy,
                label_end,
                &truncate(&r.label, label_end.saturating_sub(cols.label + 1)),
                style,
            );
            let mut value = r.value.clone();
            let mut caret = None;
            if selected {
                match &self.mode {
                    Mode::Number {
                        value: v, typed, min, max, ..
                    } => {
                        value = match typed {
                            Some(t) => format!("{t}  ({}–{})", format_number(*min), format_number(*max)),
                            None => format!("‹ {} ›  ({}–{})", format_number(*v), format_number(*min), format_number(*max)),
                        };
                        caret = Some(match typed {
                            Some(t) => str_width(t),
                            None => 2,
                        });
                    }
                    Mode::Text { buf, cursor: c, then, .. } if !matches!(then, Then::ListItem(..)) => {
                        value = buf.clone();
                        let before: String = buf.chars().take(*c).collect();
                        caret = Some(str_width(&before));
                    }
                    Mode::Recording(_) => value = "press the shortcut…".into(),
                    _ => {}
                }
            }
            let mut vend = vx;
            if let Some(rgb) = r.swatch
                && caret.is_none()
                && vx + 3 < end
            {
                f.text(vx, yy, vx + 2, "  ", Style::default().bg(Color::Rgb(rgb)));
                vend = vx + 3;
            }
            // Editing, the value takes the description's room too.
            let desc = cols
                .desc
                .filter(|_| caret.is_none() && !matches!((&self.mode, selected), (Mode::Recording(_), true)));
            let value_end = match desc {
                Some((dx, _)) => dx.saturating_sub(2),
                None => end.saturating_sub(1),
            };
            let after = f.text(vend, yy, value_end, &truncate(&value, value_end.saturating_sub(vend)), style);
            // A label that runs on into the value's room ("+ add a key
            // that sends text") pushes its description along.
            if let Some((dx, dw)) = desc
                && !r.description.is_empty()
            {
                let x0 = dx.max(label_after.max(after) + 2);
                let room = (dx + dw).saturating_sub(x0);
                if room >= MIN_DESCRIPTION / 2 {
                    f.text(x0, yy, dx + dw, &truncate(&r.description, room), faint);
                }
            }
            let note_w = str_width(&r.note);
            let note_x = end.saturating_sub(note_w + 1);
            let clear = match desc {
                Some((dx, dw)) => note_x >= dx + dw,
                None => after + 2 <= note_x,
            };
            if caret.is_none() && !r.note.is_empty() && clear {
                f.text(note_x, yy, end, &r.note, faint);
            }
            if selected {
                cursor = Some((
                    match caret {
                        Some(c) => (vend + c).min(end - 1),
                        None => vend.min(end - 1),
                    },
                    yy,
                ));
            }
        }
        f.cursor = cursor;
    }

    fn draw_popups(&mut self, f: &mut Frame, look: &Look) {
        match &self.mode.clone() {
            Mode::Help => self.draw_help(f, look),
            Mode::Choice {
                title,
                items,
                filter,
                sel,
                then,
                fonts,
            } => {
                let fl = filter.to_lowercase();
                let shown: Vec<&(String, String)> = items
                    .iter()
                    .filter(|(v, l)| fl.is_empty() || l.to_lowercase().contains(&fl) || v.to_lowercase().contains(&fl))
                    .collect();
                let labels: Vec<String> = shown.iter().map(|(_, l)| l.clone()).collect();
                let swatches: Vec<Option<ThemeInfo>> = if matches!(then, Then::ThemeSide(_)) {
                    shown.iter().map(|(v, _)| self.theme_info(v)).collect()
                } else {
                    Vec::new()
                };
                // Theme lists: a faint header where the background turns
                // from dark to light (or back), for the groups that have a
                // match.
                let mut groups: Vec<(usize, &str)> = Vec::new();
                // Font lists: monospaced families, then the others, which
                // draw unevenly in a terminal.
                if matches!(then, Then::Font(_) | Then::ListItem(..)) {
                    let mut last = None;
                    for (k, (v, _)) in shown.iter().enumerate() {
                        let Some(mono) = self.is_monospace(v) else { continue };
                        if last != Some(mono) {
                            groups.push((k, if mono { "Monospaced" } else { NOT_MONOSPACED }));
                            last = Some(mono);
                        }
                    }
                }
                if matches!(then, Then::ThemeSide(_)) {
                    let mut last = None;
                    for (k, (v, _)) in shown.iter().enumerate() {
                        let Some(info) = self.theme_info(v) else { continue };
                        let dark = info.is_dark();
                        if last != Some(dark) {
                            groups.push((k, if dark { "Dark" } else { "Light" }));
                            last = Some(dark);
                        }
                    }
                }
                let footer = if *fonts { SAMPLE_ROWS } else { 0 };
                let (cx, cy, at) = self.popup(f, title, Some(filter), &labels, &swatches, &groups, *sel, footer, look);
                f.cursor = Some((cx + str_width(filter), cy));
                if let Some((x, y, w)) = at {
                    self.draw_sample_footer(f, x, y, w, look);
                }
            }
            Mode::List { path, sel, .. } => {
                let items = self.list_items(path);
                let title = schema::get(&path.join(".")).map_or("List", |s| s.label);
                let labels = if items.is_empty() {
                    vec!["(empty: a adds)".to_string()]
                } else {
                    items
                };
                self.popup(f, title, None, &labels, &[], &[], *sel, 0, look);
            }
            Mode::Text {
                title,
                buf,
                cursor: c,
                then: Then::ListItem(..),
                ..
            } => {
                let before: String = buf.chars().take(*c).collect();
                let (cx, cy, _) = self.popup(f, title, Some(buf), &[], &[], &[], 0, 0, look);
                f.cursor = Some((cx + str_width(&before), cy));
            }
            _ => {}
        }
    }

    /// The font list's sample, in the `w` cells from `(x, y)` at the
    /// bottom of the list: a rule, the font's name (and whether it is
    /// monospaced), then the picture of [`SAMPLE_TEXT`] in it once the
    /// selection has rested ([`SAMPLE_AFTER`]). Pictures go only to
    /// Terminal Decline, which shows them (kitty graphics).
    fn draw_sample_footer(&mut self, f: &mut Frame, x: usize, y: usize, w: usize, look: &Look) {
        f.rule(x, y, x + w, look.dim);
        let end = x + w;
        let Some(name) = self.sample.clone() else {
            let t = if self.sample_pace.is_held() { "…" } else { "" };
            f.text(x + 1, y + 1, end, t, look.dim);
            return;
        };
        let nx = f.text(x + 1, y + 1, end, &truncate(&name, w.saturating_sub(2)), look.base.bold());
        let (note, style) = match self.font_is_monospace(&name) {
            Some(true) => ("  monospaced", look.dim),
            Some(false) => ("  not monospaced: letters won't line up in a terminal", look.error),
            None => ("  not installed", look.error),
        };
        f.text(nx, y + 1, end, &truncate(note, end.saturating_sub(nx + 1)), style);
        let (cols, rows) = (w.saturating_sub(2), SAMPLE_ROWS - 2);
        if !self.in_app || cols < 4 {
            return;
        }
        let c = self.previewed.as_ref().map_or(&self.checked, |(_, c)| c);
        let (fg, bg) = (c.foreground.unwrap_or(0xd8dee9), c.background.unwrap_or(0x16181c));
        match self.sample_image(&name, cols, rows, fg, bg) {
            Some((key, width, height, rgb)) => {
                f.image = Some(Image {
                    key,
                    x: x + 1,
                    y: y + 2,
                    cols,
                    rows,
                    width,
                    height,
                    rgb,
                });
            }
            None => {
                f.text(x + 1, y + 2, end, "no sample for this font", look.dim);
            }
        }
    }

    /// The picture of [`SAMPLE_TEXT`] in font `name`, `cols` × `rows`
    /// cells in `fg` on `bg`, drawn by the host at the cells' size in
    /// pixels (a guess if the terminal doesn't say), and kept for the
    /// next frames: its key, size and pixels.
    fn sample_image(&self, name: &str, cols: usize, rows: usize, fg: u32, bg: u32) -> Option<(u64, u32, u32, Rc<Vec<u8>>)> {
        let (cw, ch) = match self.cell_px {
            (w, h) if w > 0 && h > 0 => (w, h),
            _ => (8, 16),
        };
        let (mut width, mut height) = (cols as u32 * cw, rows as u32 * ch);
        // Kept small: about a megapixel at most (3 MB of RGB).
        let scale = ((width as f64 * height as f64) / 1_000_000.0).sqrt();
        if scale > 1.0 {
            width = (width as f64 / scale) as u32;
            height = (height as f64 / scale) as u32;
        }
        let key: SampleKey = (name.to_string(), cols, rows, width, height, fg, bg);
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            h.finish()
        };
        let mut samples = self.samples.borrow_mut();
        let rgb = match samples.iter().find(|(k, _)| *k == key) {
            Some((_, rgb)) => rgb.clone(),
            None => {
                let rgb = self.host.font_sample(name, SAMPLE_TEXT, width, height, fg, bg).map(Rc::new);
                // The last few, for moving back and forth.
                if samples.len() >= 8 {
                    samples.remove(0);
                }
                samples.push((key, rgb.clone()));
                rgb
            }
        };
        let rgb = rgb.filter(|p| p.len() == (width * height * 3) as usize)?;
        Some((hash, width, height, rgb))
    }

    /// A box over the right of the screen: an optional input line, then
    /// the items with `sel` marked, each with its theme's colours if
    /// `swatches` has them. `groups` are headers, each with the index of
    /// the item it comes before. `footer` rows at the bottom are left for
    /// the caller when there is room for them and three items. Returns
    /// where the input text starts, and the footer's x, y and width.
    #[allow(clippy::too_many_arguments)]
    fn popup(
        &mut self,
        f: &mut Frame,
        title: &str,
        head: Option<&str>,
        items: &[String],
        swatches: &[Option<ThemeInfo>],
        groups: &[(usize, &str)],
        sel: usize,
        footer: usize,
        look: &Look,
    ) -> (usize, usize, Option<(usize, usize, usize)>) {
        let (w, h) = (self.width, self.height);
        // Wider on a wide screen, for long names and the sample.
        let most = if footer > 0 { 80 } else { 60 };
        let pw = (w * 3 / 4).clamp(10.min(w), most.min(w));
        let x = w - pw;
        let extra = usize::from(head.is_some());
        // Under the header, when there is one.
        let y = usize::from(w >= 60 && h >= 12);
        // With a footer, room for three items even when fewer match, so
        // the box doesn't jump as the filter narrows.
        let body = (items.len() + groups.len()).max(if footer > 0 { 3 } else { 0 });
        let ph = (body + extra + 2 + footer).min(h.saturating_sub(1 + y)).max(3.min(h - y));
        let footer = if ph >= extra + 2 + footer + 3 { footer } else { 0 };
        f.fill(x, y, pw, ph, look.base);
        f.boxed(x, y, pw, ph, title, look.dim, look.accent.bold());
        let inner = pw - 2;
        let footer_at = (footer > 0).then(|| (x + 1, y + ph - 1 - footer, inner));
        let mut input = (x + 1, y + 1);
        if let Some(head) = head {
            let tx = f.text(x + 2, y + 1, x + 1 + inner, "❯ ", look.accent.bold());
            f.text(
                tx,
                y + 1,
                x + 1 + inner,
                &truncate(head, (x + 1 + inner).saturating_sub(tx)),
                look.base,
            );
            input = (tx, y + 1);
        }
        let room = ph.saturating_sub(2 + extra + footer);
        // The rows: items, with a header row before each group's first.
        let mut rows: Vec<(Option<usize>, &str)> = Vec::new();
        for k in 0..items.len() {
            if let Some((_, name)) = groups.iter().find(|(at, _)| *at == k) {
                rows.push((None, name));
            }
            rows.push((Some(k), ""));
        }
        // Scrolled to the selection, and to its header if it is a group's
        // first and there is room.
        let at = rows.iter().position(|(k, _)| *k == Some(sel)).unwrap_or(usize::MAX);
        let at = if at != usize::MAX && at > 0 && rows[at - 1].0.is_none() && room > 1 {
            at - 1
        } else {
            at
        };
        let first = if at != usize::MAX && at + 1 > room { at + 1 - room } else { 0 };
        self.hit_rows = if groups.is_empty() {
            Vec::new()
        } else {
            rows.iter().skip(first).take(room).map(|(k, _)| *k).collect()
        };
        let iy = y + 1 + extra;
        // Theme swatches: `Aa` in the theme's text colour on its
        // background, then its first six ANSI colours.
        let swatch_w = if swatches.iter().any(Option::is_some) && inner >= 30 {
            11
        } else {
            0
        };
        for (r, (k, title)) in rows.iter().enumerate().skip(first).take(room) {
            let yy = iy + r - first;
            let Some(k) = *k else {
                f.text(x + 3, yy, x + 1 + inner, title, look.dim);
                continue;
            };
            let item = &items[k];
            let selected = k == sel;
            let style = if selected { look.sel } else { look.base };
            if selected {
                f.fill(x + 1, yy, inner, 1, style);
            }
            let text_end = x + 1 + inner - swatch_w;
            f.text(x + 1, yy, text_end, if selected { " ▸ " } else { "   " }, style);
            f.text(x + 4, yy, text_end, &truncate(item, text_end.saturating_sub(x + 5)), style);
            if let Some(Some(info)) = swatches.get(k)
                && swatch_w > 0
            {
                let sx = text_end;
                let on = Style::default().bg(Color::Rgb(info.background));
                let sx = f.text(sx, yy, sx + 4, " Aa ", on.fg(Color::Rgb(info.foreground)));
                for (j, c) in info.palette.iter().skip(1).take(6).enumerate() {
                    f.text(sx + j, yy, sx + j + 1, " ", Style::default().bg(Color::Rgb(*c)));
                }
                f.text(sx + 6, yy, sx + 7, " ", on);
            }
            if selected && head.is_none() {
                f.cursor = Some((x + 2, yy));
            }
        }
        self.hit.popup = Some((x + 1, x + 1 + inner, iy, first, room.min(rows.len().saturating_sub(first))));
        (input.0, input.1, footer_at)
    }

    fn draw_help(&mut self, f: &mut Frame, look: &Look) {
        let keys: [(&str, &str); 10] = [
            ("↑↓ j k", "move"),
            ("←→ tab", "change section"),
            ("enter space", "edit the setting"),
            ("/", "search all settings"),
            ("r", "reset: remove it from this file"),
            ("u", "undo the last change"),
            ("e", "open the file in your editor"),
            ("enter a ⌫", "on a shortcut: record, add, clear"),
            ("ctrl+l", "redraw"),
            ("q esc", "quit"),
        ];
        let notes = [
            "Each change is saved at once; the file can",
            "always be edited by hand.",
            "",
            "Run it in any terminal with:",
            "  terminal-decline settings  (or decline settings)",
        ];
        let lines: Vec<String> = keys
            .iter()
            .map(|(k, v)| format!("{k:<12} {v}"))
            .chain(std::iter::once(String::new()))
            .chain(notes.iter().map(|s| s.to_string()))
            .collect();
        self.popup(f, "Help", None, &lines, &[], &[], usize::MAX, 0, look);
        // The keys in the accent.
        if let Some((x0, x1, y0, first, count)) = self.hit.popup {
            for (k, (key, _)) in keys.iter().enumerate().skip(first).take(count) {
                f.text(x0 + 3, y0 + k - first, x1, key, look.accent);
            }
        }
        self.hit.popup = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Labels get what the longest needs, up to about half the width;
    /// a description column only where there is room for it.
    #[test]
    fn columns_fit_the_width() {
        // At 80 columns the settings are 62 wide from x 17: whole labels
        // (23 + 2), no description.
        let c = columns(17, 62, 23, 20, 11);
        assert_eq!((c.label, c.value, c.desc), (20, 45, None));
        // Wide: the value's column, then the description up to the note.
        let c = columns(17, 142, 23, 17, 11);
        assert_eq!((c.label, c.value), (20, 45));
        assert_eq!(c.desc, Some((65, 79)));
        // A very long label is cut near half the width.
        let c = columns(0, 100, 80, 10, 0);
        assert_eq!(c.value - c.label, 43);
        // Narrow: half of it, as the one-column layout has always had.
        let c = columns(0, 20, 20, 6, 0);
        assert_eq!((c.label, c.value, c.desc), (3, 13, None));
        // No values or notes: a description still needs its room.
        assert!(columns(0, 60, 10, 0, 0).desc.is_some());
        assert!(columns(0, 40, 10, 0, 0).desc.is_none());
    }

    #[test]
    fn the_accent_is_a_readable_theme_colour() {
        // Contrast: black on white is 21, a colour on itself 1.
        assert!((contrast(0x000000, 0xffffff) - 21.0).abs() < 0.01);
        assert!((contrast(0x336699, 0x336699) - 1.0).abs() < 1e-9);
        let mut palette = vec![0x808080; 16];
        palette[4] = 0x1a1b26; // blue unreadable on this background
        palette[12] = 0x7aa2f7;
        let look = Look::new(Some(0xc0caf5), Some(0x1a1b26), &palette);
        assert_eq!(look.accent.fg, Color::Ansi(12));
        assert_eq!(look.base, Style::default(), "the theme's own colours");
        assert!(look.sel.reverse && look.sel.fg == Color::Ansi(12));
        assert!(look.dim.dim);
        // Unknown palette: blue. Unreadable foreground: our own colours.
        assert_eq!(Look::new(None, None, &[]).accent.fg, Color::Ansi(4));
        let look = Look::new(Some(0x101010), Some(0x101010), &[]);
        assert_eq!(look.base.bg, Color::Rgb(0x101010));
        assert_eq!(look.base.fg, Color::Rgb(0xe8e8e8));
    }
}
