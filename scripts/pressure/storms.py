#!/usr/bin/env python3
"""Resize, font and tab storms against the real app, with output flooding.

usage: storms.py BINARY WORKDIR SCENARIO [SCENARIO...]
       storms.py --list

Each scenario starts one or more app processes (own temp config home, own
DECLINE_TEST_DUMP file) whose programs are this script in `--worker` mode:
they flood output and drive the app through OSC 7770 control commands
(resize, font, tab, front, close, dump, quit). The harness samples every
process with /usr/bin/footprint, ps (CPU, threads) and lsof (fds), reads the
dumps (glyph counts, texture sizes, footprint), checks for hangs (a worker
whose progress file stops moving) and crash logs, and prints one JSON line
per scenario. Exits 1 if any scenario crashed or hung.

Windows in one process are tabs; "N windows" scenarios start N processes.
Durations scale with STORM_SCALE (default 1.0, e.g. 0.2 for a quick check).
"""
import json, os, random, re, shutil, subprocess, sys, tempfile, threading, time

SCALE = float(os.environ.get("STORM_SCALE", "1"))
LOGS = os.path.expanduser("~/Library/Logs/terminal-decline")
DIAG = os.path.expanduser("~/Library/Logs/DiagnosticReports")


# ---------------------------------------------------------------- scenarios

def S(**kw):
    return kw

SCENARIOS = {
    # N windows (processes), each flooding and resized at random, as fast as it can.
    **{f"resize-{n}w": S(mode="resize", procs=n, tabs=1, dur=60) for n in (1, 4, 8, 16)},
    # Font size random walk while flooding: one window, then 4 tabs sharing glyph textures.
    # Control: the same flood without resizing.
    "flood-1w": S(mode="resize", procs=1, tabs=1, dur=60, noresize=True),
    "font-1w": S(mode="font", procs=1, tabs=1, dur=90),
    "font-4t": S(mode="font", procs=1, tabs=4, dur=90),
    "font-4w": S(mode="font", procs=4, tabs=1, dur=60),
    # Many tabs, hidden ones idle, then rapid switching with some flooding.
    "tabs-10": S(mode="tabs", procs=1, tabs=10, dur=40, settle=12, cool=12, flood_every=3),
    "tabs-30": S(mode="tabs", procs=1, tabs=30, dur=60, settle=15, cool=15, flood_every=5),
    "tabs-60": S(mode="tabs", procs=1, tabs=60, dur=60, settle=20, cool=15, flood_every=6),
    # Open/close churn of tabs for minutes.
    "churn-tabs": S(mode="churn", procs=1, tabs=1, dur=300),
    # Whole-app open/close churn: windows started and quit again.
    "churn-procs": S(mode="procs", procs=1, tabs=1, dur=180),
}


# ------------------------------------------------------------------ worker

def osc(cmd):
    return f"\x1b]7770;{cmd}\x07".encode()

class Out:
    def __init__(self):
        self.fd = 1
    def w(self, b):
        while b:
            n = os.write(self.fd, b)
            b = b[n:]

def flood_block(rng, size=24000):
    """Coloured text lines with a spread of glyphs, so the glyph atlas is used."""
    lines, n = [], 0
    alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 {}[]()<>=+-*/_.,;:!?#$%&@"
    extra = "àéîöüñçßøåæЖЯщжλΩπ→←↑↓│─┌┐└┘█▒░✓✗★日本語中文한국어"
    while n < size:
        s = "".join(rng.choice(alphabet if rng.random() > 0.08 else extra) for _ in range(rng.randint(20, 160)))
        l = f"\x1b[3{rng.randint(1, 7)}m{s}\x1b[0m\r\n"
        lines.append(l)
        n += len(l)
    return "".join(lines).encode()

def tick(path, n=0):
    with open(path, "w") as f:
        f.write(str(n))

