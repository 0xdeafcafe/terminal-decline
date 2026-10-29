#!/usr/bin/env python3
"""Keypress-to-present latency of decline, idle and under load, and the
same keypress-to-pixels measurement for decline and Ghostty from outside.

    scripts/latency/run.py [--app target/release/decline] [--keys 200]
                           [--scenarios idle,flood,htop] [--rates 120,60]
                           [--external decline,ghostty] [--out target/latency]

In-app (always): each run starts decline with `DECLINE_LATENCY_LOG`, so it
logs the timing of every key (crates/term-app/src/latency.rs), and
`DECLINE_LATENCY_INPUT`, a FIFO through which this script asks decline to
post key events to itself with `CGEventPostToPid`. That needs no
permission, and the event still goes through the window server: `t0` is
the event's own timestamp. Scenarios:
- idle: `echo.py`, which echoes each key straight back;
- flood: the same, while a second window of the same app runs `yes`;
- htop: `htop -d 1` in the same window, typing into its search bar;
each at the display link rates given (`DECLINE_LATENCY_HZ`).

External (`--external`): `probe.swift` posts the keys from outside and
watches the window's pixels with ScreenCaptureKit, so any terminal can be
measured the same way. It needs Accessibility and Screen Recording for the
terminal running this script. Same scenarios, except that flood opens the
second window with Cmd+N (`echo.py --flood-first`); the display runs at
its own rate. For decline, the in-app log is kept too, to compare the two
clocks.

Keys are sent one at a time, 50-150 ms (random) after the previous one
showed, so their timing doesn't lock onto the display's refresh. The first
keys of each run are warm-up and not counted.

Prints p50/p95/p99 in ms and writes the raw logs and summary.json to --out.
"""

import argparse
import json
import os
import plistlib
import random
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
WARM_UP = 6
# More environment for decline (--env).
EXTRA_ENV = {}

# macOS virtual key codes (ANSI layout).
KEY_A, KEY_X, KEY_N, KEY_SLASH, KEY_DELETE = 0, 7, 45, 44, 51
# With --type: the bytes each key sends, typed in without the window server.
TYPED = {KEY_A: b"a", KEY_X: b"x", KEY_SLASH: b"/", KEY_DELETE: b"\x7f"}
TYPE = False

STAGES = [
    ("total", "t0", "t3", "key event → on glass"),
    ("in_app", "key_down", "t3", "keyDown → on glass"),
    ("deliver", "t0", "key_down", "event → keyDown"),
    ("handle", "key_down", "t1", "keyDown → bytes to writer"),
    ("echo", "t1", "t2", "writer → program → parsed"),
    ("wait", "t2", "frame_start", "parsed → frame starts"),
    ("build", "frame_start", "commit", "frame build + encode"),
    ("gpu", "commit", "gpu_done", "commit → GPU done"),
    ("show", "commit", "t3", "commit → on glass"),
]

GHOSTTY = Path("/Applications/Ghostty.app")
# Ghostty set up like decline's defaults: same font, grid and padding.
GHOSTTY_ARGS = [
    "--config-default-files=false",
    "--font-family=Menlo",
    "--font-size=13",
    "--window-width=100",
    "--window-height=30",
    "--window-padding-x=6",
    "--window-padding-y=6",
    "--cursor-style-blink=false",
    "--confirm-close-surface=false",
    "--quit-after-last-window-closed=true",
]

BUNDLE_ID = "red.forbes.decline.latency"
LSREGISTER = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"


def pct(values, p):
    """Nearest-rank percentile."""
    if not values:
        return float("nan")
    s = sorted(values)
    k = max(0, min(len(s) - 1, int(round(p / 100 * len(s) + 0.5)) - 1))
    return s[k]


def stats(v):
    return {"n": len(v), "p50": pct(v, 50), "p95": pct(v, 95), "p99": pct(v, 99), "max": max(v) if v else float("nan")}


def summarise(keys):
    out = {}
    for name, a, b, _ in STAGES:
        out[name] = stats([(k[b] - k[a]) * 1000 for k in keys if k.get(a) is not None and k.get(b) is not None])
    out["dropped"] = sum(1 for k in keys if k.get("t3") is None)
    return out


