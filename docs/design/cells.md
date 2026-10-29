# Cells, styles, runs and scrollback: the storage model for 1.0 (F.0)

Status: **accepted with changes** by two Opus (xhigh) reviews, and revised
on 29 Oct 2026 with those changes and with Alex's decisions (§0). Two
pieces have landed: the code-drawn glyphs (F.6, in term-font) and the vte
parser changes (PR 3, `0bb234d`). Everything else is still to be built. The
design builds on the parser work now on `main`: vendored vte
with `print_ascii`, `write_ascii` row-segment writes, the screen as a ring
of rows, the memset row reset and the faster scrollback encoder. Every
phase-F item, and the image, ligature, search and frame-outside-the-lock
work in phases B, E and R, sits on this model, so it has to serve all of
them at once.

Modules named here:
- **term-core**: escape-code handling, the screen, scrollback, selection and
  search; no platform code.
- **term-font**: Core Text fonts, shaping, rasterising, and the code-drawn
  glyphs.
- **term-render**: turns terminal state into GPU instances and draws them
  with Metal.
- **term-app**: the AppKit window, plus the `session` module, which owns the
  PTY threads and the shared terminal.
- **the vendored vte parser**: `vendor/vte`.

---

## 0. Revisions

### After the review

| Review item | Where |
|---|---|
| Who holds style ids: the sweep clears `recent`; screen switches drop the pen and erase templates; DECSC saves the style and link by value; no sweep mid-resize; shrinking renumbers ids; a test hook forces sweeps | §5.2, §5.3 |
| Change counters and line numbers can repeat (RIS, the session replacing its terminal after a caught panic, `pop()`): the counter carries across RIS; `terminal_id` and `history_epoch` go in the snapshot | §9.1, §9.2 |
| `write_ascii` leaves the grapheme state on its last cell, so keycaps and `e` + U+0301 join | §4.3, §6.2 |
| Positions inside pulled-back lines are recorded before `pop()` | §12.2 |
| The app's own Point and view-offset arithmetic moves into term-core (`point_at`, `select_word`, `select_line`, `select_all`, `cursor_in_view`, `link_at`) | §13.2, §17 |
| History dictionaries have 16-bit offsets: seal on either limit; the byte backstop counts every byte; gradient estimate corrected to ~10 B per cell | §11.2, §11.4 |
| Region scrolls move or clear search hits and selection ends | §8.2, §13.1 |
| Row cache: cursor, selection, search, hovered link, placeholder images and blink are overlays; rows are drawn relative to their top; highlight ranges in the snapshot | §9.3 |
| The snapshot copies only newly interned styles | §9.2 |
| A small cache from screen style id to block slot when writing history; parse-speed gate on a fixed Mac | §11.2, §17.4 |
| Shaped-cluster cache per process | §10.2 |
| The view position as `ViewTop::At { point, offset }` | §12.3 |
| A "fills to the end of the row" flag for full-width backgrounds | §11.3, §12.2 |
| Sixel and iTerm2 images become placeholder cells | §8.3 |
| Images are decoded outside the lock, PNG/JPEG/GIF only, one pixel copy | §8.4 |
| Width rule: emoji sequences 2, VS15 1, otherwise the first scalar's width (string-width) | §6.3 |
| Placeholder inference step by step; guessed columns to 511; placement id 0 | §7 |
| What R.7 compression leaves uncompressed | §11.8 |
| Stray spacers, REP by value, OSC parameter rejoining | §6.4, §14 |
| PR plan: PR 1 is M and makes `Terminal`'s fields private; reflow split in three; dogfooding gaps stated | §17 |
| Smaller items (could-fix) | where they apply, marked "(could)" |
| Reflow budget corrected to 0.5–1 ms; neutral for the 35% memory target | §12.4, §2 |

### Alex's decisions (29 Oct)

| Decision | Where |
|---|---|
| **Scrollback limit**: lines (the existing `scrollback = 10000` setting), plus a hidden 32 MB byte backstop. The Ghostty comparison is made at the same number of lines kept | §11.2 |
| **Image budget**: 32 MB per process by default, a setting; CPU and GPU copies counted once | §8.4 |
| **East Asian Ambiguous width**: a setting in 1.0 (`ambiguous_width = 1 \| 2`, default 1) | §6.3 |
| **Search (Cmd+F) and VoiceOver cover all of history**, with per-block UTF-16 counts: in-view hits first, then history outwards; fast block search with a per-block filter | §11.2, §13.3–§13.10 |
| Everything else stands as the engineering answers | §19 |

The search design changed history's layout slightly: each block now keeps
its lines' text contiguous, in a separate array from the rest of each line
(§11.2). That costs one byte on plain lines (42 → 43 B) and nothing on
styled ones.

### After the second review (search, ambiguous width, checks on the fixes)

The reviewer measured search with a prototype over 100k stored lines of
real output (logs, `ls`, source code, man pages, git log). Those numbers
replace the earlier estimates in §1, §13.6 and §13.7.

| Review item | Where |
|---|---|
| Matches across a block boundary, and from history onto screen row 0: search a small buffer around each boundary | §13.5 |
| Blocks split into shared, immutable `BlockData` (in an `Arc`) and history's own `BlockEntry` (length, UTF-16 count, row counts, filter); `pop()` lowers the length only; the old `Cell` (not `Sync`) is gone | §11.2, §11.8 |
| Search has its own word rule; double-click keeps the path-friendly one | §13.2, §13.8 |
| One style mirror per screen; epochs from a terminal-wide counter | §5.2, §9.2 |
| Filter numbers measured; built in the background only | §13.6, §13.7 |
| `pop()` no longer bumps `history_epoch`; a "lowest line popped" mark limits what searches and the renderer drop | §9.1, §13.4 |
| Ignoring case: lower-case each block into a scratch buffer, then memmem | §13.5 |
| Case folding defined exactly, with its limits | §13.5 |
| Matches start and end on cluster boundaries | §13.5 |
| One rule for searching live output, with no duplicate hits | §13.4 |
| Hits kept per block, so freeing a block is O(1); the reflow budget claim fixed | §13.4, §12.4 |
| Narrowing while typing only for Contains, same case setting, below the cap | §13.8 |
| View API: `view_position`, `scroll_to`, the UTF-16 calls, and what `point_at` returns at edges | §13.2 |
| `new_ids` capped when interning | §5.2 |
| PR plan: the gap between PRs 6 and 7 in daily builds; PR 13 split in three; which PRs touch term-app | §17 |
| Ambiguous width: seal only a non-empty block, VS15 stays 1, narrow box drawing later, running programs keep their width | §6.3, §15 |
| Keycap bases in the ASCII table | §6.2 |
| Smaller items (could) | where they apply |

### Alex's decisions (second round)

| Decision | Where |
|---|---|
| **Keep the per-block filter in 1.0,** built in the background only and never while someone waits; a block without one is simply scanned. 4-byte grams in a 4 KB filter | §13.6 |
| **Cmd+F uses our own native-style find bar, not NSTextFinder:** a search field with previous/next, a count, Done and an options menu; the standard Edit > Find actions; the shared find pasteboard | §13.9 |
| **The UTF-16 piece API is for VoiceOver only;** its full text is an NSString subclass backed by history blocks, and the E.12 spike measures how often VoiceOver asks | §13.10 |

---

## 1. Decisions at a glance

