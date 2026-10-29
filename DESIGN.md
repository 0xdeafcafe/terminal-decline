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
| `term-core`   | escape-code handling (via vendored `vte`), screen grid, scrollback, selection |
| `term-pty`    | spawns the shell on a pseudo-terminal, reader thread, resize          |
| `term-font`   | Core Text: font loading, cell metrics, glyph rasterising              |
| `term-render` | Metal: glyph texture, per-cell instance buffer, one instanced draw    |
| `term-app`    | AppKit window/view, keyboard + IME, mouse, clipboard, config, pins    |

`term-core` has no platform code and is tested and benchmarked on its own.

`term-app` (the `decline` package) is a library plus a thin `main.rs`. The
AppKit code (`app`, `view`) only turns events into the inputs of plain
modules, each tested on its own: `session` (the program, its PTY, the
reader and writer threads, the shared terminal and the dirty flag),
`keys::translate` (a key press → bytes, input method or menu), `mouse`
(reporting encodings), `layout` (padding, grid size, cell
under a point), `frame_policy::decide` (draw, skip, pause, hold or hide on
each display tick), `config` and `pin`. Paths (config dir, apps dir, the
executable) are passed in from `main`, and pins run other programs through
an injected runner, so tests use temp dirs. `tests/pipeline.rs` runs real
programs through `session` with no window; `tests/smoke.rs` drives the real
app through a test-only control sequence (`ESC ] 7770 ; cmd BEL`, honoured
only with `DECLINE_TEST_DUMP=<file>` set) that writes JSON dumps.

## Screen storage

- A cell is 12 bytes: `u32` with the 21-bit Unicode scalar plus 11 style flag
  bits (bold, italic, underline, wide, …), and one `u32` each for foreground
  and background (default / palette index / 24-bit RGB, tagged in the top byte).
- The visible screen is a ring of rows (`VecDeque<Row>`), so scrolling the
  whole screen moves the top row to the bottom without shifting the others;
  scroll regions rotate just their rows. A row is `Vec<Cell>` plus a
  soft-wrap flag, and is cleared with a memset when the fill is the default
  blank (all zero).
- Scrollback (`term-core/src/scrollback.rs`) stores each line as UTF-8 text
  plus style runs (only non-default colours/styles are written), packed into
  ~64 KB blocks that are freed whole from the oldest end. Lines are expanded
  back into cells only when shown or copied. Measured per line: plain 52 B,
  `ls` colours 93 B, compiler errors 81 B, full-width background 183 B, CJK
  90 B (was 518 / 674 / – / ~1500 / – with full cell rows): 10k lines take
  0.5–1.8 MB. Encoding finds a line's end by skipping all-zero cells 8 at a
  time, and lines of plain default-style ASCII are a byte copy.
- term-render and term-app read cells only through `RowRef` and `CellRef`
  (content: empty, a character, cluster text or an image placeholder; width;
  and a `&Style` with colours, attributes, underline style and link), and
  the terminal only through `Terminal`'s methods: its fields are private.
  How cells are stored is term-core's own business
  (`docs/design/cells.md`).
- Points in the buffer use an ever-increasing line number so a selection stays
  attached to its text while output scrolls. The app and the renderer do no
  line arithmetic of their own: term-core gives them `point_at` (view row
  and column to a Point), `cursor_in_view`, `view_top`/`set_view_top`, the
  selection gestures (`select_word`, `select_line`, `select_all`,
  `extend_selection`) and the word rule (`term_core::word`).
- Parsing: `vte` is vendored (`vendor/vte`, through `[patch.crates-io]`)
  with `Perform::print_ascii`, which hands over each run of printable ASCII
  whole; the end of a run is found 8 bytes at a time. `Terminal::write_ascii`
  writes a run one row segment at a time (pen read once, wrap handled only at
  segment ends). Segments over part of a wide character, insert mode, DEC line
  drawing and autowrap off go one character at a time. A property test checks
  the bulk path against one character at a time, fed one byte per call.
