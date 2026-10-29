#!/usr/bin/env python3
"""Run vttest and esctest2 against decline's terminal, headless.

The terminal is `crates/term-pty/examples/headless.rs`: term-core on a real
PTY, no window. vttest runs the menus named in tests/conformance/vttest-menus.txt
(chosen by title, so renumbering in vttest doesn't matter); each screen it
stops on is compared with tests/conformance/vttest/<menu>.txt. esctest2 runs
whole and reports each test itself. Failures are checked against
tests/conformance/accepted-failures.txt: a failure not listed, or a listed
test that now passes, fails the run.

    scripts/conformance/run.py                 # fetch, build, run both
    scripts/conformance/run.py vttest [--only cursor-movements]
    scripts/conformance/run.py esctest [--include 'CUP|ED']
    scripts/conformance/run.py vttest --bless  # write the screens as expected
    scripts/conformance/run.py vttest --terminal tmux --out DIR
                                               # same menus under tmux, to compare

See docs/conformance.md.
"""

import argparse
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import time
import urllib.request

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
DATA = os.path.join(ROOT, "tests", "conformance")
WORK = os.path.join(ROOT, "target", "conformance")
RESULTS = os.path.join(WORK, "results")

VTTEST_VERSION = "20251205"
VTTEST_URL = f"https://invisible-island.net/archives/vttest/vttest-{VTTEST_VERSION}.tgz"
VTTEST_SHA256 = "cd6886f9aefe6a3f6c566fa61271a55710901a71849c630bf5376aa984bf77cc"
ESCTEST_COMMIT = "2798f12149a19c3295e9b4853ab2da4b2eff1b2b"
ESCTEST_URL = f"https://github.com/ThomasDickey/esctest2/archive/{ESCTEST_COMMIT}.tar.gz"

MENU_PROMPT = "Enter choice number"
SCREEN_RE = re.compile(r"^--- screen (\d+) ---$")


def log(msg):
    print(msg, file=sys.stderr, flush=True)


# ------------------------------------------------------------------ fetch


def download(url, dest, sha256=None):
    if not os.path.exists(dest):
        log(f"fetching {url}")
        tmp = dest + ".part"
        with urllib.request.urlopen(url) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f)
        os.replace(tmp, dest)
    if sha256:
        digest = hashlib.sha256(open(dest, "rb").read()).hexdigest()
        if digest != sha256:
            os.remove(dest)
            sys.exit(f"{dest}: sha256 {digest}, expected {sha256}")


def extract(archive, into):
    with tarfile.open(archive) as t:
        for m in t.getmembers():
            if m.name.startswith("/") or ".." in m.name.split("/"):
                sys.exit(f"{archive}: unsafe path {m.name}")
        t.extractall(into)


def fetch_vttest():
    src = os.path.join(WORK, f"vttest-{VTTEST_VERSION}")
    binary = os.path.join(src, "vttest")
    if os.path.exists(binary):
        return binary
    os.makedirs(WORK, exist_ok=True)
    archive = os.path.join(WORK, f"vttest-{VTTEST_VERSION}.tgz")
    download(VTTEST_URL, archive, VTTEST_SHA256)
    extract(archive, WORK)
    log("building vttest")
    subprocess.run(["./configure", "-q"], cwd=src, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["make", "-s"], cwd=src, check=True, stdout=subprocess.DEVNULL)
    return binary


def fetch_esctest():
    src = os.path.join(WORK, f"esctest2-{ESCTEST_COMMIT}")
    script = os.path.join(src, "esctest", "esctest.py")
    if os.path.exists(script):
        return script
    os.makedirs(WORK, exist_ok=True)
    archive = os.path.join(WORK, f"esctest2-{ESCTEST_COMMIT}.tar.gz")
    download(ESCTEST_URL, archive)
    extract(archive, WORK)
    return script


def build_headless():
    log("building the headless terminal")
    subprocess.run(
        ["cargo", "build", "--quiet", "--locked", "--release", "-p", "term-pty", "--example", "headless"],
        cwd=ROOT,
        check=True,
    )
    return os.path.join(ROOT, "target", "release", "examples", "headless")


