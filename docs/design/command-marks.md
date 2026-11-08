# Command marks: past commands coloured, commands marked off, output coloured

Status: **landed** (1 Nov 2026). Approved by Alex.

## What it looks like

```
┃ $ ls -l "a b"                                    ✓    (green bar)
┃ a b
┃ $ cargo test                         ✗ 101 · 14s    (red bar)
┃    Compiling …
┃    error: test failed
┃ $ sleep 70                                 ✓ 1m10s  (green bar)
┊ $ cargo build                                       (dim: running)
┊    Compiling …
┊ █
```

The bars are in the left padding, not a cell column; a point's gap
between runs back to back.

1. **Past commands keep their colours.** Every command line already run
   that is in view (main screen only) is coloured as it was while typed:
   flags, strings, variables, operators by the same rules as the line
   being typed (`lex`, `color_of`), commands green or red by whether
   they exist. Follows the `highlight_command_line` setting.
2. **Each command is marked off** (setting `command_marks`, default on):
   - a thin bar down the left edge of each command run, from its
     prompt's first row (its A mark) to the last row of its output, in
     the green or red of the mark below when it exited 0 or not, the
     foreground faded while it runs, before it's run (the prompt being
     typed) or when the shell gave no exit status. It replaced a faint
     line across the pane above each prompt (Alex, 8 Nov 2026), which
     marked commands off but didn't say which rows were whose or how
     they ended;
   - at the right of the command's line (the row holding its B mark), a
     dim green `✓` when it exited 0, a dim red `✗ 2` (its exit status)
     otherwise, followed by the time it took from its C mark to its D
     mark when that was 1 s or more: `1.2s`, `14s`, `2m03s`, `1h02m`.
3. **Plain output coloured** (setting `colour_output`, default off):
   commit hashes yellow, URLs blue, `->` and `=>` magenta, numbers with
   units or `%` cyan (bare numbers aren't: an `ls -l` listing's sizes,
   counts, dates and times read as random highlights when coloured), `error` `fatal` `failed` `panic` `denied` red,
   `warning` `warn` `deprecated` yellow, `ok` `done` `passed` `success`
   green.
4. **Why a command failed** (setting `mark_failure_cause`, default off):
   for each command in view that exited non-zero, its output's rows are
   read by [linetag](https://github.com/0xdeafcafe/linetag)'s small
   hand-weighted net, and a dim red `← cause` goes at the right of the
   line (at most two) it reads as the cause, where the cells there are
   blank: `error[E0425]: cannot find value` rather than the `could not
   compile` after it. Where it recognises nothing, nothing is marked.
   Read once per output (kept by its text), about 150 ns a row; the net
   is built once, in under a millisecond, the first time it's needed.

Nothing here changes a cell: the colours are `Snapshot::ink`, the bars
`Snapshot::run_bars` and the marks `Snapshot::command_marks`, drawn by
the renderer like the ghost text and the completion list.

## Rules

- **Past command lines.** The text is the shell's own from OSC 133 C's
  `cmdline_url` (zsh and fish send it), else what the cells from the B
  mark held when C came (bash). It is placed from the B mark with the
  same code as the line being typed (`ink::text_ink`, `place`): only
  cells that match the text and are drawn in the default colour with
  no attributes are coloured, and placing stops at the first cell that
  doesn't match. What the shell coloured itself keeps its colour.
- **The bar** (`RunBar { rows, state }`, view rows) runs from the
  prompt's row (the view's top when the prompt has scrolled above it)
  to the row before the next prompt. The last run in view goes to the
  last row of its output (the row before its D mark when D starts a
  row); while it runs, or before it's run, to the cursor's row, or the
  row above when the cursor stands at the start of a blank row; when
  the cursor is out of view, to the view's bottom. Clipped to the view.
  Only runs with a prompt mark get one: nothing without shell
  integration or on the alternate screen, and none for what's written
  before the pane's first prompt (the "restored from …" or welcome
  line). The pane's first command has its bar.
- **How it's drawn**: 2 points wide (`RUN_BAR_WIDTH_PT`, times the
  backing scale: 4 pixels on a Retina screen), in the middle of the
  left padding, or at its left edge when the padding is narrower than
  the bar; from the top of its first row to the bottom of its last,
  a point short at each end, so two runs back to back show as two bars.
  Green and red are the theme's ANSI 2 and 1 at alpha 150, as the mark
  at the right; dim is the foreground at alpha 96 (as the line it
  replaced, which at 48 was too faint on a dark theme). A failed
  prompt's own red mark in the padding (`Snapshot::failed`, always on)
  isn't drawn where a bar is: the red bar says it.
- **Room for it**: while `command_marks` is on the side padding is at
  least 4 points (the bar and a point either side; `padding_for` in
  `crates/term-app/src/view.rs`), so a `padding` under 4 still leaves
  the bar beside the text rather than over it. The default padding (6)
  is unchanged. Turning the setting on or off reloads the layout like
  any padding change.
- **The mark** is drawn only for a command that finished (a D mark with
  an exit status; a D mark without one gets no mark), ending one cell
  short of the right edge, and only when every cell it covers and the
  one before it is blank (empty or a space, default background, no
  attributes): a right prompt or any text there hides it. A command line
  that wraps fills its row, so it gets no mark.
