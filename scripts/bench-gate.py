#!/usr/bin/env python3
"""Gate on scrollback bytes per line, which are deterministic.

Runs `cargo run --release -p term-core --example scrollback_mem` (or reads its
saved output), compares every line against scripts/bench-limits.json and
exits 1 if any value is over its limit, a limit has no measurement, or a
measurement has no limit.

    python3 scripts/bench-gate.py                       # run the example
    python3 scripts/bench-gate.py --input out.txt       # use saved output
    python3 scripts/bench-gate.py --json bench.json     # also write history
    python3 scripts/bench-gate.py --self-test           # test this script

With $GITHUB_STEP_SUMMARY set, a Markdown table is appended to it. --json
writes github-action-benchmark's `customSmallerIsBetter` format.
"""
import argparse
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
LIMITS = os.path.join(HERE, "bench-limits.json")
COMMAND = ["cargo", "run", "--release", "--locked", "-q", "-p", "term-core", "--example", "scrollback_mem"]
LINE = re.compile(r"^(?P<name>\S.*?)\s+(?P<value>\d+)\s+bytes/line\b")


def parse(text):
    """Example output -> {label: bytes per line}."""
    out = {}
    for line in text.splitlines():
        m = LINE.match(line)
        if m:
            out[m.group("name").strip()] = int(m.group("value"))
    return out


def check(measured, limits):
    """Returns (rows, failures). rows: (name, value, limit, ok)."""
    rows, failures = [], []
    for name in sorted(set(measured) | set(limits)):
        value, limit = measured.get(name), limits.get(name)
        if value is None:
            failures.append(f"{name}: no measurement (example output changed?)")
            ok = False
        elif limit is None:
            failures.append(f"{name}: no limit in {os.path.relpath(LIMITS, ROOT)}")
            ok = False
        else:
            ok = value <= limit
            if not ok:
                failures.append(f"{name}: {value} bytes/line is over the limit of {limit}")
        rows.append((name, value, limit, ok))
    return rows, failures


def markdown(rows):
    out = ["### Scrollback bytes per line", "", "| output | bytes/line | limit | |", "|---|--:|--:|---|"]
    for name, value, limit, ok in rows:
        state = "ok" if ok else "**missing**" if value is None or limit is None else "**over**"
        out.append(f"| {name} | {'–' if value is None else value} | {'–' if limit is None else limit} | {state} |")
    return "\n".join(out) + "\n"


def history(measured):
    return [{"name": f"scrollback {name}", "unit": "bytes/line", "value": v} for name, v in sorted(measured.items())]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--input", help="saved output of the example instead of running it")
    ap.add_argument("--limits", default=LIMITS)
    ap.add_argument("--json", help="write customSmallerIsBetter JSON here")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    if args.input:
        with open(args.input) as f:
            text = f.read()
    else:
        text = subprocess.run(COMMAND, cwd=ROOT, check=True, capture_output=True, text=True).stdout
    print(text, end="")
    with open(args.limits) as f:
        limits = json.load(f)["scrollback_bytes_per_line"]

    measured = parse(text)
    rows, failures = check(measured, limits)
    table = markdown(rows)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(table)
    else:
        print(table)
    if args.json:
        with open(args.json, "w") as f:
            json.dump(history(measured), f, indent=2)
    for msg in failures:
        print(f"::error title=scrollback bytes per line::{msg}" if os.environ.get("GITHUB_ACTIONS") else f"FAIL {msg}")
    return 1 if failures else 0


def self_test():
    sample = (
        "plain 40 chars                   52 bytes/line  (0.5 MB for 10k lines)\n"
        "ls --color                       93 bytes/line  (0.9 MB for 10k lines)\n"
        "some other noise\n"
    )
    assert parse(sample) == {"plain 40 chars": 52, "ls --color": 93}, parse(sample)
    rows, failures = check({"a": 10, "b": 30}, {"a": 10, "b": 20})
    assert [r[3] for r in rows] == [True, False] and len(failures) == 1, (rows, failures)
    _, failures = check({"a": 1}, {"a": 2, "b": 2})
    assert failures == ["b: no measurement (example output changed?)"], failures
    _, failures = check({"a": 1, "new": 5}, {"a": 2})
    assert len(failures) == 1 and failures[0].startswith("new: no limit"), failures
    assert history({"x": 3}) == [{"name": "scrollback x", "unit": "bytes/line", "value": 3}]
    assert "| b | 30 | 20 | **over** |" in markdown(rows), markdown(rows)
    with open(LIMITS) as f:
        limits = json.load(f)["scrollback_bytes_per_line"]
    assert all(isinstance(v, int) and v > 0 for v in limits.values()), limits
    print("bench-gate self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