- Widths come from term-core's own Unicode table (`src/unicode`, generated
  from the Unicode 18.0.0 files; equal to `unicode-width` per scalar but
  for U+17D8). The terminal segments printed text into grapheme clusters
  (UAX #29) as it arrives, including after a bulk ASCII run.
- Known gap for the first milestone: combining marks / grapheme clusters are
  still dropped from the cells (needs a side table for multi-codepoint cells;
  cells design PR 6). Resize does not re-wrap lines yet.

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

## Failures and crashes

An app started from the Dock or as a pin has no stderr, so failures are
written into the window as dim red text (and to stderr):

- Config errors appear at the top of each window, before the program's
  output: `decline: config: font_sise: unknown field …; ignored`.
- A program that can't be started leaves its error and "press any key to
  close". One that exits within 1 s with a non-zero status (a broken pin,
  `decline -e /nonexistent`) leaves `[process exited with status N]`. In
  both cases the window stays open until a key is pressed (Cmd shortcuts
  still work, so the message can be copied). A program that exits with
  status 0, or after the first second, closes its window as before. The
  exit status comes from term-pty's `Child`, which reaps and signals under
  one lock, so no signal can reach a process that reused the pid.

Release builds unwind (`panic = "unwind"`). A panic hook writes the
message, a backtrace and the version to
`~/Library/Logs/decline/crash-<time>.log` and logs a fault to os_log
(subsystem `red.forbes.decline`, a few lines of FFI to `_os_log_impl`). A
panic while a reader thread parses output is caught inside the terminal's
lock, so the lock is never poisoned: that session's terminal is replaced by
a fresh one saying "decline hit a bug and stopped this session; details in
…", its program is killed, and other windows keep running. A panic on the
main thread (AppKit) aborts once it has been logged. Each release keeps
its `decline.dSYM` (`split-debuginfo = "packed"`) as a release download,
for symbolicating crash logs. Unwinding costs 157 KB of binary (unwind
tables and landing pads: 1.34 → 1.50 MB) and no measurable parse speed.

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
  rasterised once by Core Text; ASCII is pre-rasterised at start.
- Box drawing (U+2500–257F), block elements (U+2580–259F), braille
  (U+2800–28FF), Powerline (U+E0A0–E0A3, U+E0B0–E0BF) and the legacy
  computing sextants, wedges and eighth blocks (U+1FB00–1FB8B) are drawn
  in code (`term_font::sprite`), not taken from the font: bitmaps exactly
  the cell's size, straight lines in whole pixels centred in the cell (light
  = the underline thickness, heavy twice that, double = two light strokes a
  light thickness apart), so neighbours join with no seam at any size.
  Rounded corners, diagonals, Powerline triangles and wedges are
  anti-aliased; shades (░▒▓) are even 25/50/75 % coverage, since a pixel
  pattern can't repeat across odd-width cells. Each shape has a `SpriteId`
  for the glyph keys of the cell design (docs/design/cells.md §10.4). Uploading a
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
- Mouse: drag to select (word on double-click, line on triple-click; words
  and lines are logical, so a word split by a soft wrap is selected whole,
  and dragging after a double- or triple-click grows by words or lines), wheel
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
A missing or nonexistent `cwd` starts in `HOME` (the app's own working
directory is `/` when started from the Dock).
`decline pin --update` re-clones the binary after upgrades.
`decline pin --remove` deletes a bundle only if it is a `.app` inside the
apps dir containing `Contents/MacOS/<id>`, since the settings file naming it
can be edited. Pin names can't contain `/` or `..`.

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
  cursor-movement-heavy (vim/htop style), `seq`-style short lines, Unicode/wide
  text, all with a full scrollback; each fed in 64 KB pieces (`parse`) and in
  1 KB pieces (`parse_1k`, what a macOS PTY returns per read).
- Render benchmark: frame build time for 300×100 grid (CPU side) and GPU time
  via Metal counters.
- End-to-end: wall time of `cat` 100 MB file; idle CPU over 60 s
  (`top -l`/`powermetrics`); resident memory with empty and full scrollback;
  input latency (keypress → on screen), see "Input latency" below.
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
| Binary size | 1.5 MB (unwinding; 1.3 MB aborting) | — |
| Parse, plain text (1 KB pieces) | 423 MB/s | — |
| Parse, coloured / TUI / `seq` / Unicode | 172 / 289 / 110 / 131 MB/s | — |

