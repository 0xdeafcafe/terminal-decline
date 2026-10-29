# Splits (B.2)

Status: **reviewed** by Opus (xhigh), findings folded in (§12); open
questions for Alex in §11. Accessibility is left out (not in 1.0).

## Goal and done-when

A window (or tab) holds several panes, each its own program, PTY and
terminal, side by side or stacked, with a 1 pt divider and nothing else
drawn of our own. Done when (roadmap): 4 panes all flooding, frame p99 and
decline + WindowServer CPU recorded against one pane; footprint per extra
pane recorded; "4 tabs and 4 splits footprint ≤ 35% of Ghostty's" (§3.4).

## 1. Shape of it

```
NSWindow
└─ WindowView            backing layer = the window's only CAMetalLayer;
   │                     one display link; draws every pane in one frame;
   │                     cursor rects for panes and dividers
   ├─ PaneView (1)       plain NSView: first responder, NSTextInputClient,
   ├─ PaneView (2)       mouse, tracking area; draws nothing
   └─ DividerView …      plain NSView over each divider (±3 pt): drag;
                         last among the subviews, so it wins hit testing
```

- **One CAMetalLayer per window.** What that buys: every pane's frame is
  presented together (no pane a frame behind its neighbour during a
  divider drag or live resize), one present and one surface per frame.
  What it costs: a frame redraws and the window server composites the
  whole window even when one pane changed, so one pane flooding in four
  pays for the window (measured, §3.4). Drawable memory is about the same
  either way (per-pane layers have pane-sized drawables), except that
  idle per-pane layers would give back their spare drawable.
- **PaneViews and DividerViews don't draw.** They don't override
  `drawRect:` or `updateLayer`, so AppKit gives them layers without a
  backing store (checked with `footprint` in B2.3a: no new IOSurface per
  pane). They are there for what AppKit routes by view: key events and
  the input method (each `NSTextInputClient` view has its own
  `NSTextInputContext`), hit testing and mouse-moved tracking. They get no
  autoresizing mask: WindowView lays them out, once per `setFrameSize:`.
- **Panes are a tree** in a plain module with no AppKit (`panes`).

Considered and not taken:
- `NSSplitView`: draws its dividers through `drawRect:` into a backing
  store the size of the split view, one per nesting level, and lays its
  subviews out by its own rules, which fight the cell grid and the
  title-bar inset.
