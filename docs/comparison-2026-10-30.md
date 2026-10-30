# Terminal Decline against Ghostty, kitty, Warp and Terminal.app (30 October 2026)

Measured with [`scripts/compare/run.py`](../scripts/compare/run.py) (and its
[`helper.swift`](../scripts/compare/helper.swift)); the raw rows are in
[`compare-data/2026-10-30.jsonl`](compare-data/2026-10-30.jsonl).

| | |
|---|---|
| Mac | Apple M1 Max, macOS 27.0 (26A428), Retina (2 pixels a point) |
| Terminal Decline | `main` at 6a38c0b, release bundle from `scripts/bundle.sh` (launcher + `libterminal_decline.dylib`), ad-hoc signed, bundle id changed to `red.forbes.terminal-decline.compare` so it runs apart from the installed copy |
| Ghostty | 1.4.0 |
| kitty | 0.49.2 |
| Warp | 0.2026.10.21.08.25.02 |
| Terminal.app | 2.15 |

## Summary against the thresholds

The roadmap's "Definition of done" compares Terminal Decline's median with
Ghostty's.

| threshold | Terminal Decline | Ghostty | result |
|---|---|---|---|
| idle CPU 0% | 0.02% | 0.64% | **fail** (32× less than Ghostty, but not 0) |
| idle wakeups 0/s | 0.39/s (interrupt), 0.00/s (package idle) | 16.3/s, 0.17/s | **fail** (42× fewer, but not 0) |
| idle footprint ≤ 35% of Ghostty | 31.5 MB = 47% | 67 MB | **fail** |
| after 10k lines ≤ 35% | 39 MB = 57% | 68 MB | **fail** |
| `cat` 100 MB CPU incl. WindowServer ≤ 45% | 0.62 s = 37% | 1.68 s | **pass** |
| 4 tabs footprint ≤ 35% | 63 MB = 64% | 98 MB | **fail** |
| 4 splits footprint ≤ 35% | not measured (no script support for splits yet) | | — |
| input latency p99 ≤ Ghostty's | not measured: no Accessibility or Screen Recording | | — |

Ghostty 1.4 with the same config as Terminal Decline uses much less memory
than the 117 MB idle and 145–177 MB after a `cat` in DESIGN.md's
"Measured" table (an older Ghostty, its default config): 67 MB idle, 71 MB
after the `cat`. The 35% thresholds were set against those older numbers.

## How

Rounds interleave the terminals: Terminal Decline, Ghostty, kitty, Warp,
Terminal.app; then the same order with four tabs; then the next round. A
change in the machine's load hits them all alike. Each run starts a
separate instance from its own config in a temp folder; only processes the
script started are measured and stopped (none of the four others was
open). Per run:

- **Idle**: 5 s after the window is up, the footprint
  (`/usr/bin/footprint`, not RSS), then CPU and wakeups over 30 s
  (`proc_pid_rusage`: interrupt wakeups, and package idle wakeups, the ones
  that woke an idle CPU).
- **After 10k lines**: `clear; seq 1 10000` in the window, 2 s, footprint.
- **`cat` 100 MB** (`scripts/pressure/gen.py big`, coloured log lines):
  wall time as the window's shell sees it, CPU seconds of the terminal and
  of WindowServer (before, and 1 s after), and the peak footprint, sampled
  as fast as `footprint` answers during the `cat`.
- **4 tabs**: a second launch, three more tabs opened, 6 s, footprint.

Every number is summed over all the terminal's processes (every process
whose executable is inside its app bundle: kitty runs 3, Warp 2, the
others 1). The shell and `cat` are not counted.