`term-core` parse throughput (criterion, `cargo bench -p term-core --bench
parse`, M1 Max, MB/s; 64 KB pieces, then 1 KB pieces). Before the bulk ASCII
work, then after it:

| input | before | after |
|---|---|---|
| plain text | 113 / 110 | 431 / 423 (3.8×) |
| coloured (`ls`, compiler errors) | 128 / 125 | 172 / 172 (1.4×) |
| TUI redraws | 180 / 175 | 290 / 289 (1.6×) |
| `seq` | 39 / 39 | 111 / 110 (2.8×) |
| Unicode / wide | 121 / 124 | 132 / 131 |

By step (1 KB pieces, plain / coloured / TUI / `seq`): bulk ASCII runs
201 / 164 / 290 / 43; faster scrollback encoding 328 / 167 / 280 / 74; 8-byte
run scan 355 / 164 / 286 / 73; screen as a ring 419 / 171 / 285 / 102;
memset row reset 423 / 172 / 289 / 110. Plain text is now above the PTY's
~240 MB/s but short of the 2× goal.
Caveats: window sizes, fonts and scrollback limits were not matched exactly
yet; decline does less work (no text shaping, no grapheme clusters, no
reflow). See the performance-testing section for the fair-comparison plan.

### Input latency (keypress → on screen)

`scripts/latency/run.py` (roadmap N.5). Two methods:

- **In decline.** With `DECLINE_LATENCY_LOG=<file>` (test and bench runs
  only; unset, it costs one check per key, parse and frame) decline writes
  a JSON line per key: the event's timestamp (t0), `keyDown`, the bytes
  handed to the writer thread (t1), the next parse in that window, taken
  with the terminal locked (t2), the frame that shows it (start, commit,
  GPU done) and the drawable's `presentedTime` (t3), all on one clock
  (`CLOCK_UPTIME_RAW`, which `NSEvent`, `CACurrentMediaTime` and Metal use).
  The driver starts decline as a throwaway app bundle through `open` (an
  app started from a script may not be given the focus), running
  `scripts/latency/echo.py` (raw mode, echoes each key at once) or
  `htop -d 1` (typing into its search bar). Keys: decline posts key events
  to its own process with `CGEventPostToPid` when asked through
  `DECLINE_LATENCY_INPUT` (a FIFO). That needs no permission (posting to
  another process needs Accessibility) and still goes through the window
  server. 200 keys per run, 50–150 ms apart at random, after 6 warm-up keys.
  Scenarios: idle; flood (a second window of the same process running
  `yes`); htop in the same window; the display link fixed at 120 or 60 Hz
  (`DECLINE_LATENCY_HZ`).
- **From outside, for Ghostty and decline alike.** `scripts/latency/probe.swift`
  posts the key to the app with `CGEventPostToPid` and watches its window
  with ScreenCaptureKit at every refresh; the latency is the display time
  of the first frame whose pixels changed minus the time just before
  posting. Needs Accessibility and Screen Recording for the terminal
  running it. Error bars: one refresh (8.3 ms at 120 Hz) per sample. On the
  same decline frames, ScreenCaptureKit's display time is 5–13 ms (median
  11 ms) earlier than Metal's `presentedTime`, so add about 11 ms for the
  time to the glass; the comparison between terminals is like for like.
  Ghostty is started with decline's defaults (Menlo 13, 100×30, padding 6,
  no cursor blink, no config file).

M1 Max, built-in ProMotion display, macOS 27, release build; the Mac was
in use (other apps and agents running) during the runs. In decline, ms,
p50 / p95 / p99, 200 keys per run:

| scenario | key event → on glass | event → bytes to writer | writer → echo parsed | parsed → frame starts | frame build + encode | commit → on glass |
|---|---|---|---|---|---|---|
| idle, 120 Hz | 36.5 / 44.6 / 49.8 | 7.1 / 13.1 / 15.8 | 0.3 / 3.1 / 7.5 | 4.4 / 7.1 / 8.3 | 1.5 / 3.3 / 9.6 | 23.1 / 23.3 / 23.3 |
| idle, 60 Hz | 35.8 / 45.1 / 49.6 | 5.3 / 9.9 / 13.1 | 0.2 / 1.4 / 3.2 | 4.7 / 13.3 / 16.0 | 0.9 / 4.9 / 6.2 | 23.8 / 24.5 / 24.6 |
| flood, 120 Hz | 37.7 / 45.9 / 46.3 | 8.3 / 9.4 / 12.5 | 0.1 / 0.5 / 1.2 | 0.2 / 5.0 / 5.3 | 5.2 / 8.2 / 8.4 | 23.9 / 24.0 / 24.0 |
| flood, 60 Hz | 34.3 / 41.6 / 43.3 | 2.0 / 9.9 / 12.8 | 0.1 / 0.2 / 0.8 | 4.3 / 8.0 / 16.0 | 0.7 / 8.7 / 9.6 | 24.0 / 31.3 / 31.5 |
| htop, 120 Hz | 37.2 / 48.7 / 61.6 | 3.7 / 9.3 / 12.4 | 0.9 / 3.9 / 6.0 | 5.4 / 8.9 / 11.6 | 0.7 / 8.8 / 17.1 | 23.8 / 32.1 / 32.2 |
| htop, 60 Hz | 36.4 / 48.9 / 51.7 | 3.2 / 9.7 / 11.7 | 0.9 / 3.9 / 6.9 | 5.6 / 14.9 / 16.7 | 0.5 / 5.2 / 9.8 | 23.9 / 32.1 / 32.3 |

The GPU itself takes 0.3–0.7 ms (p50). These runs were made before t1 was
logged ahead of the hand-off to the writer: a key whose echo was parsed
before it was logged went unrecorded (46 of 200 at idle 60 Hz, at most 3
elsewhere) rather than mistimed. In htop, t2 is the first output after the
key, which can be htop's own refresh rather than its answer.

From outside (key posted → pixels changed, ms; 29 and 20 keys, too few for
a p99):

| scenario | decline p50 / p95 / max | Ghostty 1.4.0 p50 / p95 / max |
|---|---|---|
| idle | 29.7 / 43.1 / 45.4 | 23.4 / 37.0 / 40.5 |
| flood (second window running `yes`) | 19.5 / 28.5 / 28.5 | 9.8 / 48.3 / 48.3 |

Screen Recording and Accessibility were granted only briefly, so the full
outside comparison (200 keys, htop) is still to run:
`scripts/latency/run.py --rates "" --external decline,ghostty`.

What this shows: decline is 6–10 ms slower than Ghostty at the median, so
the "p99 no worse than Ghostty" threshold is not met yet. Where decline's
time goes:
- About 23 ms from commit to `presentedTime`, nearly constant at either
  display link rate: a frame drawn on a display link tick reaches the
  glass about three 120 Hz refreshes later. The largest share, and the
  first thing to try to shorten (R.6 `CAMetalDisplayLink`, presenting with
  the Core Animation transaction, or setting the layer's contents directly).
- The key's own redraw: typing marks the screen dirty and, with the display
  link paused, draws at once, before the echo has arrived; the echo then
  waits for the next tick (4–5 ms p50, up to a full tick at 60 Hz).
- 2–8 ms (p50) from the event's timestamp to the bytes reaching the writer:
  window-server delivery plus the text input system (`interpretKeyEvents`).
Each change should be kept only if this harness shows lower numbers.

CI: hosted macOS runners have no real display, so presented times mean
nothing there. CI can track the part that is decline's own work, from the
key's bytes to the encoded frame (t1 → commit): a headless test sending a
key through `session` to `echo.py` and timing until the reader has parsed
the echo, then `Renderer::prepare` and `encode` into an offscreen texture
(as term-render's offscreen test does), reported like the other benches.
Not built yet.

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

1. ~~Scrollback as text + style runs~~ (done). Next: compress sealed blocks.
2. **Wider live cells** (16 bytes or a style table) for underline styles and
   colours, hyperlinks (OSC 8) and grapheme clusters.
3. ~~Parse plain-text runs in bulk~~ (done). Next: a CSI/SGR fast path.
4. **Copy cells under the lock, build GPU instances outside it**, with
   per-row dirty flags so a spinner doesn't rebuild the whole screen.
5. ~~Box-drawing and Powerline glyphs drawn in code~~ (done).
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
