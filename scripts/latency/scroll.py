#!/usr/bin/env python3
"""How smoothly Terminal Decline scrolls back through history, frame by frame.

    scripts/latency/scroll.py [--app target/release/terminal-decline]
                              [--idle 10,60,300] [--out target/scroll] [--tag NAME]

Starts decline under the test control (in the background, with a temp
config home) running `history.py`, which asks for a 300x100 window and
prints 10,000 lines of mixed output. Through the latency harness's FIFO
(`DECLINE_LATENCY_INPUT`) it then plays trackpad gestures in the app
(`gesture`: precise deltas with their phases and momentum, at 120 Hz) and
mouse wheel clicks (`wheel`), and reads the per-frame records decline logs
while its view scrolls (`DECLINE_LATENCY_LOG`, crates/term-app/src/latency.rs):
event → on glass, snapshot (history read) and build times, lines moved,
the sub-line shift, glyphs left for the glyph thread, and the ticks that
drew nothing.

First back to back (slow, medium and fast flicks, a wheel), then the first
gesture after each idle time in --idle (seconds), to catch what idle work
(drawable trims, history compression, glyph textures made purgeable) costs
the first frames after it. Prints per-gesture worst cases and writes the raw
log and summary.json to --out.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
import run  # noqa: E402  (the latency harness: FIFO, percentiles)

# (name, command): positive points scroll back into history.
BACK_TO_BACK = [
    ("slow", "gesture 2 90 0.9 120"),
    ("medium", "gesture 8 40 0.95 120"),
    ("fast", "gesture 40 12 0.96 120"),
    ("medium-down", "gesture -8 40 0.95 120"),
    ("medium-2", "gesture 8 40 0.95 120"),
    ("wheel", "wheel 1 30 30"),
]
IDLE_GESTURE = "gesture 8 40 0.95 120"


def gesture_secs(cmd):
    w = cmd.split()
    if w[0] == "wheel":
        return int(w[2]) / float(w[3])
    pts, n, decay, hz = float(w[1]), int(w[2]), float(w[3]), float(w[4])
    k, v = 0, abs(pts) * decay
    while v >= 0.5:
        k, v = k + 1, v * decay
    return (n + k + 2) / hz


def summarise(recs, start, end):
    frames = [r for r in recs if r.get("kind") == "frame" and start <= r["frame_start"] <= end]
    ticks = [r for r in recs if r.get("kind") == "tick" and start <= r["t"] <= end]
    # A window in the background isn't put on the glass (no t3): then the
    # GPU's finishing times stand in for it.
    end_of = "t3" if any(f.get("t3") for f in frames) else "gpu_done"
    shown = [f[end_of] for f in frames if f.get(end_of)]
    gaps = [(b - a) * 1000 for a, b in zip(shown, shown[1:])]
    # The display's refresh: the shortest gap that is common (a 60 Hz
    # display never shows two frames 8 ms apart).
    refresh = 1000 / 120 if sum(1 for g in gaps if g < 12) > len(gaps) / 4 else 1000 / 60
    # (Events of an earlier gesture, which moved nothing, aren't this one's.)
    lat = [
        (f[end_of] - f["t0_first"]) * 1000
        for f in frames
        if f.get(end_of) and f.get("t0_first") and f["frame_start"] - f["t0_first"] < 0.25
    ]
    build = [(f["snapshot_us"] + f["build_us"]) / 1000 for f in frames if f["built"]]
    snap = [f["snapshot_us"] / 1000 for f in frames if f["built"]]
    offsets = [f["offset"] for f in frames if f["built"]]
    moved = [abs(b - a) for a, b in zip(offsets, offsets[1:])]
    # Where the content is, in device pixels, frame by frame (the offset
    # carried over frames that weren't built).
    pos, off, ch = [], None, max((f.get("cell_h", 0) for f in frames), default=0)
    for f in frames:
        off = f["offset"] if f["built"] else off
        if off is not None:
            pos.append(off * ch - f.get("shift_px", 0))
    px = [abs(b - a) for a, b in zip(pos, pos[1:])]
    # Judder: a step unlike the steps either side of it (a steady scroll
    # moving 16, 48, 32 px in turn), where it moves at all.
    uneven = sum(
        1
        for a, b, c in zip(px, px[1:], px[2:])
        if (a + c) / 2 >= 4 and abs(b - (a + c) / 2) > 0.25 * (a + c) / 2
    )
    # The main thread's time for a frame: the tick until its commit.
    main_ms = [(f["commit"] - f["frame_start"]) * 1000 for f in frames if f.get("commit")]
    first = frames[:5]
    return {
        "frames": len(frames),
        "until": end_of,
        "grid": max(((f.get("cols", 0), f.get("rows", 0)) for f in frames), default=(0, 0)),
        "span_ms": (shown[-1] - shown[0]) * 1000 if len(shown) > 1 else 0,
        "built": sum(1 for f in frames if f["built"]),
        "events": sum(f["events"] for f in frames),
        "not_presented": sum(1 for f in frames if not f.get("t3")),
        "refresh_ms": refresh,
        "gap_ms": run.stats(gaps),
        "long_gaps": sum(1 for g in gaps if g > 1.5 * refresh),
        "event_to_glass_ms": run.stats(lat),
        "build_ms": run.stats(build),
        "snapshot_ms": run.stats(snap),
        "copied_rows_max": max((f["copied"] for f in frames), default=0),
        "lines_per_frame": run.stats(moved),
        "px_per_frame": run.stats(px),
        "uneven_steps": uneven,
        "main_ms": run.stats(main_ms),
        "cell_h": ch,
        "shift_px_max": max((abs(f.get("shift_px", 0)) for f in frames), default=0),
        "deferred_frames": sum(1 for f in frames if f["deferred"]),
        "upload_wait_ms": run.stats([f.get("upload_wait_us", 0) / 1000 for f in frames if f["built"]]),
        "lock_wait_ms": run.stats([f.get("lock_wait_us", 0) / 1000 for f in frames]),
        "idle_ticks": {d: sum(1 for t in ticks if t["decision"].startswith(d)) for d in ("Skip", "Pause", "Hold")},
        "first_frames_ms": [
            round(((f.get(end_of) or 0) - (t0 if (t0 := f.get("t0_first")) and f["frame_start"] - t0 < 0.25 else f["frame_start"])) * 1000, 2)
            for f in first
        ],
        "first_builds_ms": [round((f["snapshot_us"] + f["build_us"]) / 1000, 3) for f in first],
    }


def show(name, s):
    def f(st):
        return f"p50 {st['p50']:.2f} p99 {st['p99']:.2f} max {st['max']:.2f}"

    print(f"\n{name}: {s['frames']} frames ({s['built']} built) in {s['span_ms']:.0f} ms for {s['events']} events, grid {s['grid']}")
    print(f"  frame gaps     {f(s['gap_ms'])} ms ({s['until']}), {s['long_gaps']} over 1.5x {s['refresh_ms']:.2f} ms")
    print(f"  event → {s['until']:8} {f(s['event_to_glass_ms'])} ms")
    print(f"  build          {f(s['build_ms'])} ms (snapshot {f(s['snapshot_ms'])})")
    print(f"  main thread    {f(s['main_ms'])} ms a frame (tick to commit)")
    print(f"  px/frame       {f(s['px_per_frame'])} (cell {s['cell_h']} px), {s['uneven_steps']} steps unlike their neighbours")
    print(f"  lines/frame    {f(s['lines_per_frame'])}, sub-line shift max {s['shift_px_max']} px, rows copied max {s['copied_rows_max']}")
    print(f"  glyphs deferred in {s['deferred_frames']} frames; glyph uploads waited {f(s['upload_wait_ms'])} ms; idle ticks {s['idle_ticks']}")
    print(f"  lock waits     {f(s['lock_wait_ms'])} ms (a scroll event, for the terminal)")
    print(f"  first frames   event→glass {s['first_frames_ms']} build {s['first_builds_ms']}")


def load():
    out = subprocess.run(["uptime"], capture_output=True, text=True).stdout.strip()
    return out.split("load averages:")[-1].strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", default=str(ROOT / "target/release/terminal-decline"))
    ap.add_argument("--idle", default="10,60,300")
    ap.add_argument("--out", default=str(ROOT / "target/scroll"))
    ap.add_argument("--tag", default="")
    ap.add_argument("--history", type=int, default=10000)
    args = ap.parse_args()
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix="decline-scroll-"))
    (tmp / "config").mkdir()
    fifo = tmp / "input"
    os.mkfifo(fifo)
    log = out / f"scroll{args.tag}.jsonl"
    log.unlink(missing_ok=True)
    env = {
        **os.environ,
        "XDG_CONFIG_HOME": str(tmp / "config"),
        "DECLINE_TEST_DUMP": str(tmp / "dump.jsonl"),
        "DECLINE_LATENCY_LOG": str(log),
        "DECLINE_LATENCY_INPUT": str(fifo),
        "TERM": "xterm-256color",
    }
    env.pop("DECLINE_LATENCY_HZ", None)
    program = [sys.executable, str(HERE / "history.py"), "--history", str(args.history)]
    app = subprocess.Popen([args.app, "-e", *program], env=env, stderr=open(out / f"scroll{args.tag}.stderr", "w"))
    summary = {"load_start": load()}
    try:
        ctl = run.open_fifo(fifo, timeout=30)
        time.sleep(8.0)
        plan = [(n, c, 1.5) for n, c in BACK_TO_BACK]
        plan += [(f"after-{s}s-idle", IDLE_GESTURE, float(s)) for s in args.idle.split(",") if s]
        for name, cmd, wait in plan:
            time.sleep(wait)
            t = time.clock_gettime(time.CLOCK_UPTIME_RAW)
            ctl.write(cmd + "\n")
            time.sleep(gesture_secs(cmd) + 0.6)
            end = time.clock_gettime(time.CLOCK_UPTIME_RAW)
            recs = run.read_jsonl(log)
            s = summarise(recs, t, end)
            s["load"] = load()
            s["command"] = cmd
            summary[name] = s
            show(f"{name} ({cmd}, load {s['load']})", s)
    finally:
        app.terminate()
        try:
            app.wait(5)
        except subprocess.TimeoutExpired:
            app.kill()
        (out / f"summary{args.tag}.json").write_text(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