def read_jsonl(path):
    try:
        return [json.loads(l) for l in Path(path).read_text().splitlines() if l.strip()]
    except FileNotFoundError:
        return []


def line_count(path):
    try:
        with open(path, "rb") as f:
            return sum(1 for _ in f)
    except FileNotFoundError:
        return 0


def make_bundle(binary, out_dir):
    """Wrap the binary in an app of its own bundle id, so `open` can start
    it: macOS lets an app started by LaunchServices take the focus, while
    one started from a script may be refused. Its own id keeps it apart
    from an installed Decline.app."""
    app = out_dir / "DeclineLatency.app"
    shutil.rmtree(app, ignore_errors=True)
    (app / "Contents/MacOS").mkdir(parents=True)
    shutil.copy2(binary, app / "Contents/MacOS/decline")
    with open(app / "Contents/Info.plist", "wb") as f:
        plistlib.dump(
            {
                "CFBundleIdentifier": BUNDLE_ID,
                "CFBundleName": "DeclineLatency",
                "CFBundleExecutable": "decline",
                "CFBundlePackageType": "APPL",
                "NSHighResolutionCapable": True,
            },
            f,
        )
    subprocess.run(["codesign", "-s", "-", "--force", str(app)], check=True, capture_output=True)
    return app


def build_probe(out_dir):
    exe = out_dir / "probe"
    src = HERE / "probe.swift"
    if not exe.exists() or exe.stat().st_mtime < src.stat().st_mtime:
        subprocess.run(["swiftc", "-O", "-swift-version", "5", str(src), "-o", str(exe)], check=True)
    return exe


def pids_of(exe):
    r = subprocess.run(["/usr/bin/pgrep", "-f", exe], capture_output=True, text=True)
    return [int(p) for p in r.stdout.split()]


class App:
    """A terminal started through `open`, running `program`."""

    def __init__(self, bundle, exe, open_args, app_args):
        before = set(pids_of(exe))
        subprocess.run(["/usr/bin/open", "-n", *open_args, str(bundle), "--args", *app_args], check=True)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            new = set(pids_of(exe)) - before
            if new:
                self.pid = new.pop()
                return
            time.sleep(0.05)
        raise SystemExit(f"{bundle.name} didn't start")

    @staticmethod
    def decline(bundle, env, program, stderr):
        open_args = ["--stderr", str(stderr)]
        for k, v in env.items():
            open_args += ["--env", f"{k}={v}"]
        return App(bundle, str(bundle / "Contents/MacOS/decline"), open_args, ["-e", *program])

    @staticmethod
    def ghostty(program):
        command = " ".join(program)
        return App(GHOSTTY, str(GHOSTTY / "Contents/MacOS/ghostty"), [], [*GHOSTTY_ARGS, f"--command={command}"])

    def stop(self):
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        for _ in range(100):
            try:
                os.kill(self.pid, 0)
            except ProcessLookupError:
                return
            time.sleep(0.05)
        os.kill(self.pid, signal.SIGKILL)


def open_fifo(path, timeout=10):
    """Open the FIFO for writing once decline has opened it for reading."""
    deadline = time.monotonic() + timeout
    while True:
        try:
            fd = os.open(path, os.O_WRONLY | os.O_NONBLOCK)
            break
        except OSError:
            if time.monotonic() > deadline:
                raise SystemExit("decline didn't open its input FIFO")
            time.sleep(0.05)
    os.set_blocking(fd, True)
    return os.fdopen(fd, "w", buffering=1)


def keys_for(scenario, n):
    """The key that starts the scenario, then the keys measured."""
    if scenario == "htop":
        # '/' opens htop's search bar; typed letters then show up in it.
        return KEY_SLASH, [KEY_X if i % 2 == 0 else KEY_DELETE for i in range(n)]
    return KEY_A, [KEY_A] * n


def program_for(scenario, marker=None):
    if scenario == "htop":
        return [shutil.which("htop") or "htop", "-d", "1"]
    echo = [sys.executable, str(HERE / "echo.py")]
    return [*echo, "--flood-first", str(marker)] if marker else echo


