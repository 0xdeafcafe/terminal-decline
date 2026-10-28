#!/usr/bin/env python3
"""Summarises criterion results (target/criterion/**/new/*.json).

Wall-clock numbers vary between CI machines, so this never fails: it posts a
Markdown table (to $GITHUB_STEP_SUMMARY when set, else stdout) and writes
history files for github-action-benchmark:

    python3 scripts/bench-summary.py [--criterion DIR]
        [--throughput out.json]   # customBiggerIsBetter, MiB/s (benches with a throughput)
        [--time out.json]         # customSmallerIsBetter, µs per iteration (all benches)
"""
import argparse
import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MIB = 1024 * 1024


def collect(criterion_dir):
    out = []
    for dirpath, _, files in os.walk(criterion_dir):
        if os.path.basename(dirpath) != "new" or "benchmark.json" not in files or "estimates.json" not in files:
            continue
        with open(os.path.join(dirpath, "benchmark.json")) as f:
            bench = json.load(f)
        with open(os.path.join(dirpath, "estimates.json")) as f:
            est = json.load(f)
        mean_ns = est["mean"]["point_estimate"]
        row = {"name": bench["full_id"], "mean_ns": mean_ns, "std_ns": est["std_dev"]["point_estimate"]}
        tp = bench.get("throughput") or {}
        if "Bytes" in tp or "BytesDecimal" in tp:
            n = tp.get("Bytes", tp.get("BytesDecimal"))
            row["mib_s"] = n / MIB / (mean_ns / 1e9)
        elif "Elements" in tp:
            row["elem_s"] = tp["Elements"] / (mean_ns / 1e9)
        out.append(row)
    return sorted(out, key=lambda r: r["name"])


def human_time(ns):
    for unit, scale in (("s", 1e9), ("ms", 1e6), ("µs", 1e3)):
        if ns >= scale:
            return f"{ns / scale:.2f} {unit}"
    return f"{ns:.0f} ns"


def markdown(rows):
    lines = [
        "### Criterion benchmarks (wall clock, trend only)",
        "",
        "| benchmark | mean | ± std dev | throughput |",
        "|---|--:|--:|--:|",
    ]
    for r in rows:
        tp = f"{r['mib_s']:.1f} MiB/s" if "mib_s" in r else f"{r['elem_s']:.0f} elem/s" if "elem_s" in r else ""
        lines.append(f"| {r['name']} | {human_time(r['mean_ns'])} | {human_time(r['std_ns'])} | {tp} |")
    if not rows:
        lines.append("| (no results found) | | | |")
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--criterion", default=os.path.join(ROOT, "target", "criterion"))
    ap.add_argument("--throughput")
    ap.add_argument("--time")
    args = ap.parse_args()
    rows = collect(args.criterion)
    table = markdown(rows)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(table)
    print(table)
    if args.throughput:
        data = [{"name": r["name"], "unit": "MiB/s", "value": round(r["mib_s"], 2)} for r in rows if "mib_s" in r]
        data += [{"name": r["name"], "unit": "elem/s", "value": round(r["elem_s"])} for r in rows if "elem_s" in r]
        with open(args.throughput, "w") as f:
            json.dump(data, f, indent=2)
    if args.time:
        data = [{"name": r["name"], "unit": "µs", "value": round(r["mean_ns"] / 1e3, 3)} for r in rows]
        with open(args.time, "w") as f:
            json.dump(data, f, indent=2)


if __name__ == "__main__":
    main()