def worker(cfgpath):
    cfg = json.load(open(cfgpath))
    state = cfg["state"]
    rng = random.Random()
    out = Out()
    slot = None
    for i in range(cfg["tabs"] + 200):
        try:
            os.mkdir(f"{state}/slot{i}")
            slot = i
            break
        except FileExistsError:
            continue
    prog = f"{state}/progress{slot}"
    tick(prog)
    t0p = f"{state}/t0"
    if slot == 0:
        with open(t0p, "w") as f:
            f.write(str(time.time()))
    while not os.path.exists(t0p):
        time.sleep(0.02)
    time.sleep(0.05)
    t0 = float(open(t0p).read())
    mode, dur = cfg["mode"], cfg["dur"] * SCALE
    block = flood_block(rng)
    for _ in range(300):  # some scrollback
        out.w(f"tab {slot}: line of scrollback text for the hidden-tab test\r\n".encode())

    def until(t):
        return time.time() < t

    if mode in ("resize", "font"):
        end = t0 + dur
        if cfg["tabs"] > 1 and slot == 0:
            for _ in range(cfg["tabs"] - 1):
                out.w(osc("tab"))
                time.sleep(0.3)
        last_dump = 0
        size = 0
        direction = 1
        n = 0
        while until(end):
            out.w(block)
            n += 1
            if mode == "resize":
                if not cfg.get("noresize"):
                    out.w(osc(f"resize {rng.randint(8, 240)} {rng.randint(3, 80)}"))
                if n % 4 == 0:
                    time.sleep(0.01)
            else:
                step = rng.choice([1, 1, 1, 2, 3])
                if size + direction * step > 60 or size + direction * step < -6:
                    direction = -direction
                size += direction * step
                out.w(osc(f"font {direction * step:+d}"))
                if n % 3 == 0:
                    time.sleep(0.03)
            if time.time() - last_dump > 1.5:
                out.w(osc("dump"))
                last_dump = time.time()
            tick(prog, n)
        out.w(osc("resize 80 24" if mode == "resize" else "dump"))
        out.w(osc("dump"))
        tick(prog, n)
        open(f"{state}/done{slot}", "w").close()
        if slot == 0:
            # wait for the other tabs to finish before quitting
            time.sleep(3)
            out.w(osc("quit"))
        time.sleep(600)
        return

    if mode == "tabs":
        n, settle, cool = cfg["tabs"], cfg["settle"] * SCALE, cfg["cool"] * SCALE
        open_s = n * 0.25 + 3
        storm_start = t0 + open_s + settle
        storm_end = storm_start + dur
        cool_end = storm_end + cool
        json.dump({"open_end": t0 + open_s, "storm_start": storm_start, "storm_end": storm_end,
                   "cool_end": cool_end}, open(f"{state}/phases.json", "w")) if slot == 0 else None
        if slot == 0:
            for _ in range(n - 1):
                out.w(osc("tab"))
                time.sleep(0.25)
        flooder = slot % cfg["flood_every"] == 0
        while until(storm_start):
            tick(prog)
            time.sleep(0.2)
        while until(storm_end):
            if flooder:
                out.w(block)
            out.w(osc("front"))
            time.sleep(rng.uniform(0.02, 0.3) if not flooder else 0.01)
            tick(prog)
        out.w(osc("dump"))
        while until(cool_end):
            tick(prog)
            time.sleep(0.5)
        out.w(osc("dump"))
        open(f"{state}/done{slot}", "w").close()
        if slot == 0:
            time.sleep(3)
            out.w(osc("quit"))
        time.sleep(600)
        return

    if mode == "churn":
        end = t0 + dur
        if slot == 0:
            last_dump = 0
            while until(end):
                out.w(osc("tab"))
                tick(prog)
                time.sleep(0.4)
                if time.time() - last_dump > 10:
                    out.w(osc("dump"))
                    last_dump = time.time()
            out.w(osc("dump"))
            open(f"{state}/done{slot}", "w").close()
            time.sleep(3)
            out.w(osc("quit"))
            time.sleep(600)
        # a short-lived tab: maybe floods briefly, then closes
        life = time.time() + rng.uniform(0.3, 2.5)
        flooder = rng.random() < 0.3
        while until(life):
            if flooder:
                out.w(block)
            else:
                time.sleep(0.05)
        out.w(osc("close"))
        time.sleep(600)
        return


