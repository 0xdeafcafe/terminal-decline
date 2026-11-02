#!/usr/bin/env python3
"""The perf watch: is Decline's memory or CPU creeping up as features land?

A report, never a gate (docs/perf-watch.md). Three steps, run by
.github/workflows/perf-watch.yml:

    perf_watch.py measure --out perf.json        # build, measure, write numbers
    perf_watch.py report --current perf.json --history runs.jsonl [--issues]
    perf_watch.py record --current perf.json --history runs.jsonl
    perf_watch.py --self-test

`measure` writes perf.json:
  - alloc.<input>.*: allocations, most heap and heap kept while parsing
    the parse bench's inputs (term-core example alloc_count);
  - scrollback.<output>: scrollback bytes per line (example scrollback_mem,
    as scripts/bench-gate.py);
  - size.*: the bundle's library, launcher, pin stub and whole bundle;
  - app.*: a release bundle started in the background under the test
    control, fed a scrollback flood (scripts/pressure/gen.py big), then
    its footprint (/usr/bin/footprint, phys_footprint), threads, and CPU
    time, wake-ups and context switches over a minute sitting idle.
A part that fails is left out with a note in `errors`; the rest still runs.

`report` compares with the previous run in the history (and with the run
from about a week before, to catch slow creep), appends a table to
$GITHUB_STEP_SUMMARY, and with --issues opens, updates or closes the one
`performance` issue this job owns (needs `gh` and GH_TOKEN). `record`
appends perf.json to the history. Every step exits 0 unless its arguments
are wrong: problems are warnings in the summary.

Python 3, standard library only.
"""
import argparse
import ctypes
import datetime
import fnmatch
import importlib.util
import json
import os
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
THRESHOLDS = os.path.join(HERE, "thresholds.json")
MARKER = "<!-- perf-watch -->"
LABEL = "performance"
SCHEMA = 1

FLOOD_BYTES = 20_000_000
SETTLE_SECONDS = 10
IDLE_SECONDS = 60
LAUNCH_TIMEOUT = 180

# Sections of the table, in order: metric name prefix, heading.
SECTIONS = [
    ("alloc.", "Allocations parsing the parse bench's inputs (4 MB each, 160x50, 10k lines of history)"),
    ("scrollback.", "Scrollback bytes per line"),
    ("app.", "App after a 20 MB scrollback flood, then sitting idle for a minute"),
    ("size.", "Binary size"),
]
# The app's rows, in order.
APP_ORDER = [
    "app.history_bytes",
    "app.footprint_after_flood",
    "app.footprint_idle",
    "app.threads",
    "app.idle_cpu_ms_per_min",
    "app.idle_wakeups_per_min",
    "app.idle_context_switches_per_min",
]


def warn(msg):
    if os.environ.get("GITHUB_ACTIONS"):
        print(f"::warning title=perf watch::{msg}")
    else:
        print(f"warning: {msg}", file=sys.stderr)


def metric(value, unit, label):
    return {"value": value, "unit": unit, "label": label}


# ---------------------------------------------------------------- measure


def cargo(*args):
    cmd = ["cargo", *args]
    print("+", " ".join(cmd), flush=True)
    return subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, text=True).stdout


def parse_alloc(text):
    """alloc_count's JSON lines -> metrics."""
    out = {}
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        row = json.loads(line)
        name = row["input"]
        out[f"alloc.{name}.allocs"] = metric(row["allocs"], "count", f"{name}: allocations")
        out[f"alloc.{name}.peak_heap"] = metric(row["peak_heap"], "bytes", f"{name}: most heap held")
        out[f"alloc.{name}.retained_heap"] = metric(row["retained_heap"], "bytes", f"{name}: heap kept")
    return out