# --------------------------------------------------------------- terminals


class Decline:
    """The headless terminal (crates/term-pty/examples/headless.rs)."""

    def __init__(self, argv, cols=80, lines=24, cwd=None):
        self.p = subprocess.Popen(
            [build_headless.path, "--cols", str(cols), "--lines", str(lines), "--"] + argv,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            cwd=cwd,
        )

    def _ask(self, line):
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        return self.p.stdout.readline().strip()

    def send(self, data):
        self._ask("send " + data.encode().hex())

    def enter(self):
        self._ask("enter")

    def idle(self, quiet_ms=700, max_ms=10000):
        self._ask(f"idle {quiet_ms} {max_ms}")

    def screen(self):
        head = self._ask("screen")
        n = int(head.split()[1])
        rows = [self.p.stdout.readline().rstrip("\n") for _ in range(n)]
        self.p.stdout.readline()  # cursor
        return rows

    def wait(self, max_ms):
        return self._ask(f"wait {max_ms}")

    def close(self):
        try:
            self.p.stdin.close()
        except OSError:
            pass
        self.p.wait(timeout=10)


class Tmux:
    """The same, under tmux: a second opinion when reviewing screens."""

    SOCKET = "decline-conformance"

    def __init__(self, argv, cols=80, lines=24, cwd=None):
        self.tmux(["kill-server"], check=False)
        self.tmux(["new-session", "-d", "-x", str(cols), "-y", str(lines), "-c", cwd or os.getcwd()] + argv)

    def tmux(self, args, check=True):
        return subprocess.run(["tmux", "-L", self.SOCKET, "-f", "/dev/null"] + args, check=check, capture_output=True, text=True).stdout

    def send(self, data):
        self.tmux(["send-keys", "-H"] + [f"{b:02x}" for b in data.encode()])

    def enter(self):
        self.send("\r")

    def screen(self):
        return [r.rstrip() for r in self.tmux(["capture-pane", "-p"]).split("\n")][:-1]

    def idle(self, quiet_ms=700, max_ms=10000):
        end = time.time() + max_ms / 1000
        last, since = None, time.time()
        while time.time() < end:
            s = self.screen()
            if s != last:
                last, since = s, time.time()
            elif time.time() - since >= quiet_ms / 1000:
                return
            time.sleep(0.05)

    def close(self):
        self.tmux(["kill-server"], check=False)


TERMINALS = {"decline": Decline, "tmux": Tmux}


# ------------------------------------------------------------------ vttest


def read_menus():
    menus = []
    for line in open(os.path.join(DATA, "vttest-menus.txt")):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        name, path = (s.strip() for s in line.split("|", 1))
        menus.append((name, [p.strip() for p in path.split(" > ")]))
    return menus


def at_menu(rows):
    return any(MENU_PROMPT in r for r in rows)


def wait_for_menu(term, timeout=20):
    end = time.time() + timeout
    while time.time() < end:
        term.idle()
        if at_menu(term.screen()):
            return
    raise RuntimeError("vttest did not show its menu:\n" + "\n".join(term.screen()))


def start_vttest(terminal, vttest):
    term = TERMINALS[terminal]([vttest], cwd=WORK)
    wait_for_menu(term)
    return term


def choose(term, title):
    """Pick the entry starting with `title` on the menu on screen."""
    for row in term.screen():
        m = re.match(r"^\s*(\d+)[.*]\s+(.*)$", row)
        if m and m.group(2).startswith(title):
            term.send(m.group(1))
            term.enter()
            term.idle()
            return
    raise RuntimeError(f"no menu entry {title!r} on screen:\n" + "\n".join(term.screen()))


def run_menu(term, path, max_screens=80):
    """Run one leaf test from the main menu; returns the screens it stopped
    on, and leaves vttest back on the main menu."""
    for title in path:
        choose(term, title)
    screens = []
    rows = term.screen()
    while not at_menu(rows):
        screens.append(rows)
        if len(screens) >= max_screens:
            raise RuntimeError(f"{' > '.join(path)}: no menu after {max_screens} screens")
        term.enter()
        term.idle()
        rows = term.screen()
    if not screens:
        to_main(term)
        raise RuntimeError(f"{' > '.join(path)}: a menu, not a test; name one of its entries")
    to_main(term)
    return screens