# ----------------------------------------------------------------- harness

def sh(*a):
    return subprocess.run(a, capture_output=True, text=True).stdout

def footprint(pid):
    m = re.search(r"phys_footprint:\s+([\d.]+)\s+(KB|MB|GB)", sh("/usr/bin/footprint", "-p", str(pid)))
    if not m:
        return None
    v = float(m.group(1))
    return v / 1024 if m.group(2) == "KB" else v * 1024 if m.group(2) == "GB" else v

def cpu_seconds(pid):
    out = sh("ps", "-o", "cputime=", "-p", str(pid)).strip()
    if not out:
        return None
    t = 0.0
    for p in out.replace("-", ":").split(":"):
        t = t * 60 + float(p)
    return t

def threads(pid):
    return max(0, len(sh("ps", "-M", "-p", str(pid)).strip().splitlines()) - 1)

def fds(pid):
    return max(0, len(sh("lsof", "-n", "-P", "-p", str(pid)).splitlines()) - 1)

def crash_files(since):
    found = []
    for d in (LOGS, DIAG):
        if os.path.isdir(d):
            for f in os.listdir(d):
                p = os.path.join(d, f)
                if os.path.getmtime(p) >= since and ("decline" in f.lower() or d == LOGS):
                    found.append(p)
    return found

class Proc:
    def __init__(self, binary, work, name, cfg):
        self.dir = f"{work}/{name}"
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(f"{self.dir}/config")
        os.makedirs(f"{self.dir}/state")
        self.cfg = dict(cfg, state=f"{self.dir}/state")
        json.dump(self.cfg, open(f"{self.dir}/cfg.json", "w"))
        self.dump_path = f"{self.dir}/dump.jsonl"
        env = dict(os.environ, XDG_CONFIG_HOME=f"{self.dir}/config", DECLINE_TEST_DUMP=self.dump_path, DECLINE_DEBUG="1")
        self.err = open(f"{self.dir}/stderr.txt", "w")
        self.p = subprocess.Popen([binary, "-e", sys.executable, os.path.abspath(__file__), "--worker", f"{self.dir}/cfg.json"],
                                  env=env, stdout=subprocess.DEVNULL, stderr=self.err, cwd=self.dir)
        self.pos = 0
        self.dumps = []          # trimmed
        self.series = []         # (t, mb, cpu_s, threads, fds)
        self.last_progress = time.time()
        self.progress_seen = 0.0
        self.hung = False
        self.done_at = None
        self.end_at = None
        self.shots = []

    def read_dumps(self):
        try:
            with open(self.dump_path, "rb") as f:
                f.seek(self.pos)
                data = f.read()
        except FileNotFoundError:
            return
        end = data.rfind(b"\n")
        if end < 0:
            return
        self.pos += end + 1
        for line in data[:end].splitlines():
            try:
                d = json.loads(line)
            except ValueError:
                continue
            d.pop("screen_full", None)
            scr = d.pop("screen", "")
            d["screen_chars"] = sum(1 for c in scr if not c.isspace() and c != "|")
            self.dumps.append(d)

    def progress(self):
        latest = 0.0
        sd = self.cfg["state"]
        for f in os.listdir(sd):
            if f.startswith("progress"):
                try:
                    latest = max(latest, os.path.getmtime(f"{sd}/{f}"))
                except OSError:
                    pass
        if latest > self.progress_seen:
            self.progress_seen = latest
            self.last_progress = time.time()
        return time.time() - self.last_progress

    def iterations(self):
        total = 0
        sd = self.cfg["state"]
        for f in os.listdir(sd):
            if f.startswith("progress"):
                try:
                    total += int(open(f"{sd}/{f}").read() or 0)
                except ValueError:
                    pass
        return total

    def window_ids(self):
        try:
            return sorted(set(int(x) for x in re.findall(r"window (\d+)", open(f"{self.dir}/stderr.txt").read())))
        except OSError:
            return []