def bench_gate():
    """scripts/bench-gate.py as a module, for its parser."""
    spec = importlib.util.spec_from_file_location("bench_gate", os.path.join(ROOT, "scripts", "bench-gate.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def parse_scrollback(text):
    return {f"scrollback.{name}": metric(v, "bytes/line", name) for name, v in bench_gate().parse(text).items()}


def measure_alloc():
    return parse_alloc(cargo("run", "--release", "--locked", "-q", "-p", "term-core", "--example", "alloc_count"))


def measure_scrollback():
    return parse_scrollback(cargo("run", "--release", "--locked", "-q", "-p", "term-core", "--example", "scrollback_mem"))


def build_bundle(out_dir):
    """Release library, launcher and stub, bundled ad hoc; the .app's path."""
    cargo("build", "--release", "--locked", "-p", "term-dylib")
    cargo("build", "--locked", "-p", "term-pin", "--bins", "--profile", "release-stub")
    env = {k: v for k, v in os.environ.items() if k not in ("DOGFOOD", "HARDENED", "CODESIGN_IDENTITY", "SPARKLE_REQUIRED")}
    made = subprocess.run(
        ["/bin/bash", os.path.join(ROOT, "scripts", "bundle.sh"), out_dir, "0.0.0", "1"],
        cwd=ROOT,
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    lines = made.stdout.strip().splitlines()
    app = lines[-1].strip() if lines else ""
    if not app.endswith(".app") or not os.path.isdir(app):
        raise RuntimeError(f"bundle.sh printed no bundle: {made.stdout!r}")
    return app


def tree_bytes(path):
    total = 0
    for dirpath, _, files in os.walk(path):
        for f in files:
            p = os.path.join(dirpath, f)
            if not os.path.islink(p):
                total += os.path.getsize(p)
    return total


def measure_sizes(app):
    c = os.path.join(app, "Contents")
    return {
        "size.dylib": metric(os.path.getsize(os.path.join(c, "Frameworks", "libterminal_decline.dylib")), "bytes", "libterminal_decline.dylib"),
        "size.launcher": metric(os.path.getsize(os.path.join(c, "MacOS", "terminal-decline")), "bytes", "launcher"),
        "size.stub": metric(os.path.getsize(os.path.join(c, "Helpers", "terminal-decline-pin")), "bytes", "pin stub"),
        "size.bundle": metric(tree_bytes(app), "bytes", "whole bundle (without Sparkle)"),
    }


class Proc:
    """Reads another process's counters through libproc (no sudo needed
    for our own processes)."""

    def __init__(self):
        self.lib = ctypes.CDLL("/usr/lib/libSystem.B.dylib")

        class Timebase(ctypes.Structure):
            _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]

        tb = Timebase()
        self.lib.mach_timebase_info(ctypes.byref(tb))
        self.ns_per_tick = tb.numer / tb.denom

    def sample(self, pid):
        """CPU ns, idle and interrupt wake-ups, phys_footprint, threads and
        context switches so far."""
        buf = ctypes.create_string_buffer(1024)
        # RUSAGE_INFO_V4; rusage_info_v4 starts with a 16-byte uuid.
        if self.lib.proc_pid_rusage(ctypes.c_int(pid), ctypes.c_int(4), buf) != 0:
            raise OSError(f"proc_pid_rusage({pid}) failed")
        user, system, idle_w, int_w, _pageins, _wired, _resident, footprint = struct.unpack_from("<16x8Q", buf.raw)
        info = ctypes.create_string_buffer(96)
        # PROC_PIDTASKINFO: struct proc_taskinfo, 6 u64 then 12 i32.
        if self.lib.proc_pidinfo(ctypes.c_int(pid), ctypes.c_int(4), ctypes.c_uint64(0), info, ctypes.c_int(96)) != 96:
            raise OSError(f"proc_pidinfo({pid}) failed")
        ints = struct.unpack_from("<6Q12i", info.raw)[6:]
        return {
            "time": time.monotonic(),
            "cpu_ns": (user + system) * self.ns_per_tick,
            "wakeups": idle_w + int_w,
            "phys_footprint": footprint,
            "csw": ints[8],
            "threads": ints[9],
        }


def footprint_tool(pid):
    """phys_footprint in bytes from /usr/bin/footprint, or None."""
    try:
        out = subprocess.run(
            ["/usr/bin/footprint", "-p", str(pid), "-f", "bytes", "--noCategories"], capture_output=True, text=True, timeout=60
        ).stdout
    except (OSError, subprocess.TimeoutExpired):
        return None
    return parse_footprint(out)


def parse_footprint(text):
    m = re.search(r"Footprint:\s*([\d.]+)\s*(B|KB|MB|GB)\b", text)
    if not m:
        return None
    scale = {"B": 1, "KB": 1 << 10, "MB": 1 << 20, "GB": 1 << 30}[m.group(2)]
    return int(float(m.group(1)) * scale)


def osc(cmd):
    return f"printf '\\033]7770;{cmd}\\007'; "


def wait_for(pred, timeout, child):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return True
        if child.poll() is not None:
            return False
        time.sleep(0.5)
    return pred()


def read_dumps(path):
    try:
        with open(path) as f:
            return [json.loads(line) for line in f if line.strip()]
    except (OSError, ValueError):
        return []


def measure_app(app, idle_seconds=IDLE_SECONDS):
    """Start the bundle in the background under the test control, flood it,
    let it sit idle, and read its counters from outside."""
    proc = Proc()
    tmp = tempfile.mkdtemp(prefix="perf-watch-")
    try:
        os.makedirs(os.path.join(tmp, "config"))
        fixture = os.path.join(tmp, "flood.txt")
        subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "pressure", "gen.py"), "big", fixture, str(FLOOD_BYTES)], check=True)
        fifo = os.path.join(tmp, "go")
        os.mkfifo(fifo)
        dump = os.path.join(tmp, "dump.jsonl")
        # `read` is a shell builtin: the shell waits on the pipe with no
        # process coming and going, so nothing in the tab changes while
        # the app sits idle.
        script = f"cat '{fixture}'; {osc('dump')}read x < '{fifo}'; {osc('dump')}{osc('quit')}sleep 30"
        env = dict(os.environ, XDG_CONFIG_HOME=os.path.join(tmp, "config"), DECLINE_TEST_DUMP=dump)
        env.pop("DECLINE_TEST_FOREGROUND", None)
        launcher = os.path.join(app, "Contents", "MacOS", "terminal-decline")
        child = subprocess.Popen([launcher, "-e", "/bin/sh", "-c", script], cwd=tmp, env=env)
        try:
            if not wait_for(lambda: len(read_dumps(dump)) >= 1, LAUNCH_TIMEOUT, child):
                raise RuntimeError(f"no dump after the flood within {LAUNCH_TIMEOUT}s (exit status {child.poll()})")
            flooded = read_dumps(dump)[0]
            time.sleep(SETTLE_SECONDS)
            a = proc.sample(child.pid)
            fp_flood = footprint_tool(child.pid) or a["phys_footprint"]
            time.sleep(idle_seconds)
            b = proc.sample(child.pid)
            fp_idle = footprint_tool(child.pid) or b["phys_footprint"]
            minutes = (b["time"] - a["time"]) / 60
            out = {
                "app.history_bytes": metric(sum(p.get("history_bytes", 0) for p in flooded.get("panes", [])), "bytes", "history held"),
                "app.footprint_after_flood": metric(fp_flood, "bytes", f"footprint {SETTLE_SECONDS}s after the flood"),
                "app.footprint_idle": metric(fp_idle, "bytes", "footprint after the idle minute"),
                "app.threads": metric(b["threads"], "threads", "threads after the idle minute"),
                "app.idle_cpu_ms_per_min": metric(round((b["cpu_ns"] - a["cpu_ns"]) / 1e6 / minutes, 1), "ms/min", "idle CPU time"),
                "app.idle_wakeups_per_min": metric(round((b["wakeups"] - a["wakeups"]) / minutes, 1), "per min", "idle wake-ups"),
                "app.idle_context_switches_per_min": metric(round((b["csw"] - a["csw"]) / minutes, 1), "per min", "idle context switches"),
            }
            # Stop the shell's `read`; the app then quits through the test
            # control. O_NONBLOCK: fails rather than waits if nobody reads.
            try:
                fd = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
                os.write(fd, b"go\n")
                os.close(fd)
                child.wait(timeout=30)
            except (OSError, subprocess.TimeoutExpired):
                pass
            return out
        finally:
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def git(*args):
    try:
        return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return ""


