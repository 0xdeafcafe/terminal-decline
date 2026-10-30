#!/usr/bin/env python3
"""Terminal Decline against other macOS terminals, measured the same way.

    scripts/compare/run.py [--terminals decline,ghostty,kitty,warp,terminal]
                           [--rounds 5] [--idle 30] [--app target/release/terminal-decline]
                           [--out target/compare] [--wait-calm] [--latency]
    scripts/compare/run.py --report [--out target/compare]

Each round runs every terminal in turn (interleaved: A B C D E, then again),
so a change in the machine's load hits them all. Per terminal and round:

- one launch, with one window running `driver.sh`, which runs commands
  sent to it through a FIFO and reports when each is done:
  - idle: footprint, then CPU and idle wakeups over --idle seconds
    (`top -stats cpu,idlew`), 5 s after the window came up;
  - `seq 1 10000`, then footprint;
  - `cat` of a 100 MB file (`scripts/pressure/gen.py big`): wall time (in
    the window's shell), CPU seconds of the terminal and of WindowServer
    (`ps -o cputime`, before and 1 s after), and the peak footprint,
    sampled as fast as `footprint` answers;
- a second launch with four tabs (three opened after start), then footprint.

A terminal's processes are every process whose executable is inside its
app bundle and that wasn't running before the launch (Warp and others have
helpers); footprints and CPU are summed over them. Programs the terminal
runs (the shell, `cat`) are not counted. Instances already running (yours)
are never touched: only processes started by this script are measured and
stopped.

Same conditions where the terminal lets them be set: Menlo 13, 100×30
cells (the window's size in points is recorded), 10k lines of scrollback,
no cursor blink, default theme, one window; each from a config in a temp
folder, not the user's (Terminal Decline: XDG_CONFIG_HOME and --config;
Ghostty: --config-default-files=false and --config-file; kitty: --config;
Terminal.app: a profile passed as launch arguments, which override the
user's defaults for that run only; Warp: no config, the user's settings,
see the comparison doc). Terminal.app's preferences are exported first and
imported back at the end in case it wrote them.

--latency: keypress → pixels with scripts/latency/probe.swift, for every
terminal, when the terminal running this has Accessibility and Screen
Recording (`probe check`); otherwise recorded as skipped.

--wait-calm: wait (up to 3 h) until the 5-minute load average is under 4
and no agent's test instance runs, twice 5 minutes apart. Before each run
the 1-minute load is checked and the run waits while it is over 6.

Raw rows go to OUT/results.jsonl (appended); --report prints the tables.
"""

import argparse
import json
import os
import plistlib
import re
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(ROOT / "scripts/latency"))

FONT, SIZE, COLS, ROWS, SCROLLBACK, PADDING = "Menlo", 13, 100, 30, 10_000, 6
WINDOWSERVER = "/System/Library/PrivateFrameworks/SkyLight.framework/Resources/WindowServer"
NAMES = {
    "decline": "Terminal Decline",
    "ghostty": "Ghostty",
    "kitty": "kitty",
    "warp": "Warp",
    "terminal": "Terminal.app",
}
BUNDLES = {
    "ghostty": Path("/Applications/Ghostty.app"),
    "kitty": Path("/Applications/kitty.app"),
    "warp": Path("/Applications/Warp.app"),
    "terminal": Path("/System/Applications/Utilities/Terminal.app"),
}
DECLINE_BUNDLE_ID = "red.forbes.terminal-decline.compare"
LSREGISTER = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
NOW = "perl -MTime::HiRes=time -e 'printf \"%.3f\", time'"

DRIVER = r"""#!/bin/sh
# Runs the commands written to $1/cmd, one per line; after each writes the
# start and end times to $1/ack. A second copy (a tab opened with the same
# program) just waits.
D="$1"
if [ -e "$D/ready" ]; then exec sleep 100000; fi
printf '\033[?25h'
touch "$D/ready"
while :; do
  read -r line < "$D/cmd" || continue
  [ "$line" = quit ] && exit 0
  S=$(NOW); eval "$line"; E=$(NOW)
  echo "$S $E" > "$D/ack.tmp"; mv "$D/ack.tmp" "$D/ack"
done
""".replace("NOW", NOW)


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, file=sys.stderr, flush=True)


# --- the machine ---------------------------------------------------------


def loads():
    out = subprocess.run(["sysctl", "-n", "vm.loadavg"], capture_output=True, text=True).stdout
    return [float(x) for x in out.strip("{} \n").split()[:3]]


