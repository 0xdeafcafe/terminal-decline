#!/usr/bin/env python3
"""Where Terminal Decline's idle footprint goes, and what a change did to it.

    scripts/memory/breakdown.py --bundle LABEL=PATH [--bundle LABEL=PATH ...]
                                [--rounds 5] [--bare] [--out target/memory]

Launches each bundle the way scripts/compare/run.py does (its Decline
class: Menlo 13, 100×30 cells, 10k lines of scrollback, a config in a temp
folder), interleaved round by round, and per launch records
`/usr/bin/footprint -f bytes` (phys_footprint to the byte, and each
category's dirty bytes) at the compare's three points:

- idle: 5 s after the window is up;
- seq: after `clear; seq 1 10000`, 2 s later;
- tabs: a second launch with four tabs (three opened after start), 6 s
  after the last.

In round 1 it also keeps, per bundle and point, `footprint -v -w` (every
region, with the library that owns it), `vmmap --summary` and `heap -s`,
and the thread count.

--bare: the same for scripts/memory/bare.swift, a bare AppKit app with a
CAMetalLayer window (1 and 4 tabs, and 1 with Menlo rasterised): the
floor any Metal terminal window stands on.

The report (OUT/report.md, also printed) has each bundle's median total
at each point, the spread, and the median of each category, side by side.
Raw rows go to OUT/rows.jsonl. For a GitHub runner, see
.github/workflows/memory.yml.
"""

import argparse
import importlib.util
import json
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

_spec = importlib.util.spec_from_file_location("compare_run", ROOT / "scripts/compare/run.py")
cmp = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cmp)


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, file=sys.stderr, flush=True)


ROW = re.compile(r"^\s*(\d+) B\s+(\d+) B\s+(\d+) B\s+(\d+)\s+(.+?)\s*$")


def footprint(pid):
    """(phys_footprint bytes, {category: dirty bytes})."""
    out = subprocess.run(["/usr/bin/footprint", "-f", "bytes", "-p", str(pid)], capture_output=True, text=True).stdout
    m = re.search(r"phys_footprint:\s+(\d+) B", out)
    cats = {}
    for line in out.splitlines():
        r = ROW.match(line)
        if r and r.group(5) != "TOTAL":
            cats[r.group(5)] = cats.get(r.group(5), 0) + int(r.group(1))
    return (int(m.group(1)) if m else 0), cats


def measure(pids):
    total, cats = 0, {}
    for p in pids:
        t, c = footprint(p)
        total += t
        for k, v in c.items():
            cats[k] = cats.get(k, 0) + v
    return total, cats


def threads(pid):
    out = subprocess.run(["ps", "-M", "-p", str(pid)], capture_output=True, text=True).stdout
    return max(0, len(out.splitlines()) - 1)


def keep_details(out_dir, name, pid):
    """footprint -v, vmmap --summary and heap -s of one process, to files."""
    for suffix, cmd in (
        ("footprint-v.txt", ["/usr/bin/footprint", "-v", "-w", "-f", "bytes", "-p", str(pid)]),
        ("vmmap.txt", ["/usr/bin/vmmap", "--summary", str(pid)]),
        ("heap.txt", ["/usr/bin/heap", "-s", str(pid)]),
    ):
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
            text = r.stdout + (f"\n[stderr]\n{r.stderr}" if r.stderr.strip() else "")
        except subprocess.TimeoutExpired:
            text = "timed out"
        (out_dir / f"{name}-{suffix}").write_text(text)


def run_decline(ctx, label, bundle, rnd, details):
    rows = []
    ctx.decline_bundle = bundle
    t = cmp.Decline(ctx)
    try:
        time.sleep(5)
        pids = t.pids()
        total, cats = measure(pids)
        rows.append({"label": label, "point": "idle", "round": rnd, "bytes": total, "cats": cats, "threads": threads(pids[0])})
        if details:
            keep_details(ctx.out_dir, f"{label}-idle", pids[0])
        t.run(f"clear; seq 1 {cmp.SCROLLBACK}")
        time.sleep(2)
        total, cats = measure(t.pids())
        rows.append({"label": label, "point": "seq", "round": rnd, "bytes": total, "cats": cats})
        if details:
            keep_details(ctx.out_dir, f"{label}-seq", pids[0])
    finally:
        t.stop()
    t = cmp.Decline(ctx, tabs=True)
    try:
        time.sleep(3)
        for n in range(1, 4):
            t.open_tab(n)
            time.sleep(1.5)
        time.sleep(6)
        pids = t.pids()
        total, cats = measure(pids)
        rows.append({"label": label, "point": "tabs", "round": rnd, "bytes": total, "cats": cats, "tabs": t.tab_count(), "threads": threads(pids[0])})
        if details:
            keep_details(ctx.out_dir, f"{label}-tabs", pids[0])
    finally:
        t.stop()
    return rows


