# decline — design

A GPU-accelerated terminal for macOS, written in Rust. Goal: lower memory and
CPU use than Ghostty and Warp, with zero UI of its own, and the ability to pin
any command as its own Dock app.

## Goals

- **Idle costs nothing.** No timers, no redraws, no wakeups while nothing
  changes. Cursor blink (when an app asks for it) is the only timer, and it
  stops when the window is hidden or unfocused.
- **Small memory.** Measured as macOS *physical footprint* (what Activity
  Monitor shows; RSS misses GPU surfaces), compared at the same window pixel
  size. The two window drawables are unavoidable (~6.6 MB each at
  1624×1018 px) and scale with window size; the target is for everything else:
  < 25 MB for one window with 10k lines of scrollback.
- **Fast output.** `cat` of a large file must never block typing. A macOS PTY
  delivers at most ~240 MB/s (1 KB per read), so parsing must stay well above
  that: target ≥ 2× the PTY limit.
- **Zero UI.** The window is the terminal surface only: transparent, full-size
  content title bar with no title text, no tab bar, no settings screen,
  no status bar. Config is a file. Menus exist only because macOS requires a
  menu bar (Quit, Copy, Paste, New Window); they are standard AppKit menus.
- **Dock pins.** `decline pin` turns any command into its own macOS app.

## Modules (Cargo workspace)

| crate         | does                                                                  |
|---------------|-----------------------------------------------------------------------|
| `term-core`   | escape-code handling (via `vte`), screen grid, scrollback, selection  |
| `term-pty`    | spawns the shell on a pseudo-terminal, reader thread, resize          |
| `term-font`   | Core Text: font loading, cell metrics, glyph rasterising              |
| `term-render` | Metal: glyph texture, per-cell instance buffer, one instanced draw    |
| `term-app`    | AppKit window/view, keyboard + IME, mouse, clipboard, config, pins    |

`term-core` has no platform code and is tested and benchmarked on its own.

## Screen storage

- A cell is 12 bytes: `u32` with the 21-bit Unicode scalar plus 11 style flag
  bits (bold, italic, underline, wide, …), and one `u32` each for foreground
  and background (default / palette index / 24-bit RGB, tagged in the top byte).
- The visible screen is `Vec<Row>`, a row is `Vec<Cell>` plus a soft-wrap flag.
- Scrollback is a `VecDeque<Row>` with a line limit (default 10,000). When a
  row moves into scrollback its trailing blank cells are dropped and spare
  capacity released, so a typical 40-character line costs ~500 bytes instead
  of `cols × 12`. Later: compress old scrollback blocks (LZ4) for 100k+ lines.
- Points in the buffer use an ever-increasing line number so a selection stays
  attached to its text while output scrolls.
- Known gap for the first milestone: combining marks / grapheme clusters are
  dropped (needs a side table for multi-codepoint cells). Resize does not
  re-wrap lines yet.

## Threads

1. **Reader thread** (one per window): the PTY is non-blocking; the reader
   waits with `poll`, then drains everything available (up to 1 MB) before
   parsing, because a macOS PTY returns at most ~1 KB per `read()`. It parses
   in 64 KB slices, taking the lock once per slice, so drawing and typing
   never wait long. Replies to queries
   (cursor position, device attributes) are written back to the PTY from here.
   After parsing it wakes the main thread **only if** no redraw is already
   pending (an atomic flag), so a flood of output causes at most one redraw
   per display refresh.
3. **Main thread** (AppKit): input, and drawing. Drawing is driven by a
   `CADisplayLink` that is paused whenever nothing changed; it is resumed by
   the reader thread's wake-up or by input. Each frame it locks the terminal
   briefly, copies the visible cells into the GPU instance buffer, unlocks,
   then encodes the draw. While the program has synchronized-update mode
   (2026) on, drawing is held (150 ms timeout). Nothing is drawn while the
   window is hidden or covered. When the display link was paused, a wake-up
   draws immediately rather than waiting a refresh.

## Rendering (Metal)

- One `CAMetalLayer`, `presentsWithTransaction = false`, triple-buffered
  instance buffers in shared memory (Apple Silicon has unified memory, so no
  copies to the GPU).
