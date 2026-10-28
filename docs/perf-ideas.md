# Memory and CPU ideas (for review before building)

Baseline, idle window 812×509 pt at 2× (release build, phys_footprint 39 MB):
IOSurface 13 MB (two drawables, 1624×1018 BGRA8 each), Malloc Small 12 MB
(7.5 MB live heap, 5.7 MB free-but-held, 44% fragmentation), IOAccelerator
2.2 MB (glyph textures + instance buffers), __DATA_DIRTY 2 MB (frameworks).
Busy numbers: 1 GB flood → 13 s CPU, 51 MB peak, 40 MB after. Parse:
plain 108 MB/s, coloured 125, TUI 176, Unicode 117. `cat` 100 MB e2e 1.4 s.

## A. When the window can't be seen (minimised, covered, other Space, app hidden)

A1. **Give back the drawables.** While occluded, shrink the layer's
    `drawableSize` to 1×1 (or detach the layer's contents) so the two
    IOSurfaces are freed: −13 MB per hidden window (most of a pin's cost
    when it sits in the background). Restore and redraw on becoming visible.
    Risk: a visible flash or one-frame delay when shown; Mission Control /
    Exposé thumbnails might show empty — needs checking.
A2. **Return free heap pages** with `malloc_zone_pressure_relief(NULL, 0)`
    when hidden and after an output burst ends: aims at the 5.7 MB of free
    Malloc Small. Cheap one call.
A3. **Drop rebuildable caches when hidden for > 30 s**: glyph textures
    (1–16 MB, re-rasterised on show), instance buffers (up to ~2 MB),
    frame vectors, the 1 MB reader buffer (shrink to 64 KB when idle).
A4. **Compress sealed scrollback blocks** (LZ4 via `lz4_flex`, pure Rust)
    when hidden, or always once a block is sealed; decompress one block on
    demand with a one-block cache. Text compresses ~3–5×: 10k lines go from
    0.5–1.8 MB to ~0.15–0.5 MB; matters for 100k-line scrollback.
A5. **Don't parse at full speed for a hidden window?** No — output must
    still be processed (the program blocks otherwise), but drawing is
    already skipped. Listed to reject.

## B. Drawing less

B1. **Per-row dirty tracking**: the core stamps each row with the
    generation it last changed; the renderer keeps per-row instance ranges
    and rebuilds only changed rows (plus cursor/selection rows). A spinner
    or clock then rebuilds 1 row, not 50. Upload only changed ranges.
B2. **Frame-rate heuristics**: when not key window, cap the display link at
    30 fps; during floods (> N MB/s for > 200 ms) cap at 30–60 fps — no one
    reads text scrolling at 120 Hz. ProMotion: ask for 120 Hz only right
    after a key press (typing latency), otherwise 60.
B3. **Skip blank cells fast** in frame building: compare the whole 12-byte
    cell against `Cell::BLANK` (one 96-bit compare) before any colour/flag
    work; most of a screen is blank.
B4. **Background as a texture, not rectangles**: upload a cols×rows RGBA8
    "background colour" texture and draw backgrounds in one full-screen
    triangle sampling it with nearest filtering (Ghostty's approach). One
    4-byte write per cell instead of rectangle runs; GPU work constant.
B5. **Damage-only present**: with dirty rows known, keep the previous frame
    (load action Load instead of Clear, needs `framebufferOnly` + 3
    drawables or a persistent offscreen texture + blit) and redraw only
    changed rows. Saves GPU, costs a persistent texture (+6.6 MB) — likely
    a net loss for memory; listed to reject unless reviewers disagree.

## C. Parsing faster (CPU under output)

C1. **Bulk ASCII runs**: vendor `vte` (small crate) to add
    `Perform::print_str(&str)` called with each run of printable text in the
    ground state (vte already finds runs with `memchr` for ESC). Write ASCII
    runs straight into the row: one bounds check per row segment, pen style
    computed once, `last_char` once, wrap only at row ends. Expect 2–4×
    on plain/coloured output.
C2. **SWAR/SIMD classify**: within a run, find the first non-ASCII or
    control byte 8/16 bytes at a time (`u64` tricks: `(x - 0x2020…) &
    !x & 0x8080…`) so ASCII-only stretches skip per-char width lookups.
C3. **Width lookup table**: 64 KB bitmap of "is wide" for the BMP (two bits
    per code point: 0/1/2 width) instead of `unicode-width`'s tries; or a
    tiny direct-mapped cache of the last 256 code points' widths.
C4. **Cheaper scroll**: the screen as a ring buffer (`VecDeque<Row>` with a
    rotating top index) so scrolling the full screen is O(1) instead of a
    `rotate_left` memmove of 50 row headers; and reset the reused row with
    a `memset`-able blank (when the pen bg is default the blank is all
    zeros, so `fill` becomes `write_bytes(0)`).
C5. **Encode scrollback faster**: compare cells as `u128`/(u64,u32) to
    detect run changes; ASCII rows are already a byte copy loop — use
    `iter().map(|c| c.packed as u8)` on an ASCII-only fast check.
C6. **Lock less**: publish modes needed by key handling (app cursor, keypad,
    kitty flags, bracketed paste, mouse mode) as atomics so typing never
    takes the terminal lock during a flood.

## D. Heuristics around the program's behaviour

D1. **Burst detection**: if a program streams > 1 MB without pausing,
    parse but only render every Nth vsync, and stop doing work for lines
    that scrolled through without ever being on screen at frame time
    (they still go to scrollback).
D2. **Skip-through for floods**: when more than a full screen of output
    arrives between two frames, the cells of rows that will scroll off
    before the next frame could be written straight to scrollback without
    going through the screen grid. Complex; maybe later.
D3. **Idle shrink**: after 60 s with no output and no input, shrink
    oversized vectors (instance buffers, frame vecs, reader buffer) to fit.

## E. Many windows / pins

E1. Pins are separate processes (each has its own bundle id), so each pays
    ~20 MB of framework baseline. Accept; A1–A3 shrink the rest when the
    pin is in the background.
E2. Multiple windows of the main app share one process (Cmd+N) — already.