def run_inapp(bundle, scenario, hz, n_keys, out_dir):
    tmp = Path(tempfile.mkdtemp(prefix="decline-latency-"))
    fifo = tmp / "input"
    os.mkfifo(fifo)
    log = out_dir / f"inapp-{scenario}-{hz}hz.jsonl"
    log.unlink(missing_ok=True)
    env = {
        "DECLINE_LATENCY_LOG": str(log),
        "DECLINE_LATENCY_INPUT": str(fifo),
        "DECLINE_LATENCY_HZ": str(hz),
        "TERM": "xterm-256color",
        **EXTRA_ENV,
    }
    app = App.decline(bundle, env, program_for(scenario), out_dir / f"inapp-{scenario}-{hz}hz.stderr")
    try:
        ctl = open_fifo(fifo)
        time.sleep(1.5)
        if scenario == "flood":
            ctl.write("open yes\n")
            time.sleep(1.0)

        def press(code):
            """Send a key; whether it was logged within a second."""
            want = line_count(log) + 1
            ctl.write(f"type {TYPED[code].hex()}\n" if TYPE else f"key {code}\n")
            deadline = time.monotonic() + 1.0
            while line_count(log) < want:
                if time.monotonic() > deadline:
                    return False
                time.sleep(0.001)
            time.sleep(random.uniform(0.05, 0.15))
            return True

        first, keys = keys_for(scenario, n_keys + WARM_UP)
        # The first key also waits for the window to become key: macOS
        # refuses to activate an app while someone is typing in another,
        # and decline asks again on each key.
        for attempt in range(300):
            if press(first):
                break
            if attempt == 2:
                print(f"{scenario}: waiting for decline's window to get the focus (stop typing elsewhere)", file=sys.stderr)
        else:
            raise SystemExit(f"{scenario}: no key reached decline; is its window on screen?")
        for code in keys[:WARM_UP]:
            press(code)
        base = line_count(log)
        missed = sum(not press(code) for code in keys[WARM_UP:])
        time.sleep(0.3)
        ctl.close()
    finally:
        app.stop()
        shutil.rmtree(tmp, ignore_errors=True)
    return read_jsonl(log)[base:], missed


def probe_out(probe, *args):
    return subprocess.run([str(probe), *map(str, args)], check=True, capture_output=True, text=True).stdout


def window_ids(probe, pid):
    return {l.split()[0] for l in probe_out(probe, "windows", pid).splitlines() if l.strip()}


def run_external(probe, bundle, terminal, scenario, n_keys, out_dir):
    tmp = Path(tempfile.mkdtemp(prefix="decline-latency-"))
    name = f"external-{terminal}-{scenario}"
    out = out_dir / f"{name}.jsonl"
    inapp = out_dir / f"{name}-inapp.jsonl"
    out.unlink(missing_ok=True)
    inapp.unlink(missing_ok=True)
    program = program_for(scenario, tmp / "flooding" if scenario == "flood" else None)
    if terminal == "decline":
        env = {"DECLINE_LATENCY_LOG": str(inapp), "TERM": "xterm-256color", **EXTRA_ENV}
        app = App.decline(bundle, env, program, out_dir / f"{name}.stderr")
    else:
        app = App.ghostty(program)
    try:
        time.sleep(2.5)
        window = []
        if scenario == "flood":
            # The first window floods; Cmd+N opens the one measured.
            before = window_ids(probe, app.pid)
            probe_out(probe, "keys", app.pid, f"cmd+{KEY_N}")
            time.sleep(2.0)
            new = window_ids(probe, app.pid) - before
            if not new:
                raise SystemExit(f"{name}: Cmd+N opened no window")
            window = ["--window", new.pop()]
        first, keys = keys_for(scenario, n_keys + WARM_UP)
        probe_out(probe, "keys", app.pid, first)
        time.sleep(0.5)
        codes = ",".join(map(str, keys[:2]))
        # htop redraws the whole screen every 100 ms: watch only its bottom
        # line, where the search text is.
        region = ["--bottom", "20"] if scenario == "htop" else []
        subprocess.run([str(probe), "measure", str(app.pid), str(n_keys + WARM_UP), codes, *window, *region, "--out", str(out)], check=True)
        time.sleep(0.3)
    finally:
        app.stop()
        shutil.rmtree(tmp, ignore_errors=True)
    rows = read_jsonl(out)[WARM_UP:]
    ms = [r["ms"] for r in rows if "ms" in r]
    result = {"total": stats(ms), "missed": len(rows) - len(ms)}
    logged = read_jsonl(inapp)
    if terminal == "decline" and logged:
        # The same keys in decline's own log: how the probe's clock (the
        # capture's display time) compares with Metal's presented time.
        by_post = sorted(rows, key=lambda r: r["t_post"])
        diffs, inner = [], []
        for k in logged:
            match = [r for r in by_post if r["t_post"] <= k["t0"] < r["t_post"] + 0.05 and "t_shown" in r]
            if match and k.get("t3"):
                diffs.append((match[-1]["t_shown"] - k["t3"]) * 1000)
                inner.append((k["t3"] - k["t0"]) * 1000)
        result["probe_minus_presented"] = stats(diffs)
        result["inapp_total"] = stats(inner)
    return result


