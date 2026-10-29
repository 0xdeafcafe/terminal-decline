# Conformance: vttest and esctest2

Two outside test suites run against the terminal, with no window:

- **vttest** (Thomas Dickey, pinned to 20251205): the classic VT100/VT220
  test program. It draws screens for a person to judge. We run the menus
  listed in `tests/conformance/vttest-menus.txt`, choosing each entry by its
  title, and compare every screen it stops on ("Push <RETURN>") with the
  expected screen in `tests/conformance/vttest/<menu>.txt`.
- **esctest2** (George Nachman and Thomas Dickey, pinned commit): about 570
  automatic tests that check the cursor and the screen after each escape
  sequence, against "xterm without its bugs".

Failures we know about are listed in `tests/conformance/accepted-failures.txt`,
each marked `to-fix` (a bug, or a gap worth closing) or `not-planned`, and
explained in [conformance-failures.md](conformance-failures.md). A run fails
on a failure that isn't listed, and on a listed test that now passes (so the
list stays true: delete the line).

## Running

```sh
scripts/conformance/run.py                  # both suites; about 20 minutes
scripts/conformance/run.py vttest           # vttest only
scripts/conformance/run.py vttest --only cursor-movements reports-dsr
scripts/conformance/run.py esctest --include 'CUP|ED'
```

Needs `python3`, a C compiler and `make` (vttest is built from source), and
the network the first time: the sources go to `target/conformance/`.
Results go to `target/conformance/results/`: `summary.txt` (every test,
pass or fail), `vttest/<menu>.txt` (the screens as this run saw them),
`vttest/<menu>-<screen>.diff` for each mismatch, and `esctest.log`
(esctest's own log, with a traceback for each failure).

A partial run (`--only`, `--include`) only reports what ran.

The nightly workflow (`.github/workflows/nightly-tests.yml`, job
`conformance`) runs both suites on macOS when `main` changed that day, and
uploads the results.

## How it works

`crates/term-pty/examples/headless.rs` is the terminal: term-core on a real
PTY, as the app has it, but driven over stdin and stdout instead of a window
(`send` bytes, press `enter`, wait until output goes quiet, read the
`screen`, `wait` for exit). Return is sent as the app sends it: CR, or CR LF
in newline mode (LNM).

`scripts/conformance/run.py` drives it:

- **vttest**: start vttest, pick each named menu entry by title, then press
  Return on each screen until vttest shows a menu again, recording the
  screen each time (after 700 ms of quiet: vttest waits about 200 ms for
  replies, and a Return typed into that wait is read as part of the reply).
  The screen is compared as text, with trailing blanks dropped.
- **esctest2**: run `esctest.py --expected-terminal=xterm
  --xterm-checksum=334` inside the terminal at 80x25 and read its log.
  esctest reads the screen back with DECRQCRA (checksum of a rectangle),
  which term-core doesn't implement; the harness answers it from the screen
  (the sum of the characters, blanks as spaces, as xterm does since patch
  334). It doesn't take origin mode into account.

`--terminal tmux` runs the same vttest menus under tmux and writes its
screens to `--out DIR`: a second opinion when judging a screen, not a
reference (tmux has its own differences, and identifies itself differently,
so some menus differ).

## What the screens can't show

The vttest comparison is text only: colours, bold, underline, blink,
reverse video (DECSCNM) and double-height rendering aren't compared. A
double-width line (DECDWL) is expected to hold 40 columns, which is
checked.

## Updating

When a change makes a vttest screen right, or vttest is updated:

```sh
scripts/conformance/run.py vttest --bless   # write the screens as expected
git diff tests/conformance/vttest           # and check every change by eye
```

`--bless` keeps the expected screens of tests listed in the accepted
failures (they were written by hand: what the screen should be, not what it
is). A screen whose right contents aren't written down is a line starting
`!unknown` and the reason (for example a 132-column screen); it never
passes.

After fixing a `to-fix` bug, delete its lines from the accepted failures and
its section in conformance-failures.md.
