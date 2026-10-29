# Ligatures (B.1)

Status: **reviewed** by Opus (xhigh), findings folded in (§9); open
questions for Alex in §8. Builds on the accepted cell design
(`docs/design/cells.md` §9 change counters, §10 runs, shaping, glyph keys)
and what has landed of it: the font registry and `GlyphKey` (F2.1),
`Fonts::shape` with both shaping paths (F2.2), font config (E.4).

The term-font part needs nothing else and can start now. The term-render
part starts after cell-design **PR 8** (row change counters, `RowKey`),
**F3.2** (glyph cache by `GlyphKey`, runs, frames built through
`Fonts::shape`) and **F3.3** (frames from the snapshot), and uses B.2's
per-pane render state (splits.md §3.2). Nothing changes in term-core.

## Goal and done-when

- JetBrains Mono, Fira Code, Cascadia Code, Iosevka and Monaspace draw
  their ligatures and contextual alternates; Menlo, Monaco and SF Mono
  (checked: no default substitutions across characters) cost nothing.
- `font_features = ["-calt", "+ss01", "zero", "cv05=2"]` turns OpenType
  features on and off.
- Measured Menlo against JetBrains Mono on the standard set, within the
  limits of §5.

## 1. Which faces shape whole runs

Decided once per face when `Fonts` is built (each of the four faces: a
family's bold can lack what its regular has):

1. **OpenType faces.** From `CTFontCopyTable(font, 'GSUB')`, read the
   features of the `latn` LangSys, else `DFLT` (ScriptList → LangSys →
   feature indices, plus the required feature; offsets of 0 mean absent;
   all reads bounds-checked; ~70 lines). The whole FeatureList is not
   enough: Courier New lists `liga` and `rlig` for Arabic only, and would
   shape every row for nothing.
2. The face **shapes runs** when, after `font_features`, a feature that
   substitutes across characters is on: `liga`, `clig`, `calt`, `rlig`
   unless turned off; `dlig` and any other tag only when turned on.
   `ccmp`, `locl`, `mark`, `mkmk`, `kern` don't count: they act inside a
   cluster, which the cluster path already shapes.
3. **Single-glyph features alone** (`zero`, `ss01`–`ss20`, `cv01`–`cv99`,
   `case`, with `liga`/`calt`/`clig`/`rlig`/`dlig` off or absent) don't
   need runs: each character is resolved once through a one-character
   CTLine into the per-(character, style) table the fast path already
   uses. So SF Mono with `+ss03`, or JetBrains Mono with `-calt -liga
   +zero`, stays on the fast path.
4. **Faces with a `morx` table** (Apple's own format; Menlo, Monaco and
   Courier have `morx`, no GSUB), and faces with both tables (Core Text
   prefers `morx`), are probed: one CTLine of ~40 programming sequences
   (`-> => != == === <= >= <=> :: ... /* */ // && || |> <| ++ -- ** ##
   fi fl ffi`) is compared with the per-character keys. Checked on macOS
   27: Menlo, Monaco and Courier give identical glyphs (their only
   default `morx` substitution is Unicode composition), so they stay on
   the fast path. A user `+tag` counts for `morx` faces too: Core Text
   maps OpenType tags onto Apple's features, so `+liga` on Menlo turns on
   its `fi` ligature (one glyph over two cells) and the face then shapes
   runs.

Exposed as `Fonts::features(face) -> Features` (`Features` stays `Copy`).
Cost to report in B1.1: table reads and the probe, per face, at
construction; and the first CTLine of a GSUB font, which can take
milliseconds while Core Text loads its tables (measured on a new window
and on Cmd+=).

## 2. `font_features`

- Config (term-app): `font_features = ["-calt", "+ss01", "zero",
  "cv05=2"]`, Ghostty's `font-feature` / kitty's syntax: `tag` or `+tag`
  = 1, `-tag` = 0, `tag=n`; four ASCII characters. A bad entry is a config
  notice naming it and is ignored.
- term-font: `FontConfig::features: Vec<(Tag, u32)>`. Each of the four
  faces gets its own copy step (`CTFontCreateCopyWithAttributes` with
  `kCTFontFeatureSettingsAttribute` = an array of
  `{kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue}`), separate
  from the cascade-list copy, which only runs when `font_fallback` is set.
  Checked: features survive the cascade copy in either order;
  `CTFontCreateForString` returns the featured face and CTRun's font is
  `CFEqual` to it.
- Turning ligatures off is `font_features = ["-calt", "-liga"]`.
- Live reload: a font setting like the others. `decline import ghostty`
  maps `font-feature` lines.

## 3. Shaping a run (term-font)

cells.md §10.1 keeps runs breaking only on face, code-drawn glyphs and
placeholders, so colours, the cursor and selection never break a
ligature. Inside `shape_line`, B.1 makes **one CTLine per run** whose text
is the run's text with **every cell the face won't draw replaced by a
space**:

- cells whose character resolves to another font (fallback CJK, emoji,
  Nerd Font icons from `font_fallback`) or is a cluster;
- strong right-to-left characters (bidi class R and AL) and the bidi
  controls U+202A–202E, U+2066–2069: CTLine would otherwise reorder the
  run and mirror its neutrals (checked: `(`, `)` and `<` next to Hebrew
  come back as `)`, `(` and `>` in Courier New and Menlo), and the
  terminal draws cells in logical order. A writing-direction override
  stops that but adds 2–4.5 µs per line and breaks Arabic joining;
- default-ignorable and format characters (U+00AD, U+200B–200F, …),
  which leave no glyph and would make the glyph before them cover their
  cell.

Those cells are drawn by today's per-character and cluster paths, which
are cached. Ligature context is unchanged, since Core Text never shapes
across a font change. One CTLine per run rather than one per stretch,
because a CTLine's fixed cost dominates short ones (measured: 2.3 µs for
one character, 4.5 µs for 160 in Menlo): a row of `eza --icons` output
with an icon every 12 cells would otherwise be ~17 CTLines.