| Question | Decision | Why |
|---|---|---|
| Cell size | **8 bytes**: 21-bit content, 2-bit kind, 2-bit width, 2 flag bits, 32-bit style id | 33% less than today's 12 B; one `u64` load/store; the blank cell is still all zero, so the memset row reset and the zero-skip in the scrollback encoder keep working |
| What a cell's content is | a Unicode scalar, **or** a slot in the row's cluster store, **or** an image placeholder's row/column | one cell layout for text, clusters and image pieces |
| Style storage | **one style table per screen** (primary, alternate); cells hold an id | fg, bg, underline colour/style, overline, link id don't fit in a cell; most screens use < 100 distinct styles |
| Freeing styles | **sweep**, no reference counts: when the table reaches 1.5 × the screen's cell count, mark the ids the screen's cells and the pen use and free the rest | only the screen refers to ids (history stores styles by value), so a sweep is bounded by screen size and the write path does no counting |
| Grapheme clusters | **per-row store**; a cluster cell holds a slot index; the store holds the whole cluster as UTF-8 | each slot belongs to exactly one cell, so it is freed when the cell is overwritten; rows move (scroll, ring rotation, insert/delete lines) with their clusters and need no fixups |
| Width of a cluster | mode **2027 on by default**: emoji sequences are 2, VS15 sequences 1, anything else its first scalar's width; mode 2027 off: each codepoint keeps its own width | matches `string-width`, which Claude Code's Ink uses; programs can still ask for the old rule |
| Ambiguous width | **a setting** (`ambiguous_width`, default 1); screen cells keep the width they were written with; history records the setting per block | lines show exactly as written after the setting changes |
| kitty placeholders | cell kind "placeholder", row and column in the content bits; image id in the style's fg (+ a high byte), placement id in its underline colour | lossless, 8 bytes per cell, no cluster storage; one style per image |
| Sixel and iTerm2 images | placeholder cells too, for an internal virtual placement | overwriting or erasing a cell removes that piece, as in xterm, foot, WezTerm and iTerm2 |
| kitty direct placements | a **placement table per screen**, keyed by the stable line number of the image's top row, freed with history blocks | one entry per image; nothing to do when output scrolls |
| Change tracking | every row carries a **change counter**, unique per row per parse call; the snapshot carries `terminal_id` and `history_epoch` | the renderer's row cache and the snapshot copy only changed rows, and can't be fooled by a reset |
| Frames outside the lock | a **snapshot** copies changed rows, newly interned styles, highlight ranges, cursor, palette and visible images | the lock is held for a memcpy, not a frame build |
| Shaping runs | break only on **font face** (bold/italic), **code-drawn glyphs** and **placeholders** | ligatures survive colour changes, the cursor and selection |
| Ligatures | one glyph, **one instance per cell**, each quad clipped to its cell on the CPU, coloured by that cell | no shader change; per-cell colours inside a ligature |
| Glyph key | **(font id, glyph id, cells)** from a **font registry** in term-font; code-drawn glyphs use a reserved font id and their `SpriteId` | CTLine picks fallback fonts itself, so a codepoint alone no longer names a glyph |
| Scrollback v2 | stored lines are **physical lines as written**; logical lines are runs of wrapped stored lines; each block keeps its **text contiguous**, plus a style dictionary and a link table; trailing fill implied by runs; sealed blocks are shared, immutable `Arc<BlockData>` | plain 43 B/line (today 46 sealed, 52 measured), `ls` 59 (86/93), full-width background 13 (174/183), CJK 66 (87/90) |
| History limits | `scrollback` lines (default 10,000) plus a hidden 32 MB backstop | Alex's decision |
| Reflow | the screen is reflowed eagerly on every resize; **history is never rewritten**: rows at the current width are worked out on demand from the stored lines | resize costs O(screen), not O(history): 0.5–1 ms with 100k lines |
| Points | `Point.line` stays the number a row got when it was written; history points never change; on-screen positions that matter are remapped in the same resize; the app does no line arithmetic of its own | selection, search hits, images and the view position survive output, scrolling and reflow |
| Search | **in view first, then history outwards** from the view; SIMD substring search (memmem) over each block's text, with case ignored by lower-casing each block into a scratch buffer; blocks skipped with a **4 KB filter of 4-byte grams**, built in the background only; done on immutable blocks, off the lock | covers all of history (Alex's decision); measured on 100k lines: a full scan takes 0.11–0.33 ms, or 0.19–0.71 ms ignoring case |
| Find bar | **our own native-style find bar** (Alex's decision): a search field, previous/next, a count, Done, an options menu; Edit > Find, Cmd+F/G/E and the shared find pasteboard work as usual | standard controls, and it uses our search directly |
| VoiceOver | UTF-16 offsets over all of history from **per-block UTF-16 counts** and checkpoints every 16 lines; the full text is an NSString subclass backed by history blocks | O(log blocks) per lookup; 4 bytes per block |
| Parser | vte gains an APC string state with start/put/end calls, a bulk `put_slice` for DCS, and a size policy for long OSC strings (streaming for OSC 1337 files) | kitty graphics, Sixel and iTerm2 images without buffering megabytes twice |

---

## 2. What this has to serve

From the roadmap (F, B, E, R and the Definition of done):

- Neovim undercurl with underline colour (SGR 4:3, 58), true colour, overline.
- OSC 8 hyperlinks with Cmd-click (E.2); OSC 133 prompt marks: jump between
  prompts, select output, Option-click to move the cursor (E.5).
- Grapheme clusters: combining marks, ZWJ emoji, flags, skin tones, keycaps,
  VS15/VS16, mode 2027 (F.1, F.2); East Asian Ambiguous width as a setting.
- Ligatures with Core Text (B.1), code-drawn box/block/Powerline glyphs (F.6,
  landed).
- kitty graphics including virtual placements and Unicode placeholders,
  iTerm2 OSC 1337 and Sixel (B.3, B.5; `yazi` previews are on the
  daily-driver checklist).
- Reflow on resize and pulling lines back when the window grows taller
  (F.4: a 120 Hz divider drag with 100k lines of history, frame ≤ 8 ms).
- Cmd+F search over all of history, in-view hits first (E.7); VoiceOver over
  all of history (E.12); selection that survives scrolling and reflow;
  rectangular selection (E.8).
- Frame build outside the lock (R.1), per-row instance caching (R.2), a
  history row cache (R.3), idle compression of history blocks (R.7).
- The thresholds: footprint ≤ 35% of Ghostty's idle and after 10k lines,
  `cat` CPU ≤ 45%, key latency p99 ≤ Ghostty's.

It must not give back the parser work's gains: in 1 KB pieces, plain 423
MB/s, coloured 172, TUI 289, `seq` 110.

**On the memory target, plainly:** this design is neutral for the ≤ 35%
footprint threshold. Terminal state is well under a megabyte in the
standard cases (§16), against ~13 MB of drawables and ~20 MB of
frameworks. Meeting the threshold depends on the drawable, glyph-texture
and idle-trim work (perf-ideas, R.5, R.8), not on this model. What this
model must do is not make things worse, and keep 100k-line history small.

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
 │  Scrollback (history): blocks of ≤ 64 KB per part, freed oldest first      │
 │    BlockEntry (history's own): Arc<BlockData>, line count, UTF-16 count,   │
 │                                row counts, filter (built in background)    │
 │    BlockData (shared, immutable): line headers, contiguous text, style     │
 │                                   dictionary, links                        │
 │    rows at the current width are worked out on demand (lazy line index)    │
 └───────┬──────────────────────────────────────────────┬────────────────────┘
         │ snapshot under the lock: changed rows,        │ search job: (Arc<BlockData>,
         │ new styles, highlight ranges, cursor,         │ line count) per block + the
         │ palette, visible images                       │ screen's hits, under the lock
         ▼                                               ▼
   term-render (main thread, outside the lock     search worker (one per process,
   from R.1): runs → term-font shapes (CTLine)    off the lock): blocks outwards
   → glyph keys → textures; overlays              from the view; hits stream back
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
| 0–20 | content | kind 0: Unicode scalar, 0 = empty. Kind 1: slot in this row's cluster store. Kind 2: image placeholder, row in bits 0–8, column in bits 9–17 (both < 512), bits 18–20 zero |
| 21–22 | kind | 0 scalar, 1 cluster, 2 placeholder, 3 never stored (reserved) |
| 23–24 | width | 0 narrow, 1 wide (left half), 2 spacer (right half of a wide cell), 3 edge spacer (the last column of a row whose next wide character didn't fit and moved to the next row) |
| 25 | P, protected | DECSCA/SPA protection from selective erase; reserved for E.13 (esctest2), always 0 until then |
| 26 | SE, shows when empty | a copy of one bit of the cell's style: it has a background, inverse, underline, strike or overline, so an empty cell with it is still drawn |
| 27–31 | zero | spare; five bits left |
| 32–63 | style id | index into the screen's style table; 0 = the default style. The table is capped below 2²⁴ entries, so the top byte is always 0 |

The blank cell is `0`: empty, scalar, narrow, default style. A cell erased
with a coloured background is `erase_style << 32 | SE`. Columns are clamped
to 4096 (could), so a column always fits in 12 bits and row headers stay
small.

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

- **Same size as Ghostty's cell, but split differently.** Ghostty packs a
  2-bit content tag, 24-bit content, a 16-bit style id, 2-bit width,
  protected, hyperlink and 2-bit semantic-content bits and 16 bits of
  padding into a `u64`. Its content tag has two background-colour-only
  forms, so erased cells don't need a style. We don't need those: erasing
  interns one "erase style" per erase command (§5.4), and keeping every cell
  uniform keeps the hot loops simple. We put the style id in a full 32 bits
  instead of 16, so a screen can never run out of ids. Ghostty's ids are per
  page, and the page grows when they run out; ours are per screen, and a 6K
  display with a small font has more than 150k cells.
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

| Today (on `main`) | With 8-byte cells |
|---|---|
| `write_ascii` writes `Cell { packed: b \| style, fg, bg }` per byte | `cells[i] = Cell(template \| b as u64)`: one OR and one 8-byte store per byte; the template is computed once per run |
| before a segment, an OR of `packed` over the cells finds wide characters, and the segment then goes one character at a time | an OR of `cell.0 & (KIND \| WIDTH)` over the segment finds wide halves **and** clusters/placeholders. If they are only at the segment's two ends (the usual case: half a wide character), those two cells are fixed and the segment still goes in bulk; otherwise every slot found is freed in one pass, then the bulk write (could) |
| after a run: `last_char` set | also `grapheme.at` = the run's last cell and `grapheme.prev` = that byte's break class, so a following U+0301, VS16 or U+20E3 joins it (`e`́, keycaps `1️⃣`); `last_grapheme` (for REP) = that byte |
| `Row::reset` memsets when the fill is `Cell::BLANK` | unchanged: blank is still 0. A coloured erase fills one `u64` value, which also vectorises |
| `stored_len` skips all-zero cells 8 at a time (an OR over 96 bytes) | OR over 8 × `u64` (64 bytes) of `c.0 & NOT_BLANK` |
| encoder fast path: OR of `packed & !0x7F \| fg \| bg` over the line | OR of `c.0 & !0x7F` over the line (style 0, scalar, narrow, < 0x80): one field instead of three |
| `set_bulk_ascii(false)` reference switch and the property test "bulk path = one character at a time, fed one byte per call" | kept; extended with "ASCII followed by a combining mark / VS16 / U+20E3" pieces and with clusters (§6.6) |
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
    /// BOLD DIM ITALIC INVERSE HIDDEN STRIKE BLINK OVERLINE OWN_IMAGE, 7 spare
    pub attrs: Attrs,           // u16
    pub underline: Underline,   // u8: None, Single, Double, Curly, Dotted, Dashed
    /// Placeholder cells only: bits 24–31 of the image id (kitty's third
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

`OWN_IMAGE` marks placeholder cells of decline's own images (Sixel, iTerm2,
§8.3), so they can't be confused with ids kitty programs choose. These
SGR codes are stored in the style:
- 1, 2, 3, 4 (with `4:0`–`4:5`), 5/6, 7, 8, 9;
- 21 (double underline), 22–29;
- 30–37, 90–97 and 38; 39;
- 40–47, 100–107 and 48; 49;
- 53 (overline), 55;
- 58 (`58;5;n`, `58;2;r;g;b`, `58:2::r:g:b`), 59. Today 58 is parsed and
  thrown away.

### 5.2 The table, and everything that holds an id

```rust
/// The distinct styles one screen's cells use, each stored once. Cells hold
/// an index into it. Id 0 is the default style and is never freed.
pub struct StyleTable {
    styles: Vec<Style>,     // by id (20 B each)
    index: Vec<u32>,        // open addressing by hash of Style, power of two, 0 = empty slot
    free: Vec<u32>,         // ids freed by the last sweep, used first
    live: u32,              // ids in use
    sweep_at: u32,          // sweep before growing past this; tests can set it (see below)
    sweeps_blocked: bool,   // during resize
    recent: [(Style, u32); 2], // the last two styles interned: alternating styles skip the hash
    new_ids: Vec<u32>,      // ids interned since the last snapshot (§9.2)
    send_all: bool,         // too many new ids: the next snapshot sends the whole table
    epoch: u64,             // from the terminal-wide counter: at creation and at each renumbering
}
```

**Interning** (`intern(&Style) -> u32`) checks `recent` first, then hashes the
20 bytes (FxHash), probes linearly and compares. On a miss it takes an id from
`free` or pushes a new one, and appends the id to `new_ids`. If that would
take `new_ids` past half the table (a gradient), the list is cleared and
`send_all` is set instead, so the list can't grow without bound between
snapshots. A hit costs about 5 ns on average, a miss 15–20 ns. If TUI parse
speed drops (the gate in §17.4 will show it), `recent` can grow to a
16-entry cache (could).

**Epochs never repeat.** Each table takes its `epoch` from a counter kept by
the terminal, not by the table, both when it is made and when it is
renumbered. The alternate screen's table is freed and made again (§19), and
a new table never repeats an epoch the renderer has already seen (§9.2).

**Holders of style ids**, all handled by the sweep:

| Holder | Rule |
|---|---|
| this screen's cells | marked by the sweep |
| the pen's cached template (`Option<Cell>`) | marked; dropped on every SGR, every 47/1047/1049 switch (it belongs to the other screen's table) and every DECRC restore |
| the erase template (`Option<Cell>`) | marked; dropped when the pen's background changes and on every screen switch |
| `recent` | **cleared** by every sweep (it may name a freed id) |
| saved cursors (DECSC, 1048, 1049) | hold the pen's `Style` **and its link (URI and id parameter) by value**. The saved `Style`'s own `link` field is an id from one screen's link table, so restoring clears it and looks the link up again from the saved URI; the next print then re-interns the style |
| history | styles by value in each block's dictionary (§11.5) |
| the snapshot and renderer | style values, mirrored from `new_ids` (§9.2) |

**The sweep** runs when an intern misses and the table already holds
`sweep_at = max(1024, 3 × cells / 2)` entries. It isn't run while
`sweeps_blocked` is set:

1. it marks a bitset with every cell of every row of this screen
   (`bit[cell >> 32] = 1`), plus the pen and erase templates;
2. it puts every unmarked id on `free` and removes it from `index` (linear
   probing with backward-shift deletion, so no tombstones);
3. it clears `recent` and resets the history writer's id-to-slot cache
   (§11.2);
4. it sweeps the link table the same way, from the live styles (§5.5);
5. **if fewer than a quarter of the entries are live, it renumbers.** It
   builds an old-to-new id map, then rewrites in one pass every cell of
   every row, the pen and erase templates, `index` and `styles`, and
   shrinks the vectors' capacity. It clears `recent` and `new_ids`, touches
   every row (each gets a new change counter) and takes a new `epoch`, so
   the next snapshot carries the whole table. Memory is given back after a
   gradient-heavy program exits, and ids stay dense.

**No sweep during resize.** Reflow and pull-back intern styles while rows
are half-built, and a sweep then would free ids that rows in the working set
still use. So resize sets `sweeps_blocked` (the table may grow past
`sweep_at` for a moment), puts the new rows in place, clears it and sweeps
once.

**A test hook**, `set_sweep_at(8)`, makes sweeps and renumbering happen
every few prints in the property tests and the fuzz target, so every holder
above is exercised.

At 300×100 a sweep reads 240 KB, about 20–40 µs. A sweep leaves at most
`cells + 2` live ids, so at least `cells / 2` new styles arrive between two
sweeps: under 2 cell reads per new style, amortised. A freed id is only
reused for a style no holder has, and an id's style never changes while it
is live, so the renderer can cache rows by their change counter (§9).

**Why not reference counts, as Ghostty uses?** Ghostty's history is pages of
live cells, so a style can be held by cells far up the history, and a sweep
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
of thousands of distinct styles; renumbering gives the memory back.

### 5.4 Truecolour gradients (a different style in every cell)

`lolcat`, a 24-bit colour test, btop's graphs: `ESC[38;2;r;g;bm█` per cell,
about 20 bytes of escape code per character.

- Each print: the SGR edits the pen (no intern), the print interns: one
  miss, ~15–20 ns, against ~100 ns to parse the 20-byte SGR. About +15%
  parse time on such output.
- A full 300×100 repaint with all-new colours makes 30k new styles, so about
  two sweeps per frame (~60 µs) against ~3.5 ms to parse the 600 KB of
  input: under 2%.
- The table peaks at 45k entries (1.6 MB), and is renumbered smaller once
  the program stops.
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

How OSC 8 links are handled:
- **Setting the link.** `OSC 8 ; id=x ; URI ST` sets the pen's link. The
  pen keeps the URI and id parameter by value, and interns them in the
  active screen's table at the first print. Switching screens therefore
  re-interns, rather than carrying an id from the other table.
- **Recovering the URI.** URIs may contain `;`, which vte splits on (§14),
  so the URI is rebuilt by joining `params[2..]` with `;`.
- **Pieces of one link.** The same id parameter and URI give the same
  `LinkId`, so all the pieces of a link, split across lines or colours,
  highlight together on hover.
- **Limits.** URIs over 4 KB and id parameters over 256 bytes are ignored.
- **Freeing.** Links are swept right after styles: a link that no live style
  and no pen refers to is freed. History keeps its own link table per block
  (§11.5).
- **`link_at(point)`** (§13.2) returns the link's value, so the app and the
  renderer compare links by value (URI and id), never by id: screen and
  history ids differ.

---

## 6. Grapheme clusters

### 6.1 Where they live: one store per row

```rust
pub struct Row {
    cells: Vec<Cell>,                // exactly `cols` cells
    seq: u64,                        // change counter (§9)
    extra: Option<Box<RowExtra>>,    // only rows with clusters or prompt marks
    flags: RowFlags,                 // u8: WRAPPED, FILL, PLACEHOLDERS, BLINKING,
                                     //     DOUBLE_WIDTH, DOUBLE_HEIGHT (2 bits, reserved for DECDWL/DECDHL)
}                                    // 48 bytes

pub struct RowExtra {
    clusters: Clusters,
    marks: [Option<Mark>; 4],        // OSC 133 marks on this row (§11.6)
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

Rules:
- **One owner per slot.** A slot belongs to exactly one cell.
- **One form per thing.** A cluster has at least two scalars; a single
  scalar is stored as a kind-0 cell.
- **Caps.** At most 32 scalars and 128 bytes per cluster (further marks are
  dropped; Ghostty caps at 64 codepoints), and 8 KB of cluster text per row.
- **Compaction.** `text` is compacted when `garbage > text.len() / 2` and it
  is over 256 bytes, so a spinner rewriting an emoji in place doesn't grow
  the row.
- **Reset.** `Row::reset` keeps the `RowExtra` allocation (emptied) when
  its buffers are under 1 KB, so a status line that redraws emoji doesn't
  allocate each time (could).

**Per row, not per screen**, because:

- ownership is one cell to one slot, so overwriting or erasing a cluster cell
  frees its slot on the spot: no reference counts and no sweep;
- rows move whole (full-screen scroll round the ring, region scrolls,
  IL/DL) and their clusters move with them, with no index fixups;
- pushing a row into history and copying it into a snapshot read one small
  local store.

A per-screen store would need a sweep like the style table, for no gain:
clusters are not shared between cells. kitty's text cache is an example of
a per-screen store.

### 6.2 Segmenting as text arrives

term-core gets its own Unicode table, generated by a script from the UCD
files of the same Unicode version `unicode-width` 0.2 uses, and checked in.
It is a two-level lookup giving, per scalar:
- width (0, 1 or 2), plus whether it is East Asian Ambiguous (§6.3);
- Grapheme_Cluster_Break class;
- Extended_Pictographic, Indic_Conjunct_Break and Emoji_Presentation;
- Emoji_Modifier_Base and Emoji_Modifier;
- whether it is the base of an emoji variation sequence.

It takes about 60–80 KB of read-only data and costs one lookup per non-ASCII
scalar. ASCII never reaches it: `write_ascii` handles ASCII and sets the
state below from a 128-entry table. That table also marks `#`, `*` and
`0`–`9` as keycap bases, so `1` + VS16 + U+20E3 becomes a keycap even when
the `1` came through the ASCII fast path.

```rust
/// Enough to decide whether the next scalar joins the last printed cell,
/// and what the cluster's width becomes.
struct GraphemeState {
    at: Option<(usize, usize)>,  // screen row and column of the last printed cell
    prev: Gcb,                   // break class of the cluster's last scalar
    ri_odd: bool,                // an odd number of regional indicators so far
    emoji_zwj: EmojiZwj,         // inside ExtPict Extend* ZWJ (rule GB11)
    incb: IncbState,             // inside an Indic conjunct (rule GB9c)
    emoji: EmojiSeq,             // what emoji sequence the cluster is so far (§6.3)
}
```

`at` is cleared by anything but a print: cursor movement, controls, erase,
scroll, resize, mode changes. Both `print` and `write_ascii` set it. When a
scalar arrives and `at` is set, the UAX #29 rules (GB3–GB13 and GB9c, about
150 lines) decide whether it joins. They are checked against
`unicode-segmentation`'s grapheme iterator, both on GraphemeBreakTest.txt
and on random strings fed whole and one byte per `advance` call.

With mode 2027 off ("legacy"), only zero-width scalars join the previous
cell: combining marks, ZWJ, variation selectors and default-ignorables.
Every other scalar starts a cell.

### 6.3 Width

**Mode 2027 on (the default).** A cluster's width is:

1. **2** if it is an emoji sequence: an emoji presentation sequence (a
   variation-sequence base + VS16), an emoji ZWJ sequence, an emoji
   modifier sequence (skin tones), a flag (a pair of regional indicators),
   or a keycap (`[0-9#*]`, optional VS16, U+20E3);
2. **1** if it is a text presentation sequence (a variation-sequence base +
   VS15);
3. otherwise, **its first scalar's width** from the table (with the
   ambiguous-width setting below), at least 1.

That is `string-width`'s rule, the one Claude Code's Ink uses, and what
most JS TUIs compute. It differs from `unicode-width` 0.2.3's string width
for some scripts. A test table pins these cases (values to confirm against
both libraries when the test is written):

| Cluster | `unicode-width` 0.2.3 | `string-width` | decline |
|---|---|---|---|
| Devanagari क्ष (क + ् + ष) | 2 | 1 | 1 |
| Thai กำ (ก + ำ) | 2 | 1 | 1 |
| U+0600 (Arabic number sign) + `1` | 2 | 1 | 1 |
| `e` + U+0301 | 1 | 1 | 1 |
| 👨‍👩‍👧, 👍🏽, 🇬🇧, `1️⃣`, ❤️ | 2 | 2 | 2 |
| ⌚ + VS15 | 1 | 1 | 1 |

**Mode 2027 off.** The cell keeps its first scalar's width; joined
zero-width scalars change nothing, VS16 included. Today's always-on VS16
widening becomes part of mode 2027.

DECRQM 2027 answers 1 (set) or 2 (reset). Today it answers 3 (permanently
set). Ghostty enables the mode by default through its
`grapheme-width-method` setting.

**When a join changes the width**, the cell is fixed in place:

- 1 → 2 (VS16, the second regional indicator, a skin tone, U+20E3): the next
  cell becomes the spacer (erasing a wide character there whole, as
  `widen_previous` does now) and the cursor moves one right. At the last
  column with autowrap on, the cluster moves to the start of the next row,
  leaving an edge spacer, and the row is marked wrapped; with autowrap off it
  stays narrow (this fixes E.14's "VS16 with autowrap off").
- 2 → 1 (VS15): the spacer becomes a blank in the same style and the cursor
  moves one left.

**The ambiguous-width setting (Alex's decision: in 1.0).**
`ambiguous_width = 1 | 2`, default 1, applies to characters whose East
Asian Width is Ambiguous (Greek and Cyrillic letters, `§`, `°`, `±`, `①`,
and most box drawing). The set is the one `unicode-width`'s `width_cjk`
uses; the Unicode table carries that flag next to the width.

- **The width rule.** Step 3 uses 2 for an ambiguous first scalar when the
  setting is 2. Emoji sequences stay 2 and VS15 sequences 1, whatever the
  setting. That includes VS15 after an ambiguous character with the setting
  at 2, on purpose: VS15 asks for text presentation, and `string-width`
  gives 1 too. Mode 2027 off uses the setting the same way. With 2, box
  drawing is two cells wide too: that is what CJK-mode terminals and
  `wcwidth_cjk`-based programs do, and why it is a setting. Keeping box
  drawing narrow under setting 2, as some terminals offer, is later work.
- **Cost.** The ambiguous flag comes from the same table lookup as the width,
  so non-ASCII characters pay one extra bit test and ASCII pays nothing
  (§15).
- **Screen cells keep their width.** A cell's width is stored in the cell, so
  changing the setting doesn't touch what is already on the screen. New
  output uses the new width. The grid doesn't change size, so nothing is
  reflowed.
- **History shows lines exactly as written.** Each block records the setting
  its lines were written with (`ambiguous` in §11.2). When the setting
  changes, the open block is sealed early if it holds any lines (an empty
  one just takes the new setting); such changes are rare, so this costs
  nothing. A row written before the change but pushed into history
  after it gets width exceptions (§11.3) for its ambiguous cells: the encoder
  compares every cell's real width with what the block's setting would give.
  Decoding, rows at another width (§12.3), search columns (§13.8) and UTF-16
  offsets (§13.10) all use the block's own setting.
- **Drawing.** An ambiguous character drawn two cells wide is centred in the
  two cells (`GlyphKey.cells = 2`, with no scaling for text glyphs).
  Code-drawn ones are drawn two cells wide by term-font's sprite module,
  which already takes a cell count.
- **Reporting.** There is no standard query for this, and decline adds no
  private mode. Programs that care measure it, as Vim does with its `t_u7`
  check: it prints an ambiguous character and asks for the cursor position.
  That works because the cursor really moves by the configured width.
- **Changing it at runtime** (config reload, E.9):
  `Terminal::set_ambiguous_width` seals the open history block (if it isn't
  empty), clears the grapheme state, and changes nothing else. Programs
  already running keep the width they worked out when they started (their
  own setting or locale), so a full-screen program may misalign until it is
  restarted. That is the same in every terminal with this setting.

### 6.4 What each operation does to clusters

| Operation | Effect |
|---|---|
| print over a cluster cell | the write path's OR check sees kind ≠ 0 and frees the slot |
| erase (ED, EL, ECH, DECSED) | slots of cluster cells in the range are freed; erasing a whole row empties its store |
| insert/delete characters (ICH, DCH, insert mode) | cells rotate within the row and keep their slots; cells pushed off the end free theirs; `repair_wide` (extended) blanks any half of a wide character that lost its partner **and any spacer away from where it belongs**: a spacer not after a wide cell, or an edge spacer not in the last column (DCH moves one left) |
| scroll, IL/DL, region scroll, ring rotation | rows move with their stores; nothing else |
| row reset (row reused at the bottom) | store emptied (allocation kept when small) |
| REP (CSI b) | repeats `last_grapheme`, the last printed grapheme **kept by value** (≤ 128 bytes, set by `print` and `write_ascii`), so it works after the cell has been overwritten; each copy gets its own slot |
| DECALN | all stores emptied |
| resize, same width | rows keep their stores |
| resize without reflow (alternate screen), narrower | cells past the new width free their slots |
| resize without reflow, wider | an edge spacer that is no longer in the last column is blanked (same rule as `repair_wide`) |
| reflow (§12) | cells are copied into the new rows; each cluster is re-inserted into its new row's store |
| push to history | the cluster's text goes inline in the stored line's text, with a boundary list (§11.3) |
| selection text, search, `dump()` | emit the cluster's text |

### 6.5 Rendering

A cluster cell's text goes to term-font's shaping (§10), through the
per-process shaped-cluster cache. Single scalars use the cached direct
lookup when the font has no ligatures. Between the clusters PR and F3.2, a
cluster cell draws at least its first character (§17.5).

### 6.6 Tests for clusters

- incremental segmentation = `unicode_segmentation::graphemes(true)` for
  random strings, fed whole and one byte per call;
- the cell grid after printing a string = the grid after printing the same
  string one character at a time (the `bulk_ascii` reference pattern), with
  pieces like `e` + U+0301, `1` + VS16 + U+20E3 and `a` + VS16 straddling
  the ASCII fast path;
- the width table above, plus every sequence in emoji-test.txt, at both
  ambiguous-width settings;
- cluster integrity invariants (§17.3).

---

## 7. kitty virtual placements: Unicode placeholders

A program places an image with `a=p,U=1` (a virtual placement: an image id,
a placement id, a size in cells, no position), then prints U+10EEEE once per
cell with:

- the image id in the **foreground colour**: 24 bits in true colour, or 8
  bits as a palette index;
- the placement id in the **underline colour**. It is optional: none (or 0)
  means "any virtual placement of this image";
- up to three combining diacritics from kitty's `rowcolumn-diacritics.txt`
  (297 of them): the row, the column, and bits 24–31 of the image id.

**Storage.** A placeholder is a cell of kind 2, narrow, content = row
(bits 0–8) and column (bits 9–17). The style holds `fg` (exactly as sent,
palette or RGB), `underline_color` and `image_id_high`. All cells of one
image share one style, so a 40×20 image costs 800 × 8 bytes and one style,
with no cluster storage.

**Missing diacritics are inferred step by step, as they arrive**, with
kitty's left-to-right rules. The cell to the left is a placeholder with the
same fg and underline colour, and is looked at once, when the base arrives:

1. on U+10EEEE: row = the left cell's row, column = its column + 1, high
   byte = its high byte (or 0, 0, 0 if there is no such cell);
2. on the row diacritic: row = that value; the column stays left + 1 only if
   the left cell has the same row, otherwise 0; the high byte likewise;
3. on the column diacritic: column = that value; the high byte stays the
   left cell's only if it has the same row and column − 1, otherwise 0;
4. on the third diacritic: high byte = that value.

So the cell is right after each step, and nothing waits for "the end of the
cluster". A guessed column can grow past the table's 296 along a long row;
it is kept up to 511 (the invariant is < 512), and a placeholder whose
guess would pass 511 is stored as a blank. Rows only come from diacritics
or the left cell, so they stay < 297. Inferring at write time matters: after
reflow the cell to the left may be on another row, or gone.

"Lossless" therefore means: the image id (all 32 bits), placement id, row
and column of every placeholder cell survive the screen, history, reflow
and pull-back exactly. History writes them compactly (§11.4: 53 bytes per
40-cell image row instead of 320 as UTF-8). Copying a selection gives a
space per placeholder cell, trimmed at the line end.

**Drawing** (B.3): the renderer finds the virtual placement by (image id,
placement id, or any placement of the image when the placement id is 0) in
the snapshot and draws, for each placeholder cell, the slice of the image
for (row, column), scaled to the placement's cell box, as an overlay
(§9.3). Rows with placeholders carry `RowFlags::PLACEHOLDERS`, and
placeholders break shaping runs (§10).

---

## 8. Images

### 8.1 Structures

```rust
/// One screen's images and where they are shown. The primary and the
/// alternate screen each have one, as kitty does.
pub struct Graphics {
    images: HashMap<ImageId, Image>,              // state (Pending | Ready(pixels) | Failed), size, last use
    /// kitty direct placements, by the stable line number of their top row.
    placements: BTreeMap<(u64, PlacementKey), Placement>,
    /// kitty `U=1` placements, and decline's own for Sixel/iTerm2 (§8.3),
    /// shown through placeholder cells only.
    virtual_placements: HashMap<(ImageId, u32), VirtualPlacement>,
    pixel_bytes: usize,                           // counted against the process budget
}

pub struct Placement {
    pub image: ImageId,
    pub id: u32,                 // kitty placement id (0 = none)
    pub line: u64, pub col: u16, // top-left cell, as a stable Point
    pub cols: u16, pub rows: u16,// size in cells
    pub offset: (u16, u16),      // pixel offset inside the top-left cell (kitty X, Y)
    pub src: [u32; 4],           // source rectangle in image pixels
    pub z: i32,                  // kitty z-index
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

Draw order:
1. clear;
2. `BelowBackground` placements;
3. cell backgrounds;
4. `BelowText` placements and placeholder images;
5. glyphs and decorations;
6. `AboveText` placements;
7. cursor overlays.

Within a layer, images are ordered by z, then by image id (the protocol's
rule).

### 8.2 Direct placements, keyed by stable line number

- **Output scrolls the whole screen:** nothing to do. The image's line
  number stays with its text as rows move up and into history.
- **Scroll regions and IL/DL:** a top margin above 0 or a bottom margin
  above the last row move rows inside the region past fixed line numbers.
  Placements entirely inside the region move by the scrolled amount. Those
  scrolled out of the region are clipped or dropped, and placements
  crossing a margin are left alone (the protocol says only images entirely
  within the margins scroll). Search hits and selection ends inside the
  region get the same treatment (§13.1). This costs O(positions on screen).
- **Freed with history blocks:** when pushing a line frees the oldest block,
  `Scrollback::push` returns the new first stored line, and the terminal
  drops placements whose bottom row is above it, in the same call.
  Placements are ordered by top line, so these are a range at the front of
  the map.
- **Alternate screen:** it has its own `Graphics`. Rows scrolling off it
  aren't kept, so its placements go when their bottom row leaves the
  screen. Entering the alternate screen clears its images (a protocol rule),
  and leaving it with 1049/1047 frees it altogether (§19).
- **ED 2 / ED 3 / RIS:** ED 2 and RIS delete the placements visible on the
  screen; ED 3 also deletes those in history. The protocol says the
  clear-screen code should also clear all images.
- **Reflow:** the anchor (line, col) is remapped like every other on-screen
  position (§13); the size in cells stays the same.
- **Image data:** an image is freed when nothing needs it and the program
  deletes it, or by least-recently-used eviction when the process-wide
  budget is exceeded. "Needs it" means a placement, a virtual placement,
  placeholder cells on the screen (found by the style sweep, since the ids
  are in styles), or a history block whose dictionary mentions it (each
  block lists the image ids it mentions, released when it is freed).

### 8.3 Sixel and iTerm2 images as placeholder cells

A Sixel image (`DCS P1;P2;P3 q … ST`, B.5) or an iTerm2 inline file
(`OSC 1337 ; File=…:<base64> BEL`, B.3) becomes:

1. an image with an id from decline's own range, and a virtual placement of
   its size in cells (`width`/`height` for iTerm2: cells, pixels, percent or
   auto);
2. kind-2 cells written at the cursor, one per covered cell, with a style
   whose fg is that id and whose attrs have `OWN_IMAGE`. They are written
   like text, replacing what was there; the cursor then moves as sixel
   scrolling mode, or iTerm2's rule, says.

The pieces are then ordinary cells:
- overwriting a cell with text, or erasing it, removes that piece of the
  image, as in xterm, foot, WezTerm and iTerm2;
- the image scrolls with its text, goes into history, comes back on
  pull-back, and is freed when no cell, screen or history, mentions it;
- programs can't address it with kitty commands, because `OWN_IMAGE` keeps
  the ids apart.

kitty direct placements stay in the placement table (§8.2), because kitty's
protocol says they are independent of the text under them.

One known effect: on reflow, an image row wider than the new width wraps
in pieces, as it does in WezTerm. If that looks bad in practice, rows whose
cells are all placeholders can be clipped instead of wrapped.

### 8.4 Decoding, keeping it out of the lock, and the budget

- **Who decodes.** term-core stays free of platform code: it defines an
  `ImageDecoder` trait, and term-app implements it with ImageIO. Tests use a
  pure-Rust PNG implementation.
- **Allowed formats.** PNG, JPEG and GIF only. The decoder checks
  `CGImageSourceGetType` before decoding anything, and reads the size from
  the image's properties first, so the pixel caps (§14) apply before any
  pixels are made.
- **Off the lock.** The reader thread stores the encoded bytes and marks
  the image `Pending`, then a background queue decodes it. When decoding
  finishes, the terminal is locked briefly to mark the image `Ready`, queue
  the kitty reply (`OK` or the error, unless `q=` silences it) and wake the
  view. A placement made while the image is pending is kept and drawn once
  it is ready. Raw RGB/RGBA (kitty `f=24/32`) and Sixel need no format
  decoder; term-core writes their pixels straight into the buffer below.
- **One copy of the pixels.** The trait also allocates the pixel buffer;
  term-app backs it with a shared-storage Metal buffer, and the renderer
  makes its texture from that buffer. On Apple Silicon's unified memory,
  that one buffer is both the CPU copy and the GPU copy. It is made
  purgeable while the window is hidden, and decoded again if macOS takes it.
- **The budget (Alex's decision): 32 MB per process by default,** as a
  setting (named with the E.9 config work, for example `image_memory`).
  Every byte of pixels decline holds counts once, whether the CPU, the GPU or
  (with the shared buffer) both use it. Encoded bytes waiting to be decoded
  count too. When a new image would pass the budget, images with nothing on
  screen are evicted, least recently used first; an image bigger than the
  whole budget is refused with an error reply.

---

## 9. Change counters, the snapshot and the row cache

### 9.1 Per-row change counters

The terminal keeps `counter: u64` and `batch_start: u64`. At the start of
each `advance`, and of resize, reset and other calls that change rows,
`batch_start = counter + 1`. Any change to a row first does:

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

So a row's `seq` is unique to (row, parse call), at the cost of one compare
per row segment written, not per cell. Things that don't change a row's
content keep its `seq`: full-screen scroll, region scroll, IL/DL (the row
moved but its cells didn't). The renderer can therefore key its row cache by
`seq` alone and reuse a row that only moved. Resize, reflow and renumbering
(§5.2) give every screen row a new `seq`.

**Counters and line numbers must never repeat for the renderer.** Three
things on `main` or in this design could make them repeat:

| Cause | Fix |
|---|---|
| RIS (`Terminal::reset`) builds a fresh `Terminal`, which would restart `counter` at 0 | `reset()` carries `counter` across, as it already carries line numbers, and bumps `history_epoch` |
| `session::replace_term` builds a new `Term` after a caught parser panic, restarting the counter and line numbers | every `Terminal` gets a process-wide unique `terminal_id` (an atomic counter) at creation; a new terminal means a new id |
| `Scrollback::pop()` (pull-back, §12.2) can hand a popped line's number to new text | history records the **lowest line number popped**, twice: since the last snapshot, and since the current search started. Only lines at or after that mark can have changed |
| ED 3 and RIS clear history | `history_epoch` is bumped by ED 3 and RIS (not by `pop()`) |

All three go in the snapshot (`terminal_id`, `history_epoch`,
`popped_from`). The renderer drops:
- its cached screen rows when `terminal_id` changes;
- all its cached history rows when `terminal_id` or `history_epoch`
  changes, or when the width changes;
- only the cached history rows at or after `popped_from` when that is set,
  so pulling a few lines back doesn't throw away the rest.

A running search drops only hits at or after its own popped mark, and starts
again after ED 3 or RIS (§13.4).

Tracked separately, because they are not row content:
- palette changes (OSC 4, 104, 10–12), tracked by `palette_seq`;
- DECSCNM reverse video.

The overlays (§9.3) are not cached at all. The global `generation` stays as
the "redraw needed" signal. WezTerm does the same kind of tracking with a
per-line sequence number (`Line::seqno`, `changed_since`).

### 9.2 What the snapshot holds

`Terminal::snapshot(&mut self, out: &mut Snapshot, have: impl Fn(RowKey) -> bool)`
runs under the lock. `have` asks the renderer's row cache whether it already
has a row, so only missing rows are copied. Every buffer is owned by the
renderer and reused, so a snapshot allocates nothing in steady state.

```rust
pub struct Snapshot {
    pub terminal_id: u64,                 // new terminal: drop every cached row and style
    pub history_epoch: u64,               // changed (ED 3, RIS): drop cached history rows
    pub popped_from: Option<u64>,         // lines at or after this were pulled back: drop those cached rows
    pub screen: ScreenId,                 // Primary or Alternate: which style mirror the ids refer to
    pub styles_epoch: u64,                // that screen's table epoch; changed: `style_updates` is the whole table
    pub counter: u64,                     // the terminal's counter when taken
    pub cols: u16, pub lines: u16,
    pub rows: Vec<SnapRow>,               // one per visible row, top to bottom
    pub cells: Vec<Cell>,                 // cells of the rows copied this time, back to back
    pub cluster_text: Vec<u8>,            // cluster text of those rows
    pub cluster_spans: Vec<(u32, u16)>,   // per copied row: slot → (offset, len) into cluster_text
    /// Screen style ids interned since the last snapshot (all of them after
    /// a renumbering or a terminal change). The renderer keeps a mirror of
    /// the table and applies these.
    pub style_updates: Vec<(u32, Style)>,
    /// Styles of the history rows copied this time; their cells' ids index here.
    pub history_styles: Vec<Style>,
    pub highlights: Vec<Highlight>,       // per row, see SnapRow
    pub cursor: SnapCursor,               // view row/col, shape, visible, blinking, on a wide cell
    pub palette: Option<Box<Palette>>,    // only when palette_seq changed
    pub reverse_video: bool,
    pub placements: Vec<SnapPlacement>,   // direct placements intersecting the view, in draw order
    pub virtual_placements: Vec<SnapVirtual>, // those referenced by placeholder cells in view
    pub new_images: Vec<(ImageId, u64, PixelHandle)>, // ready images not yet uploaded: shared, not copied
}

pub struct SnapRow {
    pub key: RowKey,                      // Screen(seq) or History { line, col } (stable first cell)
    pub flags: RowFlags,                  // wrapped, fill, placeholders, blinking, from history
    pub cells: Option<Range<u32>>,        // None: the renderer already has this row
    pub clusters: Range<u32>,             // into cluster_spans
    pub highlights: Range<u32>,           // into `highlights`
}

pub struct Highlight { pub cols: Range<u16>, pub kind: HighlightKind }
pub enum HighlightKind { Selection, SearchHit, CurrentSearchHit, HoveredLink }
```

- **Highlight ranges are worked out under the lock** (selection, search hits,
  and cells of the hovered link, which the app passes in as a point and
  term-core compares by value, §5.5). A history row shown at another width
  can span two stored lines, and only term-core can map its cells back to
  stable positions. This costs O(visible rows).
- **Styles are mirrored, not copied each frame.** `new_ids` lists ids
  interned since the last snapshot, and the snapshot copies just those. If
  the table has set `send_all` (the list would have passed half the table,
  §5.2), the snapshot copies the whole table instead. Steady-state frames
  copy no styles at all.
- **One mirror per screen.** The primary and alternate screens have separate
  tables with overlapping ids, so the renderer keeps a mirror for each.
  `screen` says which one a snapshot's cells refer to. When a screen's
  `styles_epoch` changes (renumbered, or the alternate table freed and made
  again), the renderer clears that screen's mirror and takes the whole
  table. Epochs come from a terminal-wide counter, so they never repeat.
- **Cost under the lock:**
  - a full 300×100 change copies 240 KB of cells plus the new styles, about
    15–25 µs;
  - a spinner copies one row (2.4 KB), under 1 µs;
  - scrolled back, history rows come from the R.3 decode cache.
- **Outside the lock:** building instances, shaping and rasterising. Until
  R.1 lands, the renderer builds from the snapshot while still holding the
  lock, so switching over is a matter of moving one call.

### 9.3 The renderer's row cache (R.2)

- A cached row holds its background runs, glyph instances and decorations,
  **relative to the row's top** (y = 0). The row's y is added when the frame
  is assembled, so a row that only moved (scroll, region scroll) is reused
  as is.
- Nothing that changes without the row changing is baked into it. These are
  drawn as **overlay instances** each frame:
  - the cursor: block cursors are a cursor-coloured rectangle plus the cell's
    glyph again in the cursor-text colour, clipped to the cell;
  - selection and search highlights: rectangles drawn after the backgrounds
    and before the glyphs;
  - the hovered link: an underline;
  - placeholder images;
  - blinking text: those glyphs are kept in a separate per-row list, drawn
    only in the "on" phase.
- So the cache key is just `RowKey`. The cache is dropped as §9.1 says.

---

## 10. Shaping runs, ligatures, glyph keys (term-font and term-render)

### 10.1 Runs

The renderer splits each row into runs. A run is the longest stretch of
cells that:

- are drawn by fonts. Not drawn by fonts:
  - code-drawn glyphs, which are whatever `term_font::sprite::is_code_drawn`
    says: box drawing U+2500–257F, blocks U+2580–259F, braille U+2800–28FF,
    Powerline U+E0A0–E0A3 and U+E0B0–E0BF, and legacy computing
    U+1FB00–1FB8B;
  - placeholder cells;
- and select the same font face: bold × italic, 4 faces.

Colour, background, underline, strike, inverse, links, the cursor and the
selection never split a run, so `=>` in two colours, or with the cursor on
`>`, is still one ligature. Spacer cells contribute no text. Blank cells
are part of the run, as spaces.

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

`Fonts::shape(text: &[u16], face, features, out: &mut Vec<ShapedGlyph>)`:
1. builds a CTLine from the run's text, with the face and `font_features`;
2. walks its CTRuns, putting each run's font through the registry;
3. maps each glyph's string index back to its cell, through a
   UTF-16-index-to-cell table built with the run text.

Glyphs are placed on the cell grid: each sits at its cell's x plus its
offset inside its cluster, not at Core Text's accumulated advance. So a
fallback font with a different advance can't drift.

**Shaped-cluster cache.** A least-recently-used map per process (shared by
all panes, like B.2's single glyph atlas) maps (font face, cluster text) to
the shaped glyphs, up to 4096 entries. An emoji or an Indic word is shaped
once, not once per row that contains it.

**Fast path.** When the configured font has no ligature features turned on
(Menlo, SF Mono), or ligatures are off:
- ASCII cells use a 128-entry (face, byte) → glyph table;
- other single scalars use a cached cmap lookup, with Core Text's fallback
  for that one character (what `glyph_for` does today);
- CTLine is used only for cluster cells, through the cache.

With a ligature font (JetBrains Mono, Fira Code), rows are shaped and the
result is cached by the row's `seq` (B.1). Both paths produce the same
`GlyphKey`s.

### 10.3 Ligatures: one glyph, one instance per cell

Most coding fonts implement `=>` with contextual alternates, one glyph per
cell, which needs nothing special. A true multi-cell ligature glyph
covering cells `a..b` is emitted once **per cell**:
- every piece uses the same texture rectangle;
- each quad is clipped on the CPU to its cell's columns (x, w and u
  adjusted). The first piece stays open to the left and the last to the
  right, so overhangs still draw;
- each piece is drawn in its own cell's foreground colour.

This needs no change to the instance format or the shader.

### 10.4 Code-drawn glyphs: sprite ids (F.6, landed)

`term_font::sprite`, landed in `c4033dd`, draws box drawing, blocks,
braille, Powerline and legacy computing shapes on the cell's own pixel grid
into R8 coverage bitmaps, with `rasterize(id, metrics, cells)`. Each shape
has a `SpriteId(u16)` that maps one-to-one onto its character (0x000–0x09F
box drawing and blocks, 0x100–0x1FF braille, 0x200–0x21F Powerline,
0x300–0x38B legacy computing). Ids from 0x1000 up are kept for shapes that
aren't characters. Still to come for this design:

- the curly, dotted and dashed underline patterns as sprites from 0x1000
  (F2.3);
- caching sprites under `GlyphKey { font: SPRITE_FONT, glyph: id, cells }`
  in the same mask texture as text (F3.2). Today the renderer reaches them
  through the per-character path.

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

Each block keeps every line's text **contiguous**, in its own array, apart
from the rest of each line (its flags, runs and other sections). Search
runs SIMD substring search straight over that array (§13.5), and VoiceOver
counts UTF-16 over it (§13.10).

A block is in two parts. What a sealed block holds never changes and is
shared; what history knows about it (how many of its lines are still in
use, counts worked out later, the filter) changes, and only under the lock:

```rust
/// What a sealed block holds: immutable, shared as Arc<BlockData>. The
/// renderer's decode cache, the search worker and VoiceOver read it without
/// the lock.
pub struct BlockData {
    meta: Box<[u8]>,          // each line's flags and sections (§11.3); ≤ 65,536 bytes
    starts: Box<[u16]>,       // offset of each line in `meta`
    text: Box<[u8]>,          // every line's text back to back; a 0x0A ends each logical line; ≤ 65,536 bytes
    checkpoints: Box<[[u16; 2]]>, // every 16th line: its offset in `text`, and the UTF-16 offset there
    styles: Box<[u8]>,        // the block's style dictionary, encoded (§11.5); ≤ 65,536 bytes
    style_starts: Box<[u16]>, // id 0 = default, implicit
    links: Box<[u8]>,         // the block's hyperlinks: URI + id parameter; ≤ 65,536 bytes
    link_starts: Box<[u16]>,
    images: Box<[ImageId]>,   // decline's own and kitty images its dictionary mentions (§8.2)
    first: u64,               // stable line number of the first stored line
    width: u16,               // width every line here was written at, 0 if mixed
    ambiguous: u8,            // the ambiguous-width setting its lines were written with (§6.3)
    has_marks: bool,          // some line here has prompt marks
    head_continues: bool,     // the first line continues a logical line from the block before
}

/// History's own record of a block, owned by Scrollback, changed only under the lock.
struct BlockEntry {
    data: Arc<BlockData>,
    len: u32,                 // lines still in use; pop() lowers it, the shared data is untouched
    utf16_len: u32,           // UTF-16 units in the first `len` lines (placeholders as spaces)
    rows_at: (u16, u32),      // (width, rows the logical lines starting here take), worked out lazily
    filter: Option<Arc<Filter>>, // 4 KB, built in the background (§13.6); None = just scan
}

/// The block being written: the same parts as growable vectors, plus the
/// encoder's style and link maps. Sealing turns it into a BlockData.
struct OpenBlock { /* … */ }
```

Because `rows_at` is a plain field of the entry, updated through `&mut`
under the lock, nothing shared holds a `Cell` (which isn't `Sync`). Readers
that fill it in already run under the lock. The worker can also work counts
out on its own copy and store them afterwards (§13.10).

- **Finding a line's text.** Line `k`'s text starts at
  `checkpoints[k / 16]`, plus the text lengths of the lines in between: at
  most 15 of them, each found by its stored length (wrapped lines) or by
  `memchr` for its end byte. Checkpoints cost 4 bytes per 16 lines, about
  0.25 B per line.
- **Sealing.** A block is sealed when the next line would take `meta`,
  `text`, the style dictionary or the link table past 64 KB, so every offset
  fits in 16 bits. It is also sealed, if it holds any lines, when the
  ambiguous-width setting changes. Today's offsets are 32 bits, so the
  16-bit ones save 2 bytes per line. The open block never holds more than
  64 KB of capacity in each part (today's doubling can reach 128 KB).
- **Lines per block.** For text-heavy output the text part fills first:
  about 1,600 plain lines (40 bytes of text each), 1,350 `ls` lines or 1,000
  CJK lines per block. A full 64 KB of text, plus meta, means a block is up
  to about 70–80 KB in all.
- **Line size.** A stored line can't exceed 64 KB: it has at most 4096 cells,
  and cluster text is capped at 8 KB per row.
- **Sharing.** A search captures `(Arc<BlockData>, len)` for each block and
  scans only the first `len` lines (§13.4). A block freed from history while
  a search holds it lives until the search lets go. R.7's compression
  replaces an entry's `data` with a compressed `BlockData`; a running search
  keeps the old `Arc` (§11.8).
- **Writing history when the screen scrolls.** For the open block only, the
  encoder keeps a `HashMap<Style, u32>` (and one for links) from style to
  dictionary slot, dropped on sealing. In front of it sits a 64-entry cache
  indexed by the screen style id (`id & 63` → `(id, slot)`), so the common
  case is one array compare per run, with no hashing. The cache is reset
  when the block is sealed and after every style sweep, because a sweep may
  reuse ids. The encoder also adds each line's UTF-16 length to
  `utf16_len`, counted without branches (could): bytes − continuation bytes
  (0x80–0xBF) + 4-byte lead bytes (≥ 0xF0), which vectorises; for ASCII it
  is simply the byte length.
- **`pop()`** removes the newest stored line, for pulling lines back
  (§12.2). In the open block it truncates `meta`, `text` and `starts`. For a
  sealed block it lowers the entry's `len` and `utf16_len` and leaves the
  shared data alone (could: a sealed block is never reopened); the bytes
  stay until the block is freed, and the next push starts a new open block.
  Popping records the lowest line popped (§9.1). Dictionary entries left
  unused are freed with the block.
- **Limits (Alex's decision).**
  - the limit is lines: the existing `scrollback = 10000` setting, counted
    in stored lines;
  - a **hidden 32 MB backstop** catches gradient-heavy output. It counts
    every byte a block holds: text, meta, dictionary, links, filters and all
    the offset arrays. It isn't a config key;
  - both free whole blocks from the oldest end. Until the oldest block can
    be freed whole, it still holds some lines past the limit, as today.
    Those lines are hidden: searches, `select_all` and UTF-16 offsets start
    at the first line within the limit (could);
  - the Ghostty comparison is made at the **same number of lines kept**.
    Ghostty's limit is in bytes, so the comparison script raises it until
    Ghostty keeps the same number of lines, then measures both footprints.

### 11.3 A stored line, byte by byte

```text
in `meta`:  flags:u8 [extra:u8] [tlen:varint] [runs] [clusters] [widths] [marks] [placeholders]
in `text`:  the line's text, then 0x0A if the line ends its logical line

flags  bit 0  WRAPPED   the logical line continues on the next stored line (`tlen` follows)
       bit 1  EDGE      the last column was an edge spacer (a wide character moved to the next row)
       bit 2  STYLED    runs follow
       bit 3  CLUSTERS  a cluster list follows
       bit 4  WIDTHS    width exceptions follow
       bit 5  FILL      the last run's style fills to the end of the row, whatever the width
       bit 6  MARKS     prompt marks follow
       bit 7  EXTRA     the `extra` byte follows
extra  bit 0  PLACEHOLDERS  a placeholder list follows
       bits 1–2             reserved: double width / double height (top or bottom) for DECDWL/DECDHL
       bits 3–7             zero

tlen   the text length, only for WRAPPED lines (their text has no end byte)
text   UTF-8, one entry per cell up to the last cell that isn't a blank space
         (wrapped lines keep all their cells, trailing blanks as spaces, so the
         joined logical line is exact):
         a scalar cell → the scalar; a cluster cell → all its scalars;
         a blank cell before that point → 0x20; a placeholder cell → 0x00
         (never matches a search, copied as a space); spacers → nothing
runs   count:varint, then per run  varint(cells << 3 | slot)  [index:varint if slot == 7]
         slot 0–6: the block's style #slot directly (0 = default); 7: the index follows.
         Runs count columns (a wide cell's two halves are in the same run).
         Cells after the last run are default style; cells after the text but inside
         the runs are blank cells in the run's style (this is how trailing fill is stored)
clusters  count:varint, then per cluster  gap:varint (scalars since the previous
         cluster ended)  extra:varint (scalars after the first that belong to it)
widths count:varint, then per exception  gap:varint (cells)  width:u8
         only cells whose width isn't what the table, the 2027 rule and the block's
         ambiguous setting give (a cluster written with 2027 off; a row written
         before an ambiguous-width change)
marks  count:u8, then per mark  kind:u8 (1 prompt A, 2 input B, 3 output C,
         4 end D)  col:varint  [status:varint for D with an exit status]
placeholders  count:varint, then per stretch  gap:varint (cells)  n:varint  row:varint
         col:varint: n cells with this row and columns col, col+1, …
```

- **Cell count.** It is the largest of what the text, the runs and the
  placeholders cover, plus one for EDGE. Unwrapped lines drop trailing
  blanks, as today. Wrapped lines keep theirs, so no separate cell count is
  needed.
- **Spaces.** A typed space and an empty cell both come back as an empty
  cell, as today.
- **`FILL`** is written when the row's last column is a blank cell whose
  style shows when empty (a full-width background from EL/ED or a padded
  status line). Re-splitting (§12) and drawing at another width extend that
  style to the row's end, so `delta`'s diff lines and status bars stay full
  width after the window widens.
- **Widths.** The decoder takes widths from the same table, 2027 rule and
  ambiguous setting (the block's own) as the writer. History lives only as
  long as the process, so the table can't change underneath it. Exceptions
  cover anything else. That is what removes today's per-line "wide" runs
  from CJK text.

### 11.4 Worked examples (the `scrollback_mem` inputs, 160 columns)

"Today" is the current format: sealed bytes per line, then what
`scrollback_mem` measures (which includes the open block's spare capacity).
v2 counts sealed bytes per line: meta + text + the 2-byte offset
(checkpoints add about 0.25 B per line on top).

**Plain** `00012345 the quick brown fox jumps over` (39 cells):

```text
meta: 00                    flags: nothing else
text: 30 30 30 31 … 72 0A   39 bytes + the end byte
                            = 1 + 40 + 2 = 43
today: 00 flags, 27 text length, 39 text, 00 runs, + 4 offset = 46 (52 measured)
```

**`ls --color`** `dir12345  build.sh  notes.txt  src  archive.tar` (47 cells;
`dir…` and `src` bold blue, `build.sh` green, `archive.tar` red):

```text
meta: 04                    flags: STYLED
      07                    7 runs
      41                    8 cells,  slot 1 (bold blue)     8<<3|1 = 65
      10                    2 cells,  slot 0 (default)
      42                    8 cells,  slot 2 (green)
      68                    13 cells, slot 0                 13<<3 = 104
      19                    3 cells,  slot 1
      10                    2 cells,  slot 0
      5B                    11 cells, slot 3 (red)           11<<3|3 = 91
text: 64 69 72 31 … 72 0A   47 bytes + the end byte
                            = 9 + 48 + 2 = 59   (today 86 sealed, 93 measured)
block dictionary, once per block: #1 41 04 01 (fg palette 4, attrs bold)
                                  #2 01 02    #3 01 01
```

**Full-width background** `ESC[48;5;22m+12345` padded to 159 cells:

```text
meta: 24                    flags: STYLED | FILL
      01                    1 run
      F9 09                 159 cells, slot 1                159<<3|1 = 1273
text: 2B 31 32 33 34 35 0A  "+12345" (the 153 trailing spaces are not stored)
                            = 4 + 7 + 2 = 13   (today 174 sealed, 183 measured)
dictionary: #1 04 16 (bg palette 22)
```

(The `scrollback_mem` line stops one column short of the edge; a real
EL-to-the-end line sets `FILL` the same way.)

**Compiler error** (`error[E12345]` bold red 13 cells, `: mismatched
types` bold 18 cells, ` --> src/main.rs:12345:5` default 24 cells):

```text
meta: 04 02 69 92 01        runs: 13<<3|1 = 105 (69); 18<<3|2 = 146 (92 01);
                            the default tail needs no run
text: <55 bytes> 0A
                            = 5 + 56 + 2 = 63   (today 74 sealed, 81 measured)
```

**CJK** `12345 漢字かなカナ한글 混合テキスト 中文字符` (26 scalars, 62 bytes of
UTF-8, 44 cells):

```text
meta: 00                    widths come from the table: no runs, no exceptions
text: <62 bytes> 0A
                            = 1 + 63 + 2 = 66   (today 87 sealed, 90 measured: today's
                                                 encoding needs a "wide" run per stretch)
```

**Emoji-heavy** (new) `12345 ✅ done 👍🏽 ship 🇬🇧 ❤️ 👨‍👩‍👧 ok` (35 scalars,
66 bytes; four clusters):

```text
meta: 08                    flags: CLUSTERS
      04                    4 clusters
      0D 01                 👍🏽: starts 13 scalars in, 1 more scalar
      06 01                 🇬🇧
      01 01                 ❤️ (with VS16)
      01 04                 👨‍👩‍👧: 4 more scalars (ZWJ, 👩, ZWJ, 👧)
text: <66 bytes> 0A
                            = 10 + 67 + 2 = 79   (today: not stored correctly)
```

**A row of a placeholder image** (40 cells, row 3, image in style slot 1):

```text
meta: 84 01                 flags STYLED | EXTRA, extra PLACEHOLDERS
      01 C1 02              1 run: 40 cells, slot 1
      01 00 28 03 00        1 stretch: gap 0, 40 cells, row 3, columns from 0
text: 40 × 00, 0A
                            = 10 + 41 + 2 = 53   (as UTF-8 with diacritics: 320+)
```

**A prompt line with OSC 133 marks** (A at column 0, B at column 8):
`MARKS` adds `02 01 00 02 08` to the meta, 5 bytes.

**A wrapped row** (160 plain cells, continuing on the next line): meta
`01 A0 01` (WRAPPED, tlen 160), text 160 bytes with no end byte: 3 + 160 +
2 = 165 (today 168 sealed).

**Gradient** (every cell a different RGB foreground, 160 cells), per cell:
- 1 byte of text;
- a 3-byte run (slot 7 + a 2-byte index; a block's dictionary holds up to
  ~16k such styles);
- a 4-byte dictionary entry;
- its 2-byte dictionary offset.

That is about 10 bytes per cell, **~1,600 bytes per line, against ~1,120
today (+43%)**. This is the case the hidden backstop is for.

| Input | Today sealed | Today measured | v2 | Change |
|---|---|---|---|---|
| plain 40 chars | 46 | 52 | **43** | −7% |
| `ls --color` | 86 | 93 | **59** | −31% |
| compiler error | 74 | 81 | **63** | −15% |
| full-width background | 174 | 183 | **13** | −93% |
| CJK | 87 | 90 | **66** | −24% |
| emoji-heavy | – | – | **79** | (today lossy) |
| gradient, 160 cells | ~1,120 | – | ~1,600 | +43% |

The measured figures will add about 3–6 bytes per line for the open block,
as they do today. Search filters, once built, add about 2.2–6.4 B per line
(4 KB per block, §13.6). They are counted separately, because they are
built in the background after a block is sealed. `scripts/bench-limits.json` gets lower limits with the v2 PR.
Suggested limits:
- plain 50, `ls` 70, compiler 75, full-width background 25, CJK 75;
- new entries: emoji 95, a placeholder row 60, and a gradient line 1,800
  (so the gradient figure can't creep up unseen).

### 11.5 The per-block style dictionary and link table

```text
style := head:u8 [fg] [bg] [ul] [attrs:varint] [more:u8 [underline:u8] [link:varint] [image_high:u8]]
head   bits 0–1 fg kind, 2–3 bg kind, 4–5 underline-colour kind (0 default, 1 palette, 2 RGB),
       bit 6 attrs follow, bit 7 `more` follows (bit 0 underline style, 1 link, 2 image id high byte)
colour palette → 1 byte; RGB → 3 bytes
```

- **Examples.** Bold blue is `41 04 01`, green `01 02`, a background-only
  palette colour `04 16`, an RGB foreground `02 rr gg bb`.
- **Links.** `link` is an index into the block's own link table (URI and id
  parameter), which goes when the block goes.
- **Decoding** a line decodes only the styles its runs use.

### 11.6 Prompt marks

- **On the screen**, marks live in the row's `extra` (kind, column, and the
  exit status for D), up to four per row. A and B usually share the prompt's
  row, and the previous command's D may be there too.
- **Reflow** moves them with their cells.
- **Into history** they go with the line (`MARKS`).
- **Jumping to the previous prompt** skips blocks without `has_marks`, then
  reads stored lines' flags bytes in `meta`; no text or cells are decoded.

### 11.7 Image references

Direct placements are not stored in the history bytes: they live in the
screen's placement table, keyed by stable line number, and are freed in the
same call that frees a block (§8.2). Placeholder cells, kitty's and
decline's own, are stored in the line (§11.3), and each block lists the
image ids its dictionary mentions.

### 11.8 What R.7's compression leaves uncompressed

R.7 compresses idle sealed blocks with LZ4. It does this by building a
compressed `BlockData` and swapping it into the block's entry under the
lock. A search or decode that still holds the old `Arc` carries on with it,
and the entry's own fields (`len`, `utf16_len`, `rows_at`, the filter) don't
change. Some parts stay outside the compressed data, so that line counts,
prompt jumps, rows-at-width counts, line lookups, UTF-16 offsets and search
filtering keep working without decompressing anything:
- `starts` and `checkpoints`;
- each line's flags and extra bytes and its text length, about 3 bytes per
  line, kept as side arrays;
- `has_marks`, `width`, `ambiguous` and the image list (and, in the entry,
  `rows_at`, `utf16_len` and the filter).

What is compressed:
- `text`;
- the sections in `meta`;
- the dictionary and the link table.

Style indices instead of inline RGB compress better too. A search
decompresses a block only when its filter lets it through (§13.6). R.7's
target, 100k lines of `ls`/log output ≤ 3 MB, starts from 5.9 MB instead of
9.3.

---

## 12. Reflow on resize

### 12.1 The rule

The **screen** is reflowed eagerly, on every resize event. **History is
never rewritten**; stored lines keep their width and numbers, and rows at
the current width are worked out from logical lines only when something
reads them. The alternate screen is not reflowed (programs redraw it on
SIGWINCH), as in xterm, kitty and Ghostty. While it is active, the primary
screen's reflow waits until the program leaves it, so a resize during Vim
reflows the primary screen once. Columns are clamped to 4096.

### 12.2 Reflowing the screen

```text
resize(cols, lines), primary screen (style sweeps blocked throughout, §5.2):
  same width:
    shrink: drop blank rows below the cursor first, then move rows from the top into history
    grow:   pull whole logical lines back from history, newest first, while they fit;
            the cursor moves down with them; then add blank rows at the bottom
  new width:
    1. if the first screen row continues a logical line from history, that line's stored
       lines are going to be pulled back (up to 4 × `lines` of them; older ones stay stored)
    2. note every tracked position as (logical line, cell offset): the cursor, saved cursors,
       selection ends, placement anchors, the view position — those on the screen **and
       those inside the stored lines about to be pulled back**, before pop(). Search hits
       on the screen and in pulled-back lines are dropped instead: the new screen is
       searched again afterwards (step 6)
    3. pop() those stored lines and decode them
    4. re-split each logical line at the new width: copy cells, re-insert clusters into
       the new rows' stores, move prompt marks with their cells, put an edge spacer where
       a wide character doesn't fit, extend a FILL line's style to the new row end; trim
       trailing blank cells, but never before the cursor
    5. if there are more rows than `lines`, move rows from the top into history (as stored
       lines at the new width) until the cursor row fits; if fewer, pull more logical lines
       back from history (as in "grow", noting positions inside them first)
    6. turn the noted positions back into Points; every screen row gets a new seq; with
       the find bar open, search the new screen again
  then: unblock sweeps and sweep once
```

- **Shrinking never loses text.** Blank rows below the cursor are dropped
  first, then top rows go into history. The ignored conformance test
  `resize_shrink_height_below_cursor_is_dropped` follows xterm, which drops
  the rows below the cursor; it is changed to expect what we do (§19).
- **Pulling back:** `Scrollback::pop()` the stored lines of the newest
  logical line, decode them, and re-split them at the current width. Only
  whole logical lines are pulled, so nothing is split oddly; the newest one
  is skipped if it wouldn't fit. This makes
  `resize_grow_height_pulls_from_scrollback` pass.
- **Very long lines:** when step 1 hits its limit (one logical line longer
  than four screens, such as the "one 50 MB line" pressure test), the older
  part stays stored, and where it meets the rest of the line one short row
  shows when scrolled back. Accepted.

### 12.3 History at another width

- **Counting rows.** Rows at the current width are worked out per logical
  line, using each cell's width as written (the block's ambiguous setting and
  its exceptions). An ASCII line of `n` cells takes `max(1, ⌈n / cols⌉)`
  rows; `n` comes from the stored line's text length, with no decoding.
  Lines with wide characters are walked to place edge spacers, one width
  lookup per scalar.
- **`FILL` lines** fill their last row to the current width.
- **Caching.** Each block caches the row count of the logical lines starting
  in it, for one width at a time (`rows_at`). Blocks whose lines were all
  written at the current width take one row per stored line, with no work.
- **The view position** is `ViewTop::Bottom` (following output) or
  `ViewTop::At { point, offset }`:
  - `point` is the stable Point of the first cell shown;
  - `offset` is that cell's offset in its logical line;
  - when the width changes, `point` is worked out again once, from the
    logical line's start and `offset` rounded down to a row start at the new
    width.
- **Drawing** walks forward `lines` rows from `point`: O(lines), whatever
  the history size. Scrolling walks from there too.
- **Output arriving underneath** changes nothing (today `display_offset` is
  bumped for each line pushed).
- **Scroll to top and select-all** start from the first stored line and need
  no count. Only `display_offset()` as a number needs the count from the
  bottom (the test dump; a scrollbar, which the zero-UI goal rules out). It
  works the count out from the block caches. VoiceOver's line numbers use
  the same counts (§13.10).

### 12.4 The 120 Hz, 100k-line budget

Per resize event at 300×100 with 100k lines of history:

| Step | Cost |
|---|---|
| screen re-split: 30k cells | ~0.1–0.2 ms |
| pull back up to 4 × `lines` = 400 stored lines (decode) | up to ~0.2 ms |
| push back ≤ 100 rows (encode) | ~50 µs |
| tracked positions (cursor, saved cursors, selection, placements, view): a handful | < 10 µs |
| search hits, with the find bar open: history hits untouched (kept per block); the new screen searched again | ~30 µs |
| one style sweep afterwards | ~30 µs |
| history | **0** (nothing rewritten; block caches checked lazily) |
| drawing while scrolled back: decode ≤ 100 logical lines | ~0.1 ms |
| frame rebuild (every row is new: the width changed) | ~0.05 ms CPU (today's full build) |

That comes to **0.5–1 ms**, well inside 8 ms, so the terminal itself needs
no debounce. The session still sends at most one SIGWINCH per frame. Working
out the full count after a width change, which only `display_offset()` and
VoiceOver's line numbers need, costs about 1 ms for 100k ASCII lines and
about 10 ms for 100k CJK lines, once per width.

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
| region scroll, IL/DL | **selection ends and search hits inside the region move with it; one that leaves the region clears the selection or drops the hit**; placements likewise (§8.2) | — |
| a block is freed | — | points above the first stored line are gone: selections are clipped, search hits dropped, the view moves to the first stored line |
| resize, same width | unchanged; pulled-back lines keep their numbers | unchanged |
| resize, new width | remapped in the same call (§12.2), including those inside pulled-back lines | **unchanged**: stored lines are never renumbered |
| alternate screen | numbered after the primary's, so the two can't be confused (as today) | unchanged |
| ED 3, RIS | history cleared, `history_epoch` bumped | cleared |

**Numbers are never reused while their line is stored.** A reflow gives the
re-split screen rows numbers that follow the end of history, so a number
that belonged to an old screen row or a pulled-back line can be handed to
different text. That is safe for three reasons:
- everything in term-core that holds a Point is remapped or cleared in the
  same call: the cursor, saved cursors, selection, placements, the view
  position and the grapheme state. Search hits on the screen and in
  pulled-back lines are dropped, and the screen is searched again;
- the renderer keys screen rows by change counter, and drops cached history
  rows at or after the lowest popped line (§9.1);
- **the app holds no Points and does no line arithmetic** (§13.2).

Ordering is text order: (line, col) sorts by stored line, and the stored
lines of one logical line have consecutive numbers.

### 13.2 Selection, and the API the app uses

Today term-app and term-render do Point arithmetic themselves:
- `selectAll:` in view.rs works out the first line from `scrollback_len` and
  `display_offset`;
- double- and triple-click selection in view.rs read a visible row's
  characters and build Points from columns;
- `frame.rs` finds the cursor with `cursor.row + display_offset()`.

None of that survives reflow or history shown at another width. term-core
provides instead:

```rust
impl Terminal {
    /// The stable position of what is shown at this view row and column.
    pub fn point_at(&self, view_line: usize, col: usize) -> Point;
    /// Where `p` is shown now, as (view row, column); None if it isn't in view.
    pub fn view_position(&self, p: Point) -> Option<(usize, usize)>;
    /// Scroll so that `p`'s row is in view (to the middle if it was far off).
    pub fn scroll_to(&mut self, p: Point);
    /// Select the word at `p`, on its logical line (a word can cross a soft wrap).
    pub fn select_word(&mut self, p: Point);
    /// Select the whole logical line at `p`.
    pub fn select_line(&mut self, p: Point);
    pub fn select_all(&mut self);
    pub fn extend_selection(&mut self, to: Point);
    /// Where the cursor is in the view, if it is in view.
    pub fn cursor_in_view(&self) -> Option<(usize, usize)>;
    /// The hyperlink at `p`, by value (URI and id parameter). None until links land (PR 9).
    pub fn link_at(&self, p: Point) -> Option<LinkValue>;
    /// VoiceOver (§13.10): UTF-16 offset of `p`, the Point at an offset, and
    /// the piece of text holding an offset.
    pub fn utf16_offset(&self, p: Point) -> usize;
    pub fn point_at_utf16(&self, offset: usize) -> Point;
    pub fn utf16_piece(&self, offset: usize) -> (Range<usize>, Utf16Piece);
}
```

What `point_at` returns at the edges:
- **on the right half of a wide character:** the wide character's own Point
  (its left column), so a click on either half selects the whole character;
- **past the end of a row's text:** the Point at that column, beyond the
  text. Selection treats it as the line's end, and `selection_text()` trims
  it;
- **below the last row** (a click in the bottom padding): clamped to the
  last view row.

**Two word rules.** Double-click keeps today's path-friendly rule: anything
but whitespace and a few separators is part of a word, so a double-click
takes a whole path or URL. That rule moves from `term-app/src/mouse.rs` into
term-core, with its tests, for `select_word`. Search's whole-word matching
uses a rule of its own (§13.8). The PR that adds this API also changes
term-app (view.rs) and term-render (frame.rs) to use it (§17.1, PR 2), and
`Terminal`'s public fields (`cursor`, `selection`, `modes`, …) become private
behind accessors in PR 1.

- **`Selection`** is `{ anchor: Point, head: Point }` plus a kind (normal,
  word, line, rectangular). Today it is cleared on resize; with this model
  it survives reflow, with screen ends remapped and history ends unchanged.
- **Rectangular selections** are stored as line numbers plus columns at the
  current width (could). They survive scrolling and are cleared when the
  width changes, since they have no meaning across reflow.
- **`selection_text()`** walks logical lines between the two points and
  joins wrapped stored lines without newlines, so copying a wrapped line
  gives one line at any width. Clusters copy as their text; placeholder
  cells as spaces, trimmed at the line end.
- **Copying whole history lines** takes the stored text directly, skipping
  the cell decode (could).

### 13.3 Search: what it has to do

Alex's decisions: search (Cmd+F, E.7) and VoiceOver (E.12) cover **all of
history**, search shows what is in view first, and searching history must be
fast. Cmd+F uses our own native-style find bar (§13.9). The engine below
lives in term-core (no platform code); term-app owns the bar and the worker
thread.

Requirements, as designed here:
- in-view hits within one frame of typing;
- history searched outwards from the view, nearest first, results streaming
  in;
- typing, output and drawing never wait behind a search;
- a full scan of 100k lines in well under a millisecond (measured: 0.11–0.71
  ms), and less still for rare words once filters exist;
- hits that stay right through output, scrolling and reflow, with no
  duplicates.

What is searched:
- **On the primary screen:** the screen and all of history, from the first
  line within the limit (hidden lines at the start of the oldest block are
  skipped).
- **On the alternate screen:** only the alternate screen, which has no
  history. Switching screens with the bar open starts the search again on
  the new screen.
- **After a caught parser panic,** the session's new terminal has a new
  `terminal_id`; the app starts the search again with the same query.

### 13.4 How a search runs

```rust
pub struct Query {
    pub text: String,
    pub ignore_case: bool,
    pub word: WordMatch,          // Contains, StartsWith, FullWord (the bar's menu); EndsWith (find pasteboard only)
}

impl Terminal {
    /// Under the lock: search what is in view and the rest of the screen now,
    /// and return a job for history.
    pub fn start_search(&mut self, q: Query) -> SearchJob;
    /// Under the lock, briefly: add a batch of history hits from the worker.
    pub fn add_search_hits(&mut self, batch: HitBatch);
}

pub struct SearchJob {
    matcher: Arc<Matcher>,          // the compiled query (§13.5) and its 4-byte grams
    blocks: Vec<BlockRef>,          // per sealed block, oldest first: see below
    open: OpenBlockCopy,            // the open block's meta and text, copied (≤ 128 KB)
    start: usize,                   // the block holding the view's top row
    cancel: Arc<AtomicU64>,         // the pane's search generation; a newer one stops this job
}

pub struct BlockRef {
    data: Arc<BlockData>,           // one refcount bump
    len: u32,                       // lines in use when the job was taken: only these are scanned
    text_end: u32,                  // end of those lines' text (up to 65,536)
    filter: Option<Arc<Filter>>,    // None: scan
}
```

The steps:

1. **Under the lock, about 0.1 ms** (`start_search`):
   - the rows in view are searched first, then the rest of the screen. Their
     text is built from cells (≤ 30k cells at 300×100, ~30 µs). Their hits
     go straight into the hit list and are drawn in the next frame. A match
     that starts in history and ends on screen row 0 (screen row 0 continues
     a wrapped line) is found here too, using the boundary rule in §13.5;
   - the job is taken: `(Arc<BlockData>, len)` and the filter for each sealed
     block, a copy of the open block, the block nearest the view, and a new
     search generation. History's "lowest line popped since this search
     started" mark is reset (§9.1).
2. **Off the lock, on the search worker.** There is one worker thread per
   process, and one job per pane at a time. It visits blocks by distance
   from the view: the view's own block first, then alternately the next
   older and the next newer, so hits near what the user is looking at come
   first. For each block it:
   - checks the filter (§13.6) if the block has one, and skips the block, or
     scans the text of its first `len` lines (§13.5);
   - checks the boundary with the block before it (§13.5);
   - turns each hit into Points (§13.8), which it can do without the lock
     because `BlockData` is immutable;
   - checks `cancel` between blocks.
3. **Back on the main thread,** each batch is added under the lock in
   O(batch). It drops hits on lines no longer stored, and hits at or after
   the lowest line popped since the search started (pulled back by a
   resize; those lines are searched again as live output, step 4). If any
   hit is in view, a redraw is asked for, and the bar's count is updated.
   After ED 3 or RIS (`history_epoch` changed) the search starts again.

Typing never waits behind a search. The lock is held for step 1 and for
merging batches, never while scanning history. Cancelling a stale search
costs at most one block's scan (a few µs).

**Live output while the bar is open: one rule, no duplicates.**
- Hits are keyed by their start Point, and adding a hit that is already
  there does nothing. So the same match found twice (by the worker and by
  the rules below) is kept once.
- **History:** once per frame (at snapshot time), the open block's text
  added since `searched_to` is searched, starting `q − 1` bytes before
  `searched_to` so a match spanning the old and new text is found; then
  `searched_to` moves to the end. `pop()` lowers `searched_to` to the popped
  line, so lines pushed back after a resize are searched again. Sealing
  hands the block to history unchanged; nothing is searched twice.
- **Screen:** at snapshot time, each row whose `seq` differs from the `seq`
  it was last searched at is searched again, over its whole logical line, so
  a match spanning a wrap onto an unchanged row is found. Its old screen
  hits are replaced.
- **Freed blocks** take their hits with them.

**How hits are kept.** Per block, plus the screen's own list:

```rust
pub struct SearchHits {
    blocks: VecDeque<(u64, Vec<Hit>)>, // (first line of the block, its hits in text order)
    screen: Vec<Hit>,
    total: usize,                      // capped at 100k (§19)
}
/// 16 bytes (could): start line, start column, how many lines on it ends, end column.
pub struct Hit { line: u64, col: u16, end_lines: u16, end_col: u16, _spare: u16 }
```

Freeing the oldest block pops the front entry, O(1). Reflow leaves history
hits alone and searches the new screen again (§12.2), so no hit is ever
remapped one by one.

### 13.5 Fast block search

- **One contiguous text per block.** `text` holds every line's UTF-8 back
  to back. Wrapped lines run on with no separator, so a match can cross a
  soft wrap. A 0x0A ends each logical line, so a match never crosses a real
  line end unless the query contains a newline. Placeholder cells are 0x00,
  so they never match.
- **Case-sensitive:** `memchr::memmem::Finder`, built once per query (SIMD,
  NEON on Apple Silicon). Measured over 100k stored lines (3.6–10.5 MB of
  text): 0.11–0.33 ms, about 31 GB/s. memchr is already a dependency,
  through vte.
- **Ignoring case, ASCII query:** each block's text is lower-cased (ASCII
  only) into a 64 KB scratch buffer, and the lower-cased query is found there
  with memmem. Measured: 0.19–0.71 ms for every query. Looking for the
  query's rarest byte in both cases, then checking each candidate, measured
  0.13–5.4 ms depending on the query, so it isn't used.
- **Ignoring case, a query with no cased letters** (digits, punctuation, CJK):
  plain memmem, case-sensitive.
- **Ignoring case, a query with non-ASCII cased letters:**
  - folding is Unicode **simple case folding** (CaseFolding.txt, status C and
    S), except U+212A KELVIN SIGN → `k` and U+017F LONG S → `s`. Leaving
    those two out keeps ASCII text entirely on the lower-cased byte path;
  - each query scalar matches any scalar in its **fold-equivalence set**,
    from a generated table (σ/ς/Σ, θ/ϑ/ϴ, ǆ/ǅ/Ǆ);
  - a mixed query finds its longest ASCII stretch with the lower-cased
    scratch buffer and memmem, then checks the rest scalar by scalar;
  - a query with no ASCII stretch scans scalar by scalar (estimate
    ~0.2–0.5 GB/s, off the lock);
  - not covered: full case folding (`ß` does not match `ss`), Turkish dotted
    and dotless `i`, and canonical equivalence (precomposed `é` does not
    match `e` + U+0301).
- **Cluster boundaries.** A match must start and end on a cluster boundary.
  This is checked only on lines with the `CLUSTERS` flag, so a search for `e`
  doesn't match the first half of `e` + U+0301, or one scalar of a ZWJ
  sequence.
- **Block boundaries.** A logical line can continue from one block into the
  next (`head_continues`), and from the newest stored line onto screen row
  0. Each boundary gets a small buffer: the last `q − 1` bytes before it and
  the first `q − 1` bytes after it (for the folded path, `q − 1` scalars of up
  to 4 bytes each side). That buffer is searched too, and only matches that
  cross the boundary are kept; the rest were found in the blocks themselves.
  Whole-word checks at the start of a block read the last scalar of the
  block before it. A property test puts lines across block boundaries
  (§17.3).
- **Compressed blocks (R.7)** are decompressed into a scratch buffer
  (LZ4, ~2–4 GB/s) before scanning, and not at all when the filter rules
  the block out.
- No new dependencies.

### 13.6 The per-block filter (Alex's decision: in 1.0)

- **What it is:** a 4 KB (32,768-bit) set per sealed block, with one bit per
  hashed **4-byte gram** of the block's text (ASCII lower-cased, other bytes
  as they are) and one hash function.
- **How a query uses it:** the query's own 4-byte grams are lower-cased the
  same way (for a non-ASCII query ignoring case, only its all-ASCII grams),
  and the block is scanned only if all their bits are set. It never wrongly
  skips a block that holds the text. Queries under 4 bytes skip the filter.
- **Built only in the background, never on the search path.** When a block is
  sealed, a background job (the search worker, when it is idle) builds the
  filter from the shared `BlockData` and stores it in the block's entry
  under the lock. A block without a filter is simply scanned. Building costs
  about 100 µs per block, measured: 20–50 times as long as scanning that
  block. That is why it is never built while someone is waiting. During a
  flood, blocks freed before their turn are never built at all.
- **How often it wrongly lets a block through,** measured over 100k stored
  lines of each kind of output (blocks without the word that the filter lets
  through):

  | Output | 3-byte grams, 2 KB | **4-byte grams, 4 KB (chosen)** |
  |---|---|---|
  | logs | 0.6% | 0% |
  | `ls` | 3.5% | 0.4% |
  | source code | 15% | 3.0% |
  | man pages | 31% | 8.2% |
  | git log | 22% | 7.6% |

  Common words really are in most blocks, so the filter doesn't help there,
  and doesn't need to.
- **Memory:** 4 KB per block. A block holds up to 64 KB of text, so filters
  are at most about 6% of history's text:

  | History | Text (measured range) | Blocks | Filters |
  |---|---|---|---|
  | 10k lines | 0.36–1.05 MB | 6–17 | 24–68 KB |
  | 100k lines | 3.6–10.5 MB | 55–161 | 220–644 KB |

  That is about 2.2–6.4 B per line. Filters count against the hidden 32 MB
  backstop. They are dropped under memory pressure (R.5) and rebuilt in the
  background.
- **What it buys, when a full scan already takes under a millisecond:** for a
  rare word the worker reads a few percent of history instead of all of it,
  which matters while output is flowing and other panes are drawing. R.7's
  compressed blocks are skipped without being decompressed, which would
  otherwise cost ~20–30 µs per block.

### 13.7 Speed

Measured by the second review's prototype on 100k stored lines of real
output (3.6–10.5 MB of text); the search PR adds the same cases to the
standard benchmarks. Rows marked "estimate" are not measured yet.

| Search over 100k lines | Time |
|---|---|
| case-sensitive, full scan (memmem) | **0.11–0.33 ms** (~31 GB/s) |
| ignoring case: lower-case each block into a scratch buffer, then memmem | **0.19–0.71 ms**, for every query |
| (not used) rarest byte in both cases, then check | 0.13–5.4 ms, depending on the query |
| rare word, with filters | scans only the blocks that pass: 0–8.2% of those without the word |
| building a filter (background only) | ~100 µs per block, 20–50× its scan |
| turning hits into Points | estimate: ~0.1–0.3 µs per hit, so 20k hits take 2–6 ms; hits near the view arrive first |
| non-ASCII query ignoring case, no ASCII stretch | estimate: ~10–50 ms, off the lock |
| R.7-compressed blocks | estimate: +20–30 µs per block scanned |

### 13.8 Hits: Points, columns, words, find next

- **A hit is (start Point, end Point),** both stable: a line number as
  written plus a column as written. The worker finds them like this:
  1. it binary-searches the block's checkpoints for the match's byte offset;
  2. it walks at most 15 lines to the stored line holding it;
  3. it counts the line's cells up to the match, walking its scalars with
     their widths (the block's ambiguous setting and exceptions) and its
     cluster list.

  A match that ends in a later stored line (across a soft wrap) gets its end
  Point the same way.
- **After a reflow,** history hits are drawn at the current width through
  the same Point-to-view mapping as everything else (the snapshot's
  highlight ranges, §9.2), so nothing in history has to be searched again.
  Region scrolls move or drop screen hits inside the region (§13.1).
- **Search's word rule:** letters, digits, combining marks and `_` are word
  characters; everything else is a boundary, and so is a logical line's end.
  This is not the double-click rule (§13.2), which keeps paths and URLs
  whole. Full Word doesn't find words inside CJK text written without spaces,
  because every CJK character is a letter.
- **Match options:** Contains, Starts With and Full Word, from the bar's
  menu; Ends With is accepted if it arrives through the find pasteboard.
  Starts With, Full Word and Ends With are checked on each raw match against
  the scalars on either side, so they need **overlapping** candidates: after
  a candidate fails the check, the search goes on one byte after its start,
  not after its end.
- **Find next and previous.** Hits are in Point order (blocks in order, hits
  in text order within each). "Next" is the first hit after the current one,
  or after the view's top when there is no current hit; "previous" is the
  same the other way. If the worker hasn't finished that direction yet, it
  searches that direction first, and the jump happens when the hit arrives.
  At either end it wraps around. The jump calls `scroll_to` and selects the
  hit.
- **Typing more letters** starts a new search. It may scan only the blocks
  where the old query matched (and the screen) when all of these hold:
  - the new query contains the old one;
  - both are Contains, with the same Ignore Case setting;
  - the old search finished and didn't reach the hit cap.

  The narrowing uses the old query's raw substring matches, before any word
  checks.
- **Regex is not in 1.0.** The bar's options menu has no regex mode. The
  engine can take a `regex::bytes` matcher later, with the regex's required
  literals feeding the filter.

### 13.9 The find bar (Alex's decision: our own, not NSTextFinder)

Cmd+F shows a native-style find bar of decline's own, made only of standard
AppKit controls. It isn't NSTextFinder's bar, because NSTextFinder does its
own matching over the client's text and can't take matches from our search.

- **The bar** sits at the top of the pane:
  - an `NSSearchField`;
  - previous and next buttons;
  - a count ("12 of 340", or "340+" while the search is still running);
  - Done;
  - an options menu on the field's magnifying glass: Ignore Case, and
    Contains / Starts With / Full Word.

  Whether it covers the top rows or makes the grid shorter is decided in
  E.7; covering avoids resizing the program.
- **Menus and keys.** The view implements `performTextFinderAction:` and
  `validateUserInterfaceItem:` for the standard NSTextFinderAction tags, so
  Edit > Find and its shortcuts work as in every Mac app:

  | Tag | Action | Key |
  |---|---|---|
  | 1 | show the find bar | Cmd+F |
  | 2 | next match | Cmd+G |
  | 3 | previous match | Shift+Cmd+G |
  | 7 | use the selection for find | Cmd+E |
  | 11 | hide the find bar | Escape, Done |
- **The shared find text.** The bar reads the find pasteboard
  (`NSPasteboard(name: .find)`) when it opens and when the app becomes
  active. That means the string, plus `NSPasteboardTypeTextFinderOptions`
  with `NSTextFinderCaseInsensitiveKey` and `NSTextFinderMatchingTypeKey`.
  It writes them back when the query changes or on Cmd+E, so the find text
  is shared with Safari, Xcode and the rest, as users expect.
- **What it calls:** `start_search`, `add_search_hits`, find next and
  previous over the hit list (§13.8), and `scroll_to`. Its count is the hit
  list's `total`.
- This is the one piece of decline's own UI, by Alex's decision; it is built
  from standard controls so it looks and behaves like the system's.

### 13.10 VoiceOver: UTF-16 offsets over all of history

The accessibility element (E.12) addresses text by UTF-16 offset into one
string. That string is every logical line from the first line within the
limit to the bottom of the screen, each followed by a newline, with
placeholder cells as spaces. The UTF-16 calls in §13.2 are for VoiceOver
only.

- **Counts.** Each block's entry keeps `utf16_len` (4 bytes), and its
  checkpoints hold the UTF-16 offset of every 16th line. The terminal keeps
  the sums of `utf16_len` over blocks, rebuilt in O(blocks) when a block is
  sealed, freed or popped from.
- **Point to offset:** the blocks' sum, plus the checkpoint, plus at most 15
  lines, plus the line's own scalars up to the column. **Offset to Point:**
  the same in reverse, with binary searches. Each is O(log blocks), plus a
  line or two.
- **The full text** goes to VoiceOver as an `NSString` subclass backed by the
  history blocks: `length` is the sum of the counts plus the screen's part,
  and `characterAtIndex:` and `getCharacters:range:` decode from the blocks
  (UTF-8 to UTF-16 on demand, keeping the last block decoded). Nothing is
  copied up front. The E.12 spike measures how often VoiceOver asks, and for
  how much.
- **The screen's part** is built on demand, under the lock.
- **Offsets move.** They count from the first line within the limit, so they
  shift when a block is freed, and the screen's part changes with output.
  The app posts accessibility's value-changed notification at most once per
  frame.
- **VoiceOver's lines** are rows at the current width. A line number maps to
  a UTF-16 range through the blocks' row counts (§12.3) and a short walk
  inside one block. After a width change, those counts are worked out by the
  search worker on the shared `BlockData`, and stored in the entries under
  the lock (could), so VoiceOver never makes the main thread walk all of
  history.

---

## 14. Parser support (vendored vte)

**Landed (PR 3, `0bb234d`).** The vendored copy already had `print_ascii`
and the UTF-8 split fix. Before PR 3, its state machine:
- sent SOS, PM and APC (`ESC X`, `ESC ^`, `ESC _`) to one state that
  ignored them;
- passed DCS through `hook`/`put`/`unhook`, one byte at a time;
- with the `std` feature, buffered OSC strings in an unbounded `Vec`, and
  split them on `;` into at most 16 parts, dropping the separators from its
  buffer.

PR 3 made these changes, as designed, with tests in
`vendor/vte/src/string_tests.rs` and a list in its README.
`osc_dispatch` itself is unchanged: at most 16 parts, and the rest of the
string is dropped. The term-core side (what to do with each string) comes with
the PRs that use it.

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

    /// An OSC string has passed 4 KB; `prefix` is what has arrived, separators
    /// included. What to do with the rest.
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

- **APC:** it gets a state of its own; SOS and PM stay ignored. In it, bytes
  are handed over in slices found with `memchr3` for ESC, CAN and SUB, so a
  multi-megabyte kitty payload costs a few calls, not one per byte.
- **DCS:** `put_slice` is called from the passthrough state in the same way.
  term-core decides what the string is on `hook`:
  - final `q` with no intermediates is Sixel, streamed into the decoder;
  - `+q` is XTGETTCAP and `$q` is DECRQSS (F.5), buffered up to 4 KB.
- **OSC separators:** vte keeps the `;` separators in its raw buffer, and
  the parameters become slices around them. `osc_large` prefixes are then
  exact. term-core still rebuilds multi-part arguments by joining parts with
  `;`: `params[2..]` for an OSC 8 URI, and `params[1..]` for OSC 1337
  arguments. When vte reports exactly 16 parts, the string may have been
  truncated, so the link or image is ignored.
- **OSC size policy** (term-core's answers to `osc_large`):

  | OSC | Answer |
  |---|---|
  | `1337;File=` | `Stream` into the iTerm2 image reader, capped by the image budget (32 MB) |
  | `1337;FilePart=` chunks | `Buffer(1 MB)` each |
  | `52;` (clipboard) | `Buffer(4 MB)` |
  | anything else | `Buffer(64 KB)` |
- **kitty graphics in term-core:**
  - control keys up to the `;`, at most 4 KB;
  - the payload is base64-decoded as it streams into the image being
    received;
  - chunked transmissions (`m=1`) build up to the image budget;
  - pixel caps: width and height ≤ 10,000, and width × height × 4 ≤ the
    budget;
  - file and temp-file transmission limits as in B.3;
  - decoding as in §8.4, with replies sent when it finishes.
- **Sixel caps:** the same pixel caps, repeat counts clamped to the image
  width, and at most 1024 palette registers.
- **Fuzz targets** for each (B.3, B.5).

A later, optional change: hand non-ASCII stretches over whole as
`print_str(&str)` (vte already has the `&str`), split at C1 controls, so
term-core can segment and write CJK text a row segment at a time. Unicode
input parses at 131 MB/s against 423 for ASCII. It isn't needed for
correctness.

---

## 15. Hot paths: cost per operation

| Operation | Today | This model |
|---|---|---|
| ASCII run of n bytes in one row segment | n × 12-byte stores, OR over the cells | n × 8-byte stores (`template \| byte`), OR over n `u64`s, one `touch_row`, grapheme state set once |
| non-ASCII scalar | `unicode-width` lookup, store | one table lookup (width, break class and the ambiguous flag; the ambiguous-width setting adds one bit test), O(1) break check, store; a join appends to the cluster, amortised O(1). ASCII pays nothing for either |
| SGR | edit the pen | edit the pen's `Style`, clear its cached template |
| first print after SGR | — | intern: ~5 ns hit, ~15–20 ns miss |
| erase n cells | fill 12-byte cells | fill the 8-byte template; free cluster slots in the range only if the row has clusters |
| full-screen scroll | ring rotation, encode one row, memset | same; encoding adds one 64-entry cache lookup per run and a UTF-16 count |
| region scroll, IL/DL | rotate rows in the region | same, plus positions inside the region |
| ICH/DCH | rotate within the row | same; free slots of cells pushed off |
| sweep | — | O(cells), at most once per cells/2 new styles |
| push to history | O(cells) | O(cells) + O(runs); with the find bar open, the new text is searched once per frame (§13.4) |
| resize, new width | O(screen), truncating | O(screen + pulled lines); history untouched |
| snapshot | (frame built under the lock, O(cells)) | O(changed rows × cols) memcpy + new styles |
| start a search | — | O(screen) + O(blocks) refcount bumps under the lock; history off the lock (0.11–0.71 ms for a full scan of 100k lines) |
| merge a batch of hits | — | O(batch) under the lock; freeing a block drops its hits in O(1) |
| build a block's filter | — | ~100 µs, in the background only |

---

## 16. Memory budget

As §2 says, none of this moves the 35% footprint threshold by itself; the
tables show that the model stays small and that 100k lines of history stay
in single-digit megabytes.

### 16.1 Per structure

| Structure | 80×24 | 300×100 | Notes |
|---|---|---|---|
| cells, per screen | **15.0 KB** (today 22.5) | **234 KB** (today 352) | the alternate screen only while in use (§19) |
| row headers (48 B) | 1.1 KB | 4.7 KB | |
| style table, plain / coloured | < 1 KB / ≤ 1 KB | < 1 KB / ≤ 1 KB | |
| style table, TUI with a colour scheme | ≤ 18 KB | ≤ 18 KB | |
| style table, worst (every cell different) | 104 KB | 1.6 MB | transient; renumbering gives it back |
| cluster stores, plain / coloured / CJK | 0 | 0 | CJK characters are single scalars |
| cluster stores, emoji-heavy (8 or 30 clusters per row) | ~6 KB | ~70 KB | ~96 B per row with clusters + ~20 B per cluster |
| cluster stores, worst (8 KB cap per row) | ~200 KB | ~1 MB | combining-mark floods only |
| link table | ~100 B per link | | bounded by the styles |
| placement table | ~100 B per direct placement | | pixels count against the 32 MB image budget |
| snapshot buffers (renderer, reused) | 15 KB | 234 KB | plus the renderer's style mirrors, one per screen (each the size of its table) |
| renderer row cache (R.2) | ≤ 46 KB | ≤ 720 KB | same as today's frame vectors |
| shaping cache (ligature fonts only) | ≤ 23 KB | ≤ 360 KB | 12 B per glyph |
| shaped-cluster cache (per process) | ≤ ~400 KB | | 4096 entries |
| search hit list | 16 B per hit | | 20k hits ≈ 320 KB, only while the find bar is open; capped at 100k hits (1.6 MB) |
| search filters | 4 KB per sealed block | | built in the background; see §16.2 |
| font registry | ~100 B per face + Core Text's own | | tens of faces |
| Unicode table | 60–80 KB read-only, shared | | paged in on use |

### 16.2 History

Sealed bytes per line from §11.4, plus per block about 200 bytes of header
and dictionary and 0.25 B per line of checkpoints, plus one open block
(≤ 64 KB of each part) per terminal. Search filters add 2.2–6.4 B per line
once they are built (4 KB per block, §13.6).

| Content | B/line | 10k lines | 100k lines | Today, 10k (measured) |
|---|---|---|---|---|
| plain | 43 | 0.43 MB | 4.3 MB | 0.52 MB |
| coloured (`ls`) | 59 | 0.59 MB | 5.9 MB | 0.93 MB |
| compiler errors | 63 | 0.63 MB | 6.3 MB | 0.81 MB |
| full-width background (delta, status lines) | 13 | 0.13 MB | 1.3 MB | 1.83 MB |
| CJK | 66 | 0.66 MB | 6.6 MB | 0.90 MB |
| emoji-heavy | 79 | 0.79 MB | 7.9 MB | – |
| gradient, 160 cells | ~1,600 | 16 MB | stopped at the 32 MB backstop | ~11 MB |
| search filters, once built | 2.2–6.4 | 24–68 KB | 220–644 KB | – |

### 16.3 Whole terminal state, worked

| Case | Screen + tables | History | Snapshot + render caches | Total |
|---|---|---|---|---|
| 80×24, plain, 10k lines | 17 KB | 0.43 MB + ≤ 64 KB (+ ~28 KB filters) | ~60 KB | **~0.6 MB** |
| 80×24, CJK, 10k lines | 17 KB | 0.66 MB | ~60 KB | ~0.8 MB |
| 300×100, coloured, 10k lines | 240 KB | 0.59 MB | ~1 MB | ~1.9 MB |
| 300×100, emoji-heavy, 100k lines | 310 KB | 7.9 MB (+ ~0.4 MB filters) | ~1.1 MB | ~9.7 MB |
| 300×100, gradient TUI, 10k lines | 1.9 MB (peak) | 16 MB | ~3.5 MB (incl. style mirror) | ~21 MB |

For comparison, Ghostty keeps full cells for history: 10k lines at 160
columns are about 13 MB of cells.

---

## 17. Migration plan

### 17.1 Order of PRs

Agents work in worktrees, one crate each at a time (the roadmap's rule).
PRs 1 and 2 touch term-core, term-render and term-app together, so the main
session does them before the agents start. After that the core agent's PRs
go in order, and the font and render agents run alongside.

| # | Crate | PR | Size | After |
|---|---|---|---|---|
| 1 | core + render + app | `refactor(core)!: read cells through RowRef/CellRef and a Style value`: content (empty, char, cluster text, placeholder), width, `&Style`; `Style`, `Underline`, `Attrs` defined; implemented on today's 12-byte cells. **`Terminal`'s public fields (`cursor` and its pen, `selection`, `modes`, …) become private behind accessors.** Every caller moves over (frame.rs, view.rs, the conformance/term/props tests, the pipeline test). No behaviour change | **M** | — |
| 2 | core + app + render | **view API and selection gestures at a fixed width** (§13.2): `point_at` (with its edge rules), `view_position`, `scroll_to`, `select_word`/`select_line`/`select_all`/`extend_selection` on logical lines, `cursor_in_view`, `link_at` (None until PR 9), `ViewTop`; view.rs and frame.rs stop doing line arithmetic; the double-click word rule moves into term-core | M | 1 |
| 3 | vte | APC state, `put_slice`, OSC separators kept and size policy (§14), no-op defaults. **Landed, `0bb234d`** | S | — |
| 4 | core | Unicode table (with the ambiguous flag) + incremental segmentation; width lookups move to the table (a test checks it equals `unicode-width` per scalar); clusters not stored yet | M | 2 |
| 5 | core | **8-byte cells and the per-screen style table**: SGR 4:x, 21, 53/55, 58/59 stored; erase style; sweep with every holder (§5.2); renumbering; `set_sweep_at`; `check_invariants` extended. History is still v1 | M | 4 |
| 6 | core | **clusters per row and mode 2027** (the width rule of §6.3, legacy mode, REP by value, the extended `repair_wide`); `Terminal::set_ambiguous_width`; selection text and `dump()` show clusters | M | 5 |
| 7 | core | **scrollback v2** (§11) at the written width only: `BlockData` (shared) and `BlockEntry`, meta and contiguous text, checkpoints, per-block ambiguous setting, `FILL`, `pop()` with the lowest-popped mark, the 32 MB backstop, the 64-entry slot cache; `scrollback_mem` gains emoji, placeholder, gradient and sealed figures; bench limits lowered | L | 6 |
| 8 | core | row change counters, `terminal_id`, `history_epoch`, `popped_from`, the snapshot with highlight ranges, the screen it shows and style updates (§9) | S | 5 |
| 9 | core | OSC 8 link table and OSC 133 prompt marks, on the screen and in v2 | S | 7 |
| 10 | core | kitty placeholders (cell kind, diacritic table, step-by-step inference, v2 list) | S | 7 |
| 11 | core | **screen reflow, pull-back and tracked positions** (F.4, §12.2): the two ignored resize tests settled (§19) | L | 7, 8 |
| 12 | core | **history at another width** (§12.3): rows-at-width counts, `ViewTop::At { point, offset }` across width changes, `FILL` at another width | M | 11 |
| 13a | core | **UTF-16 counts and mapping** (§13.10): `utf16_len` per entry, UTF-16 checkpoints, `utf16_offset` / `point_at_utf16` / `utf16_piece` | S | 7 |
| 13b | core | **search engine and hits** (§13.3–§13.8): `start_search`, `SearchJob` over `(Arc<BlockData>, len)`, block boundaries, the live-output rule, hits per block, the case-folding table, and a search benchmark with the prototype's cases | M | 11 |
| 13c | core | **search filters** (§13.6): 4-byte grams in 4 KB, built in the background only; still in 1.0 | S | 13b |
| 14 | core (+ app for the decoder) | placement table and decline's own images (§8) for B.3/B.5, with the `ImageDecoder` trait and the 32 MB budget; the ImageIO decoder is a term-app part | M | 12 |
| 15 | app | settings: `ambiguous_width` (and later `image_memory` with B.3), read by config and passed to term-core; main session | S | 7 |
| F2.1 | font | font registry, `GlyphKey`, rasterise by (font, glyph); `rasterize(char, style)` kept as a wrapper | M | 1 |
| F2.2 | font | CTLine shaping of runs (`shape()`) and the shaped-cluster cache; clusters now, ligatures in B.1 | M | F2.1 |
| F2.3 | font | underline pattern sprites (curly, dotted, dashed) from id 0x1000; box/block/Powerline/braille shapes are done (F.6, `c4033dd`) | S | F2.1 |
| F3.1 | render | resolve through `Style`: underline colour, double underline, overline, strike | S | 1 |
| F3.2 | render | glyph cache keyed by `GlyphKey` (sprites included); runs (§10.1); cluster cells via F2.2; curly/dotted/dashed via F2.3; ambiguous characters at two cells | M | 6, F2.2, F2.3 |
| F3.3 | render | build frames from the snapshot with overlays (§9.3), still under the lock (R.1 moves it out) | M | 8 |

PR 13 as a whole is L, so it is split as above. E.7 (our find bar, §13.9, in
term-app) builds on 13b; E.12 (VoiceOver, term-app) builds on 13a, and its
spike measures how often VoiceOver asks for text (§13.10).

### 17.2 Existing tests that carry over

| Tests | What happens |
|---|---|
| `term-core/tests/conformance.rs` (xterm behaviour per sequence) | all carry over; the 62 places that read `flags::WIDE`, `.fg` and `.bg` move to `CellRef` in PR 1; the two `#[ignore]` resize tests are settled in PR 11: grow-pulls-back passes, and shrink-below-cursor changes to expect no loss |
| `term-core/tests/props.rs` (invariants under random input and resizes; scrollback round trips; bulk ASCII = one char at a time) | carry over; the round trip becomes exact in PR 7 (styles, clusters, marks, placeholders); new generators below |
| `term-core/tests/selection.rs`, `term.rs` | carry over; selection gestures (PR 2) and selection across reflow (PR 11) added |
| `term-core/tests/fuzz.rs`, `fuzz_corpus.rs`, `fuzz/fuzz_targets/parse.rs` | carry over; `read_back` reads every row at the current width (today it can't, because history isn't re-wrapped); the fuzz target sets `sweep_at` to 8 and runs a search now and then |
| `cell.rs` unit tests | rewritten for the new layout (bit positions, blank = 0, SE) |
| `scrollback.rs` unit tests | varint tests carry over; the "fast encoder = reference encoder" test is kept until PR 7 replaces the format, then becomes "v2 decode of v2 encode = the row" |
| `term-render/tests/frame.rs` | carry over (they build frames from `Term::advance`); the fake glyph source takes a `GlyphKey`; from F3.3 each frame is also built from a snapshot and compared |
| `term-app/src/mouse.rs` word tests | move into term-core with the word rule (PR 2) |
| `term-font/tests/font.rs`, the sprite tests from F.6 | carry over; registry and shaping tests added |
| `term-app/tests/pipeline.rs`, `smoke.rs` | carry over (text dumps); the caught-panic test checks that the renderer drops its caches (a new `terminal_id`) |
| benches (`parse`, `parse_1k`, `frame`) | carry over; numbers recorded before and after PRs 5, 6, 7, 8, 11, 12 and F3.2; a `search` bench with the review prototype's cases joins in PR 13b |

### 17.3 New invariants

They live in `check_invariants`, run after every `advance` in debug builds,
in the property tests and in fuzzing, with `set_sweep_at(8)` so sweeps and
renumbering happen constantly:

1. **Styles:**
   - every cell's style id is live in its screen's table, and so are the
     pen's and erase template's cached ids;
   - `recent` names only live ids;
   - no two live entries are equal (interning gives one id per style);
   - the index finds every live entry and nothing else, and `free` holds
     exactly the dead ids;
   - after a forced sweep, live ids = ids used; after a renumbering, ids are
     dense and every row's `seq` changed;
   - a cached template never belongs to the other screen's table: switch
     screens at random in the property test;
   - no table epoch repeats, even when the alternate table is freed and
     made again;
   - `new_ids` never holds more than half the table.

   (Ghostty's equivalent checks reference counts; ours checks the sweep's
   result and every holder.)
2. **SE bit:** `cell.SE == styles[cell.style_id].shows_empty()`.
3. **Clusters:**
   - every kind-1 cell's slot is live in its row's store, and each live slot
     is held by exactly one cell;
   - cluster text is valid UTF-8, 2–32 scalars, ≤ 128 bytes;
   - `garbage` is right;
   - rows without a store have no kind-1 cells;
   - `last_grapheme` is ≤ 128 bytes.
4. **Widths:**
   - a wide cell is followed by a spacer, and a spacer follows a wide cell
     (today's rule);
   - a spacer has its wide cell's style;
   - an edge spacer appears only in the last column of a wrapped row.
5. **Placeholders:** narrow; row < 297; column < 512.
6. **Change counters:**
   - every row's `seq ≤ counter`;
   - property: if a row's cells differ between two snapshots, its `seq`
     differs;
   - after RIS, `counter` hasn't gone back and `history_epoch` has changed;
   - after a session's terminal is replaced, `terminal_id` has changed.
7. **Snapshot:** a frame built from a snapshot equals a frame built from the
   live terminal; applying each snapshot's `style_updates` to the mirror of
   its `screen` gives that screen's live table, across random screen
   switches.
8. **History:**
   - any row pushed and read back is the same row (cells, resolved styles,
     clusters, marks, placeholders, `FILL`), up to trailing blanks and the
     space/empty equivalence;
   - `pop()` after `push()` gives the row back, leaves the shared
     `BlockData` unchanged, and sets the lowest-popped mark;
   - no part of the open block holds more than 64 KB;
   - the backstop counts every part;
   - checkpoints and `utf16_len` agree with the text.
9. **Ambiguous width:** after a random sequence of setting changes, every
   stored line decodes to exactly the cells it was written with, and screen
   cells are unchanged by a change.
10. **Reflow:**
    - the text of every logical line (joined, trailing blanks trimmed) is the
      same before and after any resize sequence;
    - resizing to W2 and back to W1 with no output in between gives the same
      screen when nothing was dropped at the history limit;
    - the cursor stays on the same character;
    - selection text is unchanged, including selections that start inside
      lines that get pulled back;
    - the view shows the same first cell.
11. **Lines at the current width:** for any width, block row counts agree
    with walking every logical line; view row → Point → view row is a round
    trip.
12. **Region scrolls:** after random DECSTBM, IL/DL and scrolling, selection
    ends and search hits still point at the same characters, or are gone.
13. **Search:**
    - for random content and queries (both cases, every word option), the
      engine's hits equal a plain scan of the decoded text, whichever order
      blocks are visited in, with no hit twice;
    - **lines that cross a block boundary**, and a line continuing from
      history onto screen row 0, are found like any other;
    - matches never start or end inside a cluster;
    - with output arriving and pull-backs during the search, the final hit
      list equals a fresh search;
    - a block's filter never rejects a query the block contains;
    - hits keep pointing at the same text through output, region scrolls
      and reflow;
    - Point → UTF-16 offset → Point is a round trip, and the document's
      UTF-16 length is the sum of the blocks' plus the screen's.
14. **Fuzzing:** the parse target's dictionary gains:
    - ZWJ, VS15/VS16, regional indicators, keycaps and combining marks after
      ASCII;
    - U+10EEEE with diacritics;
    - SGR 4:3 and 58;
    - OSC 8 with `;` in URIs, and OSC 133;
    - `ESC _ G` and `ESC P q`.

    Resize steps now reflow. New targets come with B.3/B.5: kitty commands,
    the Sixel decoder and OSC 1337.

### 17.4 Keeping main green

- **Before merging,** every PR passes `cargo fmt --all`,
  `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` (AGENTS.md).
- **PRs 1 and 2 put every reader behind term-core's API,** so the core PRs
  change term-core's insides without touching term-render. Some later work
  does touch term-app, as its own app PRs: the ImageIO decoder (PR 14), the
  find bar and search worker (E.7, on 13b), VoiceOver (E.12, on 13a) and
  the settings (PR 15).
- **Each switch over keeps the old path as a test-only reference for one
  PR,** as the parser work did (`set_bulk_ascii`, the reference encoder):
  - the cluster writer is checked against one codepoint at a time;
  - v2 history is checked against v1's text;
  - reflow is checked against today's resize where they should agree. That
    holds **only when shrinking**: when growing, today adds blank rows and
    the new code pulls lines back, so for growth the reference is the
    conformance test, not the old code;
  - the search engine is checked against a plain scan of decoded text.
- **Gates:**
  - the deterministic bytes-per-line gate (`scripts/bench-gate.py`) runs in
    CI. Its limits are lowered in PR 7 and never raised; the gradient limit
    is added then.
  - parse speed is gated on PRs 5, 6, 7, 8, 11, 12 and 13b. A 3% threshold is
    below CI noise, so this gate is a script (`scripts/parse-gate.py`, new
    in PR 5). It builds the base and the PR, runs `parse_1k` interleaved
    (base, PR, base, PR… at least 5 pairs) on the fixed Mac from N.5,
    compares medians, and fails on a drop of more than 3% for plain,
    coloured, TUI or `seq`. Its output goes in the PR description. CI runs
    only the bytes gate.
  - the frame-build bench for F3.x, and the search bench for PRs 13b and
    13c;
  - footprint on the standard set for PRs 5, 7 and 11.
- **Fuzzing:** the parse target is fuzzed on each merge (the nightly job),
  with 14 clean nights before 1.0, as planned.
- **No data migration:** the v1 → v2 history change needs none, because
  history lives only in memory.

### 17.5 What dogfooding loses in between

Alex runs builds of `main` daily (`scripts/dogfood.sh`). Three gaps show up
in that app:

- **Between PRs 5 and 7,** history still uses v1. Lines that scroll into
  history lose underline colour and underline style (curly, dotted and
  dashed become a single underline), and also overline and links. On the
  screen they are correct.
- **Between PRs 6 and 7,** clusters work on the screen, but v1 history keeps
  only each cluster's first scalar: a family emoji scrolled into history
  comes back as 👨, and `e` + U+0301 as `e`.
- **Between PR 6 and F3.2,** cluster cells draw at least their first
  character (the renderer draws the base scalar). Widths and cursor
  positions are already right, so text lines up; only the joined glyph is
  missing.

---

## 18. Compared with Ghostty and WezTerm

| | Ghostty | WezTerm | decline (this design) |
|---|---|---|---|
| Cell | 8 B packed: content tag, 21–24-bit content (or a bg colour), 16-bit style id, width, protected, hyperlink bit, semantic content | `Cell { text: TeenyString, attrs: CellAttributes }`, colours inline, rare attributes (link, underline colour, image) behind a `Box` | 8 B: content, kind, width, 32-bit style id |
| Styles | per page, reference counted, page grows when full | inline in each cell | per screen, swept |
| Graphemes | base codepoint in the cell, the rest in a per-page map keyed by cell offset (≤ 64 codepoints) | in the cell's string | per-row store, slot in the cell |
| History | the same pages of live cells (limit in bytes) | `Line`s; `compress_for_scrollback` packs them into a text + attribute-run form | encoded stored lines in 64 KB blocks, text contiguous, styles by value |
| Change tracking | dirty bit per row | `Line::seqno` | `seq` per row, unique per parse call, plus terminal id and history epoch |
| Images | placement storage per screen (kitty) | an image fragment attached to every covered cell | kitty direct placements in a table keyed by stable line; placeholders, Sixel and iTerm2 as cells |
| Reflow | all pages rewritten on resize | all lines re-wrapped on resize | screen only; history worked out on demand |
| Tracked positions | pins (page, row, column) updated by reflow | stable row index | stable line numbers for stored lines, remapped on-screen positions |

**Where we differ and why.**

- *Sweep instead of reference counts:* this is only possible because history
  doesn't hold style ids. In exchange, reading history needs a decode (R.3
  caches it), and history styles are stored once per block rather than once
  per page.
- *History is not cells:* Ghostty's 8 B per cell keeps history and the
  screen one structure (simple, no decode), at about 13 MB per 10k
  160-column lines. Our history is encoded instead (~0.4–0.8 MB), and its
  text is contiguous, which is what makes SIMD search over all of it cheap.
- *No history rewrite on resize:* both Ghostty and WezTerm reflow everything
  they hold. With 100k lines that is the cost F.4 rules out. Our stored
  lines never change, which also makes history Points, and so search hits,
  permanent.
- *kitty direct placements as a table, Sixel/iTerm2 as cells:* WezTerm
  attaches an image fragment to every covered cell for all image kinds. That
  follows text everywhere, but costs a large attribute per covered cell. We
  use cells (8 B each) only where the protocols expect text-like behaviour
  (placeholders, Sixel, iTerm2), and one table entry per kitty direct
  placement, which the protocol keeps independent of text.
- *Cluster store per row:* Ghostty's per-page map needs entries moved when
  cells move, and WezTerm's per-cell string makes every cell bigger. Ours
  costs one indirection, for cluster cells only.

---

## 19. Risks and decisions

### Risks

1. **Sweep pauses on very large screens.** 160k cells means a sweep of about
   150 µs under the lock, amortised over ≥ 80k new styles. Measure it in
   PR 5 with a full-screen truecolour animation at 120 Hz.
2. **The renderer's style mirrors** (one per screen) double the style
   tables' memory in the gradient case (a second 0.9 MB at 300×100).
   Accepted; it goes when the table is renumbered.
3. **Cluster store garbage** under spinners that rewrite emoji in place. A
   property test rewrites one cell a million times and checks the store
   stays bounded.
4. **History at a new width with a lot of CJK** needs scalar walks. Only the
   `display_offset()` count and VoiceOver's line numbers walk all of history
   (about 10 ms per 100k lines, once per width); drawing never does.
5. **A number handed to new text after a reflow or `pop()`.** This is safe
   only while every holder of a Point lives in term-core and is remapped,
   and the renderer and search watch `history_epoch` and the lowest-popped
   mark. New features that keep
   Points (E.5 prompt jumping, E.7 search, B.3 placements, E.12
   accessibility) must register as tracked positions; the reflow property
   test (§17.3, 10) covers each one as it lands.
6. **Width rules vs. what Core Text draws.** A ZWJ sequence that Apple's
   emoji font doesn't know renders as several glyphs; we fit them into the
   cluster's two cells. The same goes for flags of unknown regions.
7. **Shell prompts after reflow.** zsh redraws its prompt on SIGWINCH, and a
   reflowed old prompt plus the redraw can leave a duplicate line; every
   reflowing terminal has this problem. With OSC 133 marks we can clear the
   prompt's rows, from mark A to the cursor, before sending SIGWINCH, as
   kitty does with its shell integration. Decide in E.5.
8. **Legacy programs that assume `wcwidth`** see different widths for ZWJ
   sequences with 2027 on; they can turn it off. macOS's own `wcwidth` is
   old, so this will show in some tools.
9. **Image rows on reflow** wrap in pieces (§8.3); if that looks bad, clip
   rows that are all placeholders.
10. **How often VoiceOver asks for text** (§13.10). The NSString backed by
    history decodes on demand; if VoiceOver reads the whole string often, each
    read decodes megabytes. The E.12 spike measures it; a cache of decoded
    UTF-16 for the blocks near the view is the fallback.
11. **Search memory with a very common query** (hundreds of thousands of
    hits). The hit list is capped at 100k hits; past that, the find bar shows
    "100,000+" and the next/previous buttons search on demand from the
    current hit.

### Answered by the review (engineering decisions)

1. **Shrinking the height with the cursor mid-screen:** keep losing nothing.
   Blank rows below the cursor are dropped first, then top rows go into
   history, and `resize_shrink_height_below_cursor_is_dropped` changes to
   expect this.
2. **The alternate screen** is freed on leaving 1049 or 1047 (both clear it
   anyway), along with its style and link tables and its images. It is kept
   after leaving 47, and freed once it has been hidden for 30 s.
3. **Text over Sixel/iTerm2 images** removes those pieces, because the
   images are placeholder cells (§8.3).
4. **Copying kitty placeholders** gives spaces, trimmed at the line end.
5. **Mode 2027** is on by default, with the width rule of §6.3.
6. **Images** are decoded through a trait in term-core, with an ImageIO
   implementation in term-app (§8.4).
7. **Prompt marks** are per row, with columns (up to four per row), not per
   cell. Five spare cell bits remain if that ever has to change.
8. **Searching history** happens on immutable, shared `BlockData`s off the
   lock, each captured with its line count (§13.4). This replaces the
   earlier "slices under the lock" answer, now that search covers all of
   history.

### Decided by Alex (29 Oct)

1. **Scrollback limit:** lines (the existing `scrollback = 10000` setting),
   plus a hidden 32 MB byte backstop (§11.2). The Ghostty comparison is made
   at the same number of lines kept.
2. **Image budget:** 32 MB per process by default, a setting, counting CPU
   and GPU copies once (§8.4).
3. **East Asian Ambiguous width:** a setting in 1.0, `ambiguous_width = 1 | 2`,
   default 1. Stored lines show as written after it changes, with no reflow
   and no reporting mode (§6.3).
4. **Search and VoiceOver cover all of history,** with per-block UTF-16
   counts: in-view hits first, then history outwards, with fast block search
   (§13.3–§13.8).

### Decided by Alex (second round)

5. **The per-block filter stays in 1.0,** built in the background only and
   never while someone waits; a block without one is simply scanned. It uses
   4-byte grams in 4 KB: 220–644 KB at 100k lines (§13.6).
6. **Cmd+F uses our own native-style find bar, not NSTextFinder:** a search
   field, previous/next, a count, Done and an options menu; the view handles
   the standard find actions (tags 1, 2, 3, 7 and 11), and the bar shares the
   find pasteboard (§13.9).
7. **The UTF-16 piece API is for VoiceOver only;** its full text is an
   NSString subclass backed by history blocks, and the E.12 spike measures
   how often VoiceOver asks (§13.10).