def agents_running():
    r = subprocess.run(["pgrep", "-f", "worktrees/agent-.*/target"], capture_output=True, text=True)
    own = str(ROOT)
    pids = []
    for p in r.stdout.split():
        cmd = subprocess.run(["ps", "-o", "command=", "-p", p], capture_output=True, text=True).stdout
        if own not in cmd:
            pids.append(int(p))
    return pids


def wait_calm(max_s=3 * 3600, every=300):
    start, calm = time.monotonic(), 0
    while time.monotonic() - start < max_s:
        l, a = loads(), agents_running()
        ok = l[1] < 4 and not a
        calm = calm + 1 if ok else 0
        log(f"calm check: load {l}, other agents' instances {len(a)}: {'calm' if ok else 'busy'} ({calm}/2)")
        if calm >= 2:
            return True
        time.sleep(every)
    log("gave up waiting for calm after 3 h; running anyway")
    return False


MAX_LOAD = [6.0]


def wait_load(max_s=1800):
    limit = MAX_LOAD[0]
    start = time.monotonic()
    while loads()[0] > limit and time.monotonic() - start < max_s:
        log(f"load {loads()[0]:.1f} > {limit}: pausing")
        time.sleep(30)
    return loads()


# --- processes -----------------------------------------------------------


def processes():
    """pid -> executable path, for every process."""
    out = subprocess.run(["ps", "-axo", "pid=,comm="], capture_output=True, text=True).stdout
    procs = {}
    for line in out.splitlines():
        parts = line.strip().split(None, 1)
        if len(parts) == 2:
            procs[int(parts[0])] = parts[1]
    return procs


def in_bundle(bundle):
    prefix = str(bundle) + "/"
    return {pid for pid, exe in processes().items() if exe.startswith(prefix)}


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


def windowserver_pid():
    return next(pid for pid, exe in processes().items() if exe == WINDOWSERVER)


def build_swift(src, exe):
    if not exe.exists() or exe.stat().st_mtime < src.stat().st_mtime:
        subprocess.run(["swiftc", "-O", "-swift-version", "5", str(src), "-o", str(exe)], check=True)
    return exe