def show(title, rows):
    print(f"\n{title}")
    print(f"  {'':<30} {'p50':>7} {'p95':>7} {'p99':>7} {'max':>7}  (ms)")
    for label, st in rows:
        print(f"  {label:<30} {st['p50']:7.2f} {st['p95']:7.2f} {st['p99']:7.2f} {st['max']:7.2f}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--app", default=str(ROOT / "target/release/decline"))
    ap.add_argument("--keys", type=int, default=200)
    ap.add_argument("--scenarios", default="idle,flood,htop")
    ap.add_argument("--rates", default="120,60", help="in-app display link rates; empty to skip in-app runs")
    ap.add_argument("--external", default="", help="terminals to measure from outside: decline,ghostty")
    ap.add_argument("--out", default=str(ROOT / "target/latency"))
    ap.add_argument("--env", action="append", default=[], metavar="NAME=VALUE", help="set for decline; repeatable")
    ap.add_argument(
        "--type",
        action="store_true",
        help="in-app: type the keys' bytes straight into the first window instead of posting key events (no focus needed; t0 is then keyDown)",
    )
    ap.add_argument("--tag", default="", help="suffix for the summary keys, to keep variants apart")
    args = ap.parse_args()
    EXTRA_ENV.update(e.split("=", 1) for e in args.env)
    global TYPE
    TYPE = args.type
    out_dir = Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    summary_file = out_dir / "summary.json"
    summary = json.loads(summary_file.read_text()) if summary_file.exists() else {}
    bundle = make_bundle(Path(args.app), out_dir)
    scenarios = [s for s in args.scenarios.split(",") if s]
    try:
        for scenario in scenarios:
            for hz in [int(r) for r in args.rates.split(",") if r]:
                rows, missed = run_inapp(bundle, scenario, hz, args.keys, out_dir)
                s = summarise(rows)
                s["missed"] = missed
                summary[f"inapp-{scenario}-{hz}hz{args.tag}"] = s
                show(
                    f"decline in-app, {scenario} @ {hz} Hz: {len(rows)} keys, {s['dropped']} not presented, {missed} without a frame",
                    [(label, s[name]) for name, _, _, label in STAGES],
                )
        terminals = [t for t in args.external.split(",") if t]
        if terminals:
            probe = build_probe(out_dir)
            check = probe_out(probe, "check")
            if "screen_recording=false" in check or "post_events=false" in check:
                raise SystemExit(f"probe: {check.strip()}: grant Screen Recording and Accessibility to this terminal")
            for scenario in scenarios:
                # Interleaved, so a change in the machine's load hits both.
                for terminal in terminals:
                    r = run_external(probe, bundle, terminal, scenario, args.keys, out_dir)
                    summary[f"external-{terminal}-{scenario}{args.tag}"] = r
                    rows = [("key posted → pixels changed", r["total"])]
                    if "inapp_total" in r:
                        rows += [("same keys, in-app t0 → t3", r["inapp_total"]), ("probe time − presentedTime", r["probe_minus_presented"])]
                    show(f"{terminal} from outside, {scenario}: {r['total']['n']} keys, {r['missed']} without a change", rows)
    finally:
        subprocess.run([LSREGISTER, "-u", str(bundle)], capture_output=True)
        summary_file.write_text(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
