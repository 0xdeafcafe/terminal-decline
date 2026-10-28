#!/usr/bin/env python3
"""Turns run.py's JSON lines into a Markdown table.

usage: summary.py RESULTS.jsonl     # appends to $GITHUB_STEP_SUMMARY when set

Exits 1 if any scenario didn't survive or no results were found.
"""
import json
import os
import sys


def fmt(v, unit=""):
    return "–" if v is None else f"{v}{unit}"


def main():
    rows = []
    with open(sys.argv[1]) as f:
        for line in f:
            line = line.strip()
            if line.startswith("{"):
                rows.append(json.loads(line))
    out = [
        "### Pressure scenarios",
        "",
        "| scenario | result | wall | CPU | peak footprint | final footprint | idle CPU | idle wake-ups/s |",
        "|---|---|--:|--:|--:|--:|--:|--:|",
    ]
    failed = []
    for r in rows:
        if "skipped" in r:
            out.append(f"| {r['scenario']} | skipped: {r['skipped']} | | | | | | |")
            continue
        ok = r.get("survived")
        if not ok:
            failed.append(r["scenario"])
        result = "survived" if ok else f"**died** (exit {r.get('exit')})"
        out.append(
            f"| {r['scenario']} | {result} | {fmt(r.get('wall_s'), ' s')} | {fmt(r.get('cpu_s'), ' s')} "
            f"| {fmt(r.get('peak_mb'), ' MB')} | {fmt(r.get('final_mb'), ' MB')} "
            f"| {fmt(r.get('idle_cpu_pct'), '%')} | {fmt(r.get('idle_wakeups_per_s'))} |"
        )
    if not rows:
        out.append("| (no results) | | | | | | | |")
    table = "\n".join(out) + "\n"
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(table)
    print(table)
    if failed:
        print(f"did not survive: {', '.join(failed)}", file=sys.stderr)
    return 1 if failed or not rows else 0


if __name__ == "__main__":
    sys.exit(main())