def to_main(term):
    """Leave submenus until vttest's main menu is on screen."""
    for _ in range(10):
        rows = term.screen()
        if any("VT100 test program" in r for r in rows) and at_menu(rows):
            return
        if at_menu(rows):
            term.send("0")
            term.enter()
        else:
            term.enter()
        term.idle()
    raise RuntimeError("could not get back to the main menu:\n" + "\n".join(term.screen()))


def format_screens(screens):
    out = []
    for i, rows in enumerate(screens, 1):
        out.append(f"--- screen {i:02} ---")
        while rows and not rows[-1]:
            rows = rows[:-1]
        out.extend(rows)
    return "\n".join(out) + "\n"


def parse_screens(text):
    screens, cur = {}, None
    for line in text.split("\n"):
        m = SCREEN_RE.match(line)
        if m:
            cur = int(m.group(1))
            screens[cur] = []
        elif cur is not None:
            screens[cur].append(line)
    return {k: "\n".join(v).rstrip("\n") for k, v in screens.items()}


def run_vttest(args):
    vttest = fetch_vttest()
    menus = [m for m in read_menus() if not args.only or m[0] in args.only]
    out = args.out or os.path.join(RESULTS, "vttest")
    os.makedirs(out, exist_ok=True)
    results = {}
    term = start_vttest(args.terminal, vttest)
    try:
        for name, path in menus:
            log(f"vttest: {name}")
            try:
                screens = run_menu(term, path)
            except RuntimeError as e:
                log(f"  {e}")
                results[f"vttest:{name}"] = False
                term.close()
                term = start_vttest(args.terminal, vttest)
                continue
            text = format_screens(screens)
            open(os.path.join(out, name + ".txt"), "w").write(text)
            expected_path = os.path.join(DATA, "vttest", name + ".txt")
            if args.bless:
                bless(name, screens, expected_path)
            if not os.path.exists(expected_path):
                log(f"  no expected screens: {expected_path} (run with --bless and review)")
                results[f"vttest:{name}"] = False
                continue
            got, want = parse_screens(text), parse_screens(open(expected_path).read())
            for n in sorted(set(got) | set(want)):
                # "!unknown <why>" marks a screen whose right contents
                # aren't written down (it needs a feature decline lacks).
                ok = got.get(n) == want.get(n) and not (want.get(n) or "").startswith("!unknown")
                results[f"vttest:{name}:{n:02}"] = ok
                if not ok:
                    diff_screen(os.path.join(out, f"{name}-{n:02}.diff"), want.get(n), got.get(n))
    finally:
        term.close()
    return results


def bless(name, screens, path):
    """Write the screens as the expected ones, except screens listed as
    accepted failures: their expected contents were written by hand."""
    accepted = read_accepted()
    keep = parse_screens(open(path).read()) if os.path.exists(path) else {}
    out = []
    for i, rows in enumerate(screens, 1):
        out.append(f"--- screen {i:02} ---")
        if f"vttest:{name}:{i:02}" in accepted and i in keep:
            out.append(keep[i])
            continue
        while rows and not rows[-1]:
            rows = rows[:-1]
        out.extend(rows)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "w").write("\n".join(out) + "\n")


def diff_screen(path, want, got):
    import difflib

    want = (want or "").split("\n")
    got = (got or "").split("\n")
    lines = difflib.unified_diff(want, got, "expected", "actual", lineterm="")
    open(path, "w").write("\n".join(lines) + "\n")


# ----------------------------------------------------------------- esctest