Also: trailing blank cells are left out of the CTLine text (a 300-column
row often holds 20 characters); a run with fewer than two non-blank face
cells skips CTLine; `shape_line`'s three `Vec`s and its sort move to
scratch buffers kept in `Fonts`.

"The face draws it" means `glyph_key(ch, style).font` equals **the id the
registry gives that face's font**, worked out once per face. Not
`face_id(style)`: when a family has no bold, the bold face is the regular
font, and the registry's lookup gives id 0, so every bold cell would look
like a fallback.

**`ShapedGlyph::split: bool`**, a separate field (the struct has 13 bytes
of fields, so it stays 14 with the bool in its padding). Set when the
glyph id differs from what `glyph_key` gives the cell's character alone,
or the glyph covers more cells than its cell's own width. Never for
fitted icons, sprites, or a face's own wide glyphs (CJK in Sarasa Mono,
ambiguous-width characters at two cells), which keep drawing as one
instance. Fira Code and JetBrains Mono draw `->` as a blank spacer glyph
plus a glyph whose ink overhangs into the cell before it, and Helvetica-
style `fi` is one glyph over two cells; both are `split`.

**Cell width against advance.** A cell is round(advance) wide, plus E.4's
`cell_width` adjustment, so an overhanging ligature's ink is (n−1) ×
(cell width − advance) off its cells: 0.8 px on `===` at default sizes,
several pixels with `cell_width` set. Split glyphs are therefore
rasterised scaled horizontally by cell width / advance when that is off
by ≥ 1 px over the glyph's extent, keyed with a new `GlyphKey::SCALED`
variant bit so scaled and plain bitmaps never share a key.

## 4. Drawing (term-render)

### 4.1 Pieces per cell