Same conditions: Menlo 13, a 100×30 cell grid, 10,000 lines of scrollback
(Ghostty's limit is in bytes: 10k rows of 100 cells, 8,080,000 bytes), no
cursor blink, default theme, one window. Window sizes in points, as they
came out (×2 for pixels):

| Terminal Decline | Ghostty | kitty | Warp | Terminal.app |
|---|---|---|---|---|
| 812×509 | 824×506 | 825×507 | 832×484 | 820×558 |

Within 2.5% in width. Ghostty, Warp and Terminal.app only take a size in
cells, so the grid is what is matched, not the pixels exactly;
Terminal.app's window is taller by its title and tab bar, Warp's shorter.

Configs: Terminal Decline `--config` and `XDG_CONFIG_HOME` in the temp
folder; Ghostty `--config-default-files=false --config-file=…`; kitty
`--config …`; Terminal.app a "Compare" profile added to its preferences for
the run, which were exported before and put back after (checked: the same
contents); Warp has no config to pass, so `~/.warp/settings.toml` was
replaced for each run by a minimal one (font, size, grid, scrollback, no
AI, no session restore) and put back after (checked: the same file, byte
for byte), and its window comes from a launch configuration, removed after.

## The load, and how many runs

Other projects were building on the Mac throughout (rustc, clippy). Before
each run the script waits while the 1-minute load average is over 12: it
waited 18 minutes before round 1 (load 13–33). Load before each recorded
run: 1.7 – 10.0 (median 4.9). Two runs had the load rise sharply during
them; they are kept and show in the spread: round 1 for Terminal Decline
(load 9 → 23: its 1.57 s `cat`, 16 wakeups/s) and round 4 for Warp (3 → 30:
its 21.7 s `cat`).

The plan was 5 rounds. During round 4 the load went to 80–90 and stayed
over 12 for 15 minutes, and the time for this run was up, so the run was
stopped there: **4 runs each** for Terminal Decline, Ghostty, kitty and
Warp, **3** for Terminal.app, and **3 runs each** with 4 tabs. The spreads
are small next to the gaps between terminals except where the load spiked.

## Results

Median, then the spread (lowest – highest). "vs Terminal Decline" is
Terminal Decline's median as a share of that terminal's (below 100%:
Terminal Decline uses less).

### Idle footprint (MB)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 31.5 | 31.0 – 32.0 | 4 |  |
| Ghostty | 67.0 | 41.0 – 68.0 | 4 | 47% of this |
| kitty | 109 | 109 – 109 | 4 | 29% of this |
| Warp | 333 | 328 – 366 | 4 | 9% of this |
| Terminal.app | 48.0 | 48.0 – 48.0 | 3 | 66% of this |

Threshold ≤ 35% of Ghostty: **fail** (Terminal Decline 31.5, Ghostty 67.0).

### Idle CPU (%)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 0.02 | 0.01 – 0.98 | 4 |  |
| Ghostty | 0.64 | 0.21 – 0.87 | 4 | 3% of this |
| kitty | 0.13 | 0.09 – 0.28 | 4 | 12% of this |
| Warp | 1.11 | 1.01 – 2.09 | 4 | 1% of this |
| Terminal.app | 0.03 | 0.03 – 0.51 | 3 | 53% of this |

Threshold = 0: **fail** (Terminal Decline 0.02, Ghostty 0.64).

### Idle wakeups (interrupt) (/s)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 0.39 | 0.37 – 16.3 | 4 |  |
| Ghostty | 16.3 | 0.47 – 19.0 | 4 | 2% of this |
| kitty | 23.1 | 22.1 – 30.0 | 4 | 2% of this |
| Warp | 61.5 | 60.2 – 63.3 | 4 | 1% of this |
| Terminal.app | 0.67 | 0.53 – 3.33 | 3 | 57% of this |

Threshold = 0: **fail** (Terminal Decline 0.39, Ghostty 16.3).

### Idle wakeups (package idle) (/s)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 0.00 | 0.00 – 0.07 | 4 |  |
| Ghostty | 0.17 | 0.03 – 0.47 | 4 | 0 |
| kitty | 0.53 | 0.10 – 0.67 | 4 | 0 |
| Warp | 1.53 | 0.83 – 2.80 | 4 | 0 |
| Terminal.app | 0.03 | 0.00 – 0.03 | 3 | 0 |

### Footprint after `seq 1 10000` (MB)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 39.0 | 25.0 – 43.0 | 4 |  |
| Ghostty | 68.0 | 41.0 – 69.0 | 4 | 57% of this |
| kitty | 140 | 139 – 140 | 4 | 28% of this |
| Warp | 310 | 294 – 322 | 4 | 13% of this |
| Terminal.app | 64.0 | 48.0 – 74.0 | 3 | 61% of this |

Threshold ≤ 35% of Ghostty: **fail** (Terminal Decline 39.0, Ghostty 68.0).

### `cat` 100 MB: wall time (s)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 0.61 | 0.51 – 1.57 | 4 |  |
| Ghostty | 0.53 | 0.47 – 0.94 | 4 | 116% of this |
| kitty | 0.95 | 0.94 – 0.98 | 4 | 64% of this |
| Warp | 3.59 | 3.37 – 21.7 | 4 | 17% of this |
| Terminal.app | 3.97 | 3.49 – 4.61 | 3 | 15% of this |

### `cat` 100 MB: terminal CPU (s)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 0.52 | 0.46 – 0.75 | 4 |  |
| Ghostty | 1.33 | 0.66 – 2.21 | 4 | 39% of this |
| kitty | 1.18 | 1.13 – 1.25 | 4 | 44% of this |
| Warp | 3.54 | 3.48 – 8.27 | 4 | 15% of this |
| Terminal.app | 4.08 | 3.57 – 4.51 | 3 | 13% of this |

### `cat` 100 MB: WindowServer CPU (s)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 0.09 | 0.04 – 0.11 | 4 |  |
| Ghostty | 0.35 | 0.15 – 0.54 | 4 | 25% of this |
| kitty | 0.14 | 0.02 – 0.38 | 4 | 67% of this |
| Warp | 0.86 | 0.64 – 4.52 | 4 | 10% of this |
| Terminal.app | 0.54 | 0.34 – 2.84 | 3 | 17% of this |

### `cat` 100 MB: terminal + WindowServer CPU (s)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 0.62 | 0.55 – 0.79 | 4 |  |
| Ghostty | 1.68 | 0.81 – 2.75 | 4 | 37% of this |
| kitty | 1.31 | 1.15 – 1.63 | 4 | 47% of this |
| Warp | 4.38 | 4.18 – 12.8 | 4 | 14% of this |
| Terminal.app | 4.62 | 3.91 – 7.35 | 3 | 13% of this |

Threshold ≤ 45% of Ghostty: **pass** (Terminal Decline 0.62, Ghostty 1.68).

### `cat` 100 MB: peak footprint (MB)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 45.0 | 27.0 – 51.0 | 4 |  |
| Ghostty | 78.0 | 48.0 – 79.0 | 4 | 58% of this |
| kitty | 152 | 143 – 153 | 4 | 30% of this |
| Warp | 312 | 296 – 324 | 4 | 14% of this |
| Terminal.app | 83.0 | 82.0 – 104 | 3 | 54% of this |

### Footprint after the `cat` (MB)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 45.0 | 27.0 – 49.0 | 4 |  |
| Ghostty | 71.0 | 42.0 – 72.0 | 4 | 63% of this |
| kitty | 147 | 143 – 153 | 4 | 31% of this |
| Warp | 308 | 290 – 312 | 4 | 15% of this |
| Terminal.app | 67.0 | 61.0 – 69.0 | 3 | 67% of this |

### Footprint with 4 tabs (MB)

| terminal | median | min – max | runs | vs Terminal Decline |
|---|---|---|---|---|
| Terminal Decline | 63.0 | 63.0 – 65.0 | 3 |  |
| Ghostty | 98.0 | 97.0 – 129 | 3 | 64% of this |
| kitty | 115 | 114 – 121 | 3 | 55% of this |
| Warp | 332 | 330 – 340 | 3 | 19% of this |
| Terminal.app | 139 | 138 – 140 | 3 | 45% of this |

Threshold ≤ 35% of Ghostty: **fail** (Terminal Decline 63.0, Ghostty 98.0).


### Input latency

Not measured. `scripts/latency/probe.swift check` gave
`screen_recording=false post_events=false`: the terminal this ran from has
neither Screen Recording nor Accessibility, and privacy settings were left
alone. Granting both to it and running `scripts/compare/run.py --latency`
fills this in.

## Notes on the differences

- **Where Terminal Decline loses.** The `cat` takes a little longer in
  Terminal Decline than in Ghostty (0.61 s against 0.53 s, median), while
  using 37% of Ghostty's CPU; it doesn't pass the 35% footprint thresholds
  against Ghostty 1.4 (47% idle, 57% after 10k lines, 64% with 4 tabs).
  Idle CPU and wakeups are low but not zero: about 0.4 interrupt wakeups a
  second (none that woke an idle CPU) and 0.02% CPU. Terminal.app is close
  behind at idle (0.03%, 0.67 wakeups/s).
- **Ghostty** runs ~16 timer wakeups a second at idle (one round had 0.5)
  and its idle footprint was 41 MB in round 1 against 67–68 MB in the
  others; with 4 tabs one run was 129 MB against 97–98 MB.
- **kitty** keeps 3 processes; ~23 wakeups/s idle; its footprint grows
  most with scrollback (109 → 140 MB after 10k lines).
- **Warp** uses 5–10× the memory of the others (330 MB idle) and ~61
  wakeups/s idle. It always opens a window of its own at start besides the
  launch configuration's, so it ran with two windows (both counted); its
  4-tab run was four tabs in the launch configuration, not checked from
  outside (its tabs aren't macOS windows). Its other settings are Warp's
  defaults, not matched one by one.
- **Terminal.app** is the slowest at the `cat` (4 s, 4.6 s CPU), but has
  the lowest idle use after Terminal Decline.
- **Terminal Decline's 4-tab runs** use its test-only control
  (`DECLINE_TEST_DUMP`, to open tabs from inside), which also puts the app
  in the background (no Dock icon, windows behind the others). That may
  change its footprint a little against a normal launch; the main runs
  don't use it.
- `cat` wall time is when `cat` finished writing, as the shell sees it:
  terminals that read the PTY fast and draw fewer frames finish sooner.
  None of them was occluded on purpose, but windows of other apps may have
  been on screen.
- Terminal Decline does less work than the others (no text shaping or
  grapheme clusters, no reflow); see DESIGN.md.

Rerun: `scripts/compare/run.py --bundle "<dir>/Terminal Decline.app"
--max-load 12 --latency` (with a bundle from `scripts/bundle.sh`; without
`--bundle` it wraps a bare `target/release/terminal-decline`), then
`scripts/compare/run.py --report`.
