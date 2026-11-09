#!/usr/bin/env python3
"""Where Terminal Decline's `cat` of 100 MB goes, measured as compare/run.py
measures it (the same launch, config, driver and timing), with variants
beside it to split the time up:

    scripts/compare/profile_cat.py --bundle DIR/Terminal\\ Decline.app [--reps 5] [--out DIR]

In one window, after `clear; seq 1 10000` as in run.py, each rep runs
every variant once (interleaved):

- `noop`: `:`, the driver's own cost (two perl clock reads);
- `devnull`: `cat big > /dev/null`, cat without the terminal;
- `compare`: `cat big` with the footprint sampled meanwhile, as run.py;
- `quiet`: `cat big` with nothing sampled;
- `stty38400`: `cat big` with the PTY's speed at 38400 baud (the kernel
  sizes a PTY's output queue by its speed), put back after;

then one `cat big` under `sample` (1 ms) for the profile, saved as
OUT/sample.txt. Rows go to OUT/profile.jsonl; medians are printed.
"""

import argparse
import importlib.util
import json
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("compare_run", HERE / "run.py")
cmp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cmp)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bundle", required=True)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--out", default=str(cmp.ROOT / "target/profile-cat"))
    ap.add_argument("--variants", default="noop,devnull,compare,quiet,stty38400")
    args = ap.parse_args()
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)

    ctx = cmp.Ctx()
    ctx.idle = 30
    ctx.out_dir = out
    ctx.screenshots = True
    ctx.work = Path(tempfile.mkdtemp(prefix="decline-profile-"))
    ctx.helper = cmp.build_swift(HERE / "helper.swift", out / "helper")
    ctx.big = out / "big.txt"
    if not ctx.big.exists():
        subprocess.run([sys.executable, str(cmp.ROOT / "scripts/pressure/gen.py"), "big", str(ctx.big)], check=True)
    ctx.decline_bundle = Path(args.bundle).resolve()
    cmp.MAX_LOAD[0] = 4

    commands = {
        "noop": (":", False),
        "devnull": (f"cat {ctx.big} > /dev/null", False),
        "compare": (f"cat {ctx.big}", True),
        "quiet": (f"cat {ctx.big}", False),
        "stty38400": (f"stty speed 38400 > /dev/null; cat {ctx.big}; stty speed 9600 > /dev/null", True),
    }
    variants = [v for v in args.variants.split(",") if v]
    rows = []
    t = cmp.Decline(ctx)
    try:
        time.sleep(5)
        t.run(f"clear; seq 1 {cmp.SCROLLBACK}")
        time.sleep(2)
        ws = cmp.windowserver_pid()
        print("pids", t.pids(), flush=True)

        def measure(name, rep):
            command, sampled = commands[name]
            cmp.wait_load()
            cpu0, ws0 = t.cpu(), cmp.cpu_seconds(ws)
            peak = [0.0]

            def sample():
                peak[0] = max(peak[0], t.footprint())

            s, e = t.run(command, timeout=900, during=sample if sampled else None)
            time.sleep(1)
            cpu1, ws1 = t.cpu(), cmp.cpu_seconds(ws)
            row = {
                "variant": name,
                "rep": rep,
                "wall_s": round(e - s, 3),
                "cpu_s": round(cpu1 - cpu0, 3),
                "ws_cpu_s": round(ws1 - ws0, 3),
                "peak_mb": peak[0] or None,
            }
            print(json.dumps(row), flush=True)
            rows.append(row)

        for rep in range(args.reps):
            for name in variants:
                measure(name, rep)

        # The profile: every process of ours, 1 ms samples.
        pids = t.pids()
        cmp.wait_load()
        procs = [
            subprocess.Popen(["sudo", "-n", "/usr/bin/sample", str(p), "3", "1", "-mayDie", "-file", str(out / f"sample-{p}.txt")],
                             stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
            for p in pids
        ]
        time.sleep(0.7)
        s, e = t.run(f"cat {ctx.big}", timeout=900)
        print(json.dumps({"variant": "sampled", "wall_s": round(e - s, 3)}), flush=True)
        for p in procs:
            p.wait(timeout=60)
    finally:
        t.stop()

    with open(out / "profile.jsonl", "a") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
    print("\nmedians (min-max):")
    for name in variants:
        rs = [r for r in rows if r["variant"] == name]
        for k in ("wall_s", "cpu_s", "ws_cpu_s"):
            v = [r[k] for r in rs if r[k] is not None]
            if v:
                print(f"  {name:10} {k:9} {statistics.median(v):.3f} ({min(v):.3f}-{max(v):.3f})")


if __name__ == "__main__":
    main()
