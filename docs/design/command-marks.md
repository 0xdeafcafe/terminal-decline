# Command marks: past commands coloured, commands marked off, output coloured

Status: **landed** (1 Nov 2026). Approved by Alex.

## What it looks like

```
$ ls -l "a b"                                    ✓
a b
──────────────────────────────────────────────────  (faint line)
$ cargo test                         ✗ 101 · 14s
   Compiling …
   error: test failed
──────────────────────────────────────────────────
$ sleep 70                                 ✓ 1m10s
──────────────────────────────────────────────────
$ █
```

1. **Past commands keep their colours.** Every command line already run
   that is in view (main screen only) is coloured as it was while typed:
   flags, strings, variables, operators by the same rules as the line
   being typed (`lex`, `color_of`), commands green or red by whether
   they exist. Follows the `highlight_command_line` setting.
2. **Each command is marked off** (setting `command_marks`, default on):
   - a faint line about a point high (a sixteenth of the cell's height, at least a pixel) across the pane at the top of each prompt's
     first row (its A mark), except at the very top of the view;
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

Nothing here changes a cell: the colours are `Snapshot::ink`, the lines
`Snapshot::separators` and the marks `Snapshot::command_marks`, drawn by
the renderer like the ghost text and the completion list.

## Rules

- **Past command lines.** The text is the shell's own from OSC 133 C's
  `cmdline_url` (zsh and fish send it), else what the cells from the B
  mark held when C came (bash). It is placed from the B mark with the
  same code as the line being typed (`ink::text_ink`, `place`): only
  cells that match the text and are drawn in the default colour with
  no attributes are coloured, and placing stops at the first cell that
  doesn't match. What the shell coloured itself keeps its colour.
- **The line** goes at the top of every prompt row in view but row 0
  and the pane's first prompt, which has no command above it to mark off
  (under the "restored from …" or welcome line it only underlined that;
  Alex, 2 Nov 2026).
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
  the settings change, so an idle frame copies what it had; command
  lines are read into words once per command (by its number) and output
  rows coloured once per content (a hash of its characters and which
  cells are plain). The formatting is `mark_text` and `duration`.
- `crates/term-complete/src/output.rs`: the output patterns.
- `crates/term-render/src/frame.rs`: the line (a rect in the foreground
  at alpha 96, raised from 48 and one pixel, which was too faint to see on a dark theme) with the failed-prompt marks, the mark (`draw_command_mark`,
  green or red at alpha 150) with the ghost text. `Snapshot::ink` is now
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