def cmd_measure(args):
    perf = {
        "schema": SCHEMA,
        "commit": os.environ.get("GITHUB_SHA") or git("rev-parse", "HEAD"),
        "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "event": os.environ.get("GITHUB_EVENT_NAME", "local"),
        "run": os.environ.get("GITHUB_RUN_ID", ""),
        "runner": f"{os.uname().sysname} {os.uname().release} {os.uname().machine}",
        "metrics": {},
        "errors": [],
    }

    def part(name, fn):
        start = time.monotonic()
        try:
            perf["metrics"].update(fn())
            print(f"{name}: ok in {time.monotonic() - start:.0f}s", flush=True)
            return True
        except subprocess.CalledProcessError as e:
            tail = "\n".join((e.stderr or "").strip().splitlines()[-15:])
            perf["errors"].append(f"{name}: {' '.join(map(str, e.cmd))} exited {e.returncode}\n{tail}")
        except Exception as e:  # noqa: BLE001 - a report, never a gate
            perf["errors"].append(f"{name}: {type(e).__name__}: {e}")
        warn(perf["errors"][-1].splitlines()[0])
        return False

    if "alloc" not in args.skip:
        part("allocations", measure_alloc)
    if "scrollback" not in args.skip:
        part("scrollback", measure_scrollback)
    if "app" not in args.skip or "size" not in args.skip:
        out_dir = tempfile.mkdtemp(prefix="perf-watch-bundle-")
        holder = {}

        def bundle():
            holder["app"] = build_bundle(out_dir)
            return {}

        if part("bundle", bundle):
            if "size" not in args.skip:
                part("size", lambda: measure_sizes(holder["app"]))
            if "app" not in args.skip:
                part("app", lambda: measure_app(holder["app"], args.idle_seconds))
        shutil.rmtree(out_dir, ignore_errors=True)

    with open(args.out, "w") as f:
        json.dump(perf, f, indent=2, sort_keys=True)
        f.write("\n")
    print(f"wrote {args.out}: {len(perf['metrics'])} numbers, {len(perf['errors'])} errors")
    return 0