- Per cell the GPU gets one instance: glyph rectangle in the texture (u16×4),
  glyph offset (i16×2), fg colour (rgba8), bg colour (rgba8), flags (u32). The
  vertex shader places a quad per cell from its index and the grid size, so no
  vertex buffer is needed. Two passes in one encoder: backgrounds, then glyphs
  (glyphs can overflow their cell, e.g. italics and wide icons).
- Glyph texture: an `R8` texture for text (coverage, blended in sRGB space
  like native Mac text) and a `BGRA8` texture for colour emoji. Shelf-packed;
  only the texture that fills up grows (caps: 4096² and 2048², 16 MB each);
  at the cap it is emptied and refilled with what's on screen. Glyphs are
  rasterised once by Core Text; ASCII is pre-rasterised at start. Uploading a
  new glyph waits for in-flight frames, since `replaceRegion` doesn't
  synchronise with the GPU.
- Shaders are compiled to a `.metallib` at build time (falls back to runtime
  compilation if the Metal toolchain is missing).
- Cursor, selection and underline/strikethrough are drawn in the shader from
  flags; no extra geometry.
- Background blur/transparency: `NSVisualEffectView` behind a non-opaque
  Metal layer, opt-in by config.
- sRGB colours (no Display P3 tagging, which would over-saturate app colours).
- Later: dirty-row tracking so only changed rows are re-copied (currently the
  whole visible grid is re-copied when anything changed — ~30k cells × 24 B =
  720 KB memcpy, which is cheap but not free).

## Input

- Text through `NSTextInputClient` (so IME, dead keys and emoji picker work).
  Marked (in-progress IME) text is drawn at the cursor.
- Keys with Ctrl / Option / arrows / function keys / Home / End / PgUp / PgDn
  mapped to xterm sequences; application cursor and keypad modes respected.
  While an input method is composing, every key goes to it first.
- Option-as-Meta configurable, **off by default** (like Terminal.app and
  Ghostty) so Option still types `@ [ ] { } | \ ~` on European layouts.
- Kitty keyboard protocol, "disambiguate" level: Shift+Enter, Ctrl+Enter,
  Ctrl+Tab, Esc vs Alt (Claude Code, Neovim, Helix rely on these).
- Cmd+← / Cmd+→ / Cmd+⌫ send start-of-line / end-of-line / delete-line.
- Cmd shortcuts are handled by the app, never sent to the program:
  Cmd+C copy (selection), Cmd+V paste (bracketed paste when on), Cmd+N new
  window, Cmd+W close, Cmd+Q quit, Cmd+K clear scrollback, Cmd+= / Cmd+-
  font size. Pins may disable or remap these in their config.
- Mouse: drag to select (word on double-click, line on triple-click), wheel
  scrolls scrollback, or when the program asks for mouse reporting (1000/1002/
  1003 + SGR 1006) events go to the program; Shift overrides to select.

## Dock pins ("make ./rush its own app")

`decline pin Rush [--icon rush.png] [--config rush.toml] [--dock] -- ./rush`
creates `~/Applications/Rush.app`:

```
Rush.app/Contents/
  Info.plist           CFBundleIdentifier = red.forbes.decline.pin.rush
  MacOS/rush           APFS clone of the decline binary (no extra disk)
  Resources/icon.icns  generated from --icon (via `sips` + `iconutil`)
~/.config/decline/pins/rush.toml   command, cwd, env, any config setting
```

Each pin has its own bundle id, so macOS gives it its own Dock icon, Cmd+Tab
entry and window list. The binary knows it is a pin from its own file name.
Settings live outside the bundle so editing them doesn't break the signature.
The bundle is ad-hoc signed with its own identifier, the quarantine flag is
removed, and it is registered with LaunchServices. The program runs through a
login shell, so it gets the user's `PATH` even when started from the Dock.
`decline pin --update` re-clones the binary after upgrades.

Known limit: an ad-hoc signature is tied to the binary's hash, so after
`--update` macOS asks again for permissions the pin was granted (Documents,
Full Disk Access, …). The fix (next milestone) is Chrome's web-app approach:
a small launcher executable that never changes, loading decline's code from
the installed `Decline.app`.

## Config

`~/.config/decline/config.toml`: font family/size, colours (16 + fg/bg/cursor/
selection), padding, opacity/blur, option-as-meta, scrollback lines, shell.
Read once at start; later: reload on file change (FSEvents, no polling).

## Performance testing

- `term-core` benchmarks (criterion): plain ASCII, coloured output (SGR-heavy),
  cursor-movement-heavy (vim/htop style), Unicode/wide text, scrolling with a
  full scrollback.
