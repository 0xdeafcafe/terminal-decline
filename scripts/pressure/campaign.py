#!/usr/bin/env python3
"""Load-test campaign: many Terminal Decline instances at once, many tabs in
one instance, and a long soak of mixed load, sampling every instance as it
goes. Re-runnable; results go to OUT as JSON lines, `report` turns them
into Markdown.

usage: campaign.py sweep BINARY WORK OUT [--levels 1,4,8,16,32] [--scenarios a,b]
       campaign.py tabs  BINARY WORK OUT [--levels 1,8,32] [--scenario flood-1gb]
       campaign.py soak  BINARY WORK OUT [--minutes 25] [--mixed 5] [--tab-instances 1]
       campaign.py report OUT

WORK holds gen.py's inputs (big.txt, fuzz.bin, glyphs.txt), as for run.py.
Each instance runs with its own empty config (XDG_CONFIG_HOME) and
DECLINE_TEST_DUMP, so its program can drive it with OSC 7770 (dump,
resize, font, tab, front). An instance survives when its program got to
the end, the app exited 0 by itself, its stderr has no panic, and its last
dump shows the output's last line (END-<tag>, plus the scenario's own
last line where it has one).

A sampler thread reads, every --interval seconds and without running
anything, each instance's footprint (phys_footprint, the number
/usr/bin/footprint prints), CPU time, idle and interrupt wake-ups, threads
and open fds (libproc), and WindowServer's CPU time (ps). /usr/bin/footprint
itself is run at the end of each instance to check the sampled numbers.
If the instances' footprints add up to more than --mem-limit-gb, every
instance is killed and the run is recorded as stopped.
"""
import argparse, ctypes, ctypes.util, glob, json, os, re, signal, statistics, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
NOW = "perl -MTime::HiRes=time -e 'printf \"%.3f\", time'"
MB = 1024 * 1024

# --- libproc -----------------------------------------------------------------

_libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
_RUSAGE_V4 = [
    "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups", "pageins", "wired_size", "resident_size",
    "phys_footprint", "proc_start_abstime", "proc_exit_abstime", "child_user_time", "child_system_time",
    "child_pkg_idle_wkups", "child_interrupt_wkups", "child_pageins", "child_elapsed_abstime", "diskio_bytesread",
    "diskio_byteswritten", "qos_default", "qos_maintenance", "qos_background", "qos_utility", "qos_legacy",
    "qos_user_initiated", "qos_user_interactive", "billed_system_time", "serviced_system_time", "logical_writes",
    "lifetime_max_phys_footprint", "instructions", "cycles", "billed_energy", "serviced_energy",
    "interval_max_phys_footprint", "runnable_time",
]