# ----------------------------------------------------------------- report


def load_rules(path=THRESHOLDS):
    with open(path) as f:
        return json.load(f)["rules"]


def rule_for(name, rules):
    for r in rules:
        if fnmatch.fnmatchcase(name, r["match"]):
            return r
    return {"percent": 10, "min_delta": 0}


def is_over(now, ref, rule):
    if now is None or ref is None:
        return False
    return now > ref * (1 + rule["percent"] / 100) and now - ref > rule["min_delta"]


def read_history(path):
    runs = []
    if path and os.path.exists(path):
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        runs.append(json.loads(line))
                    except ValueError:
                        warn(f"skipped a line of {path} that isn't JSON")
    return runs


def parse_date(s):
    return datetime.datetime.strptime(s, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc)


def week_before(runs, now_date, days=7):
    """The newest run at least `days` older than `now_date`, else the
    oldest run; None for no runs."""
    if not runs:
        return None
    cutoff = parse_date(now_date) - datetime.timedelta(days=days)
    older = [r for r in runs if parse_date(r["date"]) <= cutoff]
    return older[-1] if older else runs[0]


def value(run, name):
    if not run:
        return None
    m = run.get("metrics", {}).get(name)
    return None if m is None else m["value"]


def references(current, previous, week):
    """Per metric, the lower of the previous run's number and the week-old
    run's: a jump since the last run or a creep over the week."""
    out = {}
    for name in current["metrics"]:
        vals = [v for v in (value(previous, name), value(week, name)) if v is not None]
        if vals:
            out[name] = min(vals)
    return out


def compare(current, previous, week, rules, baseline=None):
    """Rows (name, metric, before, week, ref, rule, state) and the names
    over their limits. With an open issue's `baseline`, its numbers are
    the references (falling back to the usual ones)."""
    refs = references(current, previous, week)
    if baseline:
        refs.update({k: v for k, v in baseline.items() if k in current["metrics"] and v is not None})
    rows, over = [], []
    names = list(current["metrics"])
    for name in (previous or {}).get("metrics", {}):
        if name not in current["metrics"]:
            names.append(name)
    for name in names:
        m = current["metrics"].get(name)
        rule = rule_for(name, rules)
        ref = refs.get(name)
        if m is None:
            state = "missing"
        elif ref is None:
            state = "new"
        elif is_over(m["value"], ref, rule):
            state = "over"
            over.append(name)
        else:
            state = "ok"
        rows.append((name, m or previous["metrics"][name], value(previous, name), value(week, name), ref, rule, state))
    return rows, over