def screenshot(proc, path):
    ids = proc.window_ids()
    if not ids:
        return None
    subprocess.run(["screencapture", "-x", "-o", f"-l{ids[-1]}", path])
    return path if os.path.exists(path) else None

def run_scenario(binary, work, name, spec):
    since = time.time()
    procs = []
    base_cfg = dict(spec)
    for i in range(spec["procs"]):
        procs.append(Proc(binary, work, f"{name}-{i}", base_cfg))
        time.sleep(0.4)
    t_start = time.time()
    limit = spec["dur"] * SCALE + spec.get("settle", 0) * SCALE + spec.get("cool", 0) * SCALE + spec["tabs"] * 0.3 + 180
    shot_every, last_shot = 15, 0
    blank = []
    peak_total = 0.0
    while time.time() - t_start < limit:
        if all(p.p.poll() is not None for p in procs):
            break
        total = 0.0
        for p in procs:
            if p.p.poll() is not None:
                p.end_at = p.end_at or time.time()
                continue
            f = footprint(p.p.pid)
            if f:
                total += f
                p.series.append((round(time.time() - t_start, 1), round(f, 1), cpu_seconds(p.p.pid), threads(p.p.pid), fds(p.p.pid) if len(p.series) % 3 == 0 else None))
            p.read_dumps()
            stalled = p.progress()
            if os.path.exists(f"{p.cfg['state']}/done0"):
                if p.done_at is None:
                    p.done_at = time.time()
                continue
            if stalled > 25 and not p.hung and time.time() - t_start > 20:
                p.hung = True
                subprocess.run(["sample", str(p.p.pid), "3", "-file", f"{p.dir}/hang-sample.txt"], capture_output=True)
        peak_total = max(peak_total, total)
        if time.time() - last_shot > shot_every and procs[0].p.poll() is None:
            n = len(procs[0].shots)
            s = screenshot(procs[0], f"{procs[0].dir}/shot-{n}.png")
            if s:
                procs[0].shots.append(s)
            last_shot = time.time()
        time.sleep(1.0)
    wall = time.time() - t_start
    for p in procs:
        p.read_dumps()
    survivors = [p for p in procs if p.p.poll() is None]
    if survivors:
        # those still running at the limit: final screenshot, then kill
        for p in survivors:
            screenshot(p, f"{p.dir}/shot-final.png")
    exits = [p.p.poll() for p in procs]
    for p in procs:
        if p.p.poll() is None:
            p.p.kill()
        p.p.wait()
    crashes = crash_files(since)
    # per-process summaries
    summ = []
    for p in procs:
        mb = [s[1] for s in p.series] or [0]
        cpu = [s[2] for s in p.series if s[2] is not None]
        th = [s[3] for s in p.series]
        fd = [s[4] for s in p.series if s[4] is not None]
        d = p.dumps
        glyph_sizes = sorted({(x["mask_texture"], x["color_texture"]) for x in d})
        # blank glyph check: a visible window full of flooded text with few glyphs
        sus = [(x["window"], x["seq"], x["glyphs"], x["cols"] * x["lines"]) for x in d
               if x["visible"] and x["glyphs"] < 0.15 * x["cols"] * x["lines"] and x["screen_chars"] > 0.5 * x["cols"] * x["lines"]]
        summ.append({
            "exit": p.p.returncode, "hung": p.hung, "drain_s": round(p.end_at - p.done_at, 1) if p.end_at and p.done_at else None, "worker_loops": p.iterations(), "dumps": len(d),
            "mb_first": mb[0], "mb_peak": max(mb), "mb_last": mb[-1],
            "cpu_s": cpu[-1] if cpu else None,
            "threads_min_max": [min(th), max(th)] if th else None,
            "fds_first_last": [fd[0], fd[-1]] if fd else None,
            "mask_tex_max": max((x["mask_texture"] for x in d), default=None),
            "color_tex_max": max((x["color_texture"] for x in d), default=None),
            "tex_sizes": glyph_sizes[-6:],
            "max_tabs": max((x["tabs"] for x in d), default=None),
            "font_range": [min((x["font_size"] for x in d), default=None), max((x["font_size"] for x in d), default=None)],
            "blank_suspects": sus[:5], "n_blank_suspects": len(sus),
            "footprint_series": [s[1] for s in p.series][:: max(1, len(p.series) // 40)],
            "shots": p.shots,
        })
        # tabs: footprint at phase ends
        ph = f"{p.cfg['state']}/phases.json"
        if os.path.exists(ph):
            phases = json.load(open(ph))
            t0 = float(open(f"{p.cfg['state']}/t0").read())
            def at(t):
                best = min(p.series, key=lambda s: abs(s[0] + t_start - t), default=None) if p.series else None
                return best[1] if best else None
            summ[-1]["mb_at"] = {"before_tabs": at(t0 + 0.5), **{k: at(v) for k, v in phases.items()}}
            hid = [x for x in d if x["seq"] and not x["visible"]]
            summ[-1]["hidden_dumps"] = len(hid)
            summ[-1]["hidden_count_max"] = max((x["hidden"] for x in d), default=None)
    res = {
        "scenario": name, "procs": len(procs), "wall_s": round(wall, 1), "peak_total_mb": round(peak_total, 1),
        "all_exited_cleanly": all(e == 0 for e in exits), "any_hung": any(p.hung for p in procs),
        "crash_files": crashes, "procs_summary": summ,
    }
    print(json.dumps(res), flush=True)
    return res

def churn_procs(binary, work, name, spec):
    """Start and quit whole app processes, one after another and four at a time."""
    since = time.time()
    t_start = time.time()
    n, bad = 0, 0
    sizes = []
    end = t_start + spec["dur"] * SCALE
    while time.time() < end:
        batch = []
        for i in range(4):
            c = dict(mode="resize", procs=1, tabs=1, dur=3)
            batch.append(Proc(binary, work, f"{name}-{n}-{i}", c))
        for p in batch:
            try:
                p.p.wait(timeout=40)
            except subprocess.TimeoutExpired:
                p.p.kill(); bad += 1
            p.read_dumps()
            if p.p.returncode not in (0,):
                bad += 1
            sizes.append(max((x["phys_footprint"] for x in p.dumps), default=0) / 1048576)
            shutil.rmtree(p.dir, ignore_errors=True)
        n += 1
    res = {"scenario": name, "batches_of_4": n, "bad_exits": bad, "wall_s": round(time.time() - t_start, 1),
           "footprint_mb_first_last_max": [round(sizes[0], 1), round(sizes[-1], 1), round(max(sizes), 1)] if sizes else None,
           "crash_files": crash_files(since)}
    print(json.dumps(res), flush=True)
    return {"all_exited_cleanly": bad == 0, "any_hung": False, "crash_files": res["crash_files"]}

def main():
    if sys.argv[1] == "--worker":
        return worker(sys.argv[2])
    if sys.argv[1] == "--list":
        print("\n".join(SCENARIOS))
        return
    binary, work = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    os.makedirs(work, exist_ok=True)
    names = sys.argv[3:]
    bad = [n for n in names if n not in SCENARIOS]
    if bad:
        sys.exit(f"unknown: {bad}; known: {list(SCENARIOS)}")
    failed = []
    for n in names:
        spec = SCENARIOS[n]
        r = churn_procs(binary, work, n, spec) if spec["mode"] == "procs" else run_scenario(binary, work, n, spec)
        if not r["all_exited_cleanly"] or r["any_hung"] or r["crash_files"]:
            failed.append(n)
    if failed:
        print(f"problems in: {', '.join(failed)}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
