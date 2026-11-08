# The perf watch

Is Terminal Decline's memory or CPU creeping up as features land? The
perf watch measures the same things after every push to main (pushes
that only change Markdown or `docs/` are skipped), nightly when main has
changed, and when started by hand, and says how they moved. It is a
report, never a gate: a number getting worse never fails a run, and a
measurement that breaks shows as a warning in the run's summary while the
rest still report.

- Workflow: `.github/workflows/perf-watch.yml`
- Script: `scripts/perf-watch/perf_watch.py` (Python 3, standard library
  only; `--self-test` runs in CI)
- Limits: `scripts/perf-watch/thresholds.json`
- History: `runs.jsonl` on the `perf-history` branch, one line of JSON a
  run on main

## What is measured

Counts rather than times wherever there is a choice, so a busy CI
machine moves them little.

| Numbers | How |
|---|---|
| `alloc.<input>.allocs`, `.peak_heap`, `.retained_heap` | The term-core example `alloc_count`: each input of the parse benchmark (plain, coloured, tui, seq, unicode, clusters), 4 MB fed in 64 KiB pieces to a 160x50 terminal with 10k lines of history, under a counting allocator. Allocations (reallocations included), the most heap held and the heap kept at the end. Deterministic. |
| `scrollback.<output>` | Scrollback bytes per line from the example `scrollback_mem`, as `scripts/bench-gate.py` reads them. Deterministic. |
| `app.history_bytes` | A release bundle (`scripts/bundle.sh`, ad hoc signed, without Sparkle) started in the background under the test control, its window behind every other and never taking focus. Its shell `cat`s a 20 MB flood from `scripts/pressure/gen.py big`; the test control's dump then reports the bytes history holds. |
| `app.footprint_after_flood` | `/usr/bin/footprint` (phys_footprint, not RSS) 10 s after the flood, once the idle trim has run. |
| `app.footprint_idle`, `app.threads` | Footprint and thread count after the app has then sat idle for a minute, starting a minute after the flood. |
| `app.idle_cpu_ms_per_min`, `app.idle_wakeups_per_min`, `app.idle_context_switches_per_min` | CPU time, wake-ups (idle and interrupt) and context switches over that minute, from `proc_pid_rusage` and `proc_pidinfo` read from outside the app (no `sudo` needed, unlike `powermetrics`). While idle the shell waits in `read`, a builtin, so nothing in the tab changes. The minute starts a minute after the flood, not at the 10 s mark: until about 40 s after it, macOS's own launch work in the process (libapp_launch_measurement and dispatch timers on the main queue, while the app's threads all sleep) wakes it 5 to 111 times, differently each run, which made one build read 6 and 49 wake-ups a minute (issue #10). |
| `size.dylib`, `size.launcher`, `size.stub`, `size.bundle` | Bytes of the library, launcher and pin stub in the bundle, and the whole bundle. |

## Comparing, and the issue

Each run is compared with the previous run on main and with the run from
about a week before (the newest at least seven days older), so a slow
creep shows even when no single change crosses a limit. The step summary
has a table of before, now, change and the week-old number.

A number is over its limit when it is more than the rule's `percent`
above the lower of those two numbers, and also more than `min_delta`
above it (in the number's unit), so a tiny number moving a little does
not count:

| Numbers | Limit | Floor |
|---|--:|--:|
| Allocations | +10% | 100 allocations |
| Most heap, heap kept, history bytes | +10% | 64 KiB |
| Scrollback bytes per line | +5% | 1 byte |
| Footprint | +10% | 4 MiB |
| Threads | +10% | 2 threads |
| Idle CPU time | +20% | 20 ms a minute |
| Idle wake-ups | +20% | 30 a minute |
| Idle context switches | +20% | 60 a minute |
| Binary sizes | +5% | 16 KiB |

The counts are exact and get tight limits. Footprint and idle CPU move
between runs of the same build on a CI machine, so theirs are looser.

On main, a number over its limit opens a GitHub issue labelled
`performance` (the label is made if missing). The issue keeps the numbers
it was opened against; while it is open, later runs compare with those,
update its table, and comment when the set of numbers over changes. When
everything is back under, the job closes it. To accept new numbers as the
new normal, close the issue by hand: the next run compares with the
latest run again. To accept some of them while others are still being
looked into, edit the issue's body: in its hidden `perf-watch-baseline`
comment, set those numbers to the ones accepted, and say why in a
comment on the issue. Later runs compare with the edited baseline and
keep it.

## Trying a change to the watch

Push a `ci/perf-watch...` branch: the workflow runs and reports against
main's history, without recording the run or touching the issue. To run
the parts locally (a release build, so only on a quiet Mac):

    python3 scripts/perf-watch/perf_watch.py measure --out perf.json [--skip app] [--idle-seconds 10]
    python3 scripts/perf-watch/perf_watch.py report --current perf.json