A `split` glyph is emitted as one instance **per cell its ink touches**
(cells.md §10.3, widened from "cells it covers" to "cells its ink
reaches"):

- every piece uses the glyph's texture rectangle clipped on the CPU to
  that cell's columns (x, w, u adjusted); the leftmost piece stays open to
  the left and the rightmost to the right;
- each piece takes its own cell's foreground and selection colour;
- pieces in hidden (SGR 8) cells are dropped, as hidden glyphs are today;
  pieces in blinking cells go with that cell's blink;
- cells under input-method text drop their pieces;
- the block cursor redraws, in the cursor-text colour, **the pieces in the
  cursor's cell**, not the cell's own glyph (for Fira's `-` that is the
  blank spacer). The cursor on `>` of `=>` inverts the arrow's right half.

No instance format or shader change. Extra instances: one per extra cell
touched, ~1–2 per ligature; more for Monaspace, whose `calt` swaps most
letters for neighbour-dependent forms (measured in B1.2).

### 4.2 Shaping cache by row

Kept in each pane's `PaneRender` (splits.md §3.2), so panes never share or
drop each other's entries. New file `term-render/src/shape_cache.rs`:

```rust
/// Shaped glyphs of one terminal's visible rows, by row key (cells.md §9.2).
pub struct ShapeCache {
    rows: FxHashMap<RowKey, Entry>, // RowKey: Screen(seq) or History { line, col }
    frame: u32,
    terminal_id: u64, history_epoch: u64, cols: u16,
    fonts: (FontsId, u32),          // B.2's FontSet slot and its generation
}
struct Entry { glyphs: Vec<ShapedGlyph>, used: u32 }
```

- A row whose key is cached isn't shaped; instances are built from the
  cached glyphs. Entries not used in a built frame are dropped (their
  `Vec`s go to a free list of up to 16), so the cache holds only visible
  rows.
- Dropped whole when `terminal_id`, `history_epoch`, `cols` or the fonts
  change; history rows at or after `popped_from` dropped (cells.md §9.1).
- Used only for faces that shape runs; Menlo never touches it.
- Colours, cursor, selection and blink aren't stored, so they never
  invalidate it. Faces, widths and clusters are row content (`seq`).
- **With R.2** this cache becomes part of R.2's row entry (same key, same
  dropping), and each cached instance records its cell, so the cursor,
  blink and input-method overlays can pick the pieces of one cell on a
  row whose cells weren't copied this frame.

**Second level, by text (kept only on numbers).** Programs that rewrite
identical rows give them new `seq`s: Claude Code (Ink erases and rewrites
its live area each update) and tmux window switches. (Neovim scrolls with
scroll regions, so its rows keep their `seq`; htop writes only changed
cells.) An LRU in term-font maps (face, run text) to shaped glyphs, with
FxHash, sized by entries to two screens at the largest grid seen. B1.4
measures the hit rate in Claude Code and tmux, and the LRU stays only if
their frame build drops by ≥ 20%.

## 5. Cost

CTLine time depends heavily on the font. Measured on this Mac with Core
Text from Swift, one CTLine of Latin text: Menlo 4.5 µs (160 chars) /
6.4 µs (300); SF Mono 4.4 µs (160); Courier New, whose GSUB has no Latin
lookups at all, 11.4 / 19.4 µs. JetBrains Mono and Fira Code run large
`calt` lookups at every glyph, so expect ≥ 20 µs per 300-column row:
**a full 300×100 change ≥ 2–3 ms**, under the terminal lock until R.1.

So **B1.1 starts with a measurement** (`cargo bench -p term-font --bench
shape`): JetBrains Mono (checked in), plus Fira Code, Cascadia Code,
Iosevka and Monaspace installed locally, at 80, 160 and 300 cells, cold
and warm. Go/no-go: if JetBrains Mono at 300 cells is over 25 µs, B1.2
shapes outside the terminal lock, from the snapshot (F3.3 builds from it
under the lock; moving that call out is R.1's first step, done early for
shaping).

Estimates to replace with those numbers (JetBrains Mono; Menlo unchanged
in every row):

| Case | Estimate |
|---|---|
| frame, nothing changed (row cache) | 100 lookups ≈ 2 µs |
| spinner, one 300-column row | ≈ 20 µs |
| full change, 300×100 | ≈ 2–3 ms (+ cluster/fallback cells from caches) |
| full change, 100×30 | ≈ 30 × 8 µs ≈ 0.25 ms |
| row with an icon every 12 cells | one CTLine + 25 cached icon lookups |
| `cat` 100 MB at 100×30, 120 frames/s under flood | ≈ 30 ms/s, ~3% of a core |

Memory (JetBrains Mono): shaping cache 14 B per glyph on screen (≤ 27 KB
at 80×24, ≤ 420 KB at 300×100; cells.md §16.1 said 12 B), free list ≤
16 rows; ligature bitmaps in the mask texture (~150 ligatures, ~1.6 KB
for a 3-cell one at 16×34 px). The mask texture starts at 1024² (1 MB)
and doubles; more glyphs (four faces of ligatures, Monaspace's alternate
letters) could push it to 2048² (+3 MB), so its size is recorded with
every footprint number. Plus Core Text's own CTLine caches (footprint).

Limits (fixed Mac, interleaved, 5 runs, release; Menlo vs JetBrains Mono
at the same pixel size):

