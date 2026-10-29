# Test plan (draft for review)

Today: 36 tests (core 22 + scrollback 3, app 9, font 1, pty 1). Kimi is
separately writing `crates/term-core/tests/conformance.rs` (xterm behaviour
per escape sequence) and `fuzz.rs`; this plan covers everything else and
fits around that.

## Principles

- **Test behaviour through public APIs**, not private fields. Where logic is
  buried in AppKit code (`view.rs`), first move it into pure functions in
  their own module and test those; AppKit glue stays thin and untested.
- **Table-driven** tests for encodings (keys, mouse, replies): one row per
  case, so gaps are visible.
- **Property-based tests** (`proptest`, dev-dependency only) for round trips
  and invariants: scrollback encode/decode, terminal invariants under random
  input and resizes, frame building never panics and stays in bounds.
  Fixed seeds in CI; failures print a minimal case.
- **Screen snapshots as plain text**: a test helper renders the visible grid
  to a string (with a compact style marker syntax) and compares to an
  inline expected string. No snapshot-file framework; diffs read in review.
- **No sleeps in tests.** PTY tests wait on read results with a timeout
  helper (poll with deadline), never fixed `sleep`.
- **Hermetic**: config/pin tests use temp dirs and pass paths in; nothing
  touches `~/.config` or `~/Applications`, LaunchServices or the Dock.
- **Platform split**: `term-core` and pure logic run on Linux CI too;
  Core Text / PTY / Metal-free macOS tests run on macOS CI. GPU (Metal) is
  not unit-tested; covered by the pressure harness and screenshots.
- **Regression tests** for every bug the reviews found (most already exist
  in `tests/term.rs`).
- Each test file has a short header saying what it covers.

## term-core

1. `cell.rs` unit tests: Color pack/unpack for all three kinds (incl. index 0
   and 255, rgb extremes), flag bits don't overlap the char bits, `ch()` of
   0 and of invalid scalars, `is_blank`, `Row::compact` keeps a trailing wide
   char with its spacer.
2. `scrollback.rs`:
   - property: any row of random cells (any codepoints incl. wide, any
     colours/flags, wrapped or not) round-trips exactly, except trailing
     blanks dropped and codepoint-0 ↔ space equivalence.
   - limit: `len() == min(pushed, limit)` for random push counts and limits
     (incl. 0 and 1); `get(i)` returns the right line after block freeing.
   - memory: freeing keeps at most one extra block; `clear()` frees all.
   - varint edge values (0, 127, 128, 16383, 16384, u32::MAX).
