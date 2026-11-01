#!/usr/bin/env python3
"""Hostile-input campaign: runs Terminal Decline on streams meant to break a
terminal (escape-code fuzz, 50 MB lines, alternate-screen toggling, 4 MB
clipboard copies, link and title floods, emoji/ZWJ/combining floods, kitty
keyboard stacks, reset storms, query floods that are never read,
synchronized output never closed, huge SGR lists, sixel/kitty/APC strings),
several instances at once, each scenario repeated.

usage: hostile.py BINARY WORKDIR [--jobs 4] [--repeat 3] [--only a,b]
                  [--skip a,b] [--timeout 300] [--cap-mb 1500]
       hostile.py emit NAME            # run inside the terminal: the stream
       hostile.py list                 # the scenarios

Each run is its own process with `DECLINE_TEST_DUMP` set (see
crates/term-app/src/control.rs). Its program writes the hostile stream,
then puts the screen back (unless the scenario is about what is left on,
like synchronized output never closed), prints `ALIVE-<run>` and asks for
a dump: the run survived only if the stream was taken in full (the program
wasn't blocked forever), the dump arrived with `ALIVE-<run>` on screen (the
app still draws), and `ESC ] 7770 ; quit` then made it exit with 0.

Measured per run: wall time of the stream, CPU seconds, peak and final
physical footprint (`footprint`, not RSS), and panics on stderr. Crash logs
new in ~/Library/Logs/terminal-decline are listed at the end. A run whose
footprint passes --cap-mb is killed and counted as a failure, so a leak
can't take the machine down.

`osc52-valid` really puts 3 MB on the clipboard: it runs last, alone, and
the clipboard's text is put back afterwards (other kinds of clipboard
content are lost). Leave it out with `--skip osc52-valid`.

WORKDIR gets the fuzz inputs (made with gen.py on first use), one folder
per run (dump, stderr) and results.jsonl. Prints one JSON line per run and
a summary table; exits 1 if any run didn't survive.
"""
import argparse
import concurrent.futures as cf
import json
import os
import random
import re
import shlex
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SELF = os.path.abspath(__file__)
NOW = "perl -MTime::HiRes=time -e 'printf \"%.3f\", time'"
ESC = b"\x1b"
ST = b"\x1b\\"
MB = 1_000_000

# ---------------------------------------------------------------- streams


def w(data):
    sys.stdout.buffer.write(data)


