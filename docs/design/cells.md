# Cells, styles, runs and scrollback: the storage model for 1.0 (F.0)

Status: **draft for the Opus (xhigh) review**, 28 Oct 2026. Nothing here is
built yet. It builds on the parser work in `worktree-agent-a2b894bb79879a081`
(vendored vte with `print_ascii`, `write_ascii` row-segment writes, the screen
as a ring of rows, the memset row reset, the faster scrollback encoder), which
is merging into `main` now. Every phase-F item and the image, ligature,
search and frame-outside-the-lock work in phases B, E and R sits on this
model, so it has to serve all of them at once.

Modules named here: **term-core** (escape-code handling, the screen,
scrollback, selection; no platform code), **term-font** (Core Text: fonts,
shaping, rasterising), **term-render** (turns terminal state into GPU
instances, Metal), **term-app** (the AppKit window and the `session` module
that owns the PTY threads and the shared terminal), and the **vendored vte
parser** (`vendor/vte`).

---

## 1. Decisions at a glance

| Question | Decision | Why |
|---|---|---|
| Cell size | **8 bytes**: 21-bit content, 2-bit kind, 2-bit width, 2 flag bits, 32-bit style id | 33% less than today's 12 B; one `u64` load/store; the blank cell is still all zero, so the memset row reset and the zero-skip in the scrollback encoder keep working |
| What a cell's content is | a Unicode scalar, **or** a slot in the row's cluster store, **or** a kitty placeholder's row/column | one cell layout for text, clusters and image placeholders |
| Style storage | **one style table per screen** (primary, alternate); cells hold an id | fg, bg, underline colour/style, overline, link id don't fit in a cell; most screens use < 100 distinct styles |
| Freeing styles | **sweep**, no reference counts: when the table reaches 1.5 × the screen's cell count, mark the ids the screen's cells use and free the rest | only the screen refers to ids (history stores styles by value), so a sweep is bounded by screen size and the write path does no counting |
| Grapheme clusters | **per-row store**; a cluster cell holds a slot index; the store holds the whole cluster as UTF-8 | each slot belongs to exactly one cell, so it is freed when the cell is overwritten; rows move (scroll, ring rotation, insert/delete lines) with their clusters and need no fixups |
| Width of a cluster | mode **2027 on by default**: the cluster is as wide as `unicode-width` says its text is, clamped to 1–2; mode 2027 off: each codepoint keeps its own width, zero-width ones join the previous cell | matches what Rust (`unicode-width`) and JS (`string-width`) TUIs compute; programs can still ask for the old rule |
| kitty placeholders | cell kind "placeholder", row and column index in the content bits; image id in the style's fg (+ a high byte), placement id in the style's underline colour | lossless, 8 bytes per cell, no cluster storage; one style per image |
| Images | a **placement table per screen**, keyed by the stable line number of the image's top row; freed in the same call that frees history blocks | one entry per image instead of per covered cell; nothing to do when output scrolls |
| Change tracking | every row carries a **change counter**, unique per row per parse call | the renderer's row cache and the frame snapshot copy only changed rows |
| Frames outside the lock | a **snapshot** copies changed rows' cells, cluster text, the style vector, cursor, selection ranges, palette and visible placements | the lock is held for a memcpy, not a frame build |
| Shaping runs | break only on **font face** (bold/italic), **code-drawn glyphs** and **placeholders** | ligatures survive colour changes, the cursor and selection |
| Ligatures | one glyph, **one instance per cell**, each quad clipped to its cell on the CPU, coloured by that cell | no shader change; per-cell colours inside a ligature |
| Glyph key | **(font id, glyph id, cells)** from a **font registry** in term-font; code-drawn glyphs use a reserved font id and a sprite id | CTLine picks fallback fonts itself, so a codepoint alone no longer names a glyph |
| Scrollback v2 | stored lines are **physical lines as written**; logical lines are runs of wrapped stored lines; per-block style dictionary and link table; trailing fill implied by runs | plain 42 B/line (today 46 sealed, 52 measured), `ls` 59 (86/93), full-width background 13 (174/183), CJK 65 (87/90) |
| Reflow | the screen is reflowed eagerly on every resize; **history is never rewritten**: rows at the current width are worked out on demand from the stored lines | resize cost is O(screen), not O(history): well under 8 ms with 100k lines |
| Point stability | `Point.line` stays the number a row got when it was written; history points never change; on-screen positions that matter are remapped in the same resize | selection, search hits, image anchors and the view position survive output, scrolling and reflow |
| Parser | vte gains an APC string state with start/put/end calls, a bulk `put_slice` for DCS, and a size policy for long OSC strings (streaming for OSC 1337 files) | kitty graphics, Sixel and iTerm2 images without buffering megabytes twice |

---

## 2. What this has to serve

From the roadmap (F, B, E, R and the Definition of done):

- Neovim undercurl with underline colour (SGR 4:3, 58), true colour, overline.
- OSC 8 hyperlinks with Cmd-click (E.2); OSC 133 prompt marks: jump between
  prompts, select output, Option-click to move the cursor (E.5).
- Grapheme clusters: combining marks, ZWJ emoji, flags, skin tones, VS15/VS16,
  mode 2027 (F.1, F.2).
- Ligatures with Core Text (B.1), code-drawn box/block/Powerline glyphs (F.6).
- kitty graphics including virtual placements and Unicode placeholders,
  iTerm2 OSC 1337 and Sixel (B.3, B.5; `yazi` previews are on the
  daily-driver checklist).
- Reflow on resize and pulling lines back when the window grows taller
  (F.4: a 120 Hz divider drag with 100k lines of history, frame ≤ 8 ms).
- Cmd+F search over history (E.7); selection that survives scrolling and
  reflow; rectangular selection (E.8).
- Frame build outside the lock (R.1), per-row instance caching (R.2), a
  history row cache (R.3), idle compression of history blocks (R.7).
- The thresholds: footprint ≤ 35% of Ghostty's idle and after 10k lines,
  `cat` CPU ≤ 45%, key latency p99 ≤ Ghostty's.

And it must not give back the parser work's gains (plain 423 MB/s, coloured
172, TUI 289, `seq` 110 in 1 KB pieces).

---

## 3. Overview

```text
                 term-core (one Terminal per pane, behind the session's Mutex)
 ┌───────────────────────────────────────────────────────────────────────────┐
 │  Screen (primary)                     Screen (alternate, made on first use)│
 │  ┌──────────────────────────────┐     ┌──────────────────────────────┐    │
 │  │ rows: VecDeque<Row>  (ring)  │     │ rows, styles, links, graphics│    │
 │  │   Row: cells [Cell; cols]    │     │ (same shape; no history)     │    │
 │  │        change counter, flags │     └──────────────────────────────┘    │
 │  │        extra: clusters, marks│                                          │
 │  │ styles: style table          │                                          │
 │  │ links:  link table (OSC 8)   │                                          │
 │  │ graphics: images, placements │◄── keyed by stable line number           │
 │  └──────────────┬───────────────┘                                          │
 │                 │ rows scroll off the top                                  │
 │                 ▼                                                          │
 │  Scrollback (history): blocks of ≤ 64 KB, freed oldest first               │
 │    block: stored lines (physical, as written) + style dictionary + links   │
 │    rows at the current width are worked out on demand (lazy line index)    │
 └──────────────┬────────────────────────────────────────────────────────────┘
                │ snapshot under the lock: changed rows, styles, cursor,
                │ selection ranges, palette, visible placements
                ▼
   term-render (main thread, outside the lock from R.1)
     runs per row → term-font shapes (CTLine) → (font id, glyph id) keys
     → glyph textures; sprites for code-drawn glyphs; per-cell instances
```

---

## 4. The cell: 8 bytes

### 4.1 Bit layout

```text
 63                              32 31    27 26 25 24  23 22  21 20                    0
┌──────────────────────────────────┬────────┬──┬──┬──────┬──────┬───────────────────────┐
│           style id (32)          │ zero(5)│SE│ P│width │ kind │      content (21)     │
└──────────────────────────────────┴────────┴──┴──┴──────┴──────┴───────────────────────┘
```