- Render benchmark: frame build time for 300×100 grid (CPU side) and GPU time
  via Metal counters.
- End-to-end: wall time of `cat` 100 MB file; idle CPU over 60 s
  (`top -l`/`powermetrics`); resident memory with empty and full scrollback;
  input latency (keypress → present) via `CAMetalLayer` timing.
- Profile with `samply` and Instruments (`xctrace` Time Profiler + Metal
  System Trace). Compare against Ghostty on the same machine.

## Measured (M-series Mac, macOS 27, release build)

| | decline | Ghostty 1.x |
|---|---|---|
| Idle CPU | 0.0% | 0.9–2.7% |
| Idle wakeups/s | 0 | 1–2 |
| Footprint, idle window | 38 MB (13 MB is drawables) | 117 MB |
| `cat` 100 MB coloured text | 1.44–1.53 s | 1.90 s |
| CPU time during that `cat` | 1.6 s | 4.4 s |
| Footprint after it (10k lines scrollback) | 59 MB | 145–177 MB |
| Binary size | 1.1 MB | — |

`term-core` parse throughput (criterion, `cargo bench -p term-core`): plain
122 MB/s, coloured 147 MB/s, TUI redraws 188 MB/s, Unicode 172 MB/s.
Caveats: window sizes, fonts and scrollback limits were not matched exactly
yet; decline does less work (no text shaping, no grapheme clusters, no
reflow). See the performance-testing section for the fair-comparison plan.

## Reviews

Design reviewed by Opus (xhigh effort); code reviewed by Astra (OpenAI) and
tested by Kimi. Fixed from those reviews: PTY write deadlocks, 1 KB read
batching, resize in the alt screen losing shell lines, scrollback view
sliding when full, origin-mode cursor panic, wide characters split by
insert/delete, G1 charset, DECSTR as soft reset, RIS keeping counters,
pending-wrap after erase, query replies (DECRQM, OSC 10/11/12, kitty flags,
XTVERSION, window size), VS16 emoji width, OSC 52/7, kitty keyboard,
keypad mode, paste filtering, environment for bare program names, child
reaping, glyph upload synchronisation, glyph texture caps, 16-bit position
overflow, synchronized-update stall, drawing while hidden, IME text overlap,
pin identity collisions, pin signing and quarantine.

## Next milestone

1. **Scrollback as text + style runs** in 64 KB blocks (WezTerm's approach):
   ~60–100 B/line instead of ~500–1500, room for grapheme clusters and links,
   and blocks ready for LZ4 compression.
2. **Wider live cells** (16 bytes or a style table) for underline styles and
   colours, hyperlinks (OSC 8) and grapheme clusters.
3. **Parse plain-text runs in bulk** instead of one character at a time.
4. **Copy cells under the lock, build GPU instances outside it**, with
   per-row dirty flags so a spinner doesn't rebuild the whole screen.
5. **Box-drawing and Powerline glyphs drawn in code** (seamless borders).
6. **Pin launcher** (see Dock pins).
7. Reflow on resize; Ctrl keys on non-Latin layouts; OSC 8 links.

## Pressure tests

`scripts/pressure/run.py target/release/decline <workdir>` (inputs from
`scripts/pressure/gen.py`). All survived, release and debug (overflow-checked)
builds:

| scenario | time | CPU | peak | after |
|---|---|---|---|---|
| 1 GB coloured flood | 12.5 s | 13.0 s | 59 MB | 55 MB |
| 3M short lines (scrollback churn) | 6.5 s | 5.1 s | 25 MB | 24 MB |
| 64 MB escape-code fuzz | 1.5 s | 1.6 s | 65 MB | 72 MB |
| 30k unique glyphs (texture growth/eviction) | – | 0.2 s | – | 38 MB |
| one 50 MB line | 2.6 s | 0.8 s | 56 MB | 54 MB |
| synchronized-update flood, 15 s | 15 s | 1.4 s | 58 MB | 55 MB |
| htop, 20 s | 20 s | 0.17 s | 43 MB | 38 MB |
| blinking cursor idle, 20 s | 20 s | 0.17 s | 41 MB | 36 MB |
| 10 windows idle (separate processes) | – | 0% | – | 286 MB total |
| 56 resizes during 1 GB output; 60 in htop | – | – | – | survived |