def human(v, unit):
    if v is None:
        return "–"
    if unit == "bytes":
        for scale, suffix in ((1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")):
            if abs(v) >= scale:
                return f"{v / scale:.2f} {suffix}"
        return f"{v} B"
    if isinstance(v, float):
        return f"{v:,.1f} {unit}"
    return f"{v:,} {unit}" if unit not in ("count",) else f"{v:,}"


def change(now, before):
    if now is None or before is None:
        return "–"
    if before == 0:
        return "0%" if now == 0 else "new"
    pct = (now - before) / before * 100
    return f"{pct:+.1f}%" if abs(pct) >= 0.05 else "0%"


def sort_key(name):
    for i, (prefix, _) in enumerate(SECTIONS):
        if name.startswith(prefix):
            sub = APP_ORDER.index(name) if name in APP_ORDER else len(APP_ORDER)
            return (i, sub, name)
    return (len(SECTIONS), 0, name)


def heading(name):
    for prefix, title in SECTIONS:
        if name.startswith(prefix):
            return title
    return "Other"


def markdown(current, previous, week, rows, over, errors, issue_note=""):
    short = lambda run: run["commit"][:7] if run else "–"  # noqa: E731
    out = [f"## Perf watch: {short(current)}", ""]
    out.append(
        "A report, never a gate. Numbers are compared with the previous run on main"
        + (f" ({short(previous)}, {previous['date']})" if previous else " (none yet)")
        + (f" and the run from about a week before ({short(week)}, {week['date']})" if week and week is not previous else "")
        + ". Limits are in `scripts/perf-watch/thresholds.json`; see `docs/perf-watch.md`."
    )
    out.append("")
    if over:
        out.append(f"**{len(over)} over the limit:** " + ", ".join(f"`{n}`" for n in over))
    elif rows:
        out.append("Everything is within its limit.")
    if issue_note:
        out += ["", issue_note]
    last = None
    for name, m, before, wk, ref, rule, state in sorted(rows, key=lambda r: sort_key(r[0])):
        title = heading(name)
        if title != last:
            out += ["", f"### {title}", "", "| | before | now | change | week ago | limit | |", "|---|--:|--:|--:|--:|--:|---|"]
        last = title
        now = m["value"] if state != "missing" else None
        limit = f"+{rule['percent']}%"
        mark = {"ok": "ok", "new": "new", "missing": "**missing**", "over": "**over**"}[state]
        out.append(
            f"| {m['label']} | {human(before, m['unit'])} | {human(now, m['unit'])} | {change(now, before)} "
            f"| {human(wk, m['unit']) if week and week is not previous else '–'} | {limit} | {mark} |"
        )
    if errors:
        out += ["", "### Not measured", ""]
        for e in errors:
            first, _, rest = e.partition("\n")
            out.append(f"- {first}")
            if rest:
                out += ["", "  ```", *("  " + line for line in rest.splitlines()), "  ```"]
    return "\n".join(out) + "\n"


def gh(*args, input=None):
    return subprocess.run(["gh", *args], capture_output=True, text=True, check=True, input=input).stdout


def find_issue():
    """The open issue this job owns: (number, body) or None."""
    out = gh("issue", "list", "--label", LABEL, "--state", "open", "--limit", "100", "--json", "number,body")
    for issue in json.loads(out or "[]"):
        if MARKER in (issue.get("body") or ""):
            return issue["number"], issue["body"]
    return None


def hidden(body, key):
    m = re.search(rf"<!-- perf-watch-{key} (.*?) -->", body or "", re.S)
    if not m:
        return None
    try:
        return json.loads(m.group(1))
    except ValueError:
        return None


def issue_body(current, over, baseline, table, run_url):
    return "\n".join(
        [
            MARKER,
            f"<!-- perf-watch-baseline {json.dumps(baseline, sort_keys=True)} -->",
            f"<!-- perf-watch-over {json.dumps(sorted(over))} -->",
            "",
            "The perf watch (.github/workflows/perf-watch.yml) found numbers over their limits in "
            "scripts/perf-watch/thresholds.json. It compares each run on main with the numbers from before "
            "this issue was opened, updates this issue while any stay over, and closes it when all are back "
            "under. Close it by hand to accept the new numbers: the next run then compares with the latest ones.",
            "",
            f"Latest run: {run_url} ({current['commit'][:7]})" if run_url else f"Latest commit: {current['commit'][:7]}",
            "",
            table,
        ]
    )


def handle_issue(current, previous, week, rules, run_url, dry_run=False):
    """Open, update or close the issue; returns a note for the summary."""
    try:
        found = find_issue()
    except (OSError, subprocess.CalledProcessError) as e:
        warn(f"couldn't list issues: {getattr(e, 'stderr', e)}")
        return ""
    baseline = (hidden(found[1], "baseline") or {}) if found else None
    rows, over = compare(current, previous, week, rules, baseline)
    table = markdown(current, previous, week, rows, over, current.get("errors", []))
    try:
        if found:
            number, body = found
            before = hidden(body, "over") or []
            missing = [n for n in before if n not in current["metrics"]]
            if not over and not missing:
                if not dry_run:
                    gh("issue", "close", str(number), "--comment", f"Back under every limit as of {current['commit'][:7]} ({run_url}).")
                return f"Closed #{number}: everything is back under its limit."
            # Keep the baseline; add references for anything new over.
            refs = references(current, previous, week)
            for n in over:
                if baseline.get(n) is None and refs.get(n) is not None:
                    baseline[n] = refs[n]
            if not dry_run:
                with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as f:
                    f.write(issue_body(current, over, baseline, table, run_url))
                gh("issue", "edit", str(number), "--body-file", f.name)
                if sorted(over) != sorted(before):
                    gh("issue", "comment", str(number), "--body", f"Now over at {current['commit'][:7]}: {', '.join(over) or 'none (some not measured)'}. {run_url}")
                os.unlink(f.name)
            return f"Updated #{number}."
        if not over:
            return ""
        baseline = references(current, previous, week)
        if not dry_run:
            gh("label", "create", LABEL, "--color", "D93F0B", "--description", "Memory or CPU going up (the perf watch)", "--force")
            with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as f:
                f.write(issue_body(current, over, baseline, table, run_url))
            title = f"Perf watch: {len(over)} number{'s' if len(over) != 1 else ''} over the limit at {current['commit'][:7]}"
            url = gh("issue", "create", "--title", title, "--label", LABEL, "--body-file", f.name).strip()
            os.unlink(f.name)
            return f"Opened {url}."
        return "Would open an issue."
    except (OSError, subprocess.CalledProcessError) as e:
        warn(f"couldn't update the issue: {getattr(e, 'stderr', e)}")
        return ""


def cmd_report(args):
    try:
        with open(args.current) as f:
            current = json.load(f)
    except (OSError, ValueError) as e:
        warn(f"no numbers to report ({e})")
        return 0
    runs = [r for r in read_history(args.history) if r.get("schema") == SCHEMA]
    # A nightly run of the commit already measured still compares with
    # the run before: that shows the noise.
    previous = runs[-1] if runs else None
    week = week_before(runs, current["date"])
    rules = load_rules(args.thresholds)
    note = ""
    if args.issues:
        url = ""
        if os.environ.get("GITHUB_RUN_ID"):
            url = f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{os.environ.get('GITHUB_REPOSITORY')}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
        note = handle_issue(current, previous, week, rules, url)
    rows, over = compare(current, previous, week, rules)
    table = markdown(current, previous, week, rows, over, current.get("errors", []), note)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(table)
    print(table)
    for e in current.get("errors", []):
        warn(e.splitlines()[0])
    return 0


def cmd_record(args):
    with open(args.current) as f:
        current = json.load(f)
    if not current.get("metrics"):
        warn("nothing measured; not recorded")
        return 0
    os.makedirs(os.path.dirname(os.path.abspath(args.history)), exist_ok=True)
    with open(args.history, "a") as f:
        f.write(json.dumps(current, sort_keys=True, separators=(",", ":")) + "\n")
    print(f"recorded {current['commit'][:7]} in {args.history}")
    return 0


# -------------------------------------------------------------- self-test


def self_test():
    alloc = parse_alloc(
        '{"input": "plain", "bytes_in": 10, "allocs": 12, "bytes_allocated": 900, "peak_heap": 500, "retained_heap": 400, "history_lines": 3}\n'
        "noise\n"
    )
    assert alloc["alloc.plain.allocs"]["value"] == 12 and alloc["alloc.plain.retained_heap"]["unit"] == "bytes", alloc
    sb = parse_scrollback("plain 40 chars                   52 bytes/line  (0.5 MB)\n")
    assert sb == {"scrollback.plain 40 chars": metric(52, "bytes/line", "plain 40 chars")}, sb
    assert parse_footprint("sleep [1]: 64-bit    Footprint: 999640 B (16384 bytes per page)") == 999640
    assert parse_footprint("x [1]: 64-bit    Footprint: 1.5 MB (16384 bytes per page)") == 1572864
    assert parse_footprint("nothing") is None

    rules = load_rules()
    assert rule_for("alloc.plain.allocs", rules)["percent"] == 10
    assert rule_for("size.dylib", rules)["percent"] == 5
    assert rule_for("app.idle_cpu_ms_per_min", rules)["percent"] == 20
    assert rule_for("something.else", rules)["percent"] == 10
    assert all(isinstance(r["percent"], (int, float)) and r["min_delta"] >= 0 for r in rules)
    assert is_over(111, 100, {"percent": 10, "min_delta": 0})
    assert not is_over(110, 100, {"percent": 10, "min_delta": 0})
    assert not is_over(200, 100, {"percent": 10, "min_delta": 100}), "min_delta keeps small moves out"
    assert not is_over(None, 100, {"percent": 10, "min_delta": 0})

    def run(commit, date, **vals):
        return {"schema": SCHEMA, "commit": commit * 7, "date": date, "metrics": {k.replace("_", "."): metric(v, "count", k) for k, v in vals.items()}, "errors": []}

    runs = [run("a", "2026-10-01T00:00:00Z", x=100), run("b", "2026-10-20T00:00:00Z", x=105), run("c", "2026-10-28T00:00:00Z", x=108)]
    assert week_before(runs, "2026-11-02T00:00:00Z")["commit"] == "b" * 7
    assert week_before(runs[2:], "2026-11-02T00:00:00Z")["commit"] == "c" * 7, "no run a week old: the oldest"
    assert week_before([], "2026-11-02T00:00:00Z") is None
    cur = run("d", "2026-11-02T00:00:00Z", x=117, y=5)
    rules = [{"match": "*", "percent": 10, "min_delta": 0}]
    # 117 is 8% over the previous run but 11% over the week before: creep.
    rows, over = compare(cur, runs[2], runs[1], rules)
    assert over == ["x"], over
    assert [r[6] for r in rows] == ["over", "new"], rows
    rows, over = compare(cur, runs[2], runs[2], rules)
    assert over == [], over
    # An open issue's baseline holds the reference.
    _, over = compare(cur, runs[2], runs[2], rules, baseline={"x": 100})
    assert over == ["x"], over
    gone = run("e", "2026-11-02T00:00:00Z", y=5)
    rows, over = compare(gone, runs[2], None, rules)
    assert ("x", "missing") in [(r[0], r[6]) for r in rows], rows
    md = markdown(cur, runs[2], runs[1], *compare(cur, runs[2], runs[1], rules), ["app: boom\ndetail"])
    assert "**over**" in md and "### Not measured" in md and "- app: boom" in md, md
    assert human(3 << 20, "bytes") == "3.00 MiB" and human(1234, "count") == "1,234" and human(None, "x") == "–"
    assert change(110, 100) == "+10.0%" and change(100, 100) == "0%" and change(1, None) == "–"
    body = issue_body(cur, ["x"], {"x": 100}, "table", "")
    assert MARKER in body and hidden(body, "baseline") == {"x": 100} and hidden(body, "over") == ["x"]

    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "runs.jsonl")
        cur_path = os.path.join(d, "perf.json")
        with open(cur_path, "w") as f:
            json.dump(cur, f)
        assert cmd_record(argparse.Namespace(current=cur_path, history=path)) == 0
        assert read_history(path)[0]["commit"] == "d" * 7
    print("perf-watch self-test: ok")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--self-test", action="store_true")
    sub = ap.add_subparsers(dest="cmd")
    m = sub.add_parser("measure", help="build and measure; write perf.json")
    m.add_argument("--out", default="perf.json")
    m.add_argument("--skip", action="append", default=[], choices=["alloc", "scrollback", "size", "app"])
    m.add_argument("--idle-seconds", type=int, default=IDLE_SECONDS)
    r = sub.add_parser("report", help="compare with the history; summary and issue")
    r.add_argument("--current", default="perf.json")
    r.add_argument("--history", help="runs.jsonl from the perf-history branch")
    r.add_argument("--thresholds", default=THRESHOLDS)
    r.add_argument("--issues", action="store_true", help="open, update or close the performance issue")
    c = sub.add_parser("record", help="append perf.json to the history")
    c.add_argument("--current", default="perf.json")
    c.add_argument("--history", required=True)
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if args.cmd == "measure":
        return cmd_measure(args)
    if args.cmd == "report":
        return cmd_report(args)
    if args.cmd == "record":
        return cmd_record(args)
    ap.print_help()
    return 64


if __name__ == "__main__":
    sys.exit(main())