class RusageV4(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [(n, ctypes.c_uint64) for n in _RUSAGE_V4]


class TaskInfo(ctypes.Structure):
    _fields_ = [(n, ctypes.c_uint64) for n in ("virtual_size", "resident_size", "total_user", "total_system", "threads_user", "threads_system")] + [
        (n, ctypes.c_int32)
        for n in ("policy", "faults", "pageins", "cow_faults", "messages_sent", "messages_received", "syscalls_mach", "syscalls_unix", "csw", "threadnum", "numrunning", "priority")
    ]


class _Timebase(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


_tb = _Timebase()
_libc.mach_timebase_info(ctypes.byref(_tb))
# rusage times are mach absolute time units.
ABS_TO_S = _tb.numer / _tb.denom / 1e9


def proc_sample(pid):
    """Footprint, CPU, wake-ups, threads and fds of one of our processes, or
    None if it is gone."""
    ru = RusageV4()
    if _libc.proc_pid_rusage(pid, 4, ctypes.byref(ru)) != 0:
        return None
    ti = TaskInfo()
    n = _libc.proc_pidinfo(pid, 4, ctypes.c_uint64(0), ctypes.byref(ti), ctypes.sizeof(ti))
    threads = ti.threadnum if n == ctypes.sizeof(ti) else None
    buf = (ctypes.c_uint8 * (8 * 4096))()
    n = _libc.proc_pidinfo(pid, 1, ctypes.c_uint64(0), buf, ctypes.sizeof(buf))
    fds = n // 8 if n > 0 else None
    return {
        "fp_mb": ru.phys_footprint / MB,
        "fp_max_mb": ru.lifetime_max_phys_footprint / MB,
        "cpu_s": (ru.user_time + ru.system_time) * ABS_TO_S,
        "idle_w": ru.pkg_idle_wkups,
        "intr_w": ru.interrupt_wkups,
        "threads": threads,
        "fds": fds,
    }


def ps_cpu_seconds(pid):
    out = subprocess.run(["ps", "-o", "cputime=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    if not out:
        return None
    total = 0.0
    for p in out.replace("-", ":").split(":"):
        total = total * 60 + float(p)
    return total


def window_server_pid():
    out = subprocess.run(["pgrep", "-x", "WindowServer"], capture_output=True, text=True).stdout.split()
    return int(out[0]) if out else None


def footprint_tool(pids):
    """/usr/bin/footprint's phys_footprint and its peak, MB, by pid."""
    if not pids:
        return {}
    args = ["/usr/bin/footprint", "--noCategories", "-f", "bytes"]
    for p in pids:
        args += ["-p", str(p)]
    out = subprocess.run(args, capture_output=True, text=True).stdout
    res, cur = {}, None
    for line in out.splitlines():
        m = re.search(r"\[(\d+)\]:", line)
        if m:
            cur = int(m.group(1))
            res[cur] = {}
        m = re.search(r"(phys_footprint(?:_peak)?):\s+(\d+) B", line)
        if m and cur is not None:
            res[cur][m.group(1)] = int(m.group(2)) / MB
    return res


def system_memory():
    """Free + inactive + speculative memory, GB: what is left before swap."""
    out = subprocess.run(["vm_stat"], capture_output=True, text=True).stdout
    page = int(re.search(r"page size of (\d+)", out).group(1))
    v = {k.strip(): int(n.rstrip(".")) for k, n in re.findall(r"^(Pages [^:]+):\s+(\d+)\.", out, re.M)}
    free = v.get("Pages free", 0) + v.get("Pages inactive", 0) + v.get("Pages speculative", 0)
    return round(free * page / 1e9, 2)


# --- sampler -----------------------------------------------------------------


class Sampler(threading.Thread):
    """Samples every tracked pid and WindowServer; kills everything over the
    memory limit."""

    def __init__(self, path, interval, mem_limit_mb, on_limit):
        super().__init__(daemon=True)
        self.path, self.interval, self.limit, self.on_limit = path, interval, mem_limit_mb, on_limit
        self.pids = {}  # tag -> pid
        self.series = {}  # tag -> [(t, sample)]
        self.ws = []  # (t, cpu_s)
        self.ws_pid = window_server_pid()
        self.peak_total = 0.0
        self.stopped = threading.Event()
        self.over = False
        self.lock = threading.Lock()

    def track(self, tag, pid):
        with self.lock:
            self.pids[tag] = pid
            self.series.setdefault(tag, [])

    def untrack(self, tag):
        with self.lock:
            self.pids.pop(tag, None)

    def sample_once(self):
        t = time.time()
        rows, total = {}, 0.0
        with self.lock:
            pids = dict(self.pids)
        for tag, pid in pids.items():
            s = proc_sample(pid)
            if s is None:
                continue
            rows[tag] = s
            total += s["fp_mb"]
            self.series[tag].append((t, s))
        ws = ps_cpu_seconds(self.ws_pid) if self.ws_pid else None
        if ws is not None:
            self.ws.append((t, ws))
        self.peak_total = max(self.peak_total, total)
        rec = {"t": round(t, 2), "total_mb": round(total, 1), "ws_cpu_s": ws, "procs": {k: {a: (round(b, 2) if isinstance(b, float) else b) for a, b in v.items()} for k, v in rows.items()}}
        if len(self.ws) % 6 == 1:
            rec["avail_gb"] = system_memory()
        with open(self.path, "a") as f:
            f.write(json.dumps(rec) + "\n")
        if total > self.limit and not self.over:
            self.over = True
            print(f"memory limit: {total:.0f} MB > {self.limit:.0f} MB; stopping", file=sys.stderr)
            self.on_limit()

    def run(self):
        while not self.stopped.is_set():
            start = time.time()
            try:
                self.sample_once()
            except Exception as e:  # keep sampling
                print(f"sampler: {e}", file=sys.stderr)
            self.stopped.wait(max(0.0, self.interval - (time.time() - start)))

    def ws_cpu_between(self, t0, t1):
        pts = [(t, c) for t, c in self.ws if t0 <= t <= t1]
        if len(pts) < 2:
            return None
        return pts[-1][1] - pts[0][1], pts[-1][0] - pts[0][0]

    def window(self, tag, t0, t1):
        return [(t, s) for t, s in self.series.get(tag, []) if t0 <= t <= t1]


# --- instances ---------------------------------------------------------------

LOGS = os.path.expanduser("~/Library/Logs/terminal-decline")
REPORTS = os.path.expanduser("~/Library/Logs/DiagnosticReports")


def crash_files():
    return set(glob.glob(f"{LOGS}/*")) | set(glob.glob(f"{REPORTS}/*"))


class Instance:
    def __init__(self, binary, run_dir, tag, script, test_dump=True):
        self.tag = tag
        self.test_dump = test_dump
        self.dir = os.path.join(run_dir, tag)
        os.makedirs(os.path.join(self.dir, "config"), exist_ok=True)
        self.marker = os.path.join(self.dir, "marker")
        self.dump = os.path.join(self.dir, "dump.jsonl")
        self.stderr_path = os.path.join(self.dir, "stderr.txt")
        env = {k: v for k, v in os.environ.items() if not k.startswith("DECLINE_")}
        env.update(XDG_CONFIG_HOME=os.path.join(self.dir, "config"), TERM="xterm-256color")
        if test_dump:
            env["DECLINE_TEST_DUMP"] = self.dump
        with open(os.path.join(self.dir, "script.sh"), "w") as f:
            f.write(script)
        self.err = open(self.stderr_path, "w")
        self.p = subprocess.Popen([binary, "-e", "/bin/sh", os.path.join(self.dir, "script.sh")], env=env, cwd=self.dir, stdout=subprocess.DEVNULL, stderr=self.err)
        self.pid = self.p.pid
        self.t_start = time.time()
        self.t_marker = None
        self.final = None

    def done(self):
        return os.path.exists(self.marker)

    def times(self):
        try:
            s, e = map(float, open(self.marker).read().split()[:2])
            return s, e
        except Exception:
            return None

    def stderr(self):
        try:
            return open(self.stderr_path, errors="replace").read()
        except FileNotFoundError:
            return ""

    def dumps(self):
        out = []
        try:
            for line in open(self.dump, errors="replace"):
                try:
                    out.append(json.loads(line))
                except ValueError:
                    pass
        except FileNotFoundError:
            pass
        return out

    def kill(self):
        if self.p.poll() is None:
            try:
                self.p.terminate()
                self.p.wait(5)
            except Exception:
                self.p.kill()


ALL = []


def kill_all():
    for i in ALL:
        i.kill()


def install_signal_handlers():
    def stop(signum, frame):
        kill_all()
        sys.exit(f"stopped by signal {signum}")

    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)


def scenarios(work):
    big = f"{work}/big.txt"
    last = subprocess.run(["tail", "-1", big], capture_output=True, text=True).stdout[:9]
    # name: (command, bytes of output, expected text on the last screen)
    return {
        "flood-1gb": (f"for i in 1 2 3 4 5 6 7 8 9 10; do cat {big}; done", 10 * os.path.getsize(big), last),
        "flood-100mb": (f"cat {big}", os.path.getsize(big), last),
        # BSD seq prints 3e+06 from 1e6 on, as run.py's churn does.
        "churn-3M-lines": ("seq 1 3000000", 30666660, "3e+06|"),
        "fuzz-64mb": (f"cat {work}/fuzz.bin", os.path.getsize(f"{work}/fuzz.bin"), "ALIVE"),
        "glyphs-30k": (f"cat {work}/glyphs.txt", os.path.getsize(f"{work}/glyphs.txt"), None),
        "one-50mb-line": ("head -c 50000000 /dev/zero | tr '\\0' 'x'", 50_000_000, "xxxxxxxxxxxxxxxxxxxx"),
        "sync-flood-15s": (f"python3 {HERE}/gen.py sync 15", None, None),
        "idle-20s": ("sleep 20", None, None),
    }


def end_check(inst, expect, tags=None):
    """Whether the dumps show END-<tag> (each of `tags`) and `expect`."""
    if not inst.test_dump:
        return True, 0
    screens = [d.get("screen", "") for d in inst.dumps()]
    tags = tags or [inst.tag]
    ok_end = all(any(f"END-{t}" in s for s in screens) for t in tags)
    ok_expect = expect is None or any(expect in s for s in screens)
    return ok_end and ok_expect, len(screens)


def panics(text):
    return [l for l in text.splitlines() if "panick" in l or "panic" in l.lower() or "fatal" in l.lower()][:5]


def summarize_proc(sampler, tag, t0, t1):
    pts = sampler.window(tag, t0, t1)
    if not pts:
        return {}
    return {
        "threads_max": max((s["threads"] or 0) for _, s in pts),
        "fds_max": max((s["fds"] or 0) for _, s in pts),
        "fp_sampled_max_mb": round(max(s["fp_mb"] for _, s in pts), 1),
    }


def rate(pts, key):
    if len(pts) < 2:
        return None
    (ta, a), (tb, b) = pts[0], pts[-1]
    return round((b[key] - a[key]) / (tb - ta), 2) if tb > ta else None


def finish(instances, sampler, timeout, hold_s):
    """Wait for every instance's marker, take its numbers 3 s later, then its
    exit."""
    deadline = time.time() + timeout
    pending = list(instances)
    while pending and time.time() < deadline and not sampler.over:
        for i in list(pending):
            if i.t_marker is None and i.done():
                i.t_marker = time.time()
            if i.t_marker and time.time() - i.t_marker >= 3 and i.final is None:
                s = proc_sample(i.pid)
                fp = footprint_tool([i.pid]).get(i.pid, {})
                i.final = {"sample": s, "footprint": fp, "t": time.time()}
            if i.final is not None or i.p.poll() is not None:
                pending.remove(i)
        time.sleep(0.25)
    hung = [i.tag for i in pending]
    exit_deadline = time.time() + hold_s + 30
    for i in instances:
        try:
            i.p.wait(max(0.1, exit_deadline - time.time()))
        except subprocess.TimeoutExpired:
            i.exit_hung = True
            i.kill()
        sampler.untrack(i.tag)
    return hung


def instance_result(i, sampler, expect, tags=None, bytes_out=None):
    tm = i.times()
    wall = tm and round(tm[1] - tm[0], 2)
    fin = i.final or {}
    s = fin.get("sample") or {}
    fp = fin.get("footprint") or {}
    ok_dump, ndumps = end_check(i, expect, tags)
    err = i.stderr()
    pans = panics(err)
    code = i.p.returncode
    survived = code == 0 and tm is not None and ok_dump and not pans and not getattr(i, "exit_hung", False)
    r = {
        "tag": i.tag, "pid": i.pid, "survived": survived, "exit": code, "exit_hung": getattr(i, "exit_hung", False),
        "reached_end": tm is not None, "dump_ok": ok_dump, "dumps": ndumps, "panics": pans,
        "wall_s": wall, "mb_per_s": round(bytes_out / MB / wall, 1) if bytes_out and wall else None,
        "cpu_s": s.get("cpu_s") and round(s["cpu_s"], 2),
        "final_mb": fp.get("phys_footprint") and round(fp["phys_footprint"], 1),
        "peak_mb": fp.get("phys_footprint_peak") and round(fp["phys_footprint_peak"], 1),
        "final_sampled_mb": s.get("fp_mb") and round(s["fp_mb"], 1),
        "threads_end": s.get("threads"), "fds_end": s.get("fds"),
    }
    if tm:
        r.update(summarize_proc(sampler, i.tag, tm[0], tm[1] + 3))
    return r


def script_for(tag, d, cmd, hold):
    return (
        f"S=$({NOW}); {cmd}; E=$({NOW}); printf '\\r\\n\\033[0mEND-{tag}\\r\\n'; printf '\\033]7770;dump\\007'; "
        f"sleep 1; echo $S $E > {d}/marker; sleep {hold}\n"
    )


def tab_script(run_dir, n, cmd, hold):
    """For n tabs of one window: the first to start opens the others; each
    claims a slot, floods once all are open, then they dump in turn."""
    W = run_dir
    return f"""W={W}; N={n}; i=0
until mkdir $W/slot.$i 2>/dev/null; do i=$((i+1)); done
if [ $i = 0 ]; then k=1; while [ $k -lt $N ]; do printf '\\033]7770;tab\\007'; k=$((k+1)); sleep 0.05; done; fi
until [ -d $W/slot.$((N-1)) ]; do sleep 0.1; done
S=$({NOW}); {cmd}; E=$({NOW}); echo $S $E > $W/done.$i
printf '\\r\\n\\033[0mEND-tab{n}-%s\\r\\n' $i
until [ $(ls $W | grep -c '^done\\.') -ge $N ]; do sleep 0.2; done
until [ -e $W/turn.$i ] || [ $i = 0 ]; do sleep 0.1; done
printf '\\033]7770;front\\007'; sleep 0.3; printf '\\033]7770;dump\\007'; sleep 0.7; touch $W/turn.$((i+1))
until [ -e $W/turn.$N ]; do sleep 0.1; done
[ $i = 0 ] && cat $W/done.* | sort -n | awk 'NR==1{{s=$1}} {{e=$2}} END{{print s, e}}' > $W/marker
sleep {hold}
"""


def write(out, rec):
    with open(os.path.join(out, "results.jsonl"), "a") as f:
        f.write(json.dumps(rec) + "\n")
    print(json.dumps({k: v for k, v in rec.items() if k != "instances"}), flush=True)


def run_group(args, kind, name, level, make, test_dump=True):
    """Start instances from make(run_dir) -> [(tag, script)], sample them
    and record one result."""
    run_dir = os.path.join(args.out, "runs", f"{kind}-{name}-{level}")
    subprocess.run(["rm", "-rf", run_dir])
    os.makedirs(run_dir)
    before = crash_files()
    sampler = Sampler(os.path.join(run_dir, "samples.jsonl"), args.interval, args.mem_limit_gb * 1024, kill_all)
    sampler.load_start = os.getloadavg()[0]
    sampler.start()
    t0 = time.time()
    ALL.clear()
    insts = []
    for tag, script in make(run_dir):
        i = Instance(args.binary, run_dir, tag, script, test_dump)
        ALL.append(i)
        insts.append(i)
        sampler.track(tag, i.pid)
        time.sleep(args.stagger)
    return run_dir, before, sampler, t0, insts


def close_group(sampler, before, t0, insts):
    sampler.stopped.set()
    sampler.join(5)
    t1 = time.time()
    new = sorted(crash_files() - before)
    ws = sampler.ws_cpu_between(t0, t1)
    return {"crash_logs": new, "ws_cpu_s": ws and round(ws[0], 1), "ws_cpu_pct": ws and round(100 * ws[0] / ws[1], 1), "peak_total_mb": round(sampler.peak_total, 1), "elapsed_s": round(t1 - t0, 1), "mem_limit_hit": sampler.over,
            "load": [round(sampler.load_start, 1), round(os.getloadavg()[0], 1)]}


def med(xs):
    xs = [x for x in xs if x is not None]
    return round(statistics.median(xs), 2) if xs else None


def mx(xs):
    xs = [x for x in xs if x is not None]
    return round(max(xs), 2) if xs else None


def sweep(args):
    sc = scenarios(args.work)
    names = args.scenarios.split(",") if args.scenarios else ["flood-1gb", "churn-3M-lines", "fuzz-64mb", "glyphs-30k", "one-50mb-line", "sync-flood-15s", "idle-20s"]
    for level in [int(x) for x in args.levels.split(",")]:
        for name in names:
            cmd, nbytes, expect = sc[name]

            def make(run_dir):
                return [(f"i{k:02}", script_for(f"i{k:02}", os.path.join(run_dir, f"i{k:02}"), cmd, args.hold)) for k in range(level)]

            # Idle runs without the test hooks: with them on, a window
            # draws even when covered, which isn't idle as users see it.
            run_dir, before, sampler, t0, insts = run_group(args, "sweep", name, level, make, test_dump=not name.startswith("idle"))
            hung = finish(insts, sampler, args.timeout, args.hold)
            rows = [instance_result(i, sampler, expect, bytes_out=nbytes) for i in insts]
            extra = {}
            if name.startswith("idle"):
                # CPU and wake-ups while every instance sits idle: 6..18 s
                # into its sleep.
                cpus, wakes = [], []
                for i, r in zip(insts, rows):
                    tm = i.times()
                    if not tm:
                        continue
                    pts = sampler.window(i.tag, tm[0] + 6, tm[0] + 18)
                    c = rate(pts, "cpu_s")
                    cpus.append(c and c * 100)
                    wakes.append(rate(pts, "idle_w"))
                    r["idle_cpu_pct"], r["idle_wakeups_per_s"] = c and round(c * 100, 2), rate(pts, "idle_w")
                extra = {"idle_cpu_pct_med": med(cpus), "idle_cpu_pct_max": mx(cpus), "idle_wakeups_med": med(wakes), "idle_wakeups_max": mx(wakes)}
            g = close_group(sampler, before, t0, insts)
            walls = [r["wall_s"] for r in rows]
            rec = {
                "kind": "sweep", "scenario": name, "level": level, "survived": sum(r["survived"] for r in rows), "hung": hung,
                "wall_med_s": med(walls), "wall_max_s": mx(walls),
                "mb_per_s_med": med([r["mb_per_s"] for r in rows]),
                "aggregate_mb_per_s": round(level * nbytes / MB / mx(walls), 1) if nbytes and mx(walls) else None,
                "cpu_s_sum": round(sum(r["cpu_s"] or 0 for r in rows), 1),
                "peak_mb_med": med([r["peak_mb"] for r in rows]), "peak_mb_max": mx([r["peak_mb"] for r in rows]),
                "final_mb_med": med([r["final_mb"] for r in rows]),
                "threads_max": mx([r.get("threads_max") for r in rows]), "fds_max": mx([r.get("fds_max") for r in rows]),
                **extra, **g, "instances": rows,
            }
            write(args.out, rec)
            time.sleep(args.gap)


def tabs(args):
    sc = scenarios(args.work)
    cmd, nbytes, expect = sc[args.scenario]
    for level in [int(x) for x in args.levels.split(",")]:
        def make(run_dir):
            return [("w", tab_script(run_dir, level, cmd, args.hold))]

        run_dir, before, sampler, t0, insts = run_group(args, "tabs", args.scenario, level, make)
        inst = insts[0]
        inst.marker = os.path.join(run_dir, "marker")
        hung = finish(insts, sampler, args.timeout, args.hold)
        r = instance_result(inst, sampler, expect, tags=[f"tab{level}-{k}" for k in range(level)], bytes_out=nbytes * level if nbytes else None)
        per_tab = []
        for f in sorted(glob.glob(os.path.join(run_dir, "done.*"))):
            s, e = map(float, open(f).read().split())
            per_tab.append(round(e - s, 2))
        g = close_group(sampler, before, t0, insts)
        rec = {
            "kind": "tabs", "scenario": args.scenario, "level": level, "survived": int(r["survived"]), "hung": hung,
            "wall_s": r["wall_s"], "tab_wall_med_s": med(per_tab), "tab_wall_min_s": min(per_tab) if per_tab else None, "tab_wall_max_s": mx(per_tab),
            "aggregate_mb_per_s": r["mb_per_s"], "cpu_s": r["cpu_s"], "peak_mb": r["peak_mb"], "final_mb": r["final_mb"],
            "threads_max": r.get("threads_max"), "fds_max": r.get("fds_max"), **g, "instances": [r],
        }
        write(args.out, rec)
        time.sleep(args.gap)


# --- soak --------------------------------------------------------------------


def mixed_loop(d, work, deadline, delay, phases_file):
    """Mixed heavy load until `deadline`, logging each phase's start and
    dumping after it, so a hang shows as dumps that stop coming."""
    big, fuzz, glyphs = f"{work}/big.txt", f"{work}/fuzz.bin", f"{work}/glyphs.txt"
    return f"""D={int(deadline)}; it=0
ph() {{ echo "$({NOW}) $1 $it" >> {phases_file}; }}
dump() {{ printf '\\033]7770;dump\\007'; }}
sleep {delay}
while [ $(date +%s) -lt $D ]; do
  ph flood; cat {big}; dump
  ph idle; sleep 20; dump
  ph churn; seq 1 1000000; dump
  ph sync; python3 {HERE}/gen.py sync 10; dump
  ph fuzz; cat {fuzz}; dump
  ph glyphs; cat {glyphs}; dump
  ph resize; printf '\\033]7770;resize 140 45\\007'; sleep 1; printf '\\033]7770;font +1\\007'; seq 1 200000; printf '\\033]7770;font -1\\007'; printf '\\033]7770;resize 100 30\\007'; sleep 1; dump
  ph line; head -c 20000000 /dev/zero | tr '\\0' x; echo; dump
  ph altscreen; j=0; while [ $j -lt 300 ]; do printf '\\033[?1049h\\033[2J\\033[H%s alt\\033[?1049l' $j; j=$((j+1)); done; dump
  ph idle2; sleep 20; dump
  it=$((it+1))
done
ph end; printf '\\r\\n\\033[0mEND-{os.path.basename(d)}\\r\\n'; dump; sleep 1
echo 0 0 > {d}/marker; sleep 5
"""


def idle_loop(d, deadline, blink, phases_file):
    cursor = "printf '\\033[1 q'; " if blink else ""
    return f"""{cursor}D={int(deadline)}; it=0
while [ $(date +%s) -lt $D ]; do echo "$({NOW}) idle $it" >> {phases_file}; sleep 30; printf '\\033]7770;dump\\007'; it=$((it+1)); done
printf '\\r\\nEND-{os.path.basename(d)}\\r\\n'; printf '\\033]7770;dump\\007'; sleep 1; echo 0 0 > {d}/marker; sleep 5
"""


def soak_tab_script(run_dir, d, work, deadline, n):
    """n tabs in one window: even slots run the mixed loop, odd ones sit idle
    and bring themselves to the front every 45 s (tab switching)."""
    W = os.path.join(run_dir, "tabslots")
    os.makedirs(W, exist_ok=True)
    mixed = mixed_loop(d, work, deadline, 0, f"{d}/phases.$i.log").replace(f"echo 0 0 > {d}/marker; sleep 5", "")
    return f"""W={W}; N={n}; i=0
until mkdir $W/slot.$i 2>/dev/null; do i=$((i+1)); done
if [ $i = 0 ]; then k=1; while [ $k -lt $N ]; do printf '\\033]7770;tab\\007'; k=$((k+1)); sleep 0.2; done; fi
if [ $((i % 2)) = 0 ]; then
{mixed}
else
  D={int(deadline)}; while [ $(date +%s) -lt $D ]; do sleep 45; printf '\\033]7770;front\\007'; done
fi
touch $W/done.$i
until [ $(ls $W | grep -c '^done\\.') -ge $N ]; do sleep 0.5; done
[ $i = 0 ] && echo 0 0 > {d}/marker
sleep 5
"""


def soak(args):
    deadline = time.time() + args.minutes * 60
    tags = []

    def make(run_dir):
        out = []
        for k in range(args.mixed):
            tag = f"mixed{k}"
            d = os.path.join(run_dir, tag)
            out.append((tag, mixed_loop(d, args.work, deadline, k * 17, f"{d}/phases.log")))
        for k in range(args.tab_instances):
            tag = f"tabs{k}"
            d = os.path.join(run_dir, tag)
            out.append((tag, soak_tab_script(os.path.join(run_dir, tag), d, args.work, deadline, args.tabs)))
        for tag, blink in (("idle", False), ("blink", True)):
            d = os.path.join(run_dir, tag)
            out.append((tag, idle_loop(d, deadline, blink, f"{d}/phases.log")))
        tags.extend(t for t, _ in out)
        return out

    run_dir, before, sampler, t0, insts = run_group(args, "soak", f"{args.minutes:g}min", args.mixed, make)
    hung = finish(insts, sampler, args.minutes * 60 + 600, 5)
    g = close_group(sampler, before, t0, insts)
    rows = []
    for i in insts:
        r = instance_result(i, sampler, None)
        r.update(soak_analysis(i, sampler))
        rows.append(r)
    rec = {"kind": "soak", "minutes": args.minutes, "instances_n": len(insts), "survived": sum(r["survived"] for r in rows), "hung": hung, **g, "instances": rows}
    write(args.out, rec)


def linfit(xs, ys):
    if len(xs) < 2:
        return None
    mxv, myv = statistics.mean(xs), statistics.mean(ys)
    den = sum((x - mxv) ** 2 for x in xs)
    return sum((x - mxv) * (y - myv) for x, y in zip(xs, ys)) / den if den else None


def soak_analysis(inst, sampler):
    """Footprint, threads and fds at the end of each idle phase (one point
    per loop), their growth, and the longest gap between dumps."""
    phases = []
    for f in glob.glob(os.path.join(inst.dir, "phases*.log")):
        for line in open(f):
            p = line.split()
            if len(p) >= 3:
                phases.append((float(p[0]), p[1], int(p[2])))
    phases.sort()
    # A process that is going away reads as zeros.
    series = [(t, s) for t, s in sampler.series.get(inst.tag, []) if s["fp_mb"] > 0 and s["threads"]]
    points = []
    for t, ph, it in phases:
        if ph not in ("idle", "idle2"):
            continue
        # The last sample before the idle phase's 20 s ends.
        pts = [(ts, s) for ts, s in series if t + 12 <= ts <= t + 19.5]
        if pts:
            ts, s = pts[-1]
            points.append((ts, s["fp_mb"], s["threads"], s["fds"], ph))
    res = {"loops": max([it for _, _, it in phases], default=-1) + 1}
    if points:
        hrs = [(p[0] - points[0][0]) / 3600 for p in points]
        slope = linfit(hrs, [p[1] for p in points])
        # After the first loop: the first flood fills the scrollback.
        later = points[2:] if len(points) > 3 else points
        slope_later = linfit([(p[0] - later[0][0]) / 3600 for p in later], [p[1] for p in later])
        res.update(
            idle_fp_mb=[round(p[1], 1) for p in points],
            idle_fp_first_mb=round(points[0][1], 1), idle_fp_last_mb=round(points[-1][1], 1),
            idle_fp_slope_mb_per_h=slope and round(slope, 1), idle_fp_slope_after_warmup_mb_per_h=slope_later and round(slope_later, 1),
            threads_idle=[p[2] for p in points], fds_idle=[p[3] for p in points],
        )
    if series:
        res["fp_min_mb"] = round(min(s["fp_mb"] for _, s in series), 1)
        res["fp_max_mb"] = round(max(s["fp_mb"] for _, s in series), 1)
        res["threads_minmax"] = [min(s["threads"] or 0 for _, s in series), max(s["threads"] or 0 for _, s in series)]
        res["fds_minmax"] = [min(s["fds"] or 0 for _, s in series), max(s["fds"] or 0 for _, s in series)]
        dur = series[-1][0] - series[0][0]
        if dur > 0:
            res["avg_cpu_pct"] = round(100 * (series[-1][1]["cpu_s"] - series[0][1]["cpu_s"]) / dur, 1)
            res["wakeups_per_s"] = round((series[-1][1]["idle_w"] - series[0][1]["idle_w"]) / dur, 1)
            res["intr_wakeups_per_s"] = round((series[-1][1]["intr_w"] - series[0][1]["intr_w"]) / dur, 1)
    # Dumps are the app's heartbeat: each needs the main thread and a frame.
    try:
        mt = os.path.getmtime(inst.dump)
    except OSError:
        mt = None
    res["dump_count"] = len(inst.dumps())
    res["dump_file_mtime"] = mt
    return res


# --- report ------------------------------------------------------------------


def f(v, unit=""):
    return "–" if v is None else f"{v}{unit}"


def report(args):
    recs = [json.loads(l) for l in open(os.path.join(args.out, "results.jsonl")) if l.strip()]
    sweeps = [r for r in recs if r["kind"] == "sweep"]
    out = []
    if sweeps:
        out += [
            "| scenario | instances | survived | wall median / max | MiB/s per instance (median) | MiB/s all | CPU s (sum) | WindowServer CPU | peak MB median / max | after MB median | peak total MB | threads / fds max |",
            "|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|",
        ]
        for r in sorted(sweeps, key=lambda r: (r["scenario"], r["level"])):
            idle = ""
            if r.get("idle_cpu_pct_med") is not None:
                idle = f" (idle CPU {r['idle_cpu_pct_med']}% med / {r['idle_cpu_pct_max']}% max, wake-ups/s {r['idle_wakeups_med']} / {r['idle_wakeups_max']})"
            out.append(
                f"| {r['scenario']}{idle} | {r['level']} | {r['survived']}/{r['level']} | {f(r['wall_med_s'], ' s')} / {f(r['wall_max_s'], ' s')} "
                f"| {f(r['mb_per_s_med'])} | {f(r['aggregate_mb_per_s'])} | {r['cpu_s_sum']} | {f(r['ws_cpu_pct'], '%')} "
                f"| {f(r['peak_mb_med'])} / {f(r['peak_mb_max'])} | {f(r['final_mb_med'])} | {r['peak_total_mb']} | {f(r['threads_max'])} / {f(r['fds_max'])} |"
            )
        out.append("")
    tabsr = [r for r in recs if r["kind"] == "tabs"]
    if tabsr:
        out += [
            "| tabs flooding | survived | wall (all tabs) | per-tab wall min / median / max | MiB/s all | CPU s | WindowServer CPU | peak MB | after MB | threads / fds max |",
            "|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|",
        ]
        for r in sorted(tabsr, key=lambda r: r["level"]):
            out.append(
                f"| {r['level']} × {r['scenario']} | {'yes' if r['survived'] else '**no**'} | {f(r['wall_s'], ' s')} | {f(r['tab_wall_min_s'])} / {f(r['tab_wall_med_s'])} / {f(r['tab_wall_max_s'])} s "
                f"| {f(r['aggregate_mb_per_s'])} | {f(r['cpu_s'])} | {f(r['ws_cpu_pct'], '%')} | {f(r['peak_mb'])} | {f(r['final_mb'])} | {f(r['threads_max'])} / {f(r['fds_max'])} |"
            )
        out.append("")
    for r in [r for r in recs if r["kind"] == "soak"]:
        out += [
            f"Soak: {r['minutes']} min, {r['instances_n']} instances, {r['survived']} survived, WindowServer CPU {f(r['ws_cpu_pct'], '%')}, peak total {r['peak_total_mb']} MB.",
            "",
            "| instance | survived | loops | avg CPU | idle / interrupt wake-ups/s | footprint min / max | idle footprint first → last | growth MB/h (all / after 2 idles) | threads | fds | dumps |",
            "|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|",
        ]
        for i in r["instances"]:
            out.append(
                f"| {i['tag']} | {'yes' if i['survived'] else '**no**'} | {i.get('loops')} | {f(i.get('avg_cpu_pct'), '%')} | {f(i.get('wakeups_per_s'))} / {f(i.get('intr_wakeups_per_s'))} "
                f"| {f(i.get('fp_min_mb'))} / {f(i.get('fp_max_mb'))} MB | {f(i.get('idle_fp_first_mb'))} → {f(i.get('idle_fp_last_mb'))} MB "
                f"| {f(i.get('idle_fp_slope_mb_per_h'))} / {f(i.get('idle_fp_slope_after_warmup_mb_per_h'))} | {i.get('threads_minmax')} | {i.get('fds_minmax')} | {i.get('dump_count')} |"
            )
        out.append("")
    bad = [(r.get("scenario", r["kind"]), r.get("level"), i) for r in recs for i in r["instances"] if not i["survived"]]
    if bad:
        out.append("Did not survive:")
        for s, lv, i in bad:
            out.append(f"- {s} ×{lv} {i['tag']}: exit {i['exit']}, reached end {i['reached_end']}, dump ok {i['dump_ok']}, exit hung {i['exit_hung']}, panics {i['panics']}")
    crashes = sorted({c for r in recs for c in r.get("crash_logs", [])})
    if crashes:
        out.append("New crash logs: " + ", ".join(crashes))
    print("\n".join(out))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("sweep", "tabs", "soak"):
        p = sub.add_parser(name)
        p.add_argument("binary")
        p.add_argument("work")
        p.add_argument("out")
        p.add_argument("--interval", type=float, default=2.0)
        p.add_argument("--mem-limit-gb", type=float, default=11.5)
        p.add_argument("--timeout", type=float, default=1200)
        p.add_argument("--hold", type=float, default=8)
        p.add_argument("--stagger", type=float, default=0.05)
        p.add_argument("--gap", type=float, default=3)
    sub.choices["sweep"].add_argument("--levels", default="1,4,8,16,32")
    sub.choices["sweep"].add_argument("--scenarios", default="")
    sub.choices["tabs"].add_argument("--levels", default="1,8,32")
    sub.choices["tabs"].add_argument("--scenario", default="flood-1gb")
    sub.choices["soak"].add_argument("--minutes", type=float, default=25)
    sub.choices["soak"].add_argument("--mixed", type=int, default=5)
    sub.choices["soak"].add_argument("--tab-instances", type=int, default=1)
    sub.choices["soak"].add_argument("--tabs", type=int, default=4)
    p = sub.add_parser("report")
    p.add_argument("out")
    args = ap.parse_args()
    if args.cmd == "report":
        return report(args)
    args.binary, args.work, args.out = map(os.path.abspath, (args.binary, args.work, args.out))
    os.makedirs(args.out, exist_ok=True)
    install_signal_handlers()
    try:
        {"sweep": sweep, "tabs": tabs, "soak": soak}[args.cmd](args)
    finally:
        kill_all()


if __name__ == "__main__":
    main()