| Bits | Field | Values |
|---|---|---|
| 0–20 | content | kind 0: Unicode scalar, 0 = empty. Kind 1: slot in this row's cluster store. Kind 2: kitty placeholder, row index in bits 0–8, column index in bits 9–17, bits 18–20 zero |
| 21–22 | kind | 0 scalar, 1 cluster, 2 placeholder, 3 never stored (reserved) |
| 23–24 | width | 0 narrow, 1 wide (left half), 2 spacer (right half of a wide cell), 3 edge spacer (the last column of a row whose next wide character didn't fit and moved to the next row) |
| 25 | P, protected | DECSCA/SPA protection from selective erase; reserved for E.13 (esctest2), always 0 until then |
| 26 | SE, shows when empty | a copy of one bit of the cell's style: it has a background, inverse, underline, strike or overline, so an empty cell with it is still drawn |
| 27–31 | zero | spare; five bits left (enough for per-cell prompt/input/output marks if E.5 needs them, as Ghostty has) |
| 32–63 | style id | index into the screen's style table; 0 = the default style. The table is capped below 2²⁴ entries, so the top byte is always 0 |

The blank cell is `0`: empty, scalar, narrow, default style. A cell erased
with a coloured background is `erase_style << 32 | SE`.

```rust
/// One screen cell: what it shows, how wide it is, and its style id.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash, Debug)]
pub struct Cell(u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind { Scalar = 0, Cluster = 1, Placeholder = 2 }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Width { Narrow = 0, Wide = 1, Spacer = 2, EdgeSpacer = 3 }

impl Cell {
    pub const BLANK: Cell = Cell(0);
    const CONTENT: u64 = (1 << 21) - 1;
    const KIND: u64 = 3 << 21;
    const WIDTH: u64 = 3 << 23;
    const PROTECTED: u64 = 1 << 25;
    const SHOWS_EMPTY: u64 = 1 << 26;
    /// Bits that make a cell draw something when a row ends there.
    const NOT_BLANK: u64 = Self::CONTENT | Self::KIND | Self::SHOWS_EMPTY;

    #[inline] pub fn style_id(self) -> u32 { (self.0 >> 32) as u32 }
    #[inline] pub fn kind(self) -> Kind { /* (self.0 >> 21) & 3 */ }
    #[inline] pub fn width(self) -> Width { /* (self.0 >> 23) & 3 */ }
    #[inline] pub fn scalar(self) -> Option<char> { /* kind 0 and content != 0 */ }
    #[inline] pub fn is_blank(self) -> bool { self.0 & Self::NOT_BLANK == 0 }
    /// The template the pen writes with: style id, SE bit, narrow scalar.
    #[inline] pub const fn template(style: u32, shows_empty: bool) -> Cell {
        Cell((style as u64) << 32 | (shows_empty as u64) << 26)
    }
}
```

### 4.2 Why this layout

- **Same size as Ghostty's cell, different split.** Ghostty packs a 2-bit
  content tag, 24-bit content, a 16-bit style id, 2-bit width, protected,
  hyperlink and 2-bit semantic-content bits and 16 bits of padding into a
  `u64`. Its content tag has two background-colour-only forms so that erased
  cells don't need a style. We don't need those: erasing interns one "erase
  style" per erase command (§5.4), and keeping every cell uniform keeps the
  hot loops simple. We put the style id in a full 32 bits instead of 16, so
  a screen can never run out of ids (Ghostty's are per page and grow the
  page when full; ours are per screen, and a 6K display with a small font
  has > 150k cells).
- **Cluster slot in the cell, not a side map keyed by column.** Ghostty keeps
  the base codepoint in the cell and the rest in a per-page map keyed by the
  cell's offset, so moving cells means moving map entries. We store the whole
  cluster in the row's store and the slot index in the cell, so insert/delete
  characters and scrolling move plain `u64`s.
- **SE keeps blank tests inside the cell.** Row-end trimming (scrollback
  encoder, frame row-end skip) and `check_invariants` ask "is this cell
  blank?" millions of times; with SE it is one mask, no table lookup. The
  invariant `cell.SE == styles[cell.style_id].shows_empty()` is checked.

### 4.3 How the parser work's hot paths carry over

| Today (parser branch) | With 8-byte cells |
|---|---|
| `write_ascii` writes `Cell { packed: b \| style, fg, bg }` per byte | `cells[i] = Cell(template \| b as u64)`: one OR and one 8-byte store per byte; the template is computed once per run |
| before a segment, OR of `packed` over the cells to find wide characters | OR of `cell.0 & (KIND \| WIDTH)` over the segment: catches wide halves **and** clusters/placeholders (they need their slots freed), same one pass |
| `Row::reset` memsets when the fill is `Cell::BLANK` | unchanged: blank is still 0. A coloured erase fills one `u64` value, which also vectorises |
| `stored_len` skips all-zero cells 8 at a time (an OR over 96 bytes) | OR over 8 × `u64` (64 bytes) of `c.0 & NOT_BLANK` |
| encoder fast path: OR of `packed & !0x7F \| fg \| bg` over the line | OR of `c.0 & !0x7F` over the line (style 0, scalar, narrow, < 0x80): one field instead of three |
| `set_bulk_ascii(false)` reference switch and the property test "bulk path = one character at a time, fed one byte per call" | kept; extended to clusters (§6.6) |
| screen as `VecDeque<Row>`, region scrolls rotate rows | kept; rows carry their clusters and change counters with them |

---

## 5. The style table (one per screen)

### 5.1 The style

```rust
/// Everything about how a cell looks except its text. 20 bytes, no padding.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct Style {
    pub fg: Color,              // default / palette / RGB, as today's Color
    pub bg: Color,
    /// SGR 58/59. Default = draw underlines in the foreground colour.
    pub underline_color: Color,
    /// OSC 8 link in this screen's link table; 0 = none.
    pub link: LinkId,           // u32
    pub attrs: Attrs,           // u16: BOLD DIM ITALIC INVERSE HIDDEN STRIKE BLINK OVERLINE, 8 spare
    pub underline: Underline,   // u8: None, Single, Double, Curly, Dotted, Dashed
    /// kitty placeholders only: bits 24–31 of the image id (the third
    /// diacritic). 0 everywhere else.
    pub image_id_high: u8,
}

impl Style {
    pub const DEFAULT: Style = /* all zero */;
    /// An empty cell in this style still draws something.
    pub fn shows_empty(&self) -> bool {
        self.bg != Color::DEFAULT
            || self.attrs.intersects(Attrs::INVERSE | Attrs::STRIKE | Attrs::OVERLINE)
            || self.underline != Underline::None
    }
}
```

SGR handled into it: 1, 2, 3, 4 (with `4:0`–`4:5`), 5/6, 7, 8, 9, 21
(double underline), 22–29, 30–37/90–97/38, 39, 40–47/100–107/48, 49, 53
(overline), 55, 58 (`58;5;n`, `58;2;r;g;b`, `58:2::r:g:b`), 59. Today 58 is
parsed and thrown away.

### 5.2 The table

```rust
/// The distinct styles one screen's cells use, each stored once. Cells hold
/// an index into it. Id 0 is the default style and is never freed.
pub struct StyleTable {
    styles: Vec<Style>,     // by id (20 B each)
    index: Vec<u32>,        // open addressing by hash of Style, power of two, 0 = empty slot
    free: Vec<u32>,         // ids freed by the last sweep, used first
    live: u32,              // ids in use
    sweep_at: u32,          // sweep before growing past this
    recent: [(Style, u32); 2], // the last two styles interned: alternating styles skip the hash
}
```

**Interning** (`intern(&Style) -> u32`): check `recent`, then hash the 20
bytes (FxHash), probe linearly, compare. On a miss take an id from `free` or
push. Average cost on a hit ~5 ns, on a miss ~15–20 ns. The **pen** keeps
its `Style` and a cached cell template `Option<Cell>`; SGR only edits the
`Style` and clears the cache, and the first print after it interns. So
`ESC[0m ESC[1;31m` with no text between interns nothing.

**Freeing: a sweep, not reference counts.** When an intern misses and the
table already holds `sweep_at = max(1024, 3 × cells / 2)` entries:

1. mark a bitset: every cell of every row of this screen
   (`bit[cell >> 32] = 1`), plus the pen's and the erase template's cached
   ids;
2. every unmarked id goes on `free` and is removed from `index` (linear
   probing with backward-shift deletion, so no tombstones);
3. sweep the link table the same way from the live styles (§5.5);
4. if fewer than a quarter of the entries are live, rebuild smaller (memory
   goes back after a gradient-heavy program exits).

Only this screen's cells and the pen hold ids. Saved cursors (DECSC) hold a
`Style` by value; history stores styles by value in its blocks (§11); the
snapshot copies styles (§9). So the sweep never has to look past the screen:
at 300×100 it reads 240 KB, about 20–40 µs.

A sweep leaves at most `cells + 2` live ids, so at least `cells / 2` new
styles arrive between two sweeps: under 2 cell reads per new style,
amortised. A freed id is only reused for a style no cell has, so nothing
that still refers to the id changes meaning, and an id's style never changes
while it is live. That is what lets the renderer cache rows by their change
counter (§9).

**Why not reference counts (Ghostty's way)?** Ghostty's history is pages of
live cells, so a style can be held by cells far up the history and a sweep
would have to walk all of it; reference counts are the price. Our history
stores styles by value, so the sweep is bounded by the screen, and the write
path (every printed character) does no count updates: `write_ascii` stays a
plain store loop.

### 5.3 How big it can get

| Content | Live styles | Table size |
|---|---|---|
| plain text, shell prompts | 1–5 | < 1 KB |
| `ls`, `git log`, compiler output | 5–30 | ≤ 1 KB |
| Neovim with a colour scheme, lazygit, btop | 30–500 | ≤ 18 KB |
| worst case: a different style in every cell | ≤ cells + 2 | ≤ 1.5 × cells entries before a sweep: **104 KB at 80×24, 1.6 MB at 300×100** |

Per entry: 20 B style + 8–16 B of index + up to 4 B of free list, about
36 B. The worst case only lasts while a program is actually painting tens
of thousands of distinct styles, and gives the memory back afterwards.

### 5.4 Truecolour gradients (a different style in every cell)

`lolcat`, a 24-bit colour test, btop's graphs: `ESC[38;2;r;g;bm█` per cell,
about 20 bytes of escape code per character.

- Each print: the SGR edits the pen (no intern), the print interns: one
  miss, ~15–20 ns, against ~100 ns to parse the 20-byte SGR. About +15%
  parse time on such output.
- A full 300×100 repaint with all-new colours makes 30k new styles, so about
  two sweeps per frame (~60 µs) against ~3.5 ms to parse the 600 KB of
  input: under 2%.
- The table peaks at 45k entries (1.6 MB) and is rebuilt smaller once the
  program stops.
- Erase commands with a coloured pen intern one erase style
  (`Style { bg: pen.bg, ..DEFAULT }`), cached until the pen's background
  changes.

### 5.5 The link table (OSC 8)

```rust
/// The hyperlinks of one screen. A style's `link` is an index here.
pub struct LinkTable {
    links: Vec<Option<Link>>,          // by id; id 0 = no link
    index: HashMap<(Box<str>, Box<str>), LinkId>, // (id parameter, URI)
    free: Vec<LinkId>,
}
pub struct Link { pub uri: Box<str>, pub id: Option<Box<str>> }
```

`OSC 8 ; id=x ; URI ST` sets the pen's link: the pen keeps the URI and id
parameter, and interns it in the active screen's table at the first print
(so switching screens re-interns rather than carrying an id from the other
table). Same id parameter and URI → same `LinkId`, so all the pieces of one
link (split across lines or colours) highlight together on hover. URIs over
4 KB and id parameters over 256 bytes are ignored. Links are swept right
after styles: a link no live style refers to is freed. History keeps its own
link table per block (§11.5).

---

## 6. Grapheme clusters

### 6.1 Where they live: one store per row

```rust
pub struct Row {
    cells: Vec<Cell>,                // exactly `cols` cells
    seq: u64,                        // change counter (§9)
    extra: Option<Box<RowExtra>>,    // only rows with clusters or prompt marks
    flags: RowFlags,                 // u8: WRAPPED, PLACEHOLDERS, BLINKING
}                                    // 48 bytes

pub struct RowExtra {
    clusters: Clusters,
    marks: [Option<Mark>; 3],        // OSC 133 marks on this row (§11.6)
}

/// The clusters of one row. Cell content (kind 1) is a slot index here.
pub struct Clusters {
    text: Vec<u8>,       // UTF-8 of every live cluster, back to back, with holes
    spans: Vec<Span>,    // by slot
    free: u16,           // first free slot (u16::MAX = none), chained through `next_free`
    garbage: u32,        // bytes of `text` no live slot uses
}
struct Span { start: u32, len: u16, next_free: u16 }   // 8 bytes
```

Rules: a slot belongs to exactly one cell. A cluster has at least two
scalars (a single scalar is stored as a kind-0 cell, so there is one form for
each thing). Caps: 32 scalars and 128 bytes per cluster (more marks are
dropped; Ghostty caps at 64 codepoints), 8 KB of cluster text per row.
`text` is compacted when `garbage > text.len() / 2` and over 256 bytes, so
a spinner rewriting an emoji in place doesn't grow the row.

**Per row, not per screen**, because:

- ownership is one cell to one slot, so overwriting or erasing a cluster cell
  frees its slot on the spot: no reference counts and no sweep;
- rows move whole (full-screen scroll round the ring, region scrolls,
  IL/DL) and their clusters move with them, with no index fixups;
- resetting a row (the memset reset) drops its `extra`;
- pushing a row into history and copying it into a snapshot read one small
  local store.

A per-screen store (kitty's text cache is one) would need a sweep like the
style table, for no gain: clusters are not shared between cells.

### 6.2 Segmenting as text arrives

term-core gets its own Unicode table (generated by a script from the UCD
files of the same Unicode version `unicode-width` 0.2 uses, checked in): a
two-level lookup giving, per scalar, width (0/1/2), Grapheme_Cluster_Break
class, Extended_Pictographic, Indic_Conjunct_Break, Emoji_Presentation and
"is a base of an emoji variation sequence". About 60–80 KB of read-only data,
one lookup per non-ASCII scalar (ASCII never reaches it: `write_ascii`).

```rust
/// Enough to decide whether the next scalar joins the last printed cell.
struct GraphemeState {
    at: Option<(usize, usize)>,  // screen row and column of the last printed cell
    prev: Gcb,                   // break class of the cluster's last scalar
    ri_odd: bool,                // an odd number of regional indicators so far
    emoji_zwj: EmojiZwj,         // inside ExtPict Extend* ZWJ (rule GB11)
    incb: IncbState,             // inside an Indic conjunct (rule GB9c)
}
```

`at` is cleared by anything but a print: cursor movement, controls, erase,
scroll, resize, mode changes. When a scalar arrives and `at` is set, the UAX
#29 rules (GB3–GB13 and GB9c, ~150 lines) say whether it joins. Checked
against `unicode-segmentation`'s grapheme iterator on GraphemeBreakTest.txt
and on random strings, fed whole and fed one byte per `advance` call.

With mode 2027 off ("legacy"), only zero-width scalars (combining marks,
ZWJ, variation selectors, default-ignorables) join the previous cell; every
other scalar starts a cell.

### 6.3 Width

- **2027 on (default):** a cluster's width is
  `UnicodeWidthStr::width(text).clamp(1, 2)` from `unicode-width` 0.2. So a
  ZWJ family is 2, a skin-tone sequence 2, `❤` + VS16 is 2, `⌚` + VS15 is 1
  (text presentation of an emoji-presentation base), a flag (two regional
  indicators) is 2, a lone regional indicator 1, `e` + U+0301 is 1. That is
  what `unicode-width` and `string-width` based TUIs compute, so Claude Code,
  lazygit and ratatui apps line up.
- **2027 off:** the cell keeps its first scalar's width; joined zero-width
  scalars change nothing, VS16 included (today's always-on VS16 widening
  becomes part of mode 2027).
- DECRQM 2027 answers 1 (set) or 2 (reset). Today it answers 3 (permanently
  set). Ghostty enables it by default through its `grapheme-width-method`
  setting (to confirm in review).

**When a join changes the width**, the cell is fixed in place:

- 1 → 2 (VS16, the second regional indicator, an emoji modifier): the next
  cell becomes the spacer (erasing a wide character there whole, as
  `widen_previous` does now) and the cursor moves one right. At the last
  column with autowrap on, the cluster moves to the start of the next row,
  leaving an edge spacer, and the row is marked wrapped; with autowrap off it
  stays narrow (this fixes E.14's "VS16 with autowrap off").
- 2 → 1 (VS15): the spacer becomes a blank in the same style and the cursor
  moves one left.

### 6.4 What each operation does to clusters

| Operation | Effect |
|---|---|
| print over a cluster cell | the write path's OR check sees kind ≠ 0 and takes the slow path, which frees the slot |
| erase (ED, EL, ECH, DECSED) | slots of cluster cells in the range are freed; erasing a whole row drops its store |
| insert/delete characters (ICH, DCH, insert mode) | cells rotate within the row and keep their slots; cells pushed off the end or blanked by `repair_wide` free theirs |
| scroll, IL/DL, region scroll, ring rotation | rows move with their stores; nothing else |
| row reset (row reused at the bottom) | `extra` dropped |
| REP (CSI b) | repeats the last printed cell's whole cluster (new slot per copy) |
| DECALN | all stores dropped |
| resize, same width | rows keep their stores |
| resize, narrower without reflow (alternate screen) | cells past the new width free their slots |
| reflow (§12) | cells are copied into the new rows; each cluster is re-inserted into its new row's store |
| push to history | the cluster's text goes inline in the stored line's text, with a boundary list (§11.4) |
| selection text, search, `dump()` | emit the cluster's text |

### 6.5 Rendering

A cluster cell's text goes to term-font's CTLine shaping (§10). Single
scalars use the cached direct lookup when the font has no ligatures.

### 6.6 Tests for clusters

- incremental segmentation = `unicode_segmentation::graphemes(true)` for
  random strings, fed whole and one byte per call;
- the cell grid after printing a string = the grid after printing the same
  string one character at a time (the `bulk_ascii` reference pattern);
- width = `unicode-width`'s string width (clamped) for every cluster in
  emoji-test.txt;
- cluster integrity invariants (§17.3).

---

## 7. kitty virtual placements: Unicode placeholders

A program places an image with `a=p,U=1` (a virtual placement: an image id,
a placement id, a size in cells, no position), then prints U+10EEEE once per
cell with:

- the image id in the **foreground colour**: 24 bits in true colour, or 8
  bits as a palette index;
- the placement id in the **underline colour** (optional);
- up to three combining diacritics from kitty's `rowcolumn-diacritics.txt`
  (297 of them): the row, the column, and bits 24–31 of the image id.

**Storage.** A placeholder is a cell of kind 2, narrow, content = row index
(bits 0–8) and column index (bits 9–17). The style holds `fg` (exactly as
sent, palette or RGB), `underline_color` and `image_id_high`. All cells of
one image share one style, so a 40×20 image costs 800 × 8 bytes and one
style, with no cluster storage.

**Missing diacritics are resolved when the cell is written**, with kitty's
left-to-right rules: with no diacritics, take the row of the placeholder to
the left and its column + 1 (and its high byte) if it has the same fg and
underline colour; with only a row diacritic, the column is the left cell's + 1
if it has the same row; with row and column, the high byte comes from the
left cell if it has the same row, column − 1 and colours. Otherwise row 0,
column 0, high byte 0. Resolving at write time matters: after reflow the
cell to the left may be on another row, or gone.

So "lossless" means: the image id (all 32 bits), placement id, row and
column of every placeholder cell survive the screen, history, reflow and
pull-back exactly. History writes them compactly (§11.4, about 52 bytes
per 40-cell image row instead of 320 as UTF-8). Copying a selection emits a
space per placeholder cell (open question §19).

**Drawing** (B.3): the renderer finds the virtual placement by (image id,
placement id) in the snapshot and draws, for each placeholder cell, the
slice of the image for (row, column), scaled to the placement's cell box.
Rows with placeholders carry `RowFlags::PLACEHOLDERS`, so the renderer
doesn't look for them elsewhere, and placeholders break shaping runs (§10).

---

## 8. Images: the placement table

### 8.1 Structures

```rust
/// One screen's images and where they are shown. The primary and the
/// alternate screen each have one, as kitty does.
pub struct Graphics {
    images: HashMap<ImageId, Image>,                  // pixels, size, kitty image number, last use
    /// Placed images, by the stable line number of their top row.
    placements: BTreeMap<(u64, PlacementKey), Placement>,
    /// kitty `U=1` placements, shown through placeholder cells only.
    virtual_placements: HashMap<(ImageId, u32), VirtualPlacement>,
    pixel_bytes: usize,                               // counted against the process budget
}

pub struct Placement {
    pub image: ImageId,
    pub id: u32,                 // kitty placement id (0 = none)
    pub line: u64, pub col: u16, // top-left cell, as a stable Point
    pub cols: u16, pub rows: u16,// size in cells
    pub offset: (u16, u16),      // pixel offset inside the top-left cell (kitty X, Y)
    pub src: [u32; 4],           // source rectangle in image pixels
    pub z: i32,                  // kitty z-index
    pub source: Source,          // Kitty, Sixel, Iterm2
    pub seq: u64,                // change counter, for the renderer
}

pub enum Layer { BelowBackground, BelowText, AboveText }
impl Placement {
    pub fn layer(&self) -> Layer {
        match self.z {
            z if z < -(1 << 30) => Layer::BelowBackground, // under non-default cell backgrounds
            z if z < 0 => Layer::BelowText,
            _ => Layer::AboveText,
        }
    }
}
```

Draw order: clear → `BelowBackground` → cell backgrounds → `BelowText` and
placeholder images → glyphs and decorations → `AboveText` → cursor overlays.
Within a layer by z, then by image id (the protocol's rule).

### 8.2 Keyed by stable line number

- **Output scrolls the whole screen:** nothing to do. The image's line
  number stays with its text as rows move up and into history.
- **Scroll regions and IL/DL** (top margin > 0 or bottom margin above the
  last row) move rows inside the region past fixed line numbers, so
  placements entirely inside the region move by the scrolled amount; those
  scrolled out of the region are clipped or dropped, and placements crossing
  a margin are left alone (the protocol says only images entirely within the
  margins scroll). O(placements on screen).
- **Freed with history blocks:** when pushing a line frees the oldest block,
  `Scrollback::push` returns the new first stored line and the terminal drops
  placements whose bottom row is above it, in the same call. Placements are
  ordered by top line, so that is a range at the front of the map.
- **Alternate screen:** its own `Graphics`. Rows scrolling off it aren't
  kept, so its placements go when their bottom row leaves the screen.
  Entering the alternate screen clears its images (protocol rule).
- **ED 2 / ED 3 / RIS:** ED 2 and RIS delete the placements visible on the
  screen, ED 3 also those in history (protocol: "the clear screen escape code
  should also clear all images").
- **Reflow:** the anchor (line, col) is remapped like every other on-screen
  position (§13); the size in cells stays.
- **Image data:** an image is freed when no placement, virtual placement or
  placeholder needs it and the program deletes it, or by least-recently-used
  eviction when the process-wide budget (64 MB default, B.3) is exceeded.
  Pixels held on the CPU or in GPU textures both count.

### 8.3 Sixel and iTerm2 as placements

- **Sixel** (`DCS P1;P2;P3 q … ST`, B.5): the decoder streams into an RGBA
  buffer; on the end of the string it becomes an image plus one placement at
  the cursor, `source = Sixel`, layer `BelowText`; the cells it covers are
  erased first (so no stale text shows through transparent pixels) and the
  cursor moves as sixel scrolling mode says.
- **iTerm2** (`OSC 1337 ; File=…:<base64> BEL`, B.3): same, `source =
  Iterm2`, size from `width`/`height` (cells, pixels, percent or auto).
- Both then behave like kitty placements: scroll with text, freed with
  history, remapped on reflow.

Writing text over a Sixel/iTerm2 image draws the text over it rather than
punching the image out (xterm overwrites the pixels; foot splits the image):
open question §19.

---

## 9. Change counters and the frame snapshot

### 9.1 Per-row change counters

The terminal keeps `counter: u64` and `batch_start: u64`. At the start of
each `advance` (and of resize, reset and other mutating calls)
`batch_start = counter + 1`. Any change to a row first does

```rust
#[inline]
fn touch_row(&mut self, r: usize) {
    let row = &mut self.screen.rows[r];
    if row.seq < self.batch_start {
        self.counter += 1;
        row.seq = self.counter;
    }
}
```

so a row's `seq` is unique to (row, parse call): one compare per row
segment written, not per cell. Things that don't change a row's content keep
its `seq`: full-screen scroll, region scroll, IL/DL (the row moved but its
cells didn't). The renderer can therefore key its row cache by `seq` alone
and reuse cached instances for a row that only moved. Resize and reflow give
every screen row a new `seq`. History rows never change; the renderer keys
them by the stable position of their first cell and drops its whole cache
when the width changes.

Not row content, and tracked separately: palette changes (OSC 4/104/10–12,
`palette_seq`), DECSCNM reverse video, cursor, selection, blink phase, the
hovered link. The global `generation` stays as the "redraw needed" signal.

WezTerm does the same with a per-line sequence number (`Line::seqno`,
`changed_since`).

### 9.2 What the snapshot holds

Taken by `Terminal::snapshot(&self, out: &mut Snapshot, have: impl Fn(RowKey) -> bool)`
under the lock; `have` asks the renderer's row cache whether it already has
a row, so only missing rows are copied. Every buffer is owned by the
renderer and reused, so a snapshot allocates nothing in steady state.

```rust
pub struct Snapshot {
    pub cols: u16, pub lines: u16,
    pub counter: u64,                     // the terminal's counter when taken
    pub rows: Vec<SnapRow>,               // one per visible row, top to bottom
    pub cells: Vec<Cell>,                 // cells of the rows copied this time, back to back
    pub cluster_text: Vec<u8>,            // cluster text of those rows
    pub cluster_spans: Vec<(u32, u16)>,   // per copied row: slot → (offset, len) into cluster_text
    /// The screen's style vector (all ids, 20 B each; usually < 4 KB), then
    /// the styles of any history rows in view (their cells' ids are offset
    /// past the screen's).
    pub styles: Vec<Style>,
    pub cursor: SnapCursor,               // view row/col, shape, visible, blinking, on a wide cell
    pub palette: Option<Box<Palette>>,    // only when palette_seq changed: 256 colours + fg/bg/cursor
    pub reverse_video: bool,
    pub placements: Vec<SnapPlacement>,   // placements intersecting the view, in draw order:
                                          // layer, view rect in cells, pixel offset, source rect,
                                          // image id, image version
    pub virtual_placements: Vec<SnapVirtual>, // those referenced by placeholders in view
    pub new_images: Vec<(ImageId, u64, Arc<Pixels>)>, // images the renderer hasn't uploaded:
                                          // the pixels are shared, not copied
}

pub struct SnapRow {
    pub key: RowKey,                      // Screen(seq) or History { line, col } (stable first cell)
    pub flags: RowFlags,                  // wrapped, placeholders, blinking, from history
    pub cells: Option<Range<u32>>,        // None: the renderer already has this row
    pub clusters: Range<u32>,             // into cluster_spans
    pub selected: Option<Range<u16>>,     // selected columns, worked out under the lock
}
```

Selected columns are worked out under the lock because a history row shown
at a different width can span two stored lines, and the renderer has no way
to map its cells back to stable positions; it's O(visible rows).

Costs under the lock: a full 300×100 change copies 240 KB of cells plus the
style vector, ~15–25 µs; a spinner copies one row (2.4 KB), < 1 µs; scrolled
back, history rows come from the R.3 decode cache. The gradient case copies
up to 45k styles (900 KB), ~60 µs: acceptable for that case, measured in
R.1. Building instances, shaping and rasterising then run outside the lock.
Until R.1 lands, the renderer builds from the snapshot while still holding
the lock, so the switch over is a move of one call.

---

## 10. Shaping runs, ligatures, glyph keys (term-font and term-render)

### 10.1 Runs

The renderer splits each row into runs. A run is the longest stretch of
cells that:

- are drawn by fonts (not code-drawn: box drawing U+2500–257F, blocks
  U+2580–259F, Powerline U+E0A0–E0D7, braille U+2800–28FF, the legacy
  computing blocks; and not kitty placeholders), and
- select the same font face: bold × italic (4 faces).

Colour, background, underline, strike, inverse, links, the cursor and the
selection never split a run, so `=>` in two colours, or with the cursor on
`>`, is still one ligature. Spacer cells contribute no text. Blank cells
are part of the run (as spaces).

### 10.2 Shaping

term-font gets a **font registry**, because CTLine chooses fallback fonts
itself (CJK, emoji, symbols, the user's cascade list from E.4):

```rust
pub type FontId = u16;
/// Every font face a glyph has come from: the four configured faces first,
/// then fallbacks as CTLine picks them. Looked up by CFHash + CFEqual of the
/// CTFont, so the same face from different lines gets one id.
pub struct FontRegistry { faces: Vec<Face>, by_font: HashMap<CfKey, FontId> }
pub struct Face { font: CFRetained<CTFont>, color: bool }

/// Names one rasterised bitmap.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font: FontId,   // SPRITE_FONT (u16::MAX) for code-drawn glyphs
    pub glyph: u16,     // CGGlyph, or a SpriteId
    pub cells: u8,      // 1 or 2: colour glyphs are fitted to the cells they cover
    pub variant: u8,    // spare (synthetic bold, a later subpixel phase); 0 today
}

pub struct ShapedGlyph {
    pub cell: u16,      // first cell the glyph belongs to
    pub cells: u8,      // cells it covers: 1, 2 for wide, n for an n-cell ligature
    pub key: GlyphKey,
    pub dx: i16, pub dy: i16, // pixel offset from that cell's origin (combining marks)
}
```

`Fonts::shape(text: &[u16], face, features, out: &mut Vec<ShapedGlyph>)`
builds a CTLine from the run's text with the face and `font_features`,
walks its CTRuns (each run's font goes through the registry), and maps each
glyph's string index back to its cell through a UTF-16-index-to-cell table
built with the run text. Glyphs are placed on the cell grid (each at its
cell's x plus its offset inside its cluster), not at Core Text's accumulated
advance, so a fallback font with a different advance can't drift.

**Fast path:** when the configured font has no ligature features turned on
(Menlo, SF Mono) or ligatures are off, ASCII cells use a 128-entry
(face, byte) → glyph table and other single scalars a cached cmap lookup
with Core Text's fallback for that one character (what `glyph_for` does
today). CTLine is then used only for cluster cells. With a ligature font
(JetBrains Mono, Fira Code), rows are shaped and the result cached by the
row's `seq` (B.1). Both paths produce the same `GlyphKey`s.

### 10.3 Ligatures: one glyph, one instance per cell

Most coding fonts implement `=>` with contextual alternates (one glyph per
cell), which needs nothing special. A true multi-cell ligature glyph
covering cells `a..b` is emitted once **per cell**: the same texture rect,
with each quad clipped on the CPU to its cell's columns (x, w and u
adjusted), the first piece open to the left and the last to the right so
overhangs still draw, each piece in its own cell's foreground colour. No
instance-format or shader change, and selection/cursor colours apply per
cell.

### 10.4 Code-drawn glyphs: sprite ids

`term_font::sprite`, pure Rust: draws box drawing, blocks, Powerline,
braille, legacy computing shapes, and the curly/dotted/dashed underline
patterns, into coverage bitmaps at the current cell size. Each has a
`SpriteId(u16)` and is cached under `GlyphKey { font: SPRITE_FONT, glyph:
sprite }` in the same mask texture as text. F.6 fills in the shapes; F.3
uses the underline ones.

### 10.5 Decorations

Single and double underline, strike and overline stay rectangles; curly,
dotted and dashed underlines are one sprite quad per cell. All take
`underline_color`, or the foreground when it is default.

---

## 11. Scrollback v2

### 11.1 Physical and logical lines

A **stored line** is one physical line exactly as it was written: the row's
cells at the width of the time, and whether it wrapped. A **logical line**
is a run of stored lines where all but the last are marked wrapped. History
is never rewritten when the window changes width: rows at another width are
worked out from logical lines when needed (§12.3). Every stored line keeps
its stable line number (§13) for as long as it is stored.

### 11.2 Blocks

```rust
/// About 64 KB of stored lines, freed whole.
struct Block {
    data: Vec<u8>,          // stored lines; never above 65,536 bytes
    starts: Vec<u16>,       // offset of each stored line in `data`
    first: u64,             // stable line number of the first stored line
    styles: Vec<u8>,        // the block's style dictionary, encoded (§11.5); id 0 = default, implicit
    style_starts: Vec<u16>,
    links: Vec<u8>,         // the block's hyperlinks: URI + id parameter, encoded
    link_starts: Vec<u16>,
    width: u16,             // width every line here was written at, 0 if mixed
    rows_at: Cell<(u16, u32)>, // (width, rows the logical lines starting here take) — lazy
    head_continues: bool,   // the first stored line continues a logical line from the block before
}
```

- A block is sealed when the next line would take `data` past 64 KB, so
  offsets always fit `u16` (today's are `u32`: 2 bytes saved per line) and
  the open block never holds more than 64 KB of capacity (today's doubling
  can reach 128 KB).
- A stored line can't exceed 64 KB: at most `cols` cells, and cluster text is
  capped at 8 KB per row.
- The encoder keeps a `HashMap<Style, u32>` and one for links for the open
  block only, dropped on sealing.
- `pop()` removes the newest stored line (for pulling lines back, §12.2):
  truncate `data`, pop `starts`, drop the block if empty. Dictionary entries
  it leaves unused are freed with the block.
- Limits: `scrollback_limit` in lines as today, plus a byte cap (default
  32 MB, open question §19) as a backstop for gradient-heavy output. Both
  free whole blocks from the oldest end.

### 11.3 A stored line, byte by byte

```text
line   := flags:u8  [cells:varint]  [tlen:varint]  text  [runs] [clusters] [widths] [placeholders] [marks]

flags  bit 0  WRAPPED       the logical line continues on the next stored line
       bit 1  EDGE          the last column was an edge spacer (a wide character moved to the next row)
       bit 2  CELLS         a cell count follows (only when it can't be worked out, see below)
       bit 3  STYLED        runs follow
       bit 4  CLUSTERS      a cluster list follows
       bit 5  WIDTHS        width exceptions follow
       bit 6  PLACEHOLDERS  a placeholder list follows
       bit 7  MARKS         prompt marks follow

tlen   present when any of bits 3–7 is set; otherwise the text runs to the end of the line
text   UTF-8, one entry per cell up to the last cell that isn't a blank space:
         a scalar cell → the scalar; a cluster cell → all its scalars;
         a blank cell before that point → 0x20; a placeholder cell → 0x20;
         spacers → nothing
runs   count:varint, then per run  varint(cells << 3 | slot)  [index:varint if slot == 7]
         slot 0–6: the block's style #slot directly (0 = default); 7: the index follows
         runs count columns (a wide cell's two halves are in the same run);
         cells after the last run are default style; cells after the text but inside
         the runs are blank cells in the run's style (this is how trailing fill is stored)
clusters  count:varint, then per cluster  gap:varint (scalars since the previous
         cluster ended)  extra:varint (scalars after the first that belong to it)
widths count:varint, then per exception  gap:varint (cells)  width:u8
         only cells whose width isn't what the table and the 2027 rule give
         (e.g. a cluster written with 2027 off)
placeholders  count:varint, then per stretch  gap:varint (cells)  n:varint  row:varint
         col:varint — n cells with this row and columns col, col+1, …
marks  count:u8, then per mark  kind:u8 (1 prompt A, 2 input B, 3 output C,
         4 end D)  col:varint  [status:varint for D with an exit status]
```

The cell count is the larger of what the text, the runs and the
placeholders cover (plus one for EDGE). `CELLS` is only written for a
wrapped line whose last cells are blank default cells (they matter when
the logical line is joined back together); unwrapped lines drop trailing
blanks, as today. A typed space and an empty cell both come back as an
empty cell, as today.

The decoder takes widths from the same table and 2027 rule as the writer
(history lives only as long as the process, so the table can't change
underneath it), and writes exceptions for anything else. That is what
removes today's per-line "wide" runs from CJK text.

### 11.4 Worked examples (the `scrollback_mem` inputs, 160 columns)

"Today" is the current format: sealed bytes per line, then what
`scrollback_mem` measures (which includes the open block's spare capacity).
v2 counts sealed bytes per line, the 2-byte offset included.

**Plain** `00012345 the quick brown fox jumps over` (39 cells):

```text
00                      flags: nothing follows the text
30 30 30 31 32 … 72     text, 39 bytes
                        = 40 bytes + 2 (offset) = 42
today: 00 flags, 27 text length, 39 text, 00 runs, + 4 offset = 46 (52 measured)
```

**`ls --color`** `dir12345  build.sh  notes.txt  src  archive.tar` (47 cells;
`dir…` and `src` bold blue, `build.sh` green, `archive.tar` red):

```text
08                      flags: STYLED
2F                      tlen = 47
64 69 72 31 … 72        text, 47 bytes
07                      7 runs
41                      8 cells,  slot 1 (bold blue)     8<<3|1 = 65
10                      2 cells,  slot 0 (default)
42                      8 cells,  slot 2 (green)
68                      13 cells, slot 0                 13<<3 = 104
19                      3 cells,  slot 1
10                      2 cells,  slot 0
5B                      11 cells, slot 3 (red)           11<<3|3 = 91
                        = 57 bytes + 2 = 59   (today 86 sealed, 93 measured)
block dictionary, once per block: #1 41 04 01 (fg palette 4, attrs bold)
                                  #2 01 02    #3 01 01
```

**Full-width background** `ESC[48;5;22m+12345` padded to 159 cells:

```text
08                      flags: STYLED
06                      tlen = 6
2B 31 32 33 34 35       text "+12345" (the 153 trailing spaces are not stored)
01                      1 run
F9 09                   159 cells, slot 1                159<<3|1 = 1273
                        = 11 bytes + 2 = 13   (today 174 sealed, 183 measured)
dictionary: #1 04 16 (bg palette 22)
```

**Compiler error** (`error[E12345]` bold red 13 cells, `: mismatched
types` bold 18 cells, ` --> src/main.rs:12345:5` default 24 cells):

```text
08 37 <55 bytes of text> 02 69 92 01
   runs: 13<<3|1 = 105 (69); 18<<3|2 = 146 (92 01); the default tail needs no run
                        = 61 + 2 = 63   (today 74 sealed, 81 measured)
```

**CJK** `12345 漢字かなカナ한글 混合テキスト 中文字符` (26 scalars, 62 bytes of
UTF-8, 44 cells):

```text
00 <62 bytes of text>   widths come from the table: no runs, no exceptions
                        = 63 + 2 = 65   (today 87 sealed, 90 measured: today's
                                         encoding needs a "wide" run per stretch)
```

**Emoji-heavy** (new) `12345 ✅ done 👍🏽 ship 🇬🇧 ❤️ 👨‍👩‍👧 ok` (35 scalars,
66 bytes; four clusters):

```text
10                      flags: CLUSTERS
42                      tlen = 66
<66 bytes of text>
04                      4 clusters
0D 01                   👍🏽: starts 13 scalars in, 1 more scalar
06 01                   🇬🇧
01 01                   ❤️ (with VS16)
01 04                   👨‍👩‍👧: 4 more scalars (ZWJ, 👩, ZWJ, 👧)
                        = 77 + 2 = 79   (today: not stored correctly)
```

**A row of a kitty placeholder image** (40 cells, row 3, image in style
slot 1):

```text
48 28 <40 × 20> 01 C1 02 01 00 28 03 00
   flags STYLED|PLACEHOLDERS, tlen 40, 40 spaces, 1 run (40 cells, slot 1),
   1 stretch: gap 0, 40 cells, row 3, columns from 0
                        = 50 + 2 = 52   (as UTF-8 with diacritics: 320+)
```

**A prompt line with OSC 133 marks** (A at column 0, B at column 8): `MARKS`
adds `02 01 00 02 08`, 5 bytes.

**Gradient** (every cell a different RGB foreground, 160 cells): per cell
1 byte of text + a 3-byte run (slot 7 + 2-byte index) + a 4-byte dictionary
entry = 8 bytes; ~1,280 bytes per line (today ~1,120). Slightly worse than
today, which is why the byte cap exists.

| Input | Today sealed | Today measured | v2 | Change |
|---|---|---|---|---|
| plain 40 chars | 46 | 52 | **42** | −9% |
| `ls --color` | 86 | 93 | **59** | −31% |
| compiler error | 74 | 81 | **63** | −15% |
| full-width background | 174 | 183 | **13** | −93% |
| CJK | 87 | 90 | **65** | −25% |
| emoji-heavy | – | – | **79** | (today lossy) |
| gradient, 160 cells | ~1,120 | – | ~1,280 | +14% |

The measured figures will add ~3–6 bytes per line for the open block, as
today. `scripts/bench-limits.json` gets lower limits with the v2 PR
(suggested: plain 50, `ls` 70, compiler 75, full-width background 25, CJK
75, plus emoji 95 and a placeholder row 60).

### 11.5 The per-block style dictionary and link table

```text
style := head:u8 [fg] [bg] [ul] [attrs:varint] [more:u8 [underline:u8] [link:varint] [image_high:u8]]
head   bits 0–1 fg kind, 2–3 bg kind, 4–5 underline-colour kind (0 default, 1 palette, 2 RGB),
       bit 6 attrs follow, bit 7 `more` follows (bit 0 underline style, 1 link, 2 image id high byte)
colour palette → 1 byte; RGB → 3 bytes
```

Bold blue is `41 04 01`, green `01 02`, a background-only palette colour
`04 16`, an RGB foreground `02 rr gg bb`. `link` is an index into the
block's own link table (URI and id parameter), which goes when the block
goes. Decoding a line decodes only the styles its runs use.

### 11.6 Prompt marks

On the screen, marks live in the row's `extra` (`kind`, `col`, exit status
for D), up to three per row (A and B usually share the prompt's row; D of
the previous command may too). They move with their cells on reflow and go
into history with the line (`MARKS`). "Jump to the previous prompt" scans
stored lines' flags bytes, which is cheap: one byte per stored line in the
block, no decoding.

### 11.7 Image references

Placements are not stored in the history bytes: they live in the screen's
placement table, keyed by stable line number, and are freed in the same
call that frees a block (§8.2). Placeholder cells, which are text, are
stored in the line (§11.3).

---

## 12. Reflow on resize

### 12.1 The rule

The **screen** is reflowed eagerly, on every resize event. **History is
never rewritten**; stored lines keep their width and numbers, and rows at
the current width are worked out from logical lines only when something
reads them. The alternate screen is not reflowed (programs redraw it on
SIGWINCH), as in xterm, kitty and Ghostty; while it is active, the primary
screen's reflow waits until the program leaves it, so a resize during Vim
reflows the primary screen once.

### 12.2 Reflowing the screen

```text
resize(cols, lines), primary screen:
  same width:
    shrink: drop blank rows below the cursor, then move rows from the top into history
    grow:   pull whole logical lines back from history, newest first, while they fit;
            the cursor moves down with them; then add blank rows at the bottom
  new width:
    1. if the first screen row continues a logical line from history, pull that line's
       stored lines back (up to `lines` × 4 of them; older ones stay stored)
    2. note every tracked on-screen position as (logical line, cell offset): cursor,
       saved cursors, selection ends, search hits, placement anchors, the view position
    3. re-split each logical line at the new width: copy cells, re-insert clusters into
       the new rows' stores, move prompt marks with their cells, put an edge spacer
       where a wide character doesn't fit; trim trailing blank cells, but never before
       the cursor
    4. if there are more rows than `lines`, move rows from the top into history (as
       stored lines at the new width) until the cursor row fits; if fewer, pull more
       logical lines back from history (as in "grow")
    5. turn the noted positions back into Points; every screen row gets a new seq
```

Pulling back: `Scrollback::pop()` stored lines of the newest logical line,
decode, re-split at the current width. Only whole logical lines are pulled,
so nothing is split oddly; the newest one is skipped if it wouldn't fit.
This makes `resize_grow_height_pulls_from_scrollback` pass.

When step 1 hits its limit (one logical line longer than four screens, e.g.
the "one 50 MB line" pressure test), the older part stays stored and the
join shows one short row when scrolled back: accepted.

### 12.3 History at another width

Rows at the current width are worked out per logical line: an ASCII line
of `n` cells takes `max(1, ⌈n / cols⌉)` rows (n comes from the stored
line's text length, no decoding); lines with wide characters are walked to
place edge spacers (one width lookup per scalar). Each block caches the
row count of the logical lines starting in it, for one width at a time
(`rows_at`). Blocks whose lines were all written at the current width take
one row per stored line with no work.

The view keeps its position as a stable Point: `ViewTop::Bottom` (following
output) or `ViewTop::At(Point)`, the stored line and column of the first
cell shown. Drawing walks forward `lines` rows from there: O(lines),
whatever the history size. Scrolling walks from there. Output arriving
underneath changes nothing (today `display_offset` is bumped per pushed
line). Scroll-to-top and select-all start from the first stored line and
need no count. Only `display_offset()` as a number (the test dump, a future
scrollbar, which the zero-UI goal rules out) needs the count from the
bottom, and works it out from the block caches.

### 12.4 The 120 Hz, 100k-line budget

Per resize event at 300×100 with 100k lines of history:

| Step | Cost |
|---|---|
| screen re-split: 30k cells | ~0.1–0.2 ms |
| pull back ≤ 100 stored lines (decode) | ~50 µs |
| push back ≤ 100 rows (encode) | ~50 µs |
| tracked positions | < 10 µs |
| history | **0** (nothing rewritten; block caches checked lazily) |
| drawing while scrolled back: decode ≤ 100 logical lines | ~0.1 ms |
| frame rebuild (every row new: width changed) | ~0.05 ms CPU (today's full build) |

About 0.5 ms, well inside 8 ms, so no debounce is needed for the terminal
itself. The session still sends at most one SIGWINCH per frame. Working out
the full count after a width change, only for `display_offset()`, costs
~1 ms for 100k ASCII lines and ~10 ms for 100k CJK lines, once per width.

---

## 13. Points, selection and search

### 13.1 What a Point means

`Point { line: u64, col: usize }` keeps its shape. `line` is the number a
row got when it was written (counting every row that ever scrolled off),
and `col` is its column in that row as written. That is today's meaning,
made exact for reflow:

| Event | On-screen Points | History Points |
|---|---|---|
| output scrolls | unchanged (the number moves up with its row) | unchanged |
| text overwritten or erased | unchanged (the text under it changes) | — |
| region scroll, IL/DL | unchanged; placements inside the region move (§8.2) | — |
| a block is freed | — | points above the first stored line are gone: selections are clipped, search hits dropped, the view moves to the first stored line |
| resize, same width | unchanged; pulled-back lines keep their numbers | unchanged |
| resize, new width | remapped in the same call (§12.2) | **unchanged**: stored lines are never renumbered |
| alternate screen | numbered after the primary's, so the two can't be confused (as today) | unchanged |
| ED 3, RIS | history cleared | cleared |

Numbers are never reused while their line is stored. A reflow gives the
re-split screen rows numbers following the end of history, so a number that
belonged to an old screen row or a pulled-back line can be handed to
different text; that is safe because everything in term-core that holds an
on-screen Point (cursor, saved cursors, selection, search hits, placements,
the view position, the grapheme state) is remapped or cleared in the same
call, and the renderer keys screen rows by change counter, not line number.
The app holds no Points across calls.

Ordering is text order: (line, col) sorts by stored line, and the stored
lines of one logical line have consecutive numbers.

### 13.2 Selection

`Selection { anchor: Point, head: Point }` plus a kind (normal, word, line,
rectangular). Today it is cleared on resize; with this model it survives
reflow (screen ends remapped, history ends unchanged). Rectangular
selections are in view rows and columns and have no meaning after reflow:
they are cleared. `selection_text()` walks logical lines between the two
points and joins wrapped stored lines without newlines, so copying a wrapped
line gives one line at any width. Clusters copy as their text.

### 13.3 Search (Cmd+F, E.7)

- Searches logical lines: stored lines' text is already UTF-8, so a
  case-sensitive search is `memchr::memmem` over each line's text (and over
  the joined text for wrapped lines, padding back elided blanks from the
  stored cell count), with no cell decoding. Case-insensitive: ASCII fast
  path, Unicode lower-casing otherwise.
- Runs in slices under the lock, one block (64 KB) per hold, ~15 µs each,
  so output and typing never wait behind it; 5 MB of history (100k lines)
  is ~80 slices.
- A match's byte offset becomes a Point by walking its line's scalars with
  widths and the cluster list.
- Hits are stored as Point ranges: history hits stay valid through output,
  scrolling and reflow until their block is freed; screen hits are searched
  again when their row's `seq` changes, and remapped on reflow.
- Placeholder cells (spaces in the text) never match.

---

## 14. Parser support (vendored vte, after N.1 merges)

The vendored copy already has `print_ascii` and the UTF-8 split fix. Its
state machine sends SOS, PM and APC (`ESC X`, `ESC ^`, `ESC _`) to one
ignoring state, passes DCS through `hook`/`put`/`unhook` one byte at a time,
and with the `std` feature buffers OSC strings in an unbounded `Vec`.
Changes, each with tests in `vendor/vte` and listed in its README:

```rust
pub trait Perform {
    // … existing: print, print_ascii, execute, hook, put, unhook, osc_dispatch, csi_dispatch, esc_dispatch

    /// `ESC _`: an application program command starts (kitty graphics: `ESC _ G …`).
    fn apc_start(&mut self) {}
    /// Part of the command, in order, as many calls as needed.
    fn apc_put(&mut self, _data: &[u8]) {}
    /// It ended: `complete` for ST (`ESC \`), false if abandoned (CAN, SUB, or ESC + anything else).
    fn apc_end(&mut self, _complete: bool) {}

    /// Several bytes of a device control string at once. Default: `put` per byte.
    fn put_slice(&mut self, data: &[u8]) { for &b in data { self.put(b) } }

    /// An OSC string has passed 4 KB; `prefix` is what has arrived. What to do with the rest.
    fn osc_large(&mut self, _prefix: &[u8]) -> OscLarge { OscLarge::Buffer(64 * 1024) }
    /// After `OscLarge::Stream`: the rest of the string, in pieces, then its end.
    fn osc_put(&mut self, _data: &[u8]) {}
    fn osc_end(&mut self, _complete: bool) {}
}

pub enum OscLarge {
    /// Keep buffering up to this many bytes in all; past it the OSC is dropped, not dispatched.
    Buffer(usize),
    Stream,
    Drop,
}
```

- **APC:** a state of its own (SOS and PM stay ignored). In it, bytes are
  handed over in slices found with `memchr3` for ESC, CAN and SUB, so a
  multi-megabyte kitty payload costs a few calls, not one per byte.
- **DCS:** `put_slice` from the passthrough state the same way. term-core
  dispatches on `hook`: final `q` with no intermediates is Sixel (streamed
  into the decoder), `+q` is XTGETTCAP and `$q` DECRQSS (F.5; buffered,
  ≤ 4 KB).
- **OSC size policy** (term-core's answers to `osc_large`):
  `1337;File=` → `Stream` into the iTerm2 image reader (cap: the image
  budget, 64 MB of base64); `1337;FilePart=` chunks → `Buffer(1 MB)` each;
  `52;` (clipboard) → `Buffer(4 MB)`; anything else → `Buffer(64 KB)`.
- **kitty graphics in term-core:** control keys up to the `;`, ≤ 4 KB;
  payload base64-decoded as it streams into the image being received;
  chunked transmissions (`m=1`) accumulate up to the per-image cap (64 MB
  raw, also bounded by the budget); pixel caps width, height ≤ 10,000 and
  width × height × 4 ≤ the budget; file and temp-file transmission limits
  as in B.3. Replies (`OK`/errors) unless `q=1`/`q=2`.
- **Sixel caps:** the same pixel caps, repeat counts clamped to the image
  width, ≤ 1024 palette registers.
- Fuzz targets for each (B.3, B.5).

A later, optional change: hand non-ASCII stretches over whole as
`print_str(&str)` (vte already has the `&str`), split at C1 controls, so
term-core can segment and write CJK text a row segment at a time (Unicode
input is at 131 MB/s against 423 for ASCII). Not needed for correctness.

---

## 15. Hot paths: cost per operation

| Operation | Today | This model |
|---|---|---|
| ASCII run of n bytes in one row segment | n × 12-byte stores, OR over the cells | n × 8-byte stores (`template \| byte`), OR over n `u64`s, one `touch_row` |
| non-ASCII scalar | `unicode-width` lookup, store | one table lookup (width + break class), O(1) break check, store; a join appends to the cluster, amortised O(1) |
| SGR | edit the pen | edit the pen's `Style`, clear its cached template |
| first print after SGR | — | intern: ~5 ns hit, ~15–20 ns miss |
| erase n cells | fill 12-byte cells | fill 8-byte template; free cluster slots in the range only if the row has clusters |
| full-screen scroll | ring rotation, encode one row, memset | same; encoding adds one dictionary lookup per run |
| region scroll, IL/DL | rotate rows in the region | same, plus placements inside the region |
| ICH/DCH | rotate within the row | same; free slots of cells pushed off |
| sweep | — | O(cells), at most once per cells/2 new styles |
| push to history | O(cells) | O(cells) + O(runs) |
| resize, new width | O(screen), truncating | O(screen + pulled lines); history untouched |
| snapshot | (frame built under the lock, O(cells)) | O(changed rows × cols) memcpy |

---

## 16. Memory budget

### 16.1 Per structure

| Structure | 80×24 | 300×100 | Notes |
|---|---|---|---|
| cells, per screen | **15.0 KB** (today 22.5) | **234 KB** (today 352) | the alternate screen only once used (R.8) |
| row headers (48 B) | 1.1 KB | 4.7 KB | |
| style table, plain / coloured | < 1 KB / ≤ 1 KB | < 1 KB / ≤ 1 KB | |
| style table, TUI with a colour scheme | ≤ 18 KB | ≤ 18 KB | |
| style table, worst (every cell different) | 104 KB | 1.6 MB | transient, given back after |
| cluster stores, plain / coloured / CJK | 0 | 0 | CJK characters are single scalars |
| cluster stores, emoji-heavy (8 or 30 clusters per row) | ~6 KB | ~70 KB | ~96 B per row with clusters + ~20 B per cluster |
| cluster stores, worst (8 KB cap per row) | ~200 KB | ~1 MB | combining-mark floods only |
| link table | ~100 B per link | | bounded by the styles |
| placement table | ~100 B per placement | | images' pixels count against the 64 MB process budget |
| snapshot buffers (renderer, reused) | 15 KB + styles | 234 KB + styles | styles usually < 4 KB |
| renderer row cache (R.2) | ≤ 46 KB | ≤ 720 KB | same as today's frame vectors |
| shaping cache (ligature fonts only) | ≤ 23 KB | ≤ 360 KB | 12 B per glyph |
| font registry | ~100 B per face + Core Text's own | | tens of faces |
| Unicode table | 60–80 KB read-only, shared | | paged in on use |

### 16.2 History

Sealed bytes per line from §11.4, plus per block ~200 bytes of header and
dictionary (~0.2 B per line) and one open block (≤ 64 KB) per terminal.

| Content | B/line | 10k lines | 100k lines | Today, 10k (measured) |
|---|---|---|---|---|
| plain | 42 | 0.42 MB | 4.2 MB | 0.52 MB |
| coloured (`ls`) | 59 | 0.59 MB | 5.9 MB | 0.93 MB |
| compiler errors | 63 | 0.63 MB | 6.3 MB | 0.81 MB |
| full-width background (delta, status lines) | 13 | 0.13 MB | 1.3 MB | 1.83 MB |
| CJK | 65 | 0.65 MB | 6.5 MB | 0.90 MB |
| emoji-heavy | 79 | 0.79 MB | 7.9 MB | – |
| gradient, 160 cells | ~1,280 | 12.8 MB | capped at 32 MB | ~11 MB |

R.7 (idle LZ4 of sealed blocks, "100k lines of `ls`/log output ≤ 3 MB")
starts from 5.9 MB instead of 9.3; style indices in place of inline RGB
compress better too.

### 16.3 Whole terminal state, worked

| Case | Screen + tables | History | Snapshot + render caches | Total |
|---|---|---|---|---|
| 80×24, plain, 10k lines | 17 KB | 0.42 MB + ≤ 64 KB | ~60 KB | **~0.55 MB** |
| 80×24, CJK, 10k lines | 17 KB | 0.65 MB | ~60 KB | ~0.8 MB |
| 300×100, coloured, 10k lines | 240 KB | 0.59 MB | ~1 MB | ~1.9 MB |
| 300×100, emoji-heavy, 100k lines | 310 KB | 7.9 MB | ~1.1 MB | ~9.3 MB |
| 300×100, gradient TUI, 10k lines | 1.9 MB (peak) | 12.8 MB | ~2 MB | ~17 MB |

For the 10k-line threshold this is noise next to the drawables (13 MB) and
the frameworks; the point is that history stays small at 100k lines and that
no case keeps full cells for history (Ghostty's 10k lines at 160 columns are
~13 MB of cells).

---

## 17. Migration plan

### 17.1 Order of PRs

Agents work in worktrees, one crate each at a time (roadmap rule). The
core agent's PRs are in order; the font and render agents run alongside.

| # | Crate | PR | Size | After |
|---|---|---|---|---|
| 0 | core, vte | N.1 parser work merged | — | — |
| 1 | core + render + app | `refactor(core)!: read cells through RowRef/CellRef and a Style value`: content (empty, char, cluster text, placeholder), width, `&Style`; implemented on today's 12-byte cells; `Style`, `Underline`, `Attrs` defined; every caller moves over (frame.rs, conformance/term/props tests, pipeline test). No behaviour change. Main session, before the agents start | S | 0 |
| 2 | vte | APC state, `put_slice`, OSC size policy (§14), no-op defaults | S | 0 |
| 3 | core | Unicode table + incremental segmentation; width lookups move to the table (a test checks it equals `unicode-width` for every scalar); clusters not stored yet | M | 1 |
| 4 | core | **8-byte cells and the per-screen style table**; SGR 4:x, 21, 53/55, 58/59 stored; erase style; sweep; `check_invariants` extended. History is still v1 and drops the new fields (a test marks the loss; #6 flips it) | M | 3 |
| 5 | core | **clusters per row and mode 2027** (VS15/VS16, regional indicators, ZWJ, legacy mode); selection text and `dump()` show clusters; v1 history keeps the base scalar | M | 4 |
| 6 | core | **scrollback v2** (§11) at the written width only; `pop()`; byte cap; `scrollback_mem` gains emoji, placeholder and sealed figures; bench limits lowered | L | 5 |
| 7 | core | row change counters + `Snapshot` (§9) | S | 4 |
| 8 | core | OSC 8 link table and OSC 133 prompt marks, on the screen and in v2 | S | 6 |
| 9 | core | kitty placeholders (cell kind, diacritic table, write-time resolution, v2 list) | S | 6 |
| 10 | core | **reflow** (F.4): stable Points, view position, pull-back, history at any width; the two ignored resize tests decided (§19) | L | 6, 7 |
| 11 | core | placement table (§8) for B.3/B.5 | M | 10 |
| F2.1 | font | font registry, `GlyphKey`, rasterise by (font, glyph); `rasterize(char, style)` kept as a wrapper | M | 1 |
| F2.2 | font | CTLine shaping of runs (`shape()`); clusters now, ligatures in B.1 | M | F2.1 |
| F2.3 | font | `sprite`: underline patterns (curly, dotted, dashed); box/block/Powerline/braille shapes are F.6 | M | F2.1 |
| F3.1 | render | resolve through `Style`: underline colour, double underline, overline, strike | S | 1 |
| F3.2 | render | glyph cache keyed by `GlyphKey`, runs (§10.1), cluster cells via F2.2, curly/dotted/dashed via F2.3 | M | 5, F2.2, F2.3 |
| F3.3 | render | build frames from `Snapshot` (still under the lock; R.1 moves it out) | S | 7 |

### 17.2 Existing tests that carry over

| Tests | What happens |
|---|---|
| `term-core/tests/conformance.rs` (xterm behaviour per sequence) | all carry over; the 62 places that read `flags::WIDE`, `.fg`, `.bg` move to `CellRef` in PR 1; the two `#[ignore]` resize tests are decided in PR 10 |
| `term-core/tests/props.rs` (invariants under random input and resizes; scrollback round trips; bulk ASCII = one char at a time) | carry over; the round trip becomes exact in PR 6 (styles, clusters, marks, placeholders); new generators below |
| `term-core/tests/selection.rs`, `term.rs` | carry over; selection-across-reflow tests added |
| `term-core/tests/fuzz.rs`, `fuzz_corpus.rs`, `fuzz/fuzz_targets/parse.rs` | carry over; `read_back` reads every row at the current width (today it can't, history isn't re-wrapped) |
| `cell.rs` unit tests | rewritten for the new layout (bit positions, blank = 0, SE) |
| `scrollback.rs` unit tests | varint tests carry over; the parser branch's "fast encoder = reference encoder" test is kept until PR 6 replaces the format, then becomes "v2 decode of v2 encode = the row" |
| `term-render/tests/frame.rs` | carry over (they build frames from `Term::advance`); the fake glyph source takes a `GlyphKey` |
| `term-font/tests/font.rs` | carry over; registry and shaping tests added |
| `term-app/tests/pipeline.rs`, `smoke.rs` | carry over (text dumps) |
| benches (`parse`, `parse_1k`, `frame`) | carry over; numbers recorded before/after PRs 4, 5, 6, 10 and F3.2 |

### 17.3 New invariants (in `check_invariants`, run after every `advance` in debug builds, in property tests and fuzzing)

1. **Styles:** every cell's style id is live in its screen's table; the pen's
   and erase template's cached ids are live; no two live entries are equal
   (interning gives one id per style); the index finds every live entry and
   nothing else; `free` holds exactly the dead ids; after a forced sweep,
   live ids = ids used. (Ghostty's equivalent checks reference counts; ours
   checks the sweep's result.)
2. **SE bit:** `cell.SE == styles[cell.style_id].shows_empty()`.
3. **Clusters:** every kind-1 cell's slot is live in its row's store, and
   each live slot is held by exactly one cell; cluster text is valid UTF-8
   of 2–32 scalars and ≤ 128 bytes; `garbage` is right; rows without a store
   have no kind-1 cells.
4. **Widths:** a wide cell is followed by a spacer and a spacer follows a
   wide cell (today's rule); spacers have their wide cell's style; an edge
   spacer appears only in the last column of a wrapped row.
5. **Placeholders:** narrow; row and column < 297.
6. **Change counters:** every row's `seq ≤ counter`; property: if a row's
   cells differ between two snapshots, its `seq` differs.
7. **Snapshot:** a frame built from a snapshot equals a frame built from the
   live terminal (frame tests run both ways).
8. **History:** any row pushed and read back is the same row (cells,
   resolved styles, clusters, marks, placeholders), up to trailing blanks
   and the space/empty equivalence; `pop()` after `push()` gives the row
   back; the open block never holds more than 64 KB.
9. **Reflow:** the text of every logical line (joined, trailing blanks
   trimmed) is the same before and after any resize sequence; resizing to
   W2 and back to W1 with no output in between gives the same screen when
   nothing was dropped at the history limit; the cursor stays on the same
   character; selection text is unchanged; the view shows the same first
   cell.
10. **Lines at the current width:** for any width, block row counts agree
    with walking every logical line; view-row → Point → view-row round trip.
11. **Fuzzing:** the parse target's dictionary gains ZWJ, VS15/VS16,
    regional indicators, combining marks, U+10EEEE with diacritics, SGR 4:3
    and 58, OSC 8 and 133, `ESC _ G`, `ESC P q`; resize steps now reflow.
    New targets with B.3/B.5: kitty commands, the Sixel decoder, OSC 1337.

### 17.4 Keeping main green

- Every PR passes `cargo fmt --all`, `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` before merging (AGENTS.md).
- PR 1 puts every reader behind `RowRef`/`CellRef`, so PRs 4–6 change
  term-core's insides without touching term-render or term-app.
- Each switch over keeps the old path as a test-only reference for one PR,
  as the parser work did (`set_bulk_ascii`, the reference encoder): the
  cluster writer is checked against one-codepoint-at-a-time, v2 history
  against v1's text, reflow at an unchanged width against today's resize.
- Gates: `scripts/bench-gate.py` on history bytes per line (limits lowered
  in PR 6, never raised); parse throughput on `parse_1k` must not drop more
  than 3% on plain/coloured/TUI/`seq` in PRs 4, 5, 7 (8-byte cells should
  make it faster); frame-build bench for F3.x; footprint on the standard set
  for PRs 4, 6 and 10.
- Fuzz the parse target on each merge (the nightly job), 14 clean nights
  before 1.0 as planned.
- The v1 → v2 history change needs no data migration: history lives only in
  memory.

---

## 18. Compared with Ghostty and WezTerm

| | Ghostty | WezTerm | decline (this design) |
|---|---|---|---|
| Cell | 8 B packed: content tag, 21–24-bit content (or a bg colour), 16-bit style id, width, protected, hyperlink bit, semantic content | `Cell { text: TeenyString, attrs: CellAttributes }`, colours inline, rare attributes (link, underline colour, image) behind a `Box` | 8 B: content, kind, width, 32-bit style id |
| Styles | per page, reference counted, page grows when full | inline in each cell | per screen, swept |
| Graphemes | base codepoint in the cell, the rest in a per-page map keyed by cell offset (≤ 64 codepoints) | in the cell's string | per-row store, slot in the cell |
| History | the same pages of live cells (limit in bytes) | `Line`s; `compress_for_scrollback` packs them into a text + attribute-run form | encoded stored lines in 64 KB blocks, styles by value |
| Change tracking | dirty bit per row | `Line::seqno` | `seq` per row, unique per parse call |
| Images | placement storage per screen (kitty) | an image fragment attached to every covered cell | placement table keyed by stable line; placeholders are cells |
| Reflow | all pages rewritten on resize | all lines re-wrapped on resize | screen only; history worked out on demand |
| Tracked positions | pins (page, row, column) updated by reflow | stable row index | stable line numbers for stored lines, remapped on-screen positions |

**Where we differ and why.**

- *Sweep instead of reference counts:* only possible because history doesn't
  hold style ids. In exchange, reading history needs a decode (R.3 caches
  it) and history styles are stored once per block rather than once per
  page.
- *History is not cells:* Ghostty's 8 B per cell keeps history and screen
  one structure (simple, no decode) at ~13 MB per 10k 160-column lines; our
  goal is ≤ 35% of its footprint, so history is encoded (~0.4–0.8 MB).
- *No history rewrite on resize:* both Ghostty and WezTerm reflow everything
  they hold; with 100k lines that is the cost F.4 forbids. Our stored lines
  never change, which also makes history Points permanent.
- *Images as a table, not per-cell attachments:* WezTerm's way follows text
  everywhere for free but costs a fat attribute per covered cell; a
  placement keyed by line costs one entry per image, and kitty placeholders
  already give the per-cell case where programs want it.
- *Cluster store per row:* Ghostty's per-page map needs moving entries when
  cells move; WezTerm's per-cell string makes every cell bigger. Ours costs
  one indirection for cluster cells only.

---

## 19. Risks and open questions

### Risks

1. **Sweep pauses on very large screens.** 160k cells is a ~150 µs sweep
   under the lock, amortised over ≥ 80k new styles. Measure in PR 4 with a
   full-screen truecolour animation at 120 Hz.
2. **Lock time in gradient frames** (style vector copy, up to 900 KB).
   If R.1 measures it, copy only the ids the copied rows use (a bitset
   gather) instead of the whole vector.
3. **Cluster store garbage** under spinners that rewrite emoji in place:
   compaction is tested with a property test that rewrites one cell a million
   times and checks the store stays bounded.
4. **History at a new width with lots of CJK** needs scalar walks; only the
   `display_offset()` count does it for all of history (~10 ms per 100k
   lines, once per width). Drawing never does.
5. **A number handed to new text after a reflow.** Safe only while every
   holder of a Point lives in term-core and is remapped. New features that
   keep Points (E.5 prompt jumping, E.7 search, B.3 placements) must
   register as tracked positions; the reflow property test (§17.3, 9) covers
   each one as it lands.
6. **Width rules vs. what Core Text draws.** A ZWJ sequence Apple's emoji
   font doesn't know renders as several glyphs; we fit them into the
   cluster's two cells. The same for flags of unknown regions.
7. **Shell prompts after reflow.** zsh redraws its prompt on SIGWINCH; a
   reflowed old prompt plus the redraw can leave a duplicate line (every
   reflowing terminal has this). With OSC 133 marks we can clear the prompt's
   rows from mark A to the cursor before sending SIGWINCH, as kitty does
   with its shell integration. Decide in E.5.
8. **Legacy programs that assume `wcwidth`** see different widths for ZWJ
   sequences with 2027 on; they can turn it off. macOS's own `wcwidth` is
   old, so this will show in some tools.
9. **Merge order.** PR 1 touches the files the parser work just changed;
   it starts only after N.1 is on `main`.

### Open questions for the review

1. **Shrinking the height with the cursor mid-screen:** xterm drops the rows
   below the cursor (the ignored `resize_shrink_height_below_cursor_is_dropped`
   test); decline today pushes top rows into history and loses nothing.
   Recommendation: keep losing nothing, and change the test to say so.
2. **Free the alternate screen on leaving 1049** (its next entry clears it
   anyway), or keep it for re-entry through 47/1047 as xterm does?
   Recommendation: free it on 1049 exit and after 30 s hidden (R.8).
3. **Text over Sixel/iTerm2 images:** draw text over the image (this
   design), or punch the image out per cell like xterm/foot?
4. **Copying kitty placeholders:** spaces (this design) or the placeholder
   characters with explicit diacritics?
5. **History byte cap:** 32 MB default alongside the line limit? The
   roadmap's comparison uses "the same scrollback bytes as Ghostty", which
   suggests making bytes the primary limit, as Ghostty does.
6. **Mode 2027 default on** (Ghostty does the same)? And should East Asian
   Ambiguous characters be wide behind a setting for CJK users? The width
   exceptions in history already allow it.
7. **Where images are decoded:** term-core must stay free of platform code,
   but PNG/JPEG/GIF decoding wants ImageIO. Proposal for B.3: an
   `ImageDecoder` trait in term-core, an ImageIO implementation in term-app,
   a pure-Rust PNG one for tests; pixel caps checked from the header before
   decoding.
8. **Per-cell prompt/input/output marks** (Ghostty keeps 2 bits per cell)
   instead of per-row marks with columns, for Option-click cursor moves?
   There are five spare cell bits if E.5 needs them.
9. **Search on another thread** with its own copy of the text, instead of
   slices under the lock?