- **The time** runs from the C mark to the D mark, as the app's clock
  stood when each was parsed. Under 1 s it isn't shown.
- **Output colouring** applies only to rows between a C mark and its D
  mark (or the next prompt), or after C to the end of the view while it
  runs; only to stretches whose every cell the program left in the
  default colours with no attributes; never on the alternate screen.
  Words are matched whole (letters, digits and `_` make a word): `error`
  but not `errors` or `error_code`. Words match as listed, capitalised or
  in capitals (`Error`, `ERROR`). A hash is 7 to 40 hex digits with at
  least one digit and one letter; all digits is a number. A number is
  digits with `.`-separated parts, then up to three unit letters or `%`,
  ending the word.
- Nothing at all on the alternate screen.

## Where the code lives

- `crates/term-core/src/term/commands.rs`: the log of commands run, one
  entry per C mark with its text and start and end times, and
  `Terminal::view_commands`, the prompts in view (and the one above
  whose output reaches into it) in view rows and columns. A C mark
  keeps its command's number in the mark's `status` (only D marks used
  it before), so the text and times follow it into history and through
  reflow with no change to how marks are stored. The log keeps the
  latest 1,024 commands, numbered per terminal.
- term-core reads no clock: the app calls `Terminal::set_clock` (ms since
  it started) before each parse (`crates/term-app/src/session.rs`).
- `crates/term-complete/src/marks.rs`: `Marks::update`, run per pane after
  the snapshot with the terminal locked (`crates/term-app/src/view.rs`).
  It works everything out again only when the terminal's generation or
  the settings change, so an idle frame copies what it had (the bars
  too: `run_bar`, from the commands in view and the snapshot's cursor); command
  lines are read into words once per command (by its number) and output
  rows coloured once per content (a hash of its characters and which
  cells are plain). The formatting is `mark_text` and `duration`.
- `crates/term-complete/src/output.rs`: the output patterns.
- `crates/term-render/src/frame.rs`: the bars (`run_bar`, one rect
  each, sized from `FrameInput::scale`) with the failed-prompt marks,
  the mark (`draw_command_mark`, green or red at alpha 150) with the
  ghost text. `Snapshot::ink` is now
  kept sorted by row, and the renderer finds a row's colours by binary
  search instead of scanning all of them for each glyph.
- Settings: `command_marks` and `colour_output` in
  `crates/term-config/src/config.rs` and the Completion section of the
  prefs schema, both live.

## Benchmarks

Measured on 1 Nov 2026 while other agents' builds kept the Mac's load
average between 85 and 125, so frame times moved by up to 3× between
runs of the same code: the table shows no slowdown, not a speed-up.
The change to frame building is small: two loops over empty lists when
the setting is off, and a row's colours found by binary search.

`cargo bench -p term-render --bench frame`, before (cdb5e0c) and after,
run back to back:

| Benchmark | Before | After |
|---|---|---|
| frame 160x48 prompt plain unchanged | 6.2 µs | 4.3 µs |
| frame 160x48 prompt coloured unchanged | 7.6 µs | 4.4 µs |
| frame 160x48 prompt list open unchanged | 21.4 µs | 18.6 µs |
| frame 300x100 prompts history unchanged | 35.3 µs | 20.8 µs |
| frame 300x100 coloured | 409 µs | 260 µs |
| frame 300x100 coloured unchanged | 15.1 µs | 10.4 µs |
| frame 300x100 coloured history | 921 µs | 280 µs |

`cargo bench -p term-complete --bench complete -- marks` (new): the
work after each snapshot for a 120 × 40 view of eight commands with
three rows of output each:

| Case | Marks and past lines | With output coloured |
|---|---|---|
| idle (nothing changed) | 0.70 µs | 0.93 µs |
| after a change | 23 µs | 69 µs |
| cold (empty caches) | 25 µs | 99 µs |

An idle frame copies what it had. After a change, the commands in view
are found again and the past lines placed again (their words are kept);
output rows are read and hashed, and only rows not seen before are
coloured. This runs only when the terminal changed, at most once per
frame.

### Run bars (8 Nov 2026)

The bars replaced the lines with no cost to an idle frame: `Marks`
works them out only when the terminal or the settings change, and a
frame copies them into the snapshot's reused `run_bars` (no allocation
once its capacity is there); the renderer draws one rect per bar.
Measured back to back on a loaded Mac (load average 40 to 100), before
(ef0de94), after, and before again, to show the noise:

| Benchmark | Before | After | Before again |
|---|---|---|---|
| marks idle | 329 ns | 200 ns | 201 ns |
| marks with output idle | 762 ns | 680 ns | 1.51 µs |
| marks changed | 17.6 µs | 10.0 µs | 11.8 µs |
| frame 160x48 prompt plain unchanged | 2.75 µs | 2.84 µs | 5.40 µs |
| frame 160x48 prompt coloured unchanged | 3.09 µs | 3.16 µs | 3.40 µs |
| frame 300x100 prompts history unchanged | 14.8 µs | 14.7 µs | 24.7 µs |
| frame 300x100 coloured unchanged | 6.02 µs | 6.08 µs | 6.39 µs |

Every after number is inside the spread of the two before runs.