def run_bare(ctx, exe, rnd, details):
    rows = []
    for point, args in (("idle", ["1"]), ("text", ["1", "text"]), ("tabs", ["4"])):
        p = subprocess.Popen([str(exe), *args], stdout=subprocess.PIPE, text=True)
        try:
            p.stdout.readline()  # "ready"
            time.sleep(5)
            total, cats = measure([p.pid])
            rows.append({"label": "bare", "point": point, "round": rnd, "bytes": total, "cats": cats, "threads": threads(p.pid)})
            if details:
                keep_details(ctx.out_dir, f"bare-{point}", p.pid)
        finally:
            p.terminate()
            try:
                p.wait(5)
            except subprocess.TimeoutExpired:
                p.kill()
            time.sleep(0.5)
    return rows


def mb(b):
    return f"{b / 2**20:.2f}"


def report(rows):
    labels = list(dict.fromkeys(r["label"] for r in rows))
    points = list(dict.fromkeys(r["point"] for r in rows))
    lines = ["### Footprint (MiB, phys_footprint): median, min – max, runs", ""]
    lines.append("| point | " + " | ".join(labels) + " |")
    lines.append("|---|" + "---|" * len(labels))
    for pt in points:
        cells = []
        for lb in labels:
            v = [r["bytes"] for r in rows if r["label"] == lb and r["point"] == pt]
            cells.append(f"**{mb(statistics.median(v))}** ({mb(min(v))} – {mb(max(v))}, {len(v)})" if v else "")
        lines.append(f"| {pt} | " + " | ".join(cells) + " |")
    for pt in points:
        group = [r for r in rows if r["point"] == pt]
        cats = sorted({k for r in group for k in r["cats"]})
        med = {}
        for lb in labels:
            for c in cats:
                v = [r["cats"].get(c, 0) for r in group if r["label"] == lb]
                med[lb, c] = statistics.median(v) if v else None
        cats = [c for c in cats if any((med[lb, c] or 0) >= 32 * 1024 for lb in labels)]
        cats.sort(key=lambda c: -max(med[lb, c] or 0 for lb in labels))
        lines += ["", f"### {pt}: dirty KiB by category (median)", ""]
        lines.append("| category | " + " | ".join(labels) + " |")
        lines.append("|---|" + "---:|" * len(labels))
        for c in cats:
            lines.append(f"| {c} | " + " | ".join("" if med[lb, c] is None else f"{med[lb, c] / 1024:.0f}" for lb in labels) + " |")
    th = [(r["label"], r["point"], r["threads"]) for r in rows if r.get("threads") and r["round"] == 1]
    if th:
        lines += ["", "Threads (round 1): " + ", ".join(f"{a} {b} {c}" for a, b, c in th)]
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bundle", action="append", default=[], metavar="LABEL=PATH")
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--bare", action="store_true")
    ap.add_argument("--out", default=str(ROOT / "target/memory"))
    ap.add_argument("--report", action="store_true", help="only print the report of OUT/rows.jsonl")
    args = ap.parse_args()
    out_dir = Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    rows_file = out_dir / "rows.jsonl"
    if args.report:
        rows = [json.loads(l) for l in rows_file.read_text().splitlines() if l.strip()]
        print(report(rows), end="")
        return
    bundles = [(lb, Path(p).resolve()) for lb, p in (b.split("=", 1) for b in args.bundle)]
    ctx = cmp.Ctx()
    ctx.idle = 0
    ctx.out_dir = out_dir
    ctx.screenshots = True
    ctx.work = Path(tempfile.mkdtemp(prefix="decline-memory-"))
    ctx.helper = cmp.build_swift(cmp.HERE / "helper.swift", out_dir / "helper")
    bare = cmp.build_swift(HERE / "bare.swift", out_dir / "bare") if args.bare else None
    rows = []
    try:
        for rnd in range(1, args.rounds + 1):
            for label, bundle in bundles:
                cmp.wait_load()
                log(f"round {rnd}: {label}")
                try:
                    got = run_decline(ctx, label, bundle, rnd, rnd == 1)
                except Exception as e:  # keep going; the log says why
                    log(f"{label} round {rnd} failed: {e}")
                    got = []
                for r in got:
                    log(json.dumps({k: v for k, v in r.items() if k != "cats"}))
                rows += got
            if bare:
                cmp.wait_load()
                log(f"round {rnd}: bare")
                got = run_bare(ctx, bare, rnd, rnd == 1)
                for r in got:
                    log(json.dumps({k: v for k, v in r.items() if k != "cats"}))
                rows += got
            with open(rows_file, "w") as f:
                for r in rows:
                    f.write(json.dumps(r) + "\n")
    finally:
        for _, bundle in bundles:
            subprocess.run([cmp.LSREGISTER, "-u", str(bundle)], capture_output=True)
        shutil.rmtree(ctx.work, ignore_errors=True)
    text = report(rows)
    (out_dir / "report.md").write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