def run_esctest(args):
    script = fetch_esctest()
    os.makedirs(RESULTS, exist_ok=True)
    logfile = os.path.join(RESULTS, "esctest.log")
    if os.path.exists(logfile):
        os.remove(logfile)
    argv = [
        sys.executable,
        script,
        "--expected-terminal=xterm",
        "--xterm-checksum=334",
        f"--logfile={logfile}",
        f"--timeout={args.timeout}",
        "--no-print-logs",
        f"--include={args.include}",
    ]
    # esctest's reset asks for 25x80 (and the terminal can't resize itself).
    term = Decline(argv, cols=80, lines=25, cwd=os.path.dirname(script))
    try:
        status = term.wait(30 * 60 * 1000)
    finally:
        term.close()
    if status == "timeout":
        sys.exit("esctest did not finish in 30 minutes")
    results = {}
    current = None
    for line in open(logfile, errors="replace"):
        line = line.rstrip("\n")
        m = re.search(r"Run test: (\S+)", line)
        if m:
            current = m.group(1)
            continue
        if current is None:
            continue
        if "Passed." in line:
            results[f"esctest:{current}"] = True
            current = None
        elif "FAILED:" in line:
            results[f"esctest:{current}"] = False
            current = None
        elif "Fails as expected" in line or "Skipped because" in line:
            current = None
    if not results:
        sys.exit(f"esctest ran no tests ({status}); see {logfile}")
    return results


# ------------------------------------------------------------------ verdict


def read_accepted():
    accepted = {}
    for line in open(os.path.join(DATA, "accepted-failures.txt")):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split(None, 2)
        if len(parts) < 2 or parts[1] not in ("to-fix", "not-planned"):
            sys.exit(f"accepted-failures.txt: bad line {line!r}")
        accepted[parts[0]] = parts[1]
    return accepted


def verdict(results, suites):
    accepted = {k: v for k, v in read_accepted().items() if k.split(":", 1)[0] in suites}
    failed = sorted(k for k, ok in results.items() if not ok)
    new = [k for k in failed if k not in accepted]
    fixed = sorted(k for k in accepted if results.get(k) is True)
    passed = sum(1 for ok in results.values() if ok)
    lines = [f"{passed} passed, {len(failed)} failed ({len(failed) - len(new)} accepted)"]
    for k in new:
        lines.append(f"NEW FAILURE  {k}")
    for k in fixed:
        lines.append(f"NOW PASSES   {k}  (remove it from tests/conformance/accepted-failures.txt)")
    summary = "\n".join(lines)
    os.makedirs(RESULTS, exist_ok=True)
    with open(os.path.join(RESULTS, "summary.txt"), "w") as f:
        f.write(summary + "\n")
        for k in sorted(results):
            f.write(f"{'pass' if results[k] else 'FAIL'}  {k}\n")
    print(summary)
    return not new and not fixed


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("suite", nargs="?", default="all", choices=["all", "vttest", "esctest", "fetch"])
    ap.add_argument("--only", nargs="*", help="vttest: only these menus")
    ap.add_argument("--bless", action="store_true", help="vttest: write the screens as the expected ones")
    ap.add_argument("--terminal", default="decline", choices=sorted(TERMINALS), help="vttest: terminal to run in")
    ap.add_argument("--out", help="vttest: where to write the screens")
    ap.add_argument("--include", default=".*", help="esctest: tests matching this regex")
    ap.add_argument("--timeout", default="0.3", help="esctest: seconds to wait for each reply")
    args = ap.parse_args()

    if args.suite == "fetch":
        fetch_vttest()
        fetch_esctest()
        build_headless()
        return
    build_headless.path = build_headless()
    results, suites = {}, []
    if args.suite in ("all", "vttest"):
        results.update(run_vttest(args))
        suites.append("vttest")
    if args.suite in ("all", "esctest"):
        results.update(run_esctest(args))
        suites.append("esctest")
    if args.terminal != "decline" or args.bless:
        return
    if args.only or args.include != ".*":
        # A partial run: only judge what ran.
        accepted = read_accepted()
        results = {k: v for k, v in results.items()}
        ok = all(v or k in accepted for k, v in results.items())
        print(f"{sum(results.values())} passed, {sum(1 for v in results.values() if not v)} failed")
        sys.exit(0 if ok else 1)
    sys.exit(0 if verdict(results, suites) else 1)


if __name__ == "__main__":
    main()
