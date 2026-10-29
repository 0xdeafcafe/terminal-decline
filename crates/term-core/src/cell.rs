//! Packed cell storage. One cell is 8 bytes (§4 of the cells design): what
//! it shows, how wide it is, and an id in its screen's style table.

use crate::clusters::Clusters;

/// A colour as stored in a style. The top byte says what kind it is:
/// 0 = the terminal's default, 1 = a palette index, 2 = 24-bit RGB.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[repr(transparent)]
pub struct Color(pub u32);

impl Color {
    pub const DEFAULT: Color = Color(0);

    pub const fn indexed(i: u8) -> Color {
        Color(0x0100_0000 | i as u32)
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color(0x0200_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    pub const fn kind(self) -> ColorKind {
        match self.0 >> 24 {
            1 => ColorKind::Indexed(self.0 as u8),
            2 => ColorKind::Rgb((self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8),
            _ => ColorKind::Default,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorKind {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// How much of a row a cell's content takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Width {
    /// One cell.
    Narrow = 0,
    /// The left half of a two-cell character.
    Wide = 1,
    /// The right half of a two-cell character; draws nothing itself.
    Spacer = 2,
    /// The last column of a row whose next wide character didn't fit and
    /// moved to the next row.
    EdgeSpacer = 3,
}

/// One screen cell, 8 bytes:
///
/// ```text
///  63            32 31   27  26  25  24 23  22 21  20         0
/// ┌────────────────┬───────┬───┬───┬──────┬──────┬────────────┐
/// │ style id (32)  │ 0 (5) │SE │ P │width │ kind │ content(21)│
/// └────────────────┴───────┴───┴───┴──────┴──────┴────────────┘
/// ```
///
/// - content: a Unicode scalar, 0 = empty (kind 0; the other kinds come
///   with clusters and placeholders);
/// - width: a [`Width`];
/// - P: protected (DECSCA), always 0 for now;
/// - SE, "shows when empty": a copy of the style's
///   [`Style::shows_empty`](crate::Style::shows_empty), so blank tests need
///   no table lookup;
/// - style id: an index into the screen's style table, 0 = the default.
///
/// The blank cell is all zero, so a row of blanks resets with a memset.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash, Debug)]
pub struct Cell(u64);

const _: () = assert!(std::mem::size_of::<Cell>() == 8);
// A row: its change counter, cells, `wrapped` and the boxed extras (§6.1).
const _: () = assert!(std::mem::size_of::<Row>() == 48);

impl Cell {
    pub const BLANK: Cell = Cell(0);
    pub const CONTENT: u64 = (1 << 21) - 1;
    pub const KIND: u64 = 3 << 21;
    /// Kind 1: the content is a slot in the row's cluster store.
    pub const CLUSTER: u64 = 1 << 21;
    pub const WIDTH: u64 = 3 << 23;
    const WIDTH_SHIFT: u32 = 23;
    pub const PROTECTED: u64 = 1 << 25;
    pub const SHOWS_EMPTY: u64 = 1 << 26;
    /// The bits a cell's style gives it: the style id and SE.
    pub const STYLE: u64 = 0xFFFF_FFFF << 32 | Self::SHOWS_EMPTY;
    /// Bits that make a cell draw something when a row ends there.
    pub const NOT_BLANK: u64 = Self::CONTENT | Self::KIND | Self::SHOWS_EMPTY;

    /// The template the pen writes with: style id and SE, empty, narrow.
    #[inline]
    pub const fn template(style: u32, shows_empty: bool) -> Cell {
        Cell((style as u64) << 32 | (shows_empty as u64) << 26)
    }

    /// The raw bits.
    #[inline]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn from_bits(bits: u64) -> Cell {
        Cell(bits)
    }

    /// This cell's style (id and SE) with `c` in it, narrow.
    #[inline]
    pub const fn with_char(self, c: char) -> Cell {
        Cell(self.0 & Self::STYLE | c as u64)
    }

    #[inline]
    pub const fn with_width(self, w: Width) -> Cell {
        Cell(self.0 & !Self::WIDTH | (w as u64) << Self::WIDTH_SHIFT)
    }

    /// The same content and width in another style.
    #[inline]
    pub const fn with_style(self, template: Cell) -> Cell {
        Cell(self.0 & !Self::STYLE | template.0 & Self::STYLE)
    }

    /// Just the style part (id and SE), as a template.
    #[inline]
    pub const fn style_part(self) -> Cell {
        Cell(self.0 & Self::STYLE)
    }

    #[inline]
    pub const fn style_id(self) -> u32 {
        (self.0 >> 32) as u32
    }

    #[inline]
    pub const fn shows_empty(self) -> bool {
        self.0 & Self::SHOWS_EMPTY != 0
    }

    #[inline]
    pub const fn width(self) -> Width {
        match (self.0 >> Self::WIDTH_SHIFT) & 3 {
            0 => Width::Narrow,
            1 => Width::Wide,
            2 => Width::Spacer,
            _ => Width::EdgeSpacer,
        }
    }

    #[inline]
    pub const fn is_wide(self) -> bool {
        self.0 & Self::WIDTH == (Width::Wide as u64) << Self::WIDTH_SHIFT
    }

    /// The right half of a wide character, or an edge spacer.
    #[inline]
    pub const fn is_spacer(self) -> bool {
        self.0 & (2 << Self::WIDTH_SHIFT) != 0
    }

    /// Either half of a wide character (or an edge spacer).
    #[inline]
    pub const fn is_wide_part(self) -> bool {
        self.0 & Self::WIDTH != 0
    }

    /// The content bits: a Unicode scalar, 0 = empty.
    #[inline]
    pub const fn codepoint(self) -> u32 {
        (self.0 & Self::CONTENT) as u32
    }

    /// The character shown; a space for an empty cell. A cluster cell's
    /// text is in its row ([`Row::text_at`]); this gives a space for it.
    #[inline]
    pub fn ch(self) -> char {
        if self.0 & Self::KIND != 0 {
            return ' ';
        }
        match self.codepoint() {
            0 => ' ',
            c => char::from_u32(c).unwrap_or(' '),
        }
    }

    /// Nothing would be drawn here if the row ended at this cell.
    #[inline]
    pub const fn is_blank(self) -> bool {
        self.0 & Self::NOT_BLANK == 0
    }

    /// A grapheme cluster cell: its content is a slot in the row's store.
    #[inline]
    pub const fn is_cluster(self) -> bool {
        self.0 & Self::KIND == Self::CLUSTER
    }

    /// This cell (style and width kept) holding cluster `slot`.
    #[inline]
    pub const fn with_cluster(self, slot: u32) -> Cell {
        Cell(self.0 & !(Self::CONTENT | Self::KIND) | Self::CLUSTER | slot as u64)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// The row's change counter (§9.1): changes whenever its content does,
    /// and only then. 0 for rows not on a screen.
    pub seq: u64,
    /// The cells. Overwriting a cluster cell must free its slot first
    /// ([`Row::free_clusters`]); moving cells within the row keeps them.
    pub cells: Vec<Cell>,
    /// The line continues onto the next row (soft wrap).
    pub wrapped: bool,
    /// Only rows with clusters or prompt marks have one (§6.1).
    extra: Option<Box<RowExtra>>,
}

/// What only some rows need.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RowExtra {
    clusters: Clusters,
    /// OSC 133 marks on this row, one per kind, by kind (§11.6).
    marks: [Option<Mark>; 4],
}

/// What an OSC 133 mark says starts there (FinalTerm's shell
/// integration): A, B, C and D. The numbers are the stored kinds of §11.3.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum MarkKind {
    /// `133;A`: the prompt starts.
    Prompt = 1,
    /// `133;B`: the prompt ends and the command line (what is typed) starts.
    Input = 2,
    /// `133;C`: the command runs; its output starts.
    Output = 3,
    /// `133;D`: the command finished, maybe with its exit status.
    End = 4,
}

/// An OSC 133 mark: where on its row it was set, and for
/// [`MarkKind::End`] the exit status, if the shell sent one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Mark {
    pub kind: MarkKind,
    pub col: u16,
    pub status: Option<u32>,
}

/// A store whose buffers are under this is kept, emptied, when its row is
/// reset: a status line redrawing emoji doesn't allocate each time.
const KEEP_EXTRA_BYTES: usize = 1024;

impl Row {
    pub fn new(cols: usize, fill: Cell) -> Row {
        Row {
            seq: 0,
            cells: vec![fill; cols],
            wrapped: false,
            extra: None,
        }
    }

    pub fn reset(&mut self, cols: usize, fill: Cell) {
        if self.cells.len() == cols && fill == Cell::BLANK {
            // The usual case: the all-zero blank, which fills as a memset.
            self.cells.fill(Cell::BLANK);
        } else {
            self.cells.clear();
            self.cells.resize(cols, fill);
        }
        self.wrapped = false;
        if self.extra.is_some() {
            self.clear_extra();
        }
    }

    /// Free every cluster and drop every mark.
    #[cold]
    fn clear_extra(&mut self) {
        if let Some(x) = &mut self.extra {
            if x.clusters.capacity_bytes() > KEEP_EXTRA_BYTES {
                self.extra = None;
            } else {
                x.clusters.clear();
                x.marks = [None; 4];
            }
        }
    }

    /// Fill every cell with `fill` (not a cluster), freeing all clusters
    /// and dropping the row's marks.
    pub fn fill(&mut self, fill: Cell) {
        self.cells.fill(fill);
        if self.extra.is_some() {
            self.clear_extra();
        }
    }

    /// The row's OSC 133 marks, by kind (prompt, input, output, end).
    #[inline]
    pub fn marks(&self) -> impl Iterator<Item = Mark> + '_ {
        self.extra.iter().flat_map(|x| x.marks.iter().flatten().copied())
    }

    /// The row has an OSC 133 mark.
    #[inline]
    pub fn has_marks(&self) -> bool {
        self.extra.as_ref().is_some_and(|x| x.marks.iter().any(Option::is_some))
    }

    /// Set `mark`, replacing the row's mark of the same kind.
    pub fn set_mark(&mut self, mark: Mark) {
        let x = self.extra.get_or_insert_with(Default::default);
        x.marks[mark.kind as usize - 1] = Some(mark);
    }

    /// Drop the row's marks.
    pub fn clear_marks(&mut self) {
        if let Some(x) = &mut self.extra {
            x.marks = [None; 4];
        }
    }

    /// The row's cluster store, if it has one.
    #[inline]
    pub fn clusters(&self) -> Option<&Clusters> {
        self.extra.as_deref().map(|x| &x.clusters)
    }

    /// The text of the cell at `col`: its cluster, or its character
    /// (nothing for an empty cell), appended to `out`.
    pub fn text_at(&self, col: usize, out: &mut String) {
        let c = self.get(col);
        if c.is_cluster() {
            out.push_str(self.clusters().map_or("", |x| x.get(c.codepoint())));
        } else if c.codepoint() != 0 {
            out.push(c.ch());
        }
    }

    /// The first scalar of the cell at `col`; a space for an empty cell.
    pub fn first_char(&self, col: usize) -> char {
        let c = self.get(col);
        if c.is_cluster() {
            return self.clusters().and_then(|x| x.get(c.codepoint()).chars().next()).unwrap_or(' ');
        }
        c.ch()
    }

    /// Make room in the row's cluster store for `slots` clusters of
    /// `bytes` bytes in all (decoding a history line).
    pub fn reserve_clusters(&mut self, slots: usize, bytes: usize) {
        if slots > 0 {
            self.extra.get_or_insert_with(Default::default).clusters.reserve(slots, bytes);
        }
    }

    /// Free the clusters of the cells in `range`, leaving the cells as they
    /// are: the caller overwrites them next.
    #[inline]
    pub fn free_clusters(&mut self, range: std::ops::Range<usize>) {
        if let Some(x) = &mut self.extra {
            for c in &self.cells[range] {
                if c.is_cluster() {
                    x.clusters.free(c.codepoint());
                }
            }
        }
    }

    /// Make the cell at `col` hold `text` (a whole cluster within the
    /// caps), keeping its style and width. False if the row is out of
    /// room: the cell then holds the cluster's first scalar.
    pub fn set_cluster(&mut self, col: usize, text: &str) -> bool {
        self.free_clusters(col..col + 1);
        let x = self.extra.get_or_insert_with(Default::default);
        let cell = self.cells[col];
        match x.clusters.insert(text) {
            Some(slot) => {
                self.cells[col] = cell.with_cluster(slot);
                true
            }
            None => {
                let first = text.chars().next().unwrap_or(' ');
                self.cells[col] = cell.style_part().with_char(first).with_width(cell.width());
                false
            }
        }
    }

    /// Add `c` to the content of the cell at `col` (a character or a
    /// cluster). False, and nothing changes, if the caps or the row's room
    /// don't allow it, or the cell is empty.
    pub fn push_to_cell(&mut self, col: usize, c: char) -> bool {
        let cell = self.cells[col];
        if cell.is_cluster() {
            let slot = cell.codepoint();
            return self.extra.as_mut().is_some_and(|x| x.clusters.push(slot, c));
        }
        let Some(first) = char::from_u32(cell.codepoint()).filter(|_| cell.codepoint() != 0) else {
            return false;
        };
        let mut buf = [0u8; 8];
        let n = first.encode_utf8(&mut buf).len();
        let m = c.encode_utf8(&mut buf[n..]).len();
        let text = std::str::from_utf8(&buf[..n + m]).expect("two scalars are UTF-8");
        let x = self.extra.get_or_insert_with(Default::default);
        match x.clusters.insert(text) {
            Some(slot) => {
                self.cells[col] = cell.with_cluster(slot);
                true
            }
            None => false,
        }
    }

    /// Make the row `cols` cells wide: cells past it are dropped (and their
    /// clusters freed), new ones are blank.
    pub fn resize_cells(&mut self, cols: usize) {
        if cols < self.cells.len() {
            let len = self.cells.len();
            self.free_clusters(cols..len);
            if let Some(x) = &mut self.extra {
                for m in &mut x.marks {
                    if m.is_some_and(|m| m.col as usize >= cols) {
                        *m = None;
                    }
                }
            }
        }
        self.cells.resize(cols, Cell::BLANK);
    }

    pub fn get(&self, col: usize) -> Cell {
        self.cells.get(col).copied().unwrap_or(Cell::BLANK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_kinds_round_trip() {
        assert_eq!(Color::DEFAULT.kind(), ColorKind::Default);
        assert_eq!(Color::default(), Color::DEFAULT);
        for i in [0u8, 1, 7, 8, 15, 16, 231, 232, 255] {
            assert_eq!(Color::indexed(i).kind(), ColorKind::Indexed(i));
        }
        for (r, g, b) in [
            (0, 0, 0),
            (255, 255, 255),
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (1, 2, 3),
            (254, 128, 127),
        ] {
            assert_eq!(Color::rgb(r, g, b).kind(), ColorKind::Rgb(r, g, b));
        }
    }

    #[test]
    fn color_kinds_never_collide() {
        // Index 0 and black are not the default colour, nor each other.
        assert_ne!(Color::indexed(0), Color::DEFAULT);
        assert_ne!(Color::rgb(0, 0, 0), Color::DEFAULT);
        assert_ne!(Color::rgb(0, 0, 0), Color::indexed(0));
        assert_ne!(Color::rgb(0, 0, 255), Color::indexed(255));
    }

    #[test]
    fn unknown_color_tags_read_as_default() {
        assert_eq!(Color(0x0300_0000).kind(), ColorKind::Default);
        assert_eq!(Color(0xFF12_3456).kind(), ColorKind::Default);
    }

    #[test]
    fn bit_layout() {
        // The fields don't overlap and leave bits 27–31 spare.
        let fields = [
            Cell::CONTENT,
            Cell::KIND,
            Cell::WIDTH,
            Cell::PROTECTED,
            Cell::SHOWS_EMPTY,
            0xFFFF_FFFF << 32,
        ];
        let mut seen = 0u64;
        for f in fields {
            assert_eq!(seen & f, 0, "{f:#x} overlaps");
            seen |= f;
        }
        assert_eq!(!seen, 0x1F << 27);
        assert!(Cell::CONTENT >= char::MAX as u64);

        let t = Cell::template(0xABCD_1234, true);
        assert_eq!(t.bits(), 0xABCD_1234 << 32 | 1 << 26);
        assert_eq!((t.style_id(), t.shows_empty()), (0xABCD_1234, true));
        let c = t.with_char('\u{10FFFF}').with_width(Width::Wide);
        assert_eq!((c.ch(), c.codepoint(), c.width()), ('\u{10FFFF}', 0x10FFFF, Width::Wide));
        assert_eq!(c.style_part(), t);
        assert!(c.is_wide() && !c.is_spacer() && c.is_wide_part());
        for w in [Width::Narrow, Width::Wide, Width::Spacer, Width::EdgeSpacer] {
            let c = t.with_width(w);
            assert_eq!(c.width(), w);
            assert_eq!(c.is_spacer(), matches!(w, Width::Spacer | Width::EdgeSpacer));
            assert_eq!(c.is_wide(), w == Width::Wide);
            assert_eq!(c.is_wide_part(), w != Width::Narrow);
            assert_eq!(c.style_part(), t, "width leaves the style alone");
        }
        let other = Cell::template(7, false);
        assert_eq!(c.with_style(other).style_part(), other);
        assert_eq!(c.with_style(other).ch(), '\u{10FFFF}');
        assert_eq!(c.with_style(other).width(), Width::Wide);
    }

    #[test]
    fn blank_is_zero() {
        assert_eq!(Cell::BLANK.bits(), 0);
        assert_eq!(Cell::default(), Cell::BLANK);
        assert!(Cell::BLANK.is_blank());
        assert_eq!(Cell::BLANK.ch(), ' ');
        // A style id alone (a foreground colour, bold) doesn't show.
        assert!(Cell::template(5, false).is_blank());
        assert!(Cell::template(5, false).with_width(Width::Spacer).is_blank());
        // A character, even a space, or a style that shows when empty does.
        assert!(!Cell::BLANK.with_char(' ').is_blank());
        assert!(!Cell::BLANK.with_char('a').is_blank());
        assert!(!Cell::template(5, true).is_blank());
        for invalid in [0xD800u64, 0xDFFF, 0x11_0000, Cell::CONTENT] {
            assert_eq!(Cell::from_bits(invalid).ch(), ' ', "{invalid:#x}");
        }
    }

    #[test]
    fn row_helpers() {
        let fill = Cell::template(4, true);
        let mut row = Row::new(3, fill);
        assert_eq!(row.cells, vec![fill; 3]);
        assert!(!row.wrapped);
        assert_eq!(row.get(2), fill);
        assert_eq!(row.get(3), Cell::BLANK);
        row.wrapped = true;
        row.reset(5, Cell::BLANK);
        assert_eq!(row.cells, vec![Cell::BLANK; 5]);
        assert!(!row.wrapped);
        // Same length: the blank fill (a memset) and any other fill.
        row.cells[1] = Cell::template(9, true).with_char('x');
        row.wrapped = true;
        row.reset(5, Cell::BLANK);
        assert_eq!(row.cells, vec![Cell::BLANK; 5]);
        assert!(!row.wrapped);
        row.reset(5, fill);
        assert_eq!(row.cells, vec![fill; 5]);
        row.reset(2, Cell::BLANK);
        assert_eq!(row.cells, vec![Cell::BLANK; 2]);
    }

    #[test]
    fn marks_one_per_kind() {
        let mut row = Row::new(10, Cell::BLANK);
        assert!(!row.has_marks());
        let mark = |kind, col| Mark { kind, col, status: None };
        row.set_mark(mark(MarkKind::Input, 8));
        row.set_mark(mark(MarkKind::Prompt, 0));
        row.set_mark(mark(MarkKind::Input, 9));
        let end = Mark {
            kind: MarkKind::End,
            col: 0,
            status: Some(1),
        };
        row.set_mark(end);
        assert!(row.has_marks());
        assert_eq!(
            row.marks().collect::<Vec<_>>(),
            vec![mark(MarkKind::Prompt, 0), mark(MarkKind::Input, 9), end],
            "by kind, the later B replacing the earlier"
        );
        // Narrowing drops marks past the new width; reset drops them all.
        row.resize_cells(9);
        assert_eq!(row.marks().count(), 2);
        row.reset(9, Cell::BLANK);
        assert!(!row.has_marks());
        row.set_mark(mark(MarkKind::Output, 3));
        row.fill(Cell::BLANK);
        assert!(!row.has_marks());
        row.set_mark(mark(MarkKind::Output, 3));
        row.clear_marks();
        assert!(!row.has_marks());
    }
}