- One `CAMetalLayer` per pane: simpler held panes (they just don't draw),
  GPU and WindowServer work only in the changed pane; but panes present
  out of step, and N layers each with display-link and drawable handling.
  Kept as the fallback if §3.4 shows the one-of-four flooding case costing
  more than a lone window of that pane's size by > 20%.
- **One view for the whole window** (input and IME through WindowView,
  panes only Rust objects, hit testing and cursors from the layout): no
  sublayers, no first-responder changes, no overlapping cursor rects. It
  loses a per-pane input context (marked text is dropped on a pane switch
  anyway) and per-view accessibility, which is out of 1.0. The roadmap
  asked for views per pane; open question 5 asks Alex whether that still
  stands without accessibility.

## 2. Modules and data (term-app)

### 2.1 `panes` (pure)

```rust
pub type PaneId = u32;                        // unique per process, never reused
pub enum Dir { Right, Down }                  // new pane to the right of / below
pub enum Node {
    Leaf(PaneId),
    Split { dir: Dir, ratio: f32, a: Box<Node>, b: Box<Node> }, // ratio = a's share
}
pub struct Panes { root: Node, focused: PaneId, zoomed: Option<PaneId> }

/// Whole device pixels, y down; views, scissor rects and origins all use it.
pub struct PxRect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }
pub struct Geometry { pub bounds: PxRect, pub top_inset: i32, pub min: (i32, i32), pub divider: i32 }
pub struct Layout { pub panes: Vec<(PaneId, PxRect)>, pub dividers: Vec<Divider> }
pub struct Divider { pub line: PxRect, pub grab: PxRect, pub path: SplitPath }

impl Panes {
    pub fn split(&mut self, dir: Dir, g: &Geometry) -> Result<PaneId, TooSmall>; // focuses the new pane
    pub fn close(&mut self, id: PaneId) -> Option<PaneId>;  // new focus; None = window empty
    pub fn layout(&self, g: &Geometry) -> Layout;
    pub fn min_size(&self, g: &Geometry) -> (i32, i32);     // for the window's contentMinSize
    pub fn drag(&mut self, path: &SplitPath, to: i32, g: &Geometry);
    pub fn resize(&mut self, dir: Direction, by: i32, g: &Geometry) -> bool;
    pub fn equalize(&mut self);
    pub fn neighbour(&self, dir: Direction, g: &Geometry) -> Option<PaneId>;
    pub fn next(&self) -> PaneId; pub fn previous(&self) -> PaneId;  // tree order, wrapping
    pub fn toggle_zoom(&mut self);
}
```

- Splitting replaces the focused leaf with `Split { ratio: 0.5 }`;
  equalize gives each leaf of a same-direction chain an equal share.
- `layout` gives each split a divider 1 pt wide in whole pixels at the
  backing scale and divides the rest by `ratio`, then clamps so no pane is
  under `min` (10 columns × 3 lines of the window's cells plus padding).
  `split` refuses with `TooSmall` (the window beeps). The window's
  `contentMinSize` follows `min_size`.
- Dividers' grab zones start below `top_inset`, so a drag in the
  transparent title bar still moves the window; the line itself runs the
  pane's full height.
- `neighbour(Left)`: of the panes whose right edge touches the focused
  pane's left edge, the one overlapping the focused pane's vertical
  centre, else the one with the most overlap (Ghostty, iTerm2).
- `close` gives the sibling the parent's space and focuses the sibling
  (or its first leaf).
- Zoom: `layout` returns only the zoomed pane, full size, no dividers.
  Splitting, closing or focusing another pane ends it.

### 2.2 Window and pane state

Today's `TerminalView` `State` splits in two:

- **`WindowState`** (WindowView): layer, display link, `Renderer` (frame
  assembly, drawables, instance buffers), `Panes`, `HashMap<PaneId,
  Pane>`, theme and appearance, scale, top inset, visibility, rate,
  `frames`, **one idle-tick counter**, test control, the window's `Fonts`
  (font size is per window, open question 3).
- **`Pane`**: `PaneView`, `Session`, `PaneRender` (§3.2), grid size,
  `focused`, blink, marked text, selection and mouse state,
  `awaiting_echo`, `keys_down`, `text_key`, `held`, `sync_since`,
  `crashed`.

Everything that reads the view's frame today reads the pane's rect:
`layout::grid_size`, `cell_at`, `first_rect`, auto-scroll edges. The
title-bar inset applies to panes touching the window's top; padding
applies inside every pane.

### 2.3 Session hooks and wake-ups, by pane

Every session hook (`wake`, `on_exit`, `on_events`, `on_control`, the
latency log's `on_parsed`) carries `(window id, PaneId)`; a hook for a pane
that no longer exists is ignored. So a closed pane's reader reporting
`Outcome::Closed` closes that pane only, not the window as it does today
(view.rs `program_exited`).

Wake-ups keep today's rule per pane: a reader wakes the main thread only
when its own dirty flag was clear (session.rs). That is at most one
wake-up per pane per refresh, each a no-op while the display link runs.
Only a wake from the pane **awaiting an echo** draws at once; others just
unpause the link. (A shared per-window flag was dropped in review: with
neighbours flooding it swallows the focused pane's echo wake-up and puts
the echo a tick late.)

## 3. Drawing

### 3.1 Per frame

On each display-link tick (one per window, `NSView.displayLink` on
WindowView), and on the other paths that draw today (`setFrameSize:`,
`output_arrived`, `unpause`, `updateLayer`):

1. **Per pane**, `frame_policy::pane` returns `Build` (dirty, not held),
   `Reuse` (clean), `Held` (in a synchronized update under the 150 ms
   timeout: dirty but its last instances are drawn) or `Waiting` (input
   sent, echo not yet here, under 8 ms). Panes hidden by a zoom aren't
   asked; they stay dirty until shown.
2. **Per window**, `frame_policy::combine` owns what `decide` decides
   today: `Hide` (window not visible), `Draw { rate }` when any visible
   pane builds (rate = the highest any pane asks for, from input and the
   **window's** focus, so a log you watch in an unfocused pane of the key
   window isn't 30 Hz), `Hold` when a pane is held or waiting and none
   builds (keep ticking), and `Skip`/`Pause` from the window's idle
   counter, which any build, hold or wait resets. So 0% CPU and 0 wake-ups
   idle hold with any number of panes.
3. Each Build pane: lock its terminal (blocking: a reader holds the lock
   for one ~1 KB read's parse, a few µs, so waiting costs little and a
   `try_lock` would starve flooding panes), build its instances, unlock.
   A draw started by the echo of the focused pane builds only that pane;
   the others stay dirty for the next tick.
4. Encode one render pass: for each visible pane, its background rects
   then its glyphs, with a scissor rect = the pane's `PxRect` clamped to
   the drawable (it can lag a live resize, and Metal rejects a scissor
   outside the target); then the dividers.

### 3.2 Per-pane render state (term-render)

```rust
/// Everything the renderer keeps for one terminal between frames.
pub struct PaneRender {
    rects: Vec<RectInstance>,      // pane-relative positions
    glyphs: Vec<GlyphInstance>,
    clear: Rgba,                   // the pane's default background, filling its rect
    atlas_gen: u32,                // glyph texture generation it was built against
    shape: ShapeCache,             // B.1 (ligatures.md §4.2)
    // later: R.1's last snapshot, R.2's row cache
}
pub struct PaneDraw<'a> { pub pane: &'a PaneRender, pub origin: (i32, i32), pub scissor: PxRect }
impl Renderer {
    pub fn build_pane(&mut self, pane: &mut PaneRender, input: &FrameInput);
    pub fn present(&mut self, layer: &CAMetalLayer, panes: &[PaneDraw], dividers: &[RectInstance]);
    // kept until B2.3: prepare(&FrameInput) + present(layer) as one full-window pane
}
```

- Instances are pane-relative; the vertex shader adds a per-draw origin
  (`Uniforms` grow from `viewport` to `viewport, origin`, 16 bytes through
  `setVertexBytes`). A pane that only moved (a neighbour closed) needs no
  rebuild unless its grid changed.
- The instance buffers stay per window (`FRAMES_IN_FLIGHT` = 2): each
  frame copies the visible panes' instances back to back and draws each
  pane with `baseInstance` over the one bound buffer (no per-draw buffer
  offsets, which have alignment rules for `constant` buffers). The copy
  is what one full-window pane copies today: ~0.6–1 MB at 300×100.
  Keeping instances in GPU buffers between frames belongs to R.2.
- With opacity < 1 the pass clears to transparent and each pane fills its
  own rect, so the background's alpha is applied once.
- Divider colour: the theme's background mixed 25% towards its
  foreground (visible in any theme and appearance, unlike
  `separatorColor`, which is ~10% black or white); a theme may set
  `split_divider`.
- Unfocused panes draw the hollow cursor. Panes aren't dimmed (open
  question 2).

### 3.3 One device and glyph texture set per process

Today every window has its own device, queue, pipelines, `Fonts`, glyph
textures and shaped-cluster cache. They become per process, on the main
thread (`Rc<RefCell<…>>` in a thread-local; `Renderer::new` picks it up,
so term-app's calls don't change in B2.1):

- `Gpu`: device, queue, both pipelines; the shader library is loaded once.
- `FontSet`: one `Fonts` per distinct (`FontConfig`, scale), shared by the
  windows that use it; the shaped-cluster cache becomes per process as
  cells.md §10.2 wanted. `FontsId` = its slot.
- `GlyphCache`: one R8 and one BGRA texture, keyed by (`FontsId`,
  `GlyphKey`). **Two in-flight counts:** a process-wide count of command
  buffers that sample the glyph textures, which uploads (`replaceRegion`),
  resets and purgeable changes wait on; and the existing per-window count
  for instance buffers and drawables, which `encode` waits on. One count
  for both would make windows wait on each other's frames.
- When a texture grows, the old one is blitted into it, so positions stay
  valid. Only emptying at the cap changes `atlas_gen`, and then every
  `PaneRender` built against the old generation is rebuilt (Reuse and Held
  panes too; a held pane then shows its current, unfinished screen once.
  Resets at the cap need ~30k distinct glyphs; after R.1 the pane's last
  snapshot is rebuilt instead).
- A `Fonts` no window uses is dropped; its glyphs go at the next reset.
- Hiding: textures go purgeable only when every window is hidden; each
  hidden window still gives back its drawables as today.

### 3.4 Cost and how it is measured

| | 1 pane (today) | 4 panes |
|---|---|---|
| drawables | 2 × window | same |
| glyph textures | 1 set per window | 1 set per process |
| frame build, all changed | O(window cells) | same cells + 4 locks + 8 draws (µs) |
| frame, one pane changed | — | that pane's build + copy of all instances (≤ 1 MB at 300×100) |
| parsing, all flooding | 1 reader | 4 readers on 4 cores (inherent; Ghostty too) |
| WindowServer | 1 surface | 1 surface, whole window each frame |

Memory per extra pane, expected: cells 15 KB at 80×24 plus history
(0.4–0.7 MB at 10k lines, cells.md §16), `PaneRender` ≤ 60 KB at 80×24,
reader and writer threads (~30 KB resident each), PaneView and input
context (tens of KB): **under 1 MB idle, plus history.** The shell is its
own process, as in every terminal.

Measured in B2.5 (fixed Mac, release, interleaved, 5 runs; roadmap rules:
same window pixels, font and scrollback bytes as Ghostty, footprint not
RSS, WindowServer included):

| measurement | limit |
|---|---|
| 1 vs 4 panes all running `yes`: frame build + encode p50/p99, frames/s per pane, decline + WindowServer CPU | 4-pane p99 ≤ 1-pane p99 + 1 ms; every pane ≥ 55 frames/s at 60 Hz |
| 1 of 4 panes flooding vs a lone window that pane's size: decline + WindowServer CPU | ≤ +20% (else §1's fallback) |
| focused-pane key latency, 3 neighbours flooding vs idle (`scripts/latency/run.py` `split-flood`; latency log keyed by pane) | p99 ≤ idle p99 + 2 ms |
| the same from outside against Ghostty (`probe.swift`; needs Accessibility and Screen Recording) | p99 ≤ Ghostty's |
| `cat` 100 MB in one of four panes: decline + WindowServer CPU vs Ghostty | ≤ 45% |
| footprint, 1/2/4 panes idle and after 10k lines each; per extra idle pane | ≤ 1.5 MB per pane |
| footprint, 4 tabs × 4 panes, idle and after 10k lines per pane, vs Ghostty (splits by its Cmd+D through `probe.swift`) | ≤ 35% |
| idle with 4 panes (`top -stats cpu,idlew`) | 0%, 0 wake-ups |
| full screen, 1 pane, before and after B2.3a (WindowServer CPU; Metal HUD `MTL_HUD_ENABLED=1` says direct or composited) | no change; if direct, check tearing |

### 3.5 Landed (B2.1)

term-render draws panes into one layer and shares the GPU and glyph
textures (`term-render/src/metal.rs`). What changed from §3.2–3.3, and
what B2.3 needs to know:

- **API:** `Renderer::build_pane(&mut PaneRender, &mut Terminal,
  &FrameInput)` (the terminal is passed, as since F3.3);
  `encode_panes(target, clear, &[PaneDraw], dividers)` and
  `present_panes(layer, clear, …)` take the pass's clear colour: a pane
  whose background isn't it fills its scissor first, so with opacity < 1
  pass 0. `Renderer::is_current(&PaneRender)` says whether a pane must be
  built again before it is drawn (the textures were emptied at the cap,
  or the window's fonts changed); a pane that isn't draws its backgrounds
  only until it is. `PxRect` (with `clamped`) lives in term-render, for
  `panes` to use. `prepare`, `present` and `encode` stay, drawing the
  full-window pane `Renderer::pane`; term-app changed only where its test
  dump read the glyph texture sizes and instance counts.
- **`PaneRender`** holds the `Frame` (instances, pane-relative, and the
  row cache) and what it was built against: the textures' generation and
  the `FontsId`. B.1's shaping cache goes in it.
- **Per process** means per thread in a thread-local, kept while any
  renderer is alive: the last window closing frees the device and
  textures, as before. `FontsId`s are never reused.
- **Fonts no window uses** are dropped, and then (not at the next
  emptying, as §3.3 had it) both textures are emptied and go back to
  their starting sizes, and every pane is built again. Their glyphs can't
  be taken out one by one, and the app gives each window fonts of its own
  until B2.3: leaving them would let closing windows or changing the font
  size grow the textures towards 16 MB, where before a closed window's
  textures were freed and a font change emptied them.
- **`FontSet`:** `Renderer::with_font_config` / `set_font_config` share one
  `Fonts` per (`FontConfig`, scale); `Renderer::new(Fonts)` / `set_fonts`,
  which the app calls today, add fonts no other window shares. B2.3
  should move to the config calls. Shared `Fonts` sit behind an `Rc`, so
  `Fonts::allow_runs` must be set before they are handed over (B1.2).
- **Lookups:** each `FontsId` keeps the old by-character lookup (an array
  for ASCII, a hash map for the rest) in front of the map by
  (`FontsId`, `GlyphKey`); B1.2's shaped glyphs can look up by key
  directly (`PaneGlyphs::by_key`, crate-private for now).
- **Growing:** the packer keeps what it placed and packs the new space
  right of and below the old square (1 px apart); the old texture is
  copied in with a blit, waited for (a few times in a process's life).
  The pane whose build ran out drops its row cache, since its rows were
  built without the glyphs that didn't fit, and is built again.
- **Hiding:** the textures go purgeable when every window is hidden, and
  come back when one is shown or a new window opens.
- **Tests** (`tests/panes.rs`, offscreen): two panes and a divider land
  in their rectangles, pixel for pixel what each draws alone; a pane drawn
  again unbuilt is identical; nothing of a pane (an italic `f` leaning
  out, a pane cut to half) draws outside its scissor; scissors past the
  target are clamped and a pane wholly outside is skipped; a texture that
  grows keeps a built pane's pixels with no rebuild; one emptied at its cap
  makes it stale (backgrounds only) until rebuilt, then identical; two
  windows with different fonts draw correctly while the other uploads
  glyphs between their frames; closing a window empties the textures back
  to their starting sizes and the other rebuilds identically. The
  packer's property test grows as it fills and checks nothing overlaps
  across growth.
- **Measured** (DESIGN.md "Panes in one layer"): one pane is unchanged
  within noise (160×48 build 55.8 → 55.9 µs, unchanged 4.4 → 4.2 µs,
  encode 15.0 → 14.7 µs); the same cells as four 80×24 panes cost 57.6 µs
  to build, 5.8 µs unchanged and 16.8 µs to encode with two dividers.

### 3.6 Landed (B2.3a)

term-app's view is split in two, still with one pane per window:

- **`WindowView`** (`view.rs`, `DeclineWindowView`): the layer, display
  link, window delegate, title-bar observer, font size, theme and test
  control. Its state is `Win` (what the window keeps), the `Panes` tree
  and the panes by `PaneId`. It lays the panes out with
  `Panes::layout` on every `setFrameSize:` (one pane gets the view's own
  frame, so its grid is worked out exactly as before).
- **`PaneView`** (`pane_view.rs`, `DeclinePaneView`): a plain `NSView`
  with no autoresizing mask that draws nothing; first responder,
  `NSTextInputClient`, mouse, scroll, drag and drop, the context menu.
  Its `Pane` holds the session, its `PaneRender`, rect, blink, marked
  text, selection and mouse state, the echo wait, keys down and `held`.
  The app makes the pane view first responder
  (`WindowView::first_responder`).
- **Hooks** carry (window id, `PaneId`) and do nothing once the pane is
  gone. The latency log still counts by window (B2.5 keys it by pane).
- **Drawing**: each tick asks `frame_policy::pane` per shown pane and
  `combine` for the window; panes that build (or that
  `Renderer::is_current` says are stale) are built with `build_pane`,
  then all are drawn with `present_panes` (the latency path with
  `encode_panes`). One pane clears to its own background, several to 0.
  A held pane keeps its change; a pane waiting for its echo isn't marked
  dirty.
- **Fonts**: the app makes the window's renderer with
  `Renderer::with_font_config` and Cmd+= / reloads use
  `set_font_config`, so windows with the same fonts share them. A
  throwaway `Fonts` for the missing-font notices is built only when the
  config names fallbacks or faces.
- **Measured** (release, M1 Max under heavy load from other agents,
  load average 6–56; before = main with B1.3, after = this; interleaved,
  full screen 223×72 at 2×): a key whose program redraws every row,
  parsed → frame built p50 0.07–0.08 ms both, encode p50 0.11 ms both,
  bytes sent → commit p50 0.7–1.5 ms before, 0.7–2.0 after (p99 tails
  of 10–37 ms in both, from the load); `yes` flooding 59 frames/s both
  while key; footprint full screen 101–133 MB both (bimodal in both),
  after the flood 108–110 MB both, idle 66–68 MB both; idle 0% CPU and 0
  wake-ups (`top idlew`) in every run. No new IOSurface for the pane
  view. Whether full screen is shown direct or composited (Metal HUD)
  wasn't checked.

## 4. Input and focus

- The focused pane's PaneView is the window's first responder. Focus
  work lives in PaneView's `becomeFirstResponder` /
  `resignFirstResponder`, because AppKit changes the first responder
  itself (before `mouseDown:`, and when a window becomes key):
  - on resign: marked text is committed (as NSTextView does; open
    question 6), focus-out reported (mode 1004), and under kitty flags 2/8
    releases are sent for keys and modifiers still down;
  - on become: focus-in reported, and releases for keys this pane never
    saw pressed are dropped (`handle_flags_changed` gets the `keys_down`
    check it lacks today).
  - the window's own focus changes go to the focused pane only.
- Clicking a pane focuses it, and that click goes nowhere else (no
  selection, no mouse report): `becomeFirstResponder` notes that the
  current event is a mouse-down in this view. `acceptsFirstMouse` is YES
  for panes, so the click that activates an inactive window focuses the
  pane clicked.
- Mouse-moved: the window's `acceptsMouseMovedEvents` (app.rs) is turned
  off, since it sends every move to the first responder wherever the
  pointer is. Each PaneView has a tracking area (`MouseMoved |
  ActiveInKeyWindow | InVisibleRect`) for mode 1003 and link hover.
- Cursors: WindowView's `resetCursorRects` sets them all from `Layout`
  (I-beam over panes, `columnResizeCursor` / `rowResizeCursor` on macOS
  15+, else `resizeLeftRightCursor` / `resizeUpDownCursor`, over grab
  zones), since overlapping sibling cursor rects have no defined winner.
- Scrolling goes to the pane under the pointer when a gesture starts, and
  stays there through its momentum.
- `firstRectForCharacterRange` uses the pane's cursor in the pane's rect.
- Title: a pane's program sets the window/tab title only while focused;
  focusing a pane shows its last title. Notifications follow today's rule
  (window not in front) and also post when the pane is hidden by a zoom;
  a click on one focuses its pane and ends a zoom. Dock progress is kept
  per pane and cleared when it closes (notify.rs keys by pane).

## 5. Keybinds and menus

New `MenuAction`s, in the Shell menu (split, close) and the Window menu
(select, resize, zoom); all remappable through `[keybind]`. Pane items stay
enabled and beep when there is no pane in that direction.

| action | default | does |
|---|---|---|
| `split_right` / `split_down` | Cmd+D / Cmd+Shift+D | new pane right of / below the focused one, in its working directory |
| `close` | Cmd+W | closes the focused pane (`closePane:`); the tab when it is the last |
| `close_tab` | — | closes this tab and its panes (`performClose:`; also the title bar's close button) |
| `close_window` | Cmd+Shift+W | closes the whole window, every tab (Safari, Terminal) |
| `focus_pane_left/right/up/down` | Cmd+Opt+arrows | by geometry |
| `focus_pane_next/previous` | Cmd+] / Cmd+[ | tree order, wrapping |
| `resize_pane_left/right/up/down` | Cmd+Ctrl+arrows | moves the nearest divider one cell |
| `equalize_panes` | Cmd+Ctrl+= | equal shares |
| `toggle_pane_zoom` | Cmd+Shift+Enter | focused pane fills the window; again to restore |

iTerm2's and Ghostty's defaults. Cmd+[ / ] are free (tabs use Cmd+Shift+[
/ ]). The Mac line-editing keys (`keys::translate`: Cmd+← / → / ⌫) are
narrowed to Cmd and Cmd+Shift alone, since today they match whatever Opt
or Ctrl are held and would send ^A / ^E for Cmd+Opt+← with no pane to the
left. E.10's confirm-before-close asks once, listing the running panes.

## 6. Divider drag and resizing

- DividerView takes mouse down/drag/up: `mouseDragged` moves its split
  through `Panes::drag` (clamped) and requests a frame; drawing happens on
  the display-link tick at the input rate (120 Hz). Double-click
  equalizes that split.
- Each pane's grid and PTY are resized only when its cell count changes,
  not per pixel. Reflow per resize is F.4's (screen only, 0.5–1 ms with
  100k lines), so a drag across four panes stays in F.4's 8 ms frame.
- Window resize: ratios kept, every pane laid out once, PTYs resized on
  cell-count changes only.

## 7. Closing, exits and crashes

- A pane's program exiting follows today's window rule, per pane: status
  0 or after the first second closes the pane; a quick failure keeps the
  pane open with its message until a key is pressed in it. The last pane
  closing closes the tab.
- Closing a pane: focus moves first (the first responder is never a view
  being removed), then its session hangs up, its views are removed, the
  tree collapses and the rest are laid out.
- **A crash stays in its pane.**
  - Parsing: unchanged, already per session.
  - Building a pane's frame (main thread): the catch is at the build call
    site (`build_pane`), which every draw path goes through. The pane's
    terminal guard is taken outside the `catch_unwind` and the terminal
    replaced before it is released (as `session::parse` does), so the
    reader never parses into a broken terminal. Then: its program is
    killed; its `PaneRender` (instances, shaping cache) cleared; the glyph
    texture generation bumped (a half-inserted glyph can't be trusted;
    every window rebuilds once); the shared `Fonts`' shaped-cluster and
    run caches emptied (a bad cached shaping would crash every pane using
    that font). The message is the parse panic's. A second build panic in
    the same pane within 10 s stops building it: it draws its background
    and that message only, so a panic doesn't repeat every frame.
  - The shared code around it (scissor, origins, instance copy, encode)
    uses saturating arithmetic: a panic there still aborts.
  - Other main-thread panics (event handlers) abort after logging, as
    today.

## 8. Saving and restoring

`reload::SavedWindow` gains `panes: Option<SavedPanes>`: the tree with
ratios, each leaf's working directory, focused and zoomed pane. A dogfood
restart reopens the same layout; older files give one pane per window.

## 9. Tests

- `panes` unit and property tests (random splits, closes, drags, sizes,
  scales): the layout tiles the bounds exactly in whole pixels, no
  overlap; no pane under the minimum unless the window is; `split` refuses
  rather than break the minimum; closing every pane in any order ends
  empty; `next` visits every pane once; `neighbour` table tests on 2×2,
  L-shaped and nested layouts; zoom ends on split/close/focus.
- `frame_policy` table tests: one pane held while another builds → Draw
  with Held reused; all idle → Pause, with panes going idle out of step;
  awaiting echo → Hold until 8 ms; held 150 ms → Build; zoom-hidden
  flooding pane → no draw; rate from window focus.
- term-render offscreen (`tests/metal.rs`): two panes with different text
  land in their rects; an italic glyph at a pane's edge doesn't draw into
  the neighbour; a reused pane gives identical pixels; a texture grow
  keeps pixels without a rebuild, a reset rebuilds; the scissor is
  clamped to a smaller drawable; two windows sharing the glyph textures
  each draw correctly while glyphs are uploaded between their frames.
- Smoke (`tests/smoke.rs`), control commands `split right|down`, `focus
  <n>`, `close-pane`, `zoom`, `dump` listing panes (rect, grid, focused,
  builds/reuses): typing reaches only the focused pane; `stty size`
  matches each grid after splits, drags and window resizes; Cmd+W on pane
  1 of 2 leaves pane 2's program running; closing the last pane closes
  the tab; a pane whose program exits closes; a quick failure holds in
  its pane; the parse test hook in one pane leaves the other echoing; a
  build test hook (new) in one pane leaves the other drawing, and a
  second one stops that pane building; a pane in synchronized update is
  reused while the other draws; restore round-trips a 3-pane layout.
- Recorded manual checks: Japanese input (candidate window at the right
  pane's cursor; switching panes mid-composition), divider drag with
  Neovim in both panes, tmux in a pane, zoom with htop, mode 1003 hover in
  an unfocused-then-focused pane.

## 10. PRs

One agent per crate at a time. B2.1 keeps today's `Renderer` calls working
(one full-window pane), so it doesn't touch term-app.

| # | Crate | PR | Size | After |
|---|---|---|---|---|
| B2.1 | render | `feat(render): draw panes into one layer and share glyph textures`: `Gpu`, `FontSet`, process-wide `GlyphCache` keyed by (`FontsId`, `GlyphKey`) with two in-flight counts and grow-by-blit, `PaneRender`/`PaneDraw`, origin uniform, `baseInstance`, scissor; offscreen tests; bench 4 × 80×24 vs 1 × 160×48 | M | F3.3 |
| B2.2 | app, pure modules (`panes`, `frame_policy`, `keybind`, `reload`, `import`) | `feat(app): add the pane tree, layout and pane actions`: `panes` with property tests, `frame_policy::pane`/`combine`, new `MenuAction`s (not in menus yet), `SavedPanes`, Ghostty importer for `new_split`, `goto_split`, `resize_split`, `equalize_splits`, `toggle_split_zoom`, `split-divider-color` | M | — |
| B2.3a | app (AppKit) | `refactor(app): split the view into a window view and a pane view`: one pane, no behaviour change; hooks keyed by pane; full-screen and footprint before/after | M | B2.1, B2.2, B1.3 |
| B2.3b | app | `feat(app): split, close and focus panes`: split/close/focus actions and menus, responder handling, IME, tracking areas, cursor rects, line-editing keys narrowed, smoke tests | M | B2.3a |
| B2.3c | app | `feat(app): drag dividers, zoom and restore panes`: drag, resize/equalize, zoom, restore, control commands | M | B2.3b |
| B2.4 | app | `feat(app): contain a crash while drawing to its pane` | S | B2.3c |
| B2.5 | scripts + docs | `split-flood` latency scenario, 4-pane pressure runs, Ghostty comparison, DESIGN.md updates (Windows and tabs, Rendering, Failures and crashes, Measured; and the stale "64 KB slices" and "triple-buffered") | S | B2.4 |

Alongside B.1 (ligatures.md §7), once its small B1.0 has merged: B2.1
(render), B2.2 (app) and B1.1 (font) run together; then B1.2 (render)
and B1.3 (app, small, by the main session); then B2.3a–c (app) one after
another.

## 11. Open questions for Alex

Decided 29 Oct 2026, all as proposed: 1 the §5 keybinds (iTerm2's);
2 not dimmed, `unfocused_pane_opacity` default 1.0; 3 font size per
window; 4 a split in a pin runs the user's shell; 5 views per pane;
6 commit the marked text; 7 the focusing click is swallowed.

1. **Keybinds** as in §5 (iTerm2/Ghostty defaults; Cmd+W closes a pane
   first; Cmd+Shift+W closes the whole window with its tabs).
2. **Dim unfocused panes?** Ghostty does by default (70%). Proposed: no,
   the hollow cursor marks them; `unfocused_pane_opacity` (default 1.0)
   for those who want it.
3. **Font size (Cmd+= / Cmd+-) per window or per pane?** Ghostty and
   iTerm2: per pane. Proposed: per window in 1.0; the shared textures
   keyed by `FontsId` allow per pane later.
4. **A split in a pin or `-e PROGRAM` window** runs the same command or
   the user's shell? Proposed: the user's shell (a second copy of a pinned
   tool is rarely wanted), in the pane's working directory.
5. **Views per pane or one view per window?** With accessibility out of
   1.0, one view (§1) is simpler and sidesteps the responder and cursor
   handling in §4. Proposed: keep views per pane as the roadmap says,
   since accessibility comes after 1.0 and would need them; one view if
   you'd rather have less AppKit code now.
6. **Switching panes mid-composition**: commit the marked text (proposed,
   as NSTextView does) or discard it?
7. **The click that focuses a pane**: swallowed (proposed), or also sent
   to the program / starting a selection?

## 12. Review (Opus, xhigh)

Taken:
- Wake-ups stay per pane and carry the pane; only the awaited echo draws
  at once (a shared flag delayed the echo under neighbour floods). §2.3
- `try_lock` dropped: the reader holds the lock per ~1 KB read, so skips
  would starve flooding panes; blocking costs µs. §3.1
- Two in-flight counts for shared glyph textures; grow by blit. §3.3
- `frame_policy` split into per-pane and per-window decisions with one
  idle counter; zoom-hidden panes ignored; echo wait kept. §3.1
- Session hooks keyed by pane; a closed pane's exit no longer closes the
  window. §2.3
- Mouse-moved through per-pane tracking areas, not the window's
  `acceptsMouseMovedEvents`. §4
- Focus, IME and kitty releases in `become`/`resignFirstResponder`;
  `acceptsFirstMouse`. §4
- Cmd line-editing keys narrowed; pane items beep instead of falling
  through. §5
- `close` / `close_tab` / `close_window`. §5
- `PaneRender` holds all per-terminal render state (B.1's shaping cache
  included). §3.2, ligatures.md §4.2
- The one-layer rationale corrected and measured; one view per window
  added as an alternative (question 5). §1
- Crash handling at the build call site, with the guard outside the
  catch, cache resets and a repeat limit. §7
- Measurements against every roadmap threshold, latency log keyed by
  pane. §3.4
- PR plan: B2.1 leaves term-app alone; B2.3 in three; B1.3 before it;
  B2.5 last. §10
- Whole-pixel layout, `contentMinSize`, grab zones below the inset, one
  layout per resize, cursor rects in one place, divider colour from the
  theme, progress and notifications by pane, `baseInstance`, opacity
  clear, scroll gestures stay in their pane, the instance copy size
  corrected. §2–§4
- New questions 4 and 6.

Not taken: building a flooding pane at only its own rate while the window
ticks faster for typing (the echo-only build in §3.1 covers the latency
case; revisit with the §3.4 numbers).