| measurement | limit |
|---|---|
| Menlo: frame bench, `cat` CPU, footprint | within ±2% of the commit before |
| 100×30 full-change frame | ≤ 0.5 ms |
| 300×100 full-change frame | ≤ 4 ms (≤ 2 ms if shaped outside the lock) |
| steady frame, spinner frame | ≤ +5% / ≤ +30 µs |
| `cat` 100 MB: wall time and decline + WindowServer CPU | ≤ +5% |
| key latency p99, idle and under flood (`scripts/latency/run.py`) | ≤ +1 ms |
| idle CPU and wakeups | 0% and 0 |
| footprint idle, after 10k lines, and 4 tabs × 4 splits | ≤ +1 MB at the same mask texture size; texture size recorded |
| roadmap thresholds vs Ghostty, both in JetBrains Mono | `cat` ≤ 45%, footprint ≤ 35%, key p99 ≤ Ghostty's |

How: `cargo bench -p term-font --bench shape`, `cargo bench -p term-render
--bench frame` (full change, steady, spinner, emoji-heavy, icon every 12
cells; × Menlo, JetBrains Mono), `scripts/pressure/run.py` and
`scripts/latency/run.py` with `font_family = "JetBrains Mono"`,
`/usr/bin/footprint -p`.

## 6. Tests

Test font: JetBrains Mono Regular (OFL) under
`crates/term-font/tests/fonts/` with its licence, registered for the test
process (`CTFontManagerRegisterFontsForURL`, process scope), so CI needs
nothing installed; a subset (~40 KB) if the licence allows it (open
question 3). Regular only, which also tests the faces-are-one-font case.

term-font (`tests/ligatures.rs`):
- detection: Menlo, Monaco, Courier (probe) and SF Mono when installed
  don't shape runs; Courier New (Arabic-only `liga`) doesn't; JetBrains
  Mono does; `-calt -liga` turns it off; `+zero` alone stays on the fast
  path and changes `0`'s glyph; `+liga` on Menlo shapes `fi`;
- bold and italic with the Regular-only font still ligate;
- ligatures from **JetBrains Mono's own list** (`=>`, `->`, `!=`, `===`,
  `<=>`, `::`, `/*`): `split` on exactly the substituted glyphs; every
  cell covered by at least one glyph or blank;
- a true multi-cell ligature: Helvetica's `fi` with runs forced on;
- a run mixing ligatures, CJK, an emoji ZWJ sequence, a combining mark,
  a fallback icon and a sprite: the non-face cells get exactly the
  per-character path's keys; the ligatures either side survive;
- right to left: Courier New with runs forced on, `(` and `<` next to
  Hebrew keep their cmap glyphs, and the Hebrew cells get the
  per-character keys;
- U+00AD and U+200B inside a ligature run don't change neighbours' cells;
- wide glyphs of the face and fitted icons are never `split`;
- `SCALED` keys at three cell widths put a `===` ligature's ink within
  1 px of its cells;
- feature syntax: `tag`, `+tag`, `-tag`, `tag=n`; bad tags rejected.

term-render:
- pieces: an n-cell ligature gives pieces whose rectangles join into the
  glyph's exactly, each in its cell's colour, first open left and last
  open right; overhang ink lands in the cell it overhangs, in that cell's
  colour; hidden cells drop their pieces;
- block cursor on each cell of `=>` and `===` inverts only that cell's
  pieces; selection over half a ligature; input-method text over half;
- shaping cache, with a counting shaper: an unchanged row isn't shaped
  again, a changed `seq` is, a row that only moved isn't; dropped on
  `terminal_id`, `history_epoch`, width and font changes; two panes keep
  their own entries; after 10k lines only visible rows remain;
- pixels: offscreen render (as `tests/metal.rs`) of `-> => !==` in
  JetBrains Mono against a reference drawn with `CTFontDrawGlyphs` using
  the CTLine's glyph ids at grid positions (cell × cell width + dx) and
  `rasterize_key`'s Core Graphics settings; ≤ 1/255 mean difference.
  (`CTLineDraw` can't be the reference: it places glyphs at the font's
  advance, 15.6 px, not the 16 px grid.)

term-app: `font_features` parsing and notices; live reload; importer;
smoke test with the test font registered through an env var in test runs.

Recorded manual check: Neovim and Claude Code in JetBrains Mono and Fira
Code, the cursor moving through `=>`, a visual selection over a ligature.

## 7. PRs