class Terminal:
    """One launch of a terminal: its processes, its window(s), the driver."""

    key = ""

    def __init__(self, ctx, tabs=False):
        self.ctx, self.tabs = ctx, tabs
        self.dir = Path(tempfile.mkdtemp(prefix=f"cmp-{self.key}-", dir=ctx.work))
        os.mkfifo(self.dir / "cmd")
        (self.dir / "driver.sh").write_text(DRIVER)
        self.before = in_bundle(self.bundle)
        self.started = time.monotonic()
        self.launch()
        deadline = time.monotonic() + 90
        while not (self.dir / "ready").exists():
            if time.monotonic() > deadline:
                self.stop()
                raise RuntimeError(f"{self.key}: the driver never started")
            time.sleep(0.1)
        self.ready_s = time.monotonic() - self.started

    # Per terminal.
    bundle = Path("/")

    def launch(self):
        raise NotImplementedError

    def open_tab(self, n):
        raise NotImplementedError

    def tab_count(self):
        """Native tabs are windows of their own. AppKit apps also keep a
        hidden 500×500 window (seen in Terminal Decline and Ghostty alike),
        which is not a tab."""
        return sum(1 for x in self.windows() if not (x[4:6] == [500, 500] and not x[6]))

    def cleanup(self):
        pass

    # Common.
    def program(self):
        return ["/bin/sh", str(self.dir / "driver.sh"), str(self.dir)]

    def pids(self):
        return sorted(in_bundle(self.bundle) - self.before)

    def footprint(self):
        values = [footprint_mb(p) for p in self.pids()]
        return round(sum(v for v in values if v), 1)

    def cpu(self):
        return sum(v[0] for v in usage(self.ctx, self.pids()).values())

    def windows(self):
        out = subprocess.run([str(self.ctx.helper), "windows", *map(str, self.pids())], capture_output=True, text=True).stdout
        return [list(map(int, l.split())) for l in out.splitlines() if l.strip()]

    def run(self, command, timeout=600, during=None):
        """Run a command in the window's shell; the times it started and
        ended there. `during` is called repeatedly while it runs."""
        ack = self.dir / "ack"
        ack.unlink(missing_ok=True)
        with open(self.dir / "cmd", "w") as f:
            f.write(command + "\n")
        deadline = time.monotonic() + timeout
        while not ack.exists():
            if time.monotonic() > deadline:
                raise RuntimeError(f"{self.key}: {command!r} didn't finish in {timeout} s")
            if during:
                during()
            else:
                time.sleep(0.05)
        s, e = map(float, ack.read_text().split())
        return s, e

    def stop(self):
        pids = self.pids()
        for p in pids:
            try:
                os.kill(p, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline and self.pids():
            time.sleep(0.1)
        for p in self.pids():
            try:
                os.kill(p, signal.SIGKILL)
            except ProcessLookupError:
                pass
        time.sleep(0.5)
        self.cleanup()
        shutil.rmtree(self.dir, ignore_errors=True)


def open_n(bundle, args, open_args=()):
    subprocess.run(["/usr/bin/open", "-n", *open_args, str(bundle), "--args", *args], check=True)


class Decline(Terminal):
    key = "decline"

    @property
    def bundle(self):
        return self.ctx.decline_bundle

    def launch(self):
        xdg = self.dir / "xdg"
        xdg.mkdir()
        cfg = self.dir / "decline.toml"
        cfg.write_text(
            f'font_family = "{FONT}"\nfont_size = {SIZE}\ncolumns = {COLS}\nlines = {ROWS}\n'
            f"scrollback = {SCROLLBACK}\npadding = {PADDING}\nconfirm_close = false\n"
        )
        env = {"XDG_CONFIG_HOME": str(xdg)}
        if self.tabs:
            # The test-only remote control (control.rs), for `tab`.
            env["DECLINE_TEST_DUMP"] = str(self.dir / "dump.jsonl")
        open_args = ["--stderr", str(self.dir / "stderr")]
        for k, v in env.items():
            open_args += ["--env", f"{k}={v}"]
        open_n(self.bundle, ["--config", str(cfg), "-e", *self.program()], open_args)

    def open_tab(self, n):
        self.run("printf '\\033]7770;tab\\007'")

    def tab_count(self):
        # What a restart would save: one entry per tab.
        saved = self.dir / "dump.jsonl.saved"
        self.run("printf '\\033]7770;save\\007'")
        deadline = time.monotonic() + 5
        while not saved.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        try:
            data = json.loads(saved.read_text())
        except (OSError, ValueError):
            return None
        return len(data if isinstance(data, list) else data.get("windows", []))


class Ghostty(Terminal):
    key = "ghostty"
    bundle = BUNDLES["ghostty"]

    def launch(self):
        cfg = self.dir / "ghostty.conf"
        # Scrollback is a byte limit in Ghostty: 10k rows of 100 cells at
        # 8 bytes a cell plus 8 a row (its page layout), rounded up.
        cfg.write_text(
            "\n".join(
                [
                    f"font-family = {FONT}",
                    f"font-size = {SIZE}",
                    f"window-width = {COLS}",
                    f"window-height = {ROWS}",
                    f"window-padding-x = {PADDING}",
                    f"window-padding-y = {PADDING}",
                    f"scrollback-limit = {SCROLLBACK * (COLS * 8 + 8)}",
                    "cursor-style-blink = false",
                    "confirm-close-surface = false",
                    "quit-after-last-window-closed = true",
                    "window-save-state = never",
                    "macos-dock-drop-behavior = new-tab",
                    "auto-update = off",
                    f"command = {' '.join(self.program())}",
                ]
            )
            + "\n"
        )
        open_n(self.bundle, ["--config-default-files=false", f"--config-file={cfg}"])

    def open_tab(self, n):
        # A folder dropped on (opened with) Ghostty opens as a new tab. Goes
        # to the running Ghostty: ours, as no other may run (checked).
        d = self.dir / f"tab{n}"
        d.mkdir()
        subprocess.run(["/usr/bin/open", "-a", str(self.bundle), str(d)], check=True)


class Kitty(Terminal):
    key = "kitty"
    bundle = BUNDLES["kitty"]

    def launch(self):
        cfg = self.dir / "kitty.conf"
        lines = [
            f"font_family {FONT}",
            f"font_size {SIZE}",
            f"scrollback_lines {SCROLLBACK}",
            "remember_window_size no",
            f"initial_window_width {COLS}c",
            f"initial_window_height {ROWS}c",
            f"window_padding_width {PADDING}",
            "cursor_blink_interval 0",
            "macos_quit_when_last_window_closed yes",
            "confirm_os_window_close 0",
            "update_check_interval 0",
        ]
        if self.tabs:
            # A socket's path must be short (104 bytes): not in $TMPDIR.
            self.sockdir = tempfile.mkdtemp(prefix="kcmp-", dir="/tmp")
            self.sock = f"{self.sockdir}/s"
            lines += ["allow_remote_control socket-only", f"listen_on unix:{self.sock}"]
        cfg.write_text("\n".join(lines) + "\n")
        open_n(self.bundle, ["--config", str(cfg), *self.program()])

    def socket(self):
        # kitty adds its pid to the path given.
        found = [f for f in os.listdir(self.sockdir) if f.startswith("s")]
        return f"unix:{self.sockdir}/{found[0]}"

    def open_tab(self, n):
        kitten = self.bundle / "Contents/MacOS/kitten"
        subprocess.run([str(kitten), "@", "--to", self.socket(), "launch", "--type=tab"], check=True, capture_output=True)

    def cleanup(self):
        if getattr(self, "sockdir", None):
            shutil.rmtree(self.sockdir, ignore_errors=True)

    def tab_count(self):
        kitten = self.bundle / "Contents/MacOS/kitten"
        out = subprocess.run([str(kitten), "@", "--to", self.socket(), "ls"], capture_output=True, text=True).stdout
        try:
            return sum(len(w["tabs"]) for w in json.loads(out))
        except (ValueError, KeyError):
            return None


class AppleTerminal(Terminal):
    """Terminal.app, with a profile of its own: added to its preferences as
    the default for the run, which are put back as they were (exported
    before the run) when it has quit. Launch arguments can't be used: it
    writes them back into the preferences. Commands open as `.command`
    files, and with AppleWindowTabbingMode=always new windows join the
    first as tabs."""

    key = "terminal"
    bundle = BUNDLES["terminal"]
    DOMAIN = "com.apple.Terminal"

    def set_prefs(self):
        font = subprocess.run([str(self.ctx.helper), "font", f"{FONT}-Regular", str(SIZE)], capture_output=True, text=True, check=True).stdout.strip()
        prefs = plistlib.loads(subprocess.run(["defaults", "export", self.DOMAIN, "-"], capture_output=True, check=True).stdout)
        prefs.setdefault("Window Settings", {})["Compare"] = {
            "name": "Compare",
            "type": "Window Settings",
            "ProfileCurrentVersion": 2.09,
            "Font": bytes.fromhex(font),
            "columnCount": COLS,
            "rowCount": ROWS,
            "ScrollbackLines": SCROLLBACK,
            "ShouldLimitScrollback": 1,
            "CursorBlink": False,
            "shellExitAction": 1,
            "warnOnShellCloseAction": 0,
        }
        prefs["Default Window Settings"] = prefs["Startup Window Settings"] = "Compare"
        prefs["AppleWindowTabbingMode"] = "always"
        f = self.dir / "terminal-prefs.plist"
        f.write_bytes(plistlib.dumps(prefs))
        subprocess.run(["defaults", "import", self.DOMAIN, str(f)], check=True)

    def command_file(self, name, body):
        f = self.dir / f"{name}.command"
        f.write_text("#!/bin/sh\n" + body + "\n")
        f.chmod(0o755)
        return f

    def launch(self):
        self.set_prefs()
        f = self.command_file("main", "exec " + " ".join(self.program()))
        subprocess.run(["/usr/bin/open", "-n", "-a", str(self.bundle), str(f), "--args", "-ApplePersistenceIgnoreState", "YES"], check=True)

    def open_tab(self, n):
        f = self.command_file(f"tab{n}", "exec sleep 100000")
        subprocess.run(["/usr/bin/open", "-a", str(self.bundle), str(f)], check=True)

    def cleanup(self):
        restore_terminal_prefs(self.ctx.terminal_backup)


def restore_terminal_prefs(backup):
    """Terminal.app's preferences exactly as exported (import alone merges)."""
    subprocess.run(["defaults", "delete", AppleTerminal.DOMAIN], capture_output=True)
    subprocess.run(["defaults", "import", AppleTerminal.DOMAIN, str(backup)], check=True)


class Warp(Terminal):
    """Warp. It has no config to pass, so for the run ~/.warp/settings.toml
    is replaced by one with only the settings below (Warp's defaults for
    the rest), and put back when Warp has quit. The window comes from a
    launch configuration in ~/.warp/launch_configurations, removed after."""

    key = "warp"
    bundle = BUNDLES["warp"]
    SETTINGS = f"""[general]
restore_session = false
default_session_mode = "terminal"

[agents.warp_agent]
is_any_ai_enabled = false

[appearance.text]
font_name = "{FONT}"
font_size = {SIZE}.0

[appearance.cursor]
cursor_blink = "disabled"

[appearance.window]
open_windows_at_custom_size = true
new_windows_num_columns = {COLS}
new_windows_num_rows = {ROWS}

[terminal]
maximum_grid_size = {SCROLLBACK}
"""

    def write_config(self, name, tabs):
        d = Path.home() / ".warp/launch_configurations"
        d.mkdir(parents=True, exist_ok=True)
        self.config = d / f"{name}.yaml"
        tab = lambda cmd: f"      - title: compare\n        layout:\n          cwd: {self.dir}\n          commands:\n            - exec: {cmd}\n"
        body = f"---\nname: {name}\nwindows:\n  - tabs:\n" + tab(" ".join(self.program()))
        for _ in range(tabs - 1):
            body += tab("sleep 100000")
        self.config.write_text(body)

    def launch(self):
        settings = Path.home() / ".warp/settings.toml"
        self.saved = self.ctx.work / "warp-settings.toml"
        if settings.exists() and not self.saved.exists():
            shutil.copy2(settings, self.saved)
        settings.write_text(self.SETTINGS)
        name = f"decline-compare-{os.getpid()}-{int(time.time())}"
        self.write_config(name, 4 if self.tabs else 1)
        # Warp always opens a window of its own at start, besides the launch
        # configuration's (tried: a startup tab config; the driver as the
        # first shell; making the first shell exit, which leaves the window
        # open and raced with ours). It stays, idle at a prompt, and is
        # counted. Both windows' zsh is a plain one (ZDOTDIR of our own).
        zdot = self.dir / "zdot"
        zdot.mkdir()
        open_n(self.bundle, [], ["--env", f"ZDOTDIR={zdot}"])
        time.sleep(3)
        subprocess.run(["/usr/bin/open", f"warp://launch/{name}"], check=True)

    def open_tab(self, n):
        pass  # the launch configuration has all four

    def tab_count(self):
        return None  # Warp's tabs are its own UI: not visible from outside

    def cleanup(self):
        if getattr(self, "config", None):
            self.config.unlink(missing_ok=True)
        restore_warp_settings(self.ctx)


def restore_warp_settings(ctx):
    saved = ctx.work / "warp-settings.toml"
    if saved.exists() and not in_bundle(BUNDLES["warp"]):
        shutil.copy2(saved, Path.home() / ".warp/settings.toml")
        saved.unlink()


KINDS = {c.key: c for c in (Decline, Ghostty, Kitty, Warp, AppleTerminal)}


# --- measurements ----------------------------------------------------------


def usage(ctx, pids):
    """pid -> (CPU seconds, package idle wakeups, interrupt wakeups)."""
    out = subprocess.run([str(ctx.helper), "usage", *map(str, pids)], capture_output=True, text=True).stdout
    rows = {}
    for line in out.splitlines():
        pid, ns, idle, intr = map(int, line.split())
        rows[pid] = (ns / 1e9, idle, intr)
    return rows


def idle_usage(ctx, pids, seconds):
    """CPU % and wakeups per second over `seconds`, summed over the pids.
    `top`'s IDLEW column reads 0 even for a process waking 100 times a
    second (macOS 27), so this asks proc_pid_rusage: interrupt wakeups
    (timers, the wakeups a process causes) and package idle wakeups (those
    that woke an idle CPU package, rarer on a busy machine)."""
    a = usage(ctx, pids)
    t0 = time.monotonic()
    time.sleep(seconds)
    b = usage(ctx, pids)
    dt = time.monotonic() - t0
    both = [p for p in pids if p in a and p in b]
    cpu = sum(b[p][0] - a[p][0] for p in both) / dt * 100
    pkg = sum(b[p][1] - a[p][1] for p in both) / dt
    intr = sum(b[p][2] - a[p][2] for p in both) / dt
    return round(cpu, 3), round(intr, 2), round(pkg, 2)


def measure_main(ctx, key, round_no):
    row = {"terminal": key, "kind": "main", "round": round_no, "load_before": wait_load()}
    t = KINDS[key](ctx)
    try:
        row["ready_s"] = round(t.ready_s, 2)
        time.sleep(5)
        row["pids"] = t.pids()
        row["windows"] = t.windows()
        row["idle_footprint_mb"] = t.footprint()
        row["idle_cpu_pct"], row["idle_wakeups_per_s"], row["idle_pkg_wakeups_per_s"] = idle_usage(ctx, t.pids(), ctx.idle)
        t.run(f"clear; seq 1 {SCROLLBACK}")
        time.sleep(2)
        row["seq_footprint_mb"] = t.footprint()
        ws = windowserver_pid()
        cpu0, ws0 = t.cpu(), cpu_seconds(ws)
        peak = [0.0]

        def sample():
            peak[0] = max(peak[0], t.footprint())

        s, e = t.run(f"cat {ctx.big} || echo CAT-FAILED > {t.dir}/failed", timeout=900, during=sample)
        if (t.dir / "failed").exists():
            raise RuntimeError(f"{key}: cat failed")
        time.sleep(1)
        cpu1, ws1 = t.cpu(), cpu_seconds(ws)
        row["cat_wall_s"] = round(e - s, 2)
        row["cat_cpu_s"] = round(cpu1 - cpu0, 2)
        row["cat_ws_cpu_s"] = round(ws1 - ws0, 2)
        row["cat_total_cpu_s"] = round(cpu1 - cpu0 + ws1 - ws0, 2)
        row["cat_peak_mb"] = round(max(peak[0], t.footprint()), 1)
        row["cat_after_mb"] = t.footprint()
        row["windows_after"] = t.windows()
        row["load_after"] = loads()
    finally:
        t.stop()
    return row


def measure_tabs(ctx, key, round_no):
    row = {"terminal": key, "kind": "tabs", "round": round_no, "load_before": wait_load()}
    t = KINDS[key](ctx, tabs=True)
    try:
        time.sleep(3)
        for n in range(1, 4):
            t.open_tab(n)
            time.sleep(1.5)
        time.sleep(6)
        row["tabs"] = t.tab_count()
        row["windows"] = t.windows()
        row["tabs_footprint_mb"] = t.footprint()
    finally:
        t.stop()
    return row


def measure_latency(ctx, key, keys=200):
    """Keypress → pixels for one terminal, with the latency probe."""
    import run as latency  # scripts/latency/run.py

    row = {"terminal": key, "kind": "latency"}
    t = KINDS[key](ctx)
    try:
        time.sleep(3)
        main = t.pids()[0]
        out = t.dir / "latency.jsonl"
        echo = f"{sys.executable} {ROOT / 'scripts/latency/echo.py'}"
        with open(t.dir / "cmd", "w") as f:
            f.write(echo + "\n")
        time.sleep(1.5)
        n = keys + latency.WARM_UP
        subprocess.run([str(ctx.probe), "keys", str(main), str(latency.KEY_A)], check=True, capture_output=True)
        time.sleep(0.5)
        subprocess.run([str(ctx.probe), "measure", str(main), str(n), str(latency.KEY_A), "--out", str(out)], check=True)
        rows = latency.read_jsonl(out)[latency.WARM_UP :]
        ms = [r["ms"] for r in rows if "ms" in r]
        row.update(latency.stats(ms))
        row["missed"] = len(rows) - len(ms)
    finally:
        t.stop()
    return row


# --- the report --------------------------------------------------------------


METRICS = [
    ("idle_footprint_mb", "Idle footprint", "MB", "main"),
    ("idle_cpu_pct", "Idle CPU", "%", "main"),
    ("idle_wakeups_per_s", "Idle wakeups (interrupt)", "/s", "main"),
    ("idle_pkg_wakeups_per_s", "Idle wakeups (package idle)", "/s", "main"),
    ("seq_footprint_mb", "Footprint after `seq 1 10000`", "MB", "main"),
    ("cat_wall_s", "`cat` 100 MB: wall time", "s", "main"),
    ("cat_cpu_s", "`cat` 100 MB: terminal CPU", "s", "main"),
    ("cat_ws_cpu_s", "`cat` 100 MB: WindowServer CPU", "s", "main"),
    ("cat_total_cpu_s", "`cat` 100 MB: terminal + WindowServer CPU", "s", "main"),
    ("cat_peak_mb", "`cat` 100 MB: peak footprint", "MB", "main"),
    ("cat_after_mb", "Footprint after the `cat`", "MB", "main"),
    ("tabs_footprint_mb", "Footprint with 4 tabs", "MB", "tabs"),
]

# Roadmap thresholds: Terminal Decline's median against Ghostty's.
THRESHOLDS = {
    "idle_cpu_pct": ("= 0", lambda d, g: d == 0),
    "idle_wakeups_per_s": ("= 0", lambda d, g: d == 0),
    "idle_footprint_mb": ("≤ 35% of Ghostty", lambda d, g: d <= 0.35 * g),
    "seq_footprint_mb": ("≤ 35% of Ghostty", lambda d, g: d <= 0.35 * g),
    "cat_total_cpu_s": ("≤ 45% of Ghostty", lambda d, g: d <= 0.45 * g),
    "tabs_footprint_mb": ("≤ 35% of Ghostty", lambda d, g: d <= 0.35 * g),
}


def fmt(v):
    return f"{v:.2f}" if abs(v) < 10 else f"{v:.1f}" if abs(v) < 100 else f"{v:.0f}"


def report(out_dir):
    rows = [json.loads(l) for l in (out_dir / "results.jsonl").read_text().splitlines() if l.strip()]
    order = [k for k in KINDS if any(r["terminal"] == k for r in rows)]
    order.sort(key=lambda k: list(NAMES).index(k))
    lines = []
    for field, title, unit, kind in METRICS:
        values = {k: [r[field] for r in rows if r["terminal"] == k and r["kind"] == kind and r.get(field) is not None] for k in order}
        med = {k: statistics.median(v) for k, v in values.items() if v}
        lines.append(f"\n### {title} ({unit})\n")
        lines.append("| terminal | median | min – max | runs | vs Terminal Decline |")
        lines.append("|---|---|---|---|---|")
        for k in order:
            v = values[k]
            if not v:
                lines.append(f"| {NAMES[k]} | — | — | 0 | |")
                continue
            ratio = ""
            if k != "decline" and "decline" in med and med[k]:
                ratio = f"{med['decline'] / med[k]:.0%} of this" if med["decline"] else "0"
            lines.append(f"| {NAMES[k]} | {fmt(med[k])} | {fmt(min(v))} – {fmt(max(v))} | {len(v)} | {ratio} |")
        if field in THRESHOLDS and "decline" in med and "ghostty" in med:
            text, check = THRESHOLDS[field]
            ok = check(med["decline"], med["ghostty"])
            lines.append(f"\nThreshold {text}: **{'pass' if ok else 'fail'}** (Terminal Decline {fmt(med['decline'])}, Ghostty {fmt(med['ghostty'])}).")
    lat = [r for r in rows if r["kind"] == "latency"]
    if lat:
        lines.append("\n### Input latency, key posted → pixels changed (ms)\n")
        lines.append("| terminal | p50 | p95 | p99 | keys | missed |")
        lines.append("|---|---|---|---|---|---|")
        for r in lat:
            if "skipped" in r:
                lines.append(f"| {NAMES[r['terminal']]} | skipped: {r['skipped']} | | | | |")
            else:
                lines.append(f"| {NAMES[r['terminal']]} | {r['p50']:.1f} | {r['p95']:.1f} | {r['p99']:.1f} | {r['n']} | {r['missed']} |")
    sizes = {}
    for r in rows:
        if r["kind"] == "main" and r.get("windows"):
            w = max(r["windows"], key=lambda w: w[4] * w[5])
            sizes.setdefault(r["terminal"], set()).add(f"{w[4]}×{w[5]}")
    lines.append("\nWindow size (points): " + "; ".join(f"{NAMES[k]} {', '.join(sorted(sizes[k]))}" for k in order if k in sizes))
    tabs = {k: sorted({str(r.get("tabs")) for r in rows if r["terminal"] == k and r["kind"] == "tabs"}) for k in order}
    lines.append("Tabs seen in the 4-tab runs: " + "; ".join(f"{NAMES[k]} {', '.join(v)}" for k, v in tabs.items() if v))
    l = [x for r in rows for x in [r.get("load_before")] if x]
    if l:
        lines.append(f"1-minute load before runs: {min(x[0] for x in l):.1f} – {max(x[0] for x in l):.1f}")
    print("\n".join(lines))


# --- main ----------------------------------------------------------------------


class Ctx:
    pass


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--terminals", default="decline,ghostty,kitty,warp,terminal")
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--idle", type=int, default=30, help="seconds of idle CPU and wakeups")
    ap.add_argument("--app", default=str(ROOT / "target/release/terminal-decline"))
    ap.add_argument("--out", default=str(ROOT / "target/compare"))
    ap.add_argument("--big", default="", help="the file to cat (default: made with gen.py big in OUT)")
    ap.add_argument("--wait-calm", action="store_true")
    ap.add_argument("--latency", action="store_true")
    ap.add_argument("--skip", default="", help="parts to leave out: main,tabs")
    ap.add_argument("--max-load", type=float, default=6.0, help="pause before a run while the 1-minute load is above this")
    ap.add_argument("--report", action="store_true")
    args = ap.parse_args()
    out_dir = Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    if args.report:
        report(out_dir)
        return
    MAX_LOAD[0] = args.max_load
    terminals = [t for t in args.terminals.split(",") if t]
    for t in terminals:
        if t not in KINDS:
            sys.exit(f"unknown terminal {t}; known: {', '.join(KINDS)}")
        if t in BUNDLES and not BUNDLES[t].exists():
            sys.exit(f"{NAMES[t]} is not installed at {BUNDLES[t]}")
        # Never measure or stop an instance someone else is running: the
        # tab opening for Ghostty and Terminal.app goes to the running app.
        if t in BUNDLES and in_bundle(BUNDLES[t]):
            sys.exit(f"{NAMES[t]} is already running; quit it first (this script only measures instances it starts)")

    ctx = Ctx()
    ctx.idle = args.idle
    ctx.work = Path(tempfile.mkdtemp(prefix="decline-compare-"))
    ctx.helper = build_swift(HERE / "helper.swift", out_dir / "helper")
    ctx.big = Path(args.big).resolve() if args.big else out_dir / "big.txt"
    if not ctx.big.exists():
        subprocess.run([sys.executable, str(ROOT / "scripts/pressure/gen.py"), "big", str(ctx.big)], check=True)
    import run as latency  # scripts/latency/run.py

    ctx.decline_bundle = None
    if "decline" in terminals:
        latency.BUNDLE_ID = DECLINE_BUNDLE_ID
        ctx.decline_bundle = latency.make_bundle(Path(args.app), out_dir)

    backup = out_dir / "com.apple.Terminal.backup.plist"
    ctx.terminal_backup = backup
    if "terminal" in terminals:
        subprocess.run(["defaults", "export", AppleTerminal.DOMAIN, str(backup)], check=True)

    results = out_dir / "results.jsonl"
    skip = set(args.skip.split(","))
    try:
        if args.wait_calm:
            wait_calm()
        for r in range(1, args.rounds + 1):
            for kind, fn in (("main", measure_main), ("tabs", measure_tabs)):
                if kind in skip:
                    continue
                for t in terminals:
                    log(f"round {r}: {NAMES[t]} {kind}")
                    try:
                        row = fn(ctx, t, r)
                    except Exception as e:  # keep going; the row says why
                        row = {"terminal": t, "kind": kind, "round": r, "error": str(e)}
                    row["time"] = time.strftime("%Y-%m-%dT%H:%M:%S")
                    log(json.dumps(row))
                    with open(results, "a") as f:
                        f.write(json.dumps(row) + "\n")
        if args.latency:
            ctx.probe = latency.build_probe(out_dir)
            check = subprocess.run([str(ctx.probe), "check"], capture_output=True, text=True).stdout.strip()
            for t in terminals:
                if "screen_recording=false" in check or "post_events=false" in check:
                    row = {"terminal": t, "kind": "latency", "skipped": f"needs Accessibility and Screen Recording ({check})"}
                else:
                    wait_load()
                    try:
                        row = measure_latency(ctx, t)
                    except Exception as e:
                        row = {"terminal": t, "kind": "latency", "skipped": f"error: {e}"}
                log(json.dumps(row))
                with open(results, "a") as f:
                    f.write(json.dumps(row) + "\n")
    finally:
        if ctx.decline_bundle:
            subprocess.run([LSREGISTER, "-u", str(ctx.decline_bundle)], capture_output=True)
        restore_warp_settings(ctx)
        if "terminal" in terminals and backup.exists() and not in_bundle(BUNDLES["terminal"]):
            restore_terminal_prefs(backup)
        shutil.rmtree(ctx.work, ignore_errors=True)
    report(out_dir)


if __name__ == "__main__":
    main()
