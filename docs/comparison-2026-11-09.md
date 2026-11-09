# Decline against Ghostty, Warp and Terminal.app on a GitHub macOS runner (9 November 2026)

Measured with [`scripts/compare/run.py`](../scripts/compare/run.py) (and its
[`helper.swift`](../scripts/compare/helper.swift)), run by the
[Compare workflow](../.github/workflows/compare.yml) on a GitHub-hosted
macOS runner rather than a developer's Mac; the raw rows are in
[`compare-data/2026-11-09.jsonl`](compare-data/2026-11-09.jsonl) (GitHub
Actions run 42544345818).

| | |
|---|---|
| Machine | GitHub-hosted `macos-latest` runner, arm64: a virtual Mac (`VirtualMac2,1`), Apple M1 (Virtual), 3 cores, 7 GB; image macos26 20261102.0473.1 |
| macOS | 26.7.2 (25G332) |
| Display | one, 1024×768 points at 1 pixel a point (not Retina) |
| Decline | `main` at ed437f3 (the branch adds only the workflow and run.py changes), release bundle from `scripts/bundle.sh` as the perf watch builds it, ad-hoc signed |
| Ghostty | 1.4.0 (Homebrew cask) |
| kitty | 0.49.2 (Homebrew cask): **not measured**, see below |
| Warp | 0.2026.10.21.08.25.02 (Homebrew cask) |
| Terminal.app | 2.15 |

Medians of **5 rounds** for every terminal (5 runs each, and 5 each with
4 tabs), then one latency run of 200 keys each. Nothing else ran on the
runner: the 1-minute load before each run was 1.2–3.8 (the script pauses
while it is over 4, on 3 cores; most of it is the runner's own agent and
the previous terminal's `cat`).

## Summary

Decline is best (lowest) on every measure here, in this run and in the
one before it (run 42540719163, the same build, also 5 rounds).

| | Decline | Ghostty | Warp | Terminal.app |
|---|---|---|---|---|
| idle footprint | **21 MB** | 37 MB | 190 MB | 72 MB |
| after 10k lines | **22 MB** | 38 MB | 182 MB | 71 MB |
| 4 tabs | **30 MB** | 55 MB | 204 MB | 85 MB |
| `cat` 100 MB, wall | **1.12 s** | 1.63 s | 6.60 s | 5.02 s |
| `cat` CPU incl. WindowServer | **1.18 s** | 3.70 s | 7.44 s | 6.13 s |
| `cat` peak footprint | **25 MB** | 47 MB | 175 MB | 81 MB |
| idle CPU | **0.01%** | 0.37% | 1.12% | 0.04% |
| idle wakeups/s (interrupt) | **0.13** | 14.3 | 61.4 | 0.30 |
| keypress → pixels p50 / p99 | **17.2 / 38.4 ms** | 19.7 / 42.7 ms | 33.1 / 43.0 ms | 22.3 / 45.7 ms |

The run before: Decline 21 MB idle, 1.39 s and 1.49 s CPU for the `cat`
(Ghostty 2.19 s, 5.03 s), 0.17 wakeups/s, latency p50 16.9 ms (Ghostty
18.4 ms); Warp's latency there saw no key drawn, because the probe watched
the larger of its two same-sized windows rather than the front one (fixed
since: it watches the frontmost window, the one keys go to).

Against the roadmap's thresholds (Decline's median as a share of
Ghostty's) the footprint ones still fail, at 53–58% rather than 35%, and
idle CPU and wakeups are low but not 0; see the tables.

## How

As [the 30 October comparison](comparison-2026-10-30.md) describes, with
the same script: rounds interleave the terminals; each run is a separate
instance from its own config in a temp folder; 5 s after the window is up,
the footprint, then CPU and wakeups over 30 s; `clear; seq 1 10000`, then
the footprint; `cat` of 100 MB of coloured log lines, with wall time, CPU
of the terminal and of WindowServer, and the peak footprint; then a second
launch with 4 tabs. Menlo 13, 100×30 cells, 10k lines of scrollback, no
cursor blink, one window. All of a terminal's processes are summed (Warp
runs 2). Latency is `scripts/latency/probe.swift`: a key posted to the
app, and the time to the first changed frame of its frontmost window.

What the runner changes, in the workflow and not in the method:

- **A fresh machine.** Ghostty, kitty and Warp come from Homebrew casks,
  with Gatekeeper's quarantine removed (their first launch would wait on a
  dialog). Terminal.app's preferences start empty; the script's "Compare"
  profile is added and they are put back as exported.
- **Warp's onboarding.** A Warp that has never run opens to "Welcome to
  Warp" and a sign-in offer, and the launch configuration's window never
  gets a shell. The workflow sets the private preference its onboarding
  sets when done, `HasCompletedOnboarding` (warpdotdev/warp,
  `app/src/root_view.rs`), and the one-time banners a used Warp has
  dismissed; with no account it then opens to a terminal. Not signed in.
  As before, Warp opens a window of its own besides the launch
  configuration's, so it ran with two windows (both counted), and its
  4-tab count isn't visible from outside.
- **kitty can't run here.** It needs OpenGL 3.3, and the runner's virtual
  GPU gives none: `NSGL: Failed to find a suitable pixel format`. The
  workflow tries it once and, failing, records it as not measured
  (`--unavailable`). Its numbers from a real Mac are in the
  [README](../README.md#benchmarks) and the 30 October report.
- **Latency** works here: the runner's processes have Accessibility and
  Screen Recording, which the earlier runs on a Mac lacked.
- **The display** is 1024×768 at 1×, so every terminal draws a quarter of
  the pixels it would on a Retina screen; footprints (drawables) and
  WindowServer time are lower than on a Mac. The machine is slower too
  (3 virtual cores, a paravirtualised GPU): every terminal's `cat` is
  slower than on the M1 Max (Decline 1.12 s here, 0.58–0.61 s there;
  Ghostty 1.63 s, 0.53–0.66 s). Compare terminals within this
  run, not with the earlier reports.

Window sizes in points: Decline 812×524, Ghostty 812×494, Warp 832×484,
Terminal.app 837×558 (the grid is matched, not the pixels; Terminal.app's
title and tab bar).

## Idle wakeups

Decline's 0.13 wakeups a second are measured from 5 s to 35 s after its
window appeared. The perf watch found (docs/perf-watch.md, issue #10) that
for about 40 s after launch macOS's own launch work in the process
(libapp_launch_measurement, dispatch timers on the main queue) wakes it a
few to 111 times while the app's threads all sleep; its idle minute now
starts a minute after the flood and reads 2–7 wakeups a minute on recent
main commits. So most of the 0.13/s here is likely launch noise, the same
for every terminal measured this way; it isn't a gap to close (Decline is
lowest), and moving the comparison's idle window later would change the
method, which this run keeps.

## Results

Median, then the spread (lowest – highest). "vs Decline" is Decline's
median as a share of that terminal's (below 100%: Decline uses less).

### Idle footprint (MB)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 21.0 | 20.0 – 23.0 | 5 |  |
| Ghostty | 37.0 | 37.0 – 40.0 | 5 | 57% of this |
| Warp | 190 | 186 – 208 | 5 | 11% of this |
| Terminal.app | 72.0 | 70.0 – 72.0 | 5 | 29% of this |

Threshold ≤ 35% of Ghostty: **fail** (Decline 21.0, Ghostty 37.0).

### Idle CPU (%)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 0.01 | 0.01 – 0.03 | 5 |  |
| Ghostty | 0.37 | 0.32 – 0.43 | 5 | 3% of this |
| Warp | 1.12 | 1.11 – 1.25 | 5 | 1% of this |
| Terminal.app | 0.04 | 0.03 – 0.05 | 5 | 24% of this |

Threshold = 0: **fail** (Decline 0.01, Ghostty 0.37).

### Idle wakeups (interrupt) (/s)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 0.13 | 0.13 – 0.20 | 5 |  |
| Ghostty | 14.3 | 14.2 – 14.8 | 5 | 1% of this |
| Warp | 61.4 | 59.8 – 62.8 | 5 | 0% of this |
| Terminal.app | 0.30 | 0.30 – 0.33 | 5 | 43% of this |

Threshold = 0: **fail** (Decline 0.13, Ghostty 14.3).

### Idle wakeups (package idle) (/s)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 0.10 | 0.07 – 0.17 | 5 |  |
| Ghostty | 8.32 | 7.01 – 9.96 | 5 | 1% of this |
| Warp | 33.2 | 14.2 – 41.6 | 5 | 0% of this |
| Terminal.app | 0.27 | 0.20 – 0.30 | 5 | 37% of this |

### Footprint after `seq 1 10000` (MB)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 22.0 | 22.0 – 24.0 | 5 |  |
| Ghostty | 38.0 | 38.0 – 41.0 | 5 | 58% of this |
| Warp | 182 | 168 – 191 | 5 | 12% of this |
| Terminal.app | 71.0 | 69.0 – 71.0 | 5 | 31% of this |

Threshold ≤ 35% of Ghostty: **fail** (Decline 22.0, Ghostty 38.0).

### `cat` 100 MB: wall time (s)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 1.12 | 0.86 – 1.26 | 5 |  |
| Ghostty | 1.63 | 1.04 – 1.84 | 5 | 69% of this |
| Warp | 6.60 | 5.27 – 9.18 | 5 | 17% of this |
| Terminal.app | 5.02 | 4.70 – 5.90 | 5 | 22% of this |

### `cat` 100 MB: terminal CPU (s)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 1.04 | 0.81 – 1.17 | 5 |  |
| Ghostty | 3.46 | 2.27 – 4.00 | 5 | 30% of this |
| Warp | 6.64 | 5.54 – 10.1 | 5 | 16% of this |
| Terminal.app | 5.89 | 5.48 – 6.94 | 5 | 18% of this |

### `cat` 100 MB: WindowServer CPU (s)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 0.13 | 0.08 – 0.17 | 5 |  |
| Ghostty | 0.24 | 0.10 – 0.26 | 5 | 54% of this |
| Warp | 0.80 | 0.46 – 1.49 | 5 | 16% of this |
| Terminal.app | 0.24 | 0.19 – 0.32 | 5 | 54% of this |

### `cat` 100 MB: terminal + WindowServer CPU (s)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 1.18 | 0.89 – 1.34 | 5 |  |
| Ghostty | 3.70 | 2.37 – 4.26 | 5 | 32% of this |
| Warp | 7.44 | 6.12 – 11.6 | 5 | 16% of this |
| Terminal.app | 6.13 | 5.67 – 7.26 | 5 | 19% of this |

Threshold ≤ 45% of Ghostty: **pass** (Decline 1.18, Ghostty 3.70).

### `cat` 100 MB: peak footprint (MB)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 25.0 | 25.0 – 27.0 | 5 |  |
| Ghostty | 47.0 | 47.0 – 49.0 | 5 | 53% of this |
| Warp | 175 | 170 – 193 | 5 | 14% of this |
| Terminal.app | 81.0 | 80.0 – 81.0 | 5 | 31% of this |

### Footprint after the `cat` (MB)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 23.0 | 23.0 – 25.0 | 5 |  |
| Ghostty | 41.0 | 40.0 – 43.0 | 5 | 56% of this |
| Warp | 168 | 158 – 189 | 5 | 14% of this |
| Terminal.app | 79.0 | 78.0 – 80.0 | 5 | 29% of this |

### Footprint with 4 tabs (MB)

| terminal | median | min – max | runs | vs Decline |
|---|---|---|---|---|
| Decline | 30.0 | 30.0 – 30.0 | 5 |  |
| Ghostty | 55.0 | 54.0 – 55.0 | 5 | 55% of this |
| Warp | 204 | 191 – 207 | 5 | 15% of this |
| Terminal.app | 85.0 | 85.0 – 86.0 | 5 | 35% of this |

Threshold ≤ 35% of Ghostty: **fail** (Decline 30.0, Ghostty 55.0).

### Input latency, key posted → pixels changed (ms)

| terminal | p50 | p95 | p99 | keys | missed |
|---|---|---|---|---|---|
| Decline | 17.2 | 32.7 | 38.4 | 200 | 0 |
| Ghostty | 19.7 | 37.8 | 42.7 | 200 | 0 |
| Warp | 33.1 | 39.0 | 43.0 | 200 | 0 |
| Terminal.app | 22.3 | 40.3 | 45.7 | 200 | 0 |

Rerun: Actions → Compare → Run workflow (inputs: rounds, terminals,
latency), or `gh workflow run compare.yml`. The job summary has this
report; the artifact `compare` has `results.jsonl` and `report.md`.