3. `term.rs` (beyond Kimi's conformance file):
   - invariants property test: after any random byte stream interleaved with
     random resizes (1..=300 × 1..=100), scrolling the view and selection:
     cursor in bounds, every screen row has exactly `cols` cells, every
     WIDE has a following SPACER and vice versa, `display_offset <=
     scrollback_len`, `visible_row` works for every line, no panic.
   - selection: across scrollback/screen boundary, reversed anchor/head,
     selection survives output scrolling it into history (absolute lines),
     cleared on resize, wide chars copied once, trailing spaces trimmed except
     on wrapped lines.
   - view scrolling: clamps at both ends, no-op on alt screen, stays put
     while output arrives (already), resets on input (app-level).
   - replies queue: multiple queries in one chunk keep order; RIS keeps
     pending replies.
4. Benchmarks stay as they are (not tests).

## term-render (`frame.rs`, no Metal)

Build frames from a `Term` with a fake `GlyphSource` and assert on instances:
- background runs: adjacent same-colour cells merge into one rect; default
  background emits nothing; run broken by selection, inverse, cursor.
- inverse swaps fg/bg; dim lowers alpha; hidden emits no glyph; spaces and
  spacers emit no glyph; wide glyph emitted once.
- underline / double underline / strike rect positions from metrics.
- cursor: block (cell colours swapped), bar, underline, unfocused hollow box
  (4 rects), hidden when `cursor_visible` off, blink phase off, scrolled out
  of view; cursor on a wide char covers two cells.
- preedit: glyphs under it suppressed, drawn at cursor, clipped at right edge.
- padding offsets every instance; `s16` clamps instead of wrapping on huge
  grids (property: positions monotonic in col/line within clamp).
- theme: 256-colour cube and grey ramp values match xterm's; `rgb()` byte
  order; truecolour cells resolve exactly.
- glyph cache logic (GlyphSource impl) is Metal-bound → not unit-tested.

## term-font (macOS only)

- metrics sane for Menlo/SF Mono at several sizes; cell height ≥ ascent +
  descent; baseline inside the cell.
- regular/bold/italic return different bitmaps for 'A'.
- fallback: CJK, emoji (colour), symbols; missing glyph (U+10FFFD) → None.
- bitmap bounds: `width*height*bpp == data.len()`.
- unknown family name falls back without panicking.

## term-pty (macOS/Linux)

- spawns login shell when argv empty (`$0` starts with '-').
- bare program name resolves through the login shell PATH; env merge: our
  TERM/COLORTERM override inherited, caller env overrides ours, LANG default.
- cwd honoured; nonexistent cwd falls back to HOME.
- resize changes `stty size` seen by the child.
- `read_batch` drains more than one 1 KB read when available; returns 0 after
  exit (EOF/EIO).
- `Writer::write_all` of 1 MB to a child that reads slowly completes (no
  WouldBlock error leak).
- dropping `Pty` makes a SIGHUP-ignoring child die within the grace period
  (reaper SIGKILLs).
- `foreground_cwd` reports the child's cwd.

## term-app

Refactor first (pure modules, no behaviour change):
- `mouse.rs`: SGR / X10 encoding, button+modifier codes, motion/drag rules,
  wheel codes, word boundaries for double-click, line for triple-click.
- `layout.rs`: padding, `grid_size` (incl. caps), `content_size`, cell at a
  point (incl. clamping in padding).
- key handling: lift the decision logic of `key_bytes` into
  `keys::translate(input: KeyInput) -> Option<Vec<u8>>` where `KeyInput`
  holds keycode, chars, chars-ignoring-modifiers, modifier flags incl.
  left/right option bits, numpad flag, and the terminal modes.

Tests:
- `keys`: full table for every special key × modifier combination ×
  app-cursor mode; F1–F12; keypad in app mode; Ctrl table for all ASCII;
  Option-as-Meta left/right/both/none with left/right option events; kitty
  flag 1 (Enter/Tab/Backspace/Esc with and without mods, Ctrl+letter,
  Ctrl+Shift); Cmd+←/→/⌫; paste filtering (C0/C1 controls, CRLF, bracket
  injection).
- `mouse`: table for press/release/drag/motion/wheel in SGR and X10,
  coordinates at 1 and at the X10 limit (223), modifiers.
- `layout`: round trip `content_size` ↔ `grid_size` gives the configured
  cols/lines at scales 1, 2, 3; titlebar padding; tiny windows clamp to 2×1.
- `config`: defaults load with no file; layer merge (nested tables);
  unknown key → error message and defaults; bad hex colours fall back per
  field; opacity clamp; `option_as_meta` parsing; theme conversion.
- `pin` (with an injected base dir, no LaunchServices/codesign — those
  shell-outs behind a trait or skipped via a flag in tests): argument
  parsing (missing name, missing `--`, unknown flag), slug rules and
  collision detection, pin config content (command absolute for relative
  paths, cwd defaulting, title, pin_app), Info.plist escaping and keys,
  `own_pin_config` detection from an exe path inside/outside a bundle,
  list/remove/update over a temp dir.

## CI

- `cargo test --workspace` on macOS; `cargo test -p term-core` on Linux.
- proptest case count: default locally, `PROPTEST_CASES=2000` in a
  scheduled nightly job.
- Coverage report (`cargo llvm-cov`) as a non-blocking job, to find gaps.

## Integration tests (end to end)

Two levels, both in CI on macOS:

1. **Headless pipeline** (`crates/term-app/tests/` or a small `term-harness`
   crate): real `Pty` + reader loop + `Term`, no window. Runs real programs
   and asserts on the resulting screen text and replies:
   - shells: `zsh -f`, `bash --norc`, `sh` — prompt, line editing (send
     keys via the same `keys::translate` the app uses), Ctrl+C, exit codes.
   - full-screen programs: `vim -u NONE` (open, insert, `:q!`), `less`
     (scroll with arrows and wheel bytes), `top -l` / `htop` if present,
     `python3 -c` curses script (alt screen, colours, resize via SIGWINCH).
   - queries round-trip through the PTY: a program sends DA/CPR/DECRQM/OSC
     11 and reads our reply with a timeout.
   - bracketed paste seen by `zsh` (`bindkey` shows the paste bytes).
   - resize while a program runs: `stty size` and curses `LINES/COLS`.
   - each waits on screen content with a deadline (`wait_for(|screen| …)`),
     never fixed sleeps.
2. **Real app smoke tests** (`scripts/smoke/` driven from a test):
   launch the built `terminal-decline` with `DECLINE_DEBUG` and a test-only
   `DECLINE_TEST_DUMP=<file>` env var: on program exit the app writes the
   visible screen text, frame count, glyph-texture stats and its footprint to
   the file before quitting. Scenarios: `-e` program output, a pin bundle
   built into a temp dir and launched with `open --env`, window resize via
   System Events (skipped when accessibility isn't granted), font size
   change via the menu action. Asserts on the dump, not on pixels; one
   screenshot per scenario is uploaded as a CI artifact for eyeballing.

## Periodic workflows

- **Benchmarks** (`.github/workflows/bench.yml`, nightly + manual):
  `cargo bench -p term-core -p term-render` with criterion's
  `--save-baseline`; compare against the last main baseline (cached) with
  `critcmp`, fail/annotate on > 10% regression; also the scrollback
  bytes-per-line example and the pressure harness's flood/churn numbers,
  posted as a job summary table and kept as artifacts for trend history.
- **Fuzzing** (`.github/workflows/fuzz.yml`, nightly, nightly toolchain):
  `cargo-fuzz` targets in `fuzz/`:
  - `parse`: arbitrary bytes into `Term` with interleaved resizes (encoded
    in the input), checking the invariants after each step.
  - `scrollback`: arbitrary rows encode/decode round trip.
  - `keys`/`paste`: arbitrary key events and paste text never produce bytes
    that end a bracketed paste early.
  - `config`: arbitrary TOML never panics the loader.
  Each target runs N minutes; corpus cached between runs; crashes uploaded
  as artifacts and open an issue. Found crashes become regression tests.
- **Pressure** (weekly): `scripts/pressure/run.py` on macOS runner; results
  as job summary; fail if any scenario doesn't survive.

## Decisions after review (Opus, xhigh)

Adopted all "must" items and most "should" items. Key changes:

- **Structure**: `terminal-decline` becomes a library + thin `main.rs`. PTY start-up
  and the reader/writer threads move out of `view.rs` into an AppKit-free
  `session` module (takes a `wake` callback); the headless pipeline tests use
  that same code. The drawing decision (`dirty, visible, synchronized,
  elapsed, idle ticks → draw/skip/pause/hold`) becomes a pure function.
  `keys::translate` returns bytes / to-text-input / to-menu.
- **term-core** gets one `check_invariants` (also run under
  `debug_assertions` at the end of `Term::advance`) and one screen-dump
  format, shared by property tests, fuzzing, conformance and pipeline tests.
- **Bugs found while reviewing, fixed with regression tests**: VS16 widening
  breaks a neighbouring wide char; selection points at the wrong text with
  scrollback 0 or on the alt screen; glyph texture rebuild loop for glyphs
  bigger than the texture; missing `cwd` not falling back to HOME; pin
  `--remove` trusting a path from an editable file; pin names with `/`/`..`;
  Dock tile URL not percent-encoded; `Row::compact` deleted (unused).
- **Injection, not env vars**: config dir, apps dir and exe path are passed
  in; codesign/lsregister/defaults go through an injected command runner
  (one real codesign test on a temp bundle).
- **PTY tests**: `Command.shell` option, `/bin/sh` in tests, pure env/argv
  builders, every wait through `recv_timeout`, deterministic resize / reaper
  / 1 MB write cases.
- **GPU-free render checks**: const size/offset asserts matching the shader
  structs, pure glyph-texture packing logic, fake glyph source property
  (text from instances == grid text), offscreen Metal render + pixel read on
  macOS CI (fail, not skip, when CI has no Metal device); build.rs fails on
  CI if shaders can't be precompiled.
- **Reader/main-thread tests**: no lost wake-up, shutdown frees the terminal
  within 3 s, wakes ≤ draws + 1 under flood, replies intact while typing,
  resize under flood.
- **Snapshots**: insta inline snapshots for screen dumps only (no files,
  `INSTA_UPDATE=no` in CI); tables + `assert_eq` for encodings.
- **Smoke tests**: actions triggered by a private OSC honoured only with
  `DECLINE_TEST_DUMP`; the dump includes GPU-completed frame count and
  instance/texture stats; launched with `open -n -W`, unique bundle ids,
  `lsregister -u` after; opt-in locally (`#[ignore]`), required on CI.
- **CI**: `.config/nextest.toml` with slow-timeout/terminate, a serial `gui`
  group, no retries; `timeout-minutes` on every job; PR budget ≤ ~10 min.
- **Benchmarks**: gate only on stable numbers (instruction counts on Linux,
  bytes per scrollback line); wall-clock numbers are trend/annotation only,
  alert after two consecutive regressions.
- **Fuzzing**: one cargo-fuzz target (`parse`, structured with `arbitrary`)
  on ubuntu nightly; seed corpus + dictionary committed; corpus cached;
  crashes minimised, deduplicated, issue opened; corpus replayed as a
  stable test on every PR. Other proposed targets became property tests.
- **Dropped**: SF Mono / missing-glyph-None font cases (wrong), bitmap
  length tautology, `top -l`/htop in the pipeline, timing-based PTY batch
  case, hard 10% wall-clock benchmark gate.