def emit_line_wide():
    """One 50 MB line of CJK and emoji, never a newline."""
    r = random.Random(1)
    chunk = "".join(chr(r.choice([r.randint(0x4E00, 0x9FFF), r.randint(0x1F300, 0x1F64F)])) for _ in range(250_000)).encode()
    for _ in range(50 * MB // len(chunk) + 1):
        w(chunk)


def emit_line_sgr():
    """One 50 MB line where every character has its own true colour."""
    r = random.Random(2)
    chunks = []
    for _ in range(8):
        chunks.append("".join(f"\x1b[38;2;{r.randrange(256)};{r.randrange(256)};{r.randrange(256)};48;5;{r.randrange(256)}mx" for _ in range(40_000)).encode())
    n = 0
    while n < 50 * MB:
        c = chunks[(n // len(chunks[0])) % len(chunks)]
        w(c)
        n += len(c)


def emit_altscreen():
    """Alternate screen on and off 600k times (1049, 47, 1047, 1048), with
    scrollback behind it and text on both screens, nested and unbalanced."""
    w(b"".join(b"%d main line\r\n" % i for i in range(20000)))
    modes = [b"1049", b"47", b"1047"]
    for block in range(60):
        out = []
        for i in range(10_000):
            m = modes[i % 3]
            out.append(b"\x1b[?%sh\x1b[2J\x1b[Halt %d\x1b[?1048h\x1b[10;10Hx\x1b[?1048l" % (m, i))
            if i % 7 == 0:
                out.append(b"\x1b[?1049h\x1b[?1049h")  # twice in a row
            out.append(b"\x1b[?%sl" % m)
            if i % 11 == 0:
                out.append(b"\x1b[?1049l\x1b[?47l")  # off when already off
            out.append(b"m%d\r\n" % i)
        w(b"".join(out))


def osc52(payload_len, valid):
    if valid:
        # Readable text, should a person paste it before it's put back.
        raw = (b"terminal-decline hostile osc52 test " * (payload_len // 48 + 1))[: payload_len * 3 // 4]
        import base64

        return b"\x1b]52;c;" + base64.b64encode(raw) + b"\x07"
    return b"\x1b]52;c;" + b"!" * payload_len + b"\x07"


def emit_osc52_invalid():
    """25 copies of just under 4 MB that aren't base64 (no clipboard)."""
    s = osc52(4 * 1024 * 1024 - 64, False)
    for _ in range(25):
        w(s)
        w(b"after\r\n")


def emit_osc52_over():
    """25 valid copies just over the 4 MB cap: dropped whole."""
    s = osc52(4 * 1024 * 1024 + 4096, True)
    for _ in range(25):
        w(s)
        w(b"after\r\n")


def emit_osc52_valid():
    """10 valid copies just under the 4 MB cap: each goes to the clipboard."""
    s = osc52(4 * 1024 * 1024 - 64, True)
    for _ in range(10):
        w(s)
        w(b"after\r\n")


def emit_osc8_unique():
    """2M links, each to a different URI with its own id."""
    for block in range(200):
        out = []
        for i in range(10_000):
            k = block * 10_000 + i
            out.append(b"\x1b]8;id=l%d;https://example.com/%d/%s\x1b\\L\x1b]8;;\x1b\\" % (k, k, b"p" * (k % 50)))
            if i % 80 == 79:
                out.append(b"\r\n")
        w(b"".join(out))


def emit_osc8_repeated():
    """2M links to the one URI, and a run of URIs at and over the 4 KB cap."""
    one = b"\x1b]8;;https://example.com/the/same/one\x1b\\link\x1b]8;;\x1b\\ "
    for _ in range(200):
        w((one * 80 + b"\r\n") * 125)
    for n in (4095, 4096, 4097, 60_000):
        w((b"\x1b]8;;https://e.com/" + b"u" * n + b"\x1b\\big\x1b]8;;\x1b\\\r\n") * 200)


def emit_title():
    """200k titles (OSC 0/1/2, up to 4 KB, some 64 KB; 250 MB), the title
    stack pushed far past its depth and popped past empty."""
    r = random.Random(3)
    for block in range(20):
        out = []
        for i in range(10_000):
            n = r.choice([1, 20, 200, 4000])
            out.append(b"\x1b]%d;%s %d\x07" % (r.choice([0, 1, 2]), "título 🎉".encode() * (n // 12 + 1), i))
            if i % 3 == 0:
                out.append(b"\x1b[22;0t" * 30)
            if i % 5 == 0:
                out.append(b"\x1b[23;0t" * 40)
            if i % 1000 == 0:
                out.append(b"\x1b]2;" + b"T" * 65_000 + b"\x1b\\")
        w(b"".join(out))
        w(b"titles %d\r\n" % block)


def emit_emoji():
    """ZWJ families, flags and odd regional-indicator runs, skin tones,
    keycaps, combining-mark piles (over the 32-scalar cluster cap), and REP
    of each."""
    family = "👨‍👩‍👧‍👦"
    pieces = [
        family * 20,
        "🇬🇧🇫🇷🇯🇵" * 10,
        "🇬" * 33,  # odd run of regional indicators
        "👍🏽👋🏿🧑🏻‍🚀" * 10,
        "1️⃣#️⃣*️⃣" * 10,
        "e" + "́" * 500,
        "a" + "̀́̂̃̄̅" * 50 + "‍" * 40,
        "‍" * 100 + "️" * 100,
        "👩" + "‍👩" * 60,  # a 121-scalar ZWJ chain
        "🏳️‍🌈🏴‍☠️❤️‍🔥" * 10,
    ]
    out = []
    for p in pieces:
        out.append(p.encode())
        out.append(b"\x1b[65535b")  # REP of the last cluster
        out.append(b"\r\n")
    chunk = b"".join(out)
    for _ in range(50 * MB // len(chunk) + 1):
        w(chunk)


def emit_kitty_kbd():
    """Kitty keyboard flags pushed 1M times (past any stack), set, popped
    past empty, on both screens; no queries (those are in `queries-*`)."""
    for block in range(100):
        out = []
        for i in range(10_000):
            out.append(b"\x1b[>%du" % (i % 32))
            if i % 2:
                out.append(b"\x1b[=%d;%du" % (i % 64, 1 + i % 3))
            if i % 13 == 0:
                out.append(b"\x1b[<%du" % (i % 1000))
            if i % 101 == 0:
                out.append(b"\x1b[?1049h\x1b[>31u\x1b[?1049l\x1b[<99999u")
        out.append(b"k%d\r\n" % block)
        w(b"".join(out))


def emit_reset():
    """DECSTR and RIS 600k times, each after setting scroll regions, origin
    mode, the alternate screen, 2026, colours, charsets, tabs and margins."""
    setup = b"\x1b[5;20r\x1b[?6h\x1b[?1049h\x1b[?2026h\x1b[1;31;48;2;1;2;3m\x1b(0\x1b)0\x0e\x1bH\x1b[?69h\x1b[2;40s\x1b[?25l\x1b[?1000;1006h\x1b[4hreset me"
    for block in range(60):
        out = []
        for i in range(10_000):
            out.append(setup)
            out.append(b"\x1b[!p" if i % 2 else b"\x1bc")
            out.append(b"r%d\r\n" % i)
        w(b"".join(out))


QUERIES = [
    b"\x1bP+q544e\x1b\\",  # XTGETTCAP TN
    b"\x1bP+q536d756c78;536574756c63;5463;524742\x1b\\",  # Smulx Setulc Tc RGB
    b"\x1bP+q" + b"41" * 2000 + b"\x1b\\",  # a long unknown name
    b"\x1bP$qm\x1b\\",
    b"\x1bP$qr\x1b\\",
    b"\x1bP$q q\x1b\\",
    b"\x1bP$q\"p\x1b\\",
    b"\x1b]4;1;?\x07",
    b"\x1b]4;" + b";".join(b"%d;?" % i for i in range(256)) + b"\x07",
    b"\x1b]10;?\x07",
    b"\x1b]11;?\x1b\\",
    b"\x1b]12;?\x07",
    b"\x1b[c",
    b"\x1b[>c",
    b"\x1b[6n",
    b"\x1b[?6n",
    b"\x1b[5n",
    b"\x1b[?2026$p",
    b"\x1b[?u",
    b"\x1b[>q",
    b"\x1b[14t\x1b[18t",
]


def emit_queries():
    """60 MB of queries, never reading a reply."""
    chunk = b"".join(QUERIES) * 20
    n = 0
    while n < 60 * MB:
        w(chunk)
        n += len(chunk)


def emit_sync_open():
    """Synchronized output (2026) turned on and never off: 15 s of
    full-screen redraws, then `ALIVE` with 2026 still on."""
    r = random.Random(7)
    w(b"\x1b[?2026h")
    end = time.time() + 15
    while time.time() < end:
        frame = [b"\x1b[H"]
        for row in range(50):
            frame.append(b"\x1b[%d;1H\x1b[38;5;%dm" % (row + 1, r.randrange(256)) + bytes(r.randint(65, 90) for _ in range(160)))
        if r.random() < 0.1:
            frame.append(b"\x1b[?2026h")  # on again, never off
        w(b"".join(frame))
        sys.stdout.buffer.flush()
    w(b"\x1b[0m\x1b[H\x1b[2J")


def emit_sgr_huge():
    """SGR with 50k parameters, long colon sub-parameter lists, 1 MB
    numbers, and true colours with missing and extra parts."""
    r = random.Random(4)
    long_list = b"\x1b[" + b";".join(b"%d" % r.choice([0, 1, 4, 7, 38, 48, 58, 2, 5, r.randint(0, 300), 65535, 99999999]) for _ in range(50_000)) + b"mX\x1b[0m\r\n"
    colons = b"\x1b[38:2:" + b":".join(b"%d" % r.randrange(1000) for _ in range(20_000)) + b"mY\x1b[0m\r\n"
    big_num = b"\x1b[" + b"9" * MB + b"mZ\x1b[0m\r\n"
    odd = b"\x1b[38;2m\x1b[38;2;1m\x1b[48;5m\x1b[58:2::1:2:3:4:5:6m\x1b[4:9m\x1b[;;;;;;;m\x1b[38;5;-1m\x1b[38;2;256;256;256mW\x1b[0m\r\n" * 100
    n = 0
    while n < 60 * MB:
        for s in (long_list, colons, odd, long_list, colons):
            w(s)
            n += len(s)
        if n % 5 == 0:
            w(big_num)
            n += len(big_num)
    w(big_num)


def emit_images():
    """Sixel (4 MB of data, a 10000×10000 raster), kitty graphics (20 MB in
    chunks, bad keys, deletes), iTerm2 files, an unterminated APC, DCS and
    OSC of 20 MB each: all to be ignored."""
    sixel = b"\x1bPq\"1;1;10000;10000#0;2;100;0;0#1;2;0;100;0" + (b"#0!10000~-#1!500?" * 200_000)[: 4 * MB] + ST
    for _ in range(5):
        w(sixel)
        w(b"after sixel\r\n")
    kchunk = b"A" * 4096
    for img in range(5):
        w(b"\x1b_Ga=T,f=32,s=10000,v=10000,i=%d,m=1;%s\x1b\\" % (img, kchunk))
        for _ in range(1000):
            w(b"\x1b_Gm=1;" + kchunk + ST)
        w(b"\x1b_Gm=0;" + kchunk + ST)
        w(b"\x1b_Ga=d,d=A\x1b\\\x1b_Gzzz=999,a=?,q=2;!!!\x1b\\\x1b_G\x1b\\\x1b_\x1b\\")
        w(b"after kitty %d\r\n" % img)
    w(b"\x1b]1337;File=inline=1;size=4000000:" + b"QUFB" * MB + b"\x07after iterm\r\n")
    for intro in (b"\x1b_", b"\x1bP", b"\x1b]9999;", b"\x1b^", b"\x1bX"):
        w(intro)
        for _ in range(20):
            w(b"z" * MB)
        w(ST)
        w(b"after %s\r\n" % intro[1:2])


EMITTERS = {
    "line-50mb-wide": emit_line_wide,
    "line-50mb-sgr": emit_line_sgr,
    "altscreen": emit_altscreen,
    "osc52-invalid": emit_osc52_invalid,
    "osc52-over": emit_osc52_over,
    "osc52-valid": emit_osc52_valid,
    "osc8-unique": emit_osc8_unique,
    "osc8-repeated": emit_osc8_repeated,
    "title": emit_title,
    "emoji": emit_emoji,
    "kitty-kbd": emit_kitty_kbd,
    "reset": emit_reset,
    "queries": emit_queries,
    "sync-open": emit_sync_open,
    "sgr-huge": emit_sgr_huge,
    "images": emit_images,
}

# ---------------------------------------------------------------- driver

PY = shlex.quote(sys.executable)


def emit_cmd(name):
    return f"{PY} {shlex.quote(SELF)} emit {name}"


def scenarios(work):
    """name -> (shell command, put the screen back before ALIVE)."""
    s = {f"fuzz-64mb-s{seed}": (f"cat {work}/fuzz-{seed}.bin", True) for seed in (1, 2, 3)}
    s["line-50mb-ascii"] = ("head -c 50000000 /dev/zero | tr '\\0' 'x'", True)
    for name in EMITTERS:
        s[name] = (emit_cmd(name), name != "sync-open")
    # Queries never read: with the tty raw (replies queue in the PTY until
    # it's full) and cooked (the line discipline echoes them back).
    del s["queries"]
    s["queries-raw"] = (f"stty raw -echo; {emit_cmd('queries')}; stty sane", True)
    s["queries-cooked"] = (emit_cmd("queries"), True)
    return s


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


def last_dump(path):
    try:
        lines = [l for l in open(path).read().splitlines() if l.strip()]
    except FileNotFoundError:
        return None
    try:
        return json.loads(lines[-1]) if lines else None
    except json.JSONDecodeError:
        return None


LIVE = set()
LIVE_LOCK = threading.Lock()


def run_one(binary, work, name, rep, cmd, reset, timeout, cap_mb):
    d = f"{work}/runs/{name}-{rep}"
    os.makedirs(d, exist_ok=True)
    dump, marker, quit_file, err = f"{d}/dump.jsonl", f"{d}/done", f"{d}/quit", f"{d}/stderr"
    for f in (dump, marker, quit_file):
        if os.path.exists(f):
            os.remove(f)
    token = f"ALIVE-{name}-{rep}"
    back = "printf '\\033[0m\\033[?2026l\\033[?1049l\\033c'; " if reset else ""
    script = (
        f"S=$({NOW}); {cmd}; E=$({NOW}); echo $S $E > {marker}; {back}"
        f"echo {token}; printf '\\033]7770;dump\\007'; "
        f"while [ ! -e {quit_file} ]; do sleep 0.2; done; printf '\\033]7770;quit\\007'; sleep 30"
    )
    env = dict(os.environ, DECLINE_TEST_DUMP=dump, TERM="xterm-256color")
    with open(err, "w") as ef:
        p = subprocess.Popen([binary, "-e", "sh", "-c", script], env=env, stdout=subprocess.DEVNULL, stderr=ef, start_new_session=True)
    with LIVE_LOCK:
        LIVE.add(p)
    t0 = time.time()
    peak, series, status, cpu = 0.0, [], None, None
    dumped = None
    while True:
        if p.poll() is not None:
            status = f"died (exit {p.returncode})"
            break
        f = footprint_mb(p.pid)
        if f:
            peak = max(peak, f)
            series.append(round(f))
            if f > cap_mb:
                status = f"killed: footprint {f:.0f} MB over cap"
                break
        if os.path.exists(marker):
            dumped = last_dump(dump)
            if dumped is not None:
                break
            if time.time() - os.path.getmtime(marker) > 30:
                status = "hung: no dump 30 s after the stream ended"
                break
        if time.time() - t0 > timeout:
            status = "hung: stream not taken in full" if not os.path.exists(marker) else "hung"
            break
        time.sleep(0.5)
    wall, quit_s = None, None
    if os.path.exists(marker):
        s, e = map(float, open(marker).read().split())
        wall = round(e - s, 2)
    final = None
    alive_on_screen = None
    if status is None:
        cpu = cpu_seconds(p.pid)
        final = footprint_mb(p.pid)
        alive_on_screen = token in (dumped.get("screen") or "")
        if not alive_on_screen:
            status = "wrong screen: ALIVE missing from the dump"
    if p.poll() is None:
        if cpu is None:
            cpu = cpu_seconds(p.pid)
        open(quit_file, "w").close()
        tq = time.time()
        try:
            # Quitting closes the window, which waits on the window
            # server; on a busy Mac that has taken over 20 s.
            p.wait(timeout=90)
            quit_s = round(time.time() - tq, 1)
        except subprocess.TimeoutExpired:
            if status is None:
                status = "hung at quit"
            p.kill()
            p.wait()
    if status is None and p.returncode != 0:
        status = f"bad exit {p.returncode}"
    with LIVE_LOCK:
        LIVE.discard(p)
    stderr = open(err, errors="replace").read()
    panics = [l for l in stderr.splitlines() if "panicked" in l]
    if status is None and panics:
        status = "panicked"
    r = {
        "scenario": name,
        "rep": rep,
        "survived": status is None,
        "status": status or "ok",
        "wall_s": wall,
        "cpu_s": cpu and round(cpu, 2),
        "peak_mb": round(peak, 1),
        "final_mb": final and round(final, 1),
        "frames": dumped and dumped.get("frames"),
        "quit_s": quit_s,
        "panics": panics[:3],
        "footprint_series": series[-20:],
    }
    return r


def clipboard_get():
    return subprocess.run(["pbpaste"], capture_output=True).stdout


def clipboard_set(data):
    subprocess.run(["pbcopy"], input=data)


def ensure_inputs(work, names):
    for seed in [int(n.rsplit("-s", 1)[1]) for n in names if n.startswith("fuzz-")]:
        path = f"{work}/fuzz-{seed}.bin"
        if not os.path.exists(path):
            subprocess.run([sys.executable, f"{HERE}/gen.py", "fuzz", path, "64000000", str(seed)], check=True)


def crash_logs():
    d = os.path.expanduser("~/Library/Logs/terminal-decline")
    try:
        return {f for f in os.listdir(d)}
    except FileNotFoundError:
        return set()


def table(results):
    by = {}
    for r in results:
        by.setdefault(r["scenario"], []).append(r)
    out = [
        "| scenario | survived | wall s (median) | CPU s (max) | peak footprint MB (max) | final MB (max) | failures |",
        "|---|--:|--:|--:|--:|--:|---|",
    ]

    def med(v):
        v = sorted(x for x in v if x is not None)
        return v[len(v) // 2] if v else None

    def mx(v):
        v = [x for x in v if x is not None]
        return max(v) if v else None

    def f(v):
        return "–" if v is None else (f"{v:.1f}" if isinstance(v, float) else str(v))

    for name, rs in by.items():
        ok = sum(r["survived"] for r in rs)
        fails = "; ".join(sorted({r["status"] for r in rs if not r["survived"]}))
        out.append(
            f"| {name} | {ok}/{len(rs)} | {f(med(r['wall_s'] for r in rs))} | {f(mx(r['cpu_s'] for r in rs))} "
            f"| {f(mx(r['peak_mb'] for r in rs))} | {f(mx(r['final_mb'] for r in rs))} | {fails} |"
        )
    return "\n".join(out)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "emit":
        EMITTERS[sys.argv[2]]()
        sys.stdout.buffer.flush()
        return 0
    if len(sys.argv) > 1 and sys.argv[1] == "list":
        for n in scenarios("WORK"):
            print(n)
        return 0
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("binary")
    ap.add_argument("workdir")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--only", default="")
    ap.add_argument("--skip", default="")
    ap.add_argument("--timeout", type=float, default=300)
    ap.add_argument("--cap-mb", type=float, default=1500)
    a = ap.parse_args()
    binary, work = os.path.abspath(a.binary), os.path.abspath(a.workdir)
    os.makedirs(work, exist_ok=True)
    all_s = scenarios(work)
    names = [n for n in a.only.split(",") if n] or list(all_s)
    unknown = [n for n in names if n not in all_s]
    if unknown:
        sys.exit(f"unknown scenario(s): {', '.join(unknown)}; known: {', '.join(all_s)}")
    skip = {n for n in a.skip.split(",") if n}
    names = [n for n in names if n not in skip]
    ensure_inputs(work, names)
    alone = [n for n in names if n == "osc52-valid"]
    pooled = [n for n in names if n not in alone]
    logs_before = crash_logs()
    results = []
    out = open(f"{work}/results.jsonl", "a")

    def record(r):
        results.append(r)
        line = json.dumps(r)
        print(line, flush=True)
        out.write(line + "\n")
        out.flush()

    # Repetition-major, so different scenarios run side by side.
    jobs = [(n, rep) for rep in range(a.repeat) for n in pooled]
    try:
        with cf.ThreadPoolExecutor(max_workers=a.jobs) as ex:
            futs = [ex.submit(run_one, binary, work, n, rep, *all_s[n], a.timeout, a.cap_mb) for n, rep in jobs]
            for fu in cf.as_completed(futs):
                record(fu.result())
        if alone:
            saved = clipboard_get()
            try:
                for rep in range(a.repeat):
                    record(run_one(binary, work, "osc52-valid", rep, *all_s["osc52-valid"], a.timeout, a.cap_mb))
            finally:
                clipboard_set(saved)
    finally:
        with LIVE_LOCK:
            for p in LIVE:
                p.kill()
    new_logs = sorted(crash_logs() - logs_before)
    print()
    print(table(results))
    if new_logs:
        print("\nnew crash logs:", *new_logs, sep="\n  ")
    failed = [f"{r['scenario']}#{r['rep']}" for r in results if not r["survived"]]
    if failed:
        print(f"did not survive: {', '.join(failed)}", file=sys.stderr)
    return 1 if failed or new_logs else 0


if __name__ == "__main__":
    sys.exit(main())
