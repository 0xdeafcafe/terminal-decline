# Text size per pane, pane titles and arranging panes

Asked for by Alex, 4 Nov 2026: each pane its own text size; a title on
each pane; and moving the focused pane, or placing a new one, with the
keyboard only, seeing the layout it will make before it happens. Choices
are Alex's from the mockups of 4 Nov 2026 (§6). Builds on the splits
(splits.md); this replaces its open question 3 ("font size per window in
1.0").

"Zoom" already means a pane filling the window (Cmd+Shift+Enter), so the
size of a pane's text is its **text size**.

## 1. Text size per pane

```
┌──────────────────────────────────┬──────────────────────────────────┐
│ ＄ ｃａｒｇｏ ｔｅｓｔ           │ $ htop                           │
│ ｒｕｎｎｉｎｇ ２１４ ｔｅｓｔｓ │  CPU [||||||||||        43.1%]   │
│                  ╭────────────╮  │                                  │
│                  │ 16 pt  (+2)│  │                                  │
│                  ╰────────────╯  │                                  │
│                               +2 │                                  │
└──────────────────────────────────┴──────────────────────────────────┘
```

- **Cmd+= / Cmd+- / Cmd+0** (`font_bigger`, `font_smaller`,
  `font_reset`, as before) change the **focused pane** by `font_size_step`,
  or put it back to `font_size`. So do a pinch and the control's `font`.
- **Cmd+Opt+= / Cmd+Opt+- / Cmd+Opt+0** (new: `font_bigger_all`,
  `font_smaller_all`, `font_reset_all`) do it to **every pane of the
  window**: each moves by a step (their differences stay), or all go back
  to `font_size`. View menu: Bigger, Smaller, Actual Size (pane), then
  the three for the window under an Option-alternate.
- **Badge**: a change shows the pane's size, `16 pt  (+2)`, in a rounded
  label at the pane's bottom right for 1 s, then fades.
- **The faint "+2"**: while a pane's size isn't `font_size`, a small
  label in its bottom-right corner says by how much (`+2`, `−1`), in the
  theme's foreground at 35%, over the padding where it can (it covers a
  cell's corner at worst; it takes no clicks).
- **New panes** start at the size of the pane they were split from, or,
  for an arranged new pane (§3), the focused one.
- **Saved** for a restart: `SavedPane::font_size` (points; absent: the
  config's). A config reload that changes `font_size` puts every pane back
  to it, as it did the window.
- **The window's size** for a new window, `set_grid` and the control's
  `resize` use the focused pane's cells. `Geometry::min` (10 × 3 cells)
  stays in the config size's cells, so a big-text pane can be fewer than
  10 columns at the minimum, and a small-text one more; dividers move by
  one of the focused pane's cells (Cmd+Ctrl+arrows).

### 1.1 Drawing (term-render)

The process already keeps one `Fonts` per (`FontConfig`, scale) in its
`FontSet`, shared by the windows using it, and the glyph textures key
glyphs by `FontsId` (splits.md §3.3). Today a `Renderer` (a window) holds
one `FontsId`, which builds, `is_current` and `encode` use for every pane.

- `PaneRender` gets the fonts it draws with as an optional
  `PaneFonts` (id and `Rc<Fonts>`, released to the `FontSet` on drop):
  `Renderer::set_pane_fonts(&mut pane, Option<(&FontConfig, f64)>)`;
  `None` is the window's.
- Each place the renderer uses `self.fonts_id` for a pane (`build`,
  `snapshot_pane`, `build_from_snapshot`, `is_current`, the glyph-thread
  calls, `encode`'s current check, done per pane) takes the pane's.
  `Renderer::metrics_of(&pane)` gives its cells.
- Cost: none for panes at the window's size (one `Option` read). A second
  size is a second `Fonts` (its faces and shaping cache, ~1–2 MB
  footprint, measured when it lands) and its glyphs in the shared
  textures, as a second window at another size costs today.

## 2. Pane titles

```
┌ nvim · decline ──────┬ cargo test · 12s ────┐
│ fn main() {          │ running 214 tests    │
│     let x = 1;       │ test a ... ok        │
│                      ├ htop ────────────────┤
│                      │ CPU [|||||   43%]    │
└──────────────────────┴──────────────────────┘
```

- **When**: a strip across the top of every pane while the window shows
  two or more (not zoomed). With one pane, only while macOS shows no
  title bar of its own (full screen, where it slides away): the title bar
  already says it otherwise (Alex, 4 Nov 2026).
- **What**: the pane's name if it has one (§2.1), else what the window
  title would say if that pane had focus (`activity::status_title`, or
  the program's title), without the app's name: `cargo test · 12s`,
  `✓ cargo build · 1m 12s`, `waiting · ~/src/decline`.
- **Look**: one line of the system's small font, its height
  (`STRIP_HEIGHT`, 20 pt), on the pane's background; the focused pane's
  in the theme's foreground, the others' at 55%. Text is truncated in
  the middle. It is an `NSTextField` label in a plain view, a subview of
  the pane view (as the find bar is): the Metal path doesn't change.
- **Space**: it takes the top of the pane, below the title-bar inset
  for panes at the top, so the grid is one strip shorter (the program is
  resized when strips come and go: on the second pane's split and the
  last close, not on every frame). The find bar goes below it.
- **Clicks**: a click focuses the pane; a double-click renames it.

### 2.1 Renaming

- **Rename Pane…** (Window menu, `rename_pane`, no default key) and a
  double-click on the strip edit the name in place in the strip: Return
  sets it, Esc cancels, an empty name goes back to the automatic one.
  With no strip showing (one pane, title bar visible) the menu item asks
  in a sheet instead.
- The name is saved for a restart (`SavedPane::name`) and set by the
  agent API (`rename-pane N NAME`; dumps give each pane's `name` and
  `title`).

## 3. Arranging panes

**Cmd+Shift+M** (`arrange_pane`, Window menu: Move Pane…) lifts the
focused pane; **Cmd+Shift+N** (`place_new_pane`, Shell menu: New Pane…)
starts the same with a new, empty pane. The arrows step it through the
places it could go, and the window shows the whole layout that would
make, as an outline over the dimmed panes. **Return** places it; **Esc**
puts everything back. No program is resized until Return.

```
start: A lifted                         → : the layout you'd get
┌──────────────┬───────────────┐       ┌──────────────────────────────┐
│╔════════════╗│░░░░░░░░░░░░░░░│       │┌────────────┐┌──────────────┐│
│║  A  nvim   ║│░░ B cargo ░░░░│       ││▓▓ A nvim ▓▓││ B cargo test ││
│║  (lifted)  ║├───────────────┤       │└────────────┘└──────────────┘│
│║            ║│░░ C htop ░░░░░│       │┌────────────────────────────┐│
│╚════════════╝│░░░░░░░░░░░░░░░│       ││ C htop                     ││
├──────────────┴───────────────┤       │└────────────────────────────┘│
│ MOVE A ←↑↓→ ␣ swap 1-9 ⏎ esc │       │ MOVE A → left of B   ⏎ esc   │
└──────────────────────────────┘       └──────────────────────────────┘

each → steps to the next place that way:
 start        →            →             →               ↓
┌────┬────┐ ┌─────────┐ ┌─────────┐ ┌────┬────┐   ┌─────────┐
│▓▓▓▓│ B  │ │▓▓▓▓│ B  │ │ B  │▓▓▓▓│ │ B  │▓▓▓▓│   │    B    │
│▓▓▓▓├────┤ ├────┴────┤ ├────┴────┤ ├────┤▓▓▓▓│   ├─────────┤
│▓▓▓▓│ C  │ │    C    │ │    C    │ │ C  │▓▓▓▓│   │    C    │
└────┴────┘ └─────────┘ └─────────┘ └────┴────┘   ├─────────┤
 where it    left of B   right of B  window's      │▓▓▓▓▓▓▓▓▓│
 is now                              right edge    └─────────┘
```

### 3.1 Places (`panes`, pure)

```rust
pub enum Moving { Pane(PaneId), New(PaneId) }      // New: a fresh id, not in the tree
pub enum Side { Left, Right, Top, Bottom }
pub enum Spot {
    Home,                                          // where it is (Moving::Pane only)
    Beside { node: SplitPath, side: Side },        // a new split around that node
    Swap(PaneId),                                  // trade places with that pane
}
pub struct Place { pub spot: Spot, pub tree: Panes, pub rect: PxRect, pub beside: Option<PaneId> }

impl Panes {
    pub fn places(&self, moving: Moving, g: &Geometry) -> Vec<Place>;
    pub fn step(places: &[Place], from: usize, dir: Direction) -> Option<usize>;
}
```

- **Beside**: take the moving pane out of the tree (`T'`); for every node
  of `T'`, a leaf or a split (the root included: the window's edges), and
  each side, the tree with that node replaced by a split with the moving
  pane on that side, `ratio` 0.5. `rect` is where the moving pane would
  be. Places that break a pane's minimum are left out, and of places
  giving the moving pane the same rect, the one nearest the root stays
  (left of B's right neighbour and right of B are one place).
- **Swap**: one per other pane; `rect` is that pane's.
- **Step** (an arrow): of the places whose rect's centre is past the
  current one's that way, the nearest, counting distance across the
  arrow at twice its weight; ties go to the larger rect. Swap places are
  reached with Space and the digits only, not the arrows. Tab and
  Shift+Tab go through every place in order, so any place can be
  reached even when the arrows skip one.
- Property tests: every place's tree has the same panes (plus the new
  one) and lays out within the minimums; stepping is monotonic along the
  arrow, so it never cycles; Home's tree is the tree; applying a place
  and taking the pane back out gives `T'`.

### 3.2 Keys in the mode

| key | does |
|---|---|
| ← ↑ ↓ → | the next place that way (beeps when there's none) |
| Tab / Shift+Tab | the next / previous place, in order |
| Space | swap with the pane the place is beside (or, on a swap, back beside it) |
| 1–9 | swap with pane n (the outline numbers the panes) |
| Return | place it: the tree becomes the place's, each pane resized once |
| Esc, a click, the window losing key | cancel: nothing changed |

Any other key is swallowed. The moving pane keeps focus after Return; a
new pane gets focus, opened in the focused pane's folder at its text
size. Entering ends a zoom. With one pane, Cmd+Shift+M beeps;
Cmd+Shift+N offers the four sides. Cmd+D / Cmd+Shift+D are unchanged.

### 3.3 The overlay

An `ArrangeView` (`arrange_view.rs`) over the whole window view, while
the mode lasts: plain layer-backed views, no drawing of our own.

- A dimming layer over the panes (black at 45%).
- The place's layout: each pane's rect as an outline (1 pt, the theme's
  foreground at 70%) with its title (§2) and size in cells (`80×14`); the
  moving pane filled with the accent colour at 35%; a new pane dashed,
  `+ new` and its folder. Digits 1–9 at each pane's centre for a swap.
- A hint bar at the bottom: `MOVE A → left of B` (or `NEW PANE →
  below B`) and the keys.
- It takes the keys while it is up (first responder); the pane views
  don't see them, so no program gets one.

## 4. Data and modules

| where | what |
|---|---|
| term-restore `panes` | `places`, `step`, `Place`, `Spot`, `Side`, `Moving`; `Panes::with_root` for a place's tree |
| term-restore `reload` | `SavedPane::{font_size, name}` (absent in older files) |
| term-render | `PaneFonts`, `set_pane_fonts`, `metrics_of`; per-pane fonts in builds and `encode` |
| term-keys | `font_bigger_all`, `font_smaller_all`, `font_reset_all`, `rename_pane`, `arrange_pane`, `place_new_pane` |
| term-app `view` | `Pane::{font_size, name}`, per-pane metrics in grids and geometry, strips in the padding |
| term-app `pane_strip.rs` | the title strip and its rename field |
| term-app `arrange_view.rs` | the arrange mode's view, keys and overlay |
| term-app `control` | `font N DELTA`, `rename-pane N NAME`, `arrange …`; dumps: `font_size`, `name`, `title`, `arranging` |

## 5. PRs

| PR | scope | commit |
|---|---|---|
| A1 | render | `feat(render): fonts per pane` |
| A2 | app, keys, restore | `feat(app): text size per pane` |
| A3 | app, restore | `feat(app): a title strip on each pane, and renaming it` |
| A4 | restore | `feat(app): the places a pane can be moved to` (pure, tests) |
| A5 | app, keys | `feat(app): move a pane or place a new one with the keyboard` |

## 6. Decided (Alex, 4 Nov 2026)

1. Cmd+= is the pane's; Cmd+Opt+= is the window's (every pane).
2. The preview is the whole layout you'd get, outlined over dimmed panes
   (not a shaded half of the target, which can't tell "right of B" from
   "the window's right edge").
3. Each arrow press steps to the next place that way.
4. The faint `+2` stays while a pane isn't at the default size.
5. Pane titles: a strip on each pane while there are two or more; with
   one, only when the macOS title bar isn't showing.
6. Panes can be renamed; the name sticks and is saved.