| # | Crate | PR | Size | After |
|---|---|---|---|---|
| B1.0 | font | `feat(font): add the ligature api, switched off`: `ShapedGlyph::split`, `Fonts::features` (always off), `FontConfig::features`, `GlyphKey::SCALED`; `shape()` source-compatible | S | — |
| B1.1 | font | `feat(font): shape runs for faces with ligatures`: the cost measurement first (go/no-go above), detection, features applied, one CTLine per run with spaces, `split`, scaling, scratch buffers, test font, tests, bench. **Lands switched off**: `Fonts::features` reports runs only when the renderer asks with `allow_runs` (B1.2 sets it), so dogfood builds never show half-drawn ligatures | M | B1.0 |
| B1.2 | render | `feat(render): draw ligatures in pieces and cache shaping by row`: pieces, cursor/selection/IME/hidden over pieces, `ShapeCache` in `PaneRender`, shaping outside the lock if B1.1 says so, frame bench cases, pixel test; turns `allow_runs` on | M | B1.0, F3.3, PR 8, B2.1 |
| B1.3 | app | `feat(app): add the font_features setting`: config key, notices, live reload, Ghostty importer. Done by the main session between B2.2 and B2.3a (splits.md §10), so no two agents are in term-app | S | B1.0 |
| B1.4 | font + docs | second-level cache by text, kept only on the §4.2 numbers; DESIGN.md "Rendering" and "Measured", Menlo vs JetBrains Mono | S | B1.2 |

B1.0 merges first (small). Then B1.1 (font) runs beside B2.1 (render) and
B2.2 (app); B1.2 (render) follows B2.1, since both change the renderer
and B1.2 keeps its cache in B2.1's `PaneRender`. B1.1 can start before
F3.2, which touches only term-render.

## 8. Open questions for Alex

1. **Ligatures on by default** for fonts that have them? Ghostty and
   kitty: on; iTerm2: off ("Use ligatures"). Proposed: on (the font's own
   defaults). The default font stays Menlo, so default installs don't
   change.
2. **A shorthand** `ligatures = false` beside `font_features`, or only
   `font_features = ["-calt", "-liga"]` (Ghostty's way)? Proposed: only
   `font_features`, documented with that example.
3. **Checking in JetBrains Mono** (OFL; full ~270 KB, or a ~40 KB subset)
   for tests. Without it CI has no ligature coverage. Proposed: check in.
4. **The cursor inside a ligature** keeps the ligature and inverts only
   its cell's part (this design); kitty and Ghostty can break the
   ligature under the cursor instead. Proposed: keep, no option in 1.0.

## 9. Review (Opus, xhigh)

The reviewer checked Core Text behaviour and font tables on macOS 27.
Taken:
- Right-to-left characters and bidi controls are kept out of the CTLine
  (it reordered and mirrored brackets and arrows). §3
- "The face draws it" compares with the registry id of the face's font,
  so bold and italic of a single-file family still ligate. §3
- Costs redone for GSUB fonts (2.5× Menlo or more); B1.1 measures first
  with a go/no-go to shaping outside the lock; limits revised. §5
- One CTLine per run with non-face cells as spaces, not one per stretch
  (fixed CTLine cost); trailing blanks trimmed. §3
- The shaping cache lives in each pane's `PaneRender` and checks fonts
  by `FontsId`. §4.2
- `split` no longer marks a face's own wide glyphs; a separate field, not
  a bit in `cells`. §3
- Pixel test reference drawn at grid positions, not with `CTLineDraw`. §6
- GSUB read through the `latn`/`DFLT` LangSys; `morx` faces probed
  (Menlo, Monaco, Courier checked) and `+tag` honoured for them;
  single-glyph features stay on the fast path. §1
- With R.2: the cache joins R.2's row entry and instances record their
  cell, for the cursor overlay. §4.2
- Text cache sized by screens, FxHash, measured on Claude Code and tmux. §4.2
- PR plan: an API PR first, B1.1 lands switched off, B1.3 by the main
  session, B1.1 not waiting on F3.2. §7
- Thresholds added (idle, 4 tabs × 4 splits, key p99 under flood, `cat`
  wall time, 120 frames/s, mask texture size, Monaspace). §5
- Ligature ink scaled when cell width and advance differ. §3
- Hidden and blinking cells, default-ignorables, multiple substitution,
  Helvetica `fi`, the font's own ligature list, first-use cost, scratch
  buffers, 14-byte glyphs, iTerm2's default. Throughout.
