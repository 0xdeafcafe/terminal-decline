#!/usr/bin/env python3
"""Keypress-to-present latency of Terminal Decline while other instances or
another tab of the same window flood, using the in-app latency harness
(scripts/latency/run.py, crates/term-app/src/latency.rs).

usage: latency_load.py [--app target/release/terminal-decline] [--keys 200]
                       [--loads idle,others-1,others-4,others-8,tab]
                       [--hz 120] [--flood big|yes] [--post] [--out DIR]

Loads:
- idle: nothing else running (of ours);
- others-N: N more Terminal Decline processes, each with a window flooding
  (`cat big.txt` in a loop, coloured log lines; `--flood yes` for `yes`),
  started before the keys and stopped after;
- tab: the measured window has a second tab, behind it, flooding the same
  way (opened with the test control `ESC ] 7770 ; tab`, so this run has
  `DECLINE_TEST_DUMP` set; the measured tab is brought back to the front).

Keys are typed into the measured window with the FIFO's `type` command by
default (no focus needed; t0 is then keyDown, as in DESIGN.md's later
tables); `--post` posts real key events instead, which needs the window
to get the focus. Prints p50/p95/p99 per stage, the flooders' CPU and
footprint, and writes summary.json and the raw logs to --out.
"""

import argparse
import importlib.util
import json
import os
import re
import shlex
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
spec = importlib.util.spec_from_file_location("latency_run", ROOT / "scripts/latency/run.py")
lat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lat)


def footprint_mb(pid):
    out = subprocess.run(["/usr/bin/footprint", "-p", str(pid)], capture_output=True, text=True).stdout
    m = re.search(r"phys_footprint:\s+([\d.]+)\s+(KB|MB|GB)", out)
    if not m:
        return None
    v = float(m.group(1))
    return v / 1024 if m.group(2) == "KB" else v * 1024 if m.group(2) == "GB" else v


def cpu_seconds(pid):
    out = subprocess.run(["ps", "-o", "cputime=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    if not out:
        return None
    total = 0.0
    for p in out.replace("-", ":").split(":"):
        total = total * 60 + float(p)
    return total


def flood_cmd(kind, work):
    if kind == "yes":
        return "exec yes"
    return f"while :; do cat {shlex.quote(str(work / 'big.txt'))}; done"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--app", default=str(ROOT / "target/release/terminal-decline"))
    ap.add_argument("--keys", type=int, default=200)
    ap.add_argument("--loads", default="idle,others-1,others-4,others-8,tab")
    ap.add_argument("--hz", type=int, default=120)
    ap.add_argument("--flood", default="big", choices=["big", "yes"])
    ap.add_argument("--post", action="store_true", help="post key events (needs focus) instead of typing")
    ap.add_argument("--out", default=str(ROOT / "target/latency-load"))
    ap.add_argument("--tag", default="")
    a = ap.parse_args()
    out_dir = Path(a.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    work = out_dir / "work"
    work.mkdir(exist_ok=True)
    if a.flood == "big" and not (work / "big.txt").exists():
        subprocess.run([sys.executable, str(HERE / "gen.py"), "big", str(work / "big.txt"), "20000000"], check=True)
    lat.TYPE = not a.post
    flood = flood_cmd(a.flood, work)
    marker_dir = Path(tempfile.mkdtemp(prefix="decline-tab-"))

    def program_for(scenario, marker=None):
        echo = [sys.executable, str(lat.HERE / "echo.py")]
        if scenario != "tab":
            return echo
        # The first tab opens a second one (the same program), which floods;
        # the first comes back to the front and echoes.
        first = marker_dir / "first"
        script = (
            f"if mkdir {shlex.quote(str(first))} 2>/dev/null; then "
            f"printf '\\033]7770;tab\\007'; sleep 1.5; printf '\\033]7770;front\\007'; "
            f"exec {' '.join(map(shlex.quote, echo))}; else {flood}; fi"
        )
        return ["/bin/sh", "-c", script]

    lat.program_for = program_for
    bundle = lat.make_bundle(Path(a.app), out_dir)
    summary_file = out_dir / "summary.json"
    summary = json.loads(summary_file.read_text()) if summary_file.exists() else {}
    loads = [l for l in a.loads.split(",") if l]
    try:
        for load in loads:
            flooders = []
            lat.EXTRA_ENV.clear()
            if load.startswith("others-"):
                n = int(load.split("-")[1])
                for i in range(n):
                    flooders.append(
                        subprocess.Popen(
                            [a.app, "-e", "sh", "-c", flood],
                            stdout=subprocess.DEVNULL,
                            stderr=open(out_dir / f"flooder-{load}-{i}.stderr", "w"),
                            start_new_session=True,
                        )
                    )
                time.sleep(3)
            elif load == "tab":
                lat.EXTRA_ENV["DECLINE_TEST_DUMP"] = str(out_dir / "tab-dump.jsonl")
                for p in marker_dir.iterdir():
                    p.rmdir()
            cpu0 = {p.pid: cpu_seconds(p.pid) for p in flooders}
            t0 = time.time()
            scenario = "tab" if load == "tab" else "idle"
            name = f"{load}-{a.hz}hz{a.tag}"
            rows, missed = lat.run_inapp(bundle, scenario, a.hz, a.keys, out_dir)
            # run_inapp names its log after the scenario: keep this one.
            src = out_dir / f"inapp-{scenario}-{a.hz}hz.jsonl"
            if src.exists():
                src.rename(out_dir / f"inapp-{name}.jsonl")
            dt = time.time() - t0
            s = lat.summarise(rows)
            s["missed"] = missed
            alive = [p for p in flooders if p.poll() is None]
            s["flooders"] = len(flooders)
            s["flooders_alive"] = len(alive)
            cpus = [((cpu_seconds(p.pid) or 0) - (cpu0[p.pid] or 0)) / dt * 100 for p in alive]
            fps = [footprint_mb(p.pid) or 0 for p in alive]
            s["flooder_cpu_pct_each"] = round(sum(cpus) / len(cpus), 1) if cpus else None
            s["flooder_footprint_mb_total"] = round(sum(fps), 1) if fps else None
            s["load_avg"] = os.getloadavg()[0]
            summary[name] = s
            lat.show(
                f"{name}: {len(rows)} keys, {s['dropped']} not presented, {missed} without a frame; "
                f"flooders {len(alive)}/{len(flooders)} alive, {s['flooder_cpu_pct_each']}% CPU each, "
                f"{s['flooder_footprint_mb_total']} MB total; load {s['load_avg']:.1f}",
                [(label, s[k]) for k, _, _, label in lat.STAGES],
            )
            for p in flooders:
                try:
                    os.killpg(p.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            for p in flooders:
                try:
                    p.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    p.kill()
            time.sleep(2)
    finally:
        subprocess.run([lat.LSREGISTER, "-u", str(bundle)], capture_output=True)
        summary_file.write_text(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
