#!/usr/bin/env python3
"""Where a key's time went: the probe's rows joined with Decline's own log.

    join.py <probe.jsonl> <decline-latency.jsonl>

The probe (probe.swift) times a key from just before it posts it to the
first captured frame showing it (ScreenCaptureKit's display time);
Decline's log (DECLINE_LATENCY_LOG, crates/term-app/src/latency.rs) times
the same key inside the app. Both use the host clock (CLOCK_UPTIME_RAW),
so each probe key is matched with the app's key whose event timestamp
(t0) follows its post most closely, and the stages between are printed as
p50 / p95 / p99 in ms.

It also finds the display's refresh grid from the probe's display times
(they fall on refreshes) and counts, for each key, the refreshes that went
by between the frame's commit and the one it was seen at: 0 means the
window server showed it at the first refresh after it was sent. On a
display with a variable refresh rate (ProMotion) there is no such grid,
and it says so instead.
"""

import json
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from run import pct  # noqa: E402

# (name, from, to, what): fields of the joined row.
STAGES = [
    ("total", "t_post", "t_shown", "key posted → seen (the probe's number)"),
    ("post", "t_post", "t0", "posted → event's timestamp"),
    ("deliver", "t0", "key_down", "event → keyDown"),
    ("handle", "key_down", "t1", "keyDown → bytes to writer"),
    ("echo", "t1", "t2", "writer → program → echo parsed"),
    ("wait", "t2", "frame_start", "parsed → frame starts"),
    ("build", "frame_start", "built", "frame build"),
    ("drawable", "built", "drawable", "nextDrawable"),
    ("encode", "drawable", "commit", "encode + commit"),
    ("gpu", "commit", "gpu_done", "commit → GPU done"),
    ("present", "gpu_done", "t3", "GPU done → presentedTime"),
    ("seen", "commit", "t_shown", "commit → seen"),
    ("seen_vs_present", "t3", "t_shown", "presentedTime → seen"),
]


def read_jsonl(path):
    try:
        return [json.loads(l) for l in Path(path).read_text().splitlines() if l.strip()]
    except FileNotFoundError:
        return []


def join(probe, app):
    """Each probe row that saw a frame, with the app's key merged in."""
    keys = sorted((k for k in app if "t0" in k and "seq" in k), key=lambda k: k["t0"])
    out = []
    j = 0
    for r in probe:
        if "t_shown" not in r:
            continue
        t = r["t_post"]
        while j < len(keys) and keys[j]["t0"] < t - 0.002:
            j += 1
        row = dict(r)
        if j < len(keys) and keys[j]["t0"] - t < 0.2:
            row.update({k: v for k, v in keys[j].items() if k != "seq"})
            j += 1
        out.append(row)
    return out


def grid(times):
    """The refresh period and phase the display times fall on: the longest
    of the usual periods they keep to (circular mean resultant ≥ 0.9)."""
    for hz in (30, 60, 120):
        period = 1 / hz
        angles = [2 * math.pi * (t % period) / period for t in times]
        c = sum(math.cos(a) for a in angles) / len(angles)
        s = sum(math.sin(a) for a in angles) / len(angles)
        if math.hypot(c, s) >= 0.9:
            phase = (math.atan2(s, c) % (2 * math.pi)) / (2 * math.pi) * period
            return period, phase
    return None


def refreshes_after(t, until, g):
    """Refreshes between `t` and `until`, the one at `until` not counted."""
    period, phase = g
    first = math.floor((t - phase) / period) + 1
    last = round((until - phase) / period)
    return last - first


def stage_stats(rows):
    out = {}
    for name, a, b, _ in STAGES:
        v = [(r[b] - r[a]) * 1000 for r in rows if r.get(a) is not None and r.get(b) is not None]
        if v:
            out[name] = {"n": len(v), "p50": pct(v, 50), "p95": pct(v, 95), "p99": pct(v, 99)}
    g = grid([r["t_shown"] for r in rows]) if rows else None
    if g:
        out["refresh_ms"] = g[0] * 1000
        late = [refreshes_after(r["commit"], r["t_shown"], g) for r in rows if r.get("commit")]
        out["refreshes_after_commit"] = {str(n): late.count(n) for n in sorted(set(late))}
        # Where in the refresh the key was posted, and the frame committed.
        out["post_phase"] = phases([r["t_post"] for r in rows], g)
        out["commit_phase"] = phases([r["commit"] for r in rows if r.get("commit")], g)
    out["joined"] = sum(1 for r in rows if r.get("t0") is not None)
    out["keys"] = len(rows)
    return out


def phases(times, g):
    """Fractions of the refresh after the last one, as p50 / p95."""
    period, phase = g
    v = [((t - phase) % period) / period for t in times]
    return {"p50": pct(v, 50), "p95": pct(v, 95)}


def table(s):
    lines = ["| stage | p50 | p95 | p99 | n |", "|---|---|---|---|---|"]
    for name, _, _, what in STAGES:
        if name in s:
            x = s[name]
            lines.append(f"| {what} | {x['p50']:.2f} | {x['p95']:.2f} | {x['p99']:.2f} | {x['n']} |")
    if "refresh_ms" in s:
        lines.append(
            f"\nRefresh {s['refresh_ms']:.2f} ms. Refreshes between commit and seen (0: the first after commit): "
            + ", ".join(f"{k}: {v}" for k, v in s["refreshes_after_commit"].items())
            + f". Commit at {s['commit_phase']['p50']:.2f} of the refresh (p50)."
        )
    elif s.get("keys"):
        # A ProMotion display shows a frame when it comes (variable refresh),
        # so its display times keep to no grid of 30, 60 or 120 Hz.
        lines.append("\nNo refresh grid in the display times (a variable refresh rate), so no refresh counts.")
    lines.append(f"Keys joined with Decline's log: {s['joined']} of {s['keys']}.")
    return "\n".join(lines)


def main():
    probe, app = read_jsonl(sys.argv[1]), read_jsonl(sys.argv[2])
    s = stage_stats(join(probe, app))
    print(table(s))
    print(json.dumps(s))


if __name__ == "__main__":
    main()
