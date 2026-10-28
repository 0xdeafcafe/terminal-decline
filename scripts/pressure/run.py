#!/usr/bin/env python3
"""Pressure tests: runs decline with heavy or hostile workloads and records
time, CPU, memory (physical footprint) and whether it survived.

usage: run.py BINARY WORKDIR [SCENARIO...]
"""
import json, os, re, subprocess, sys, time

BIN, WORK = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
HERE = os.path.dirname(os.path.abspath(__file__))
NOW = "perl -MTime::HiRes=time -e 'printf \"%.3f\", time'"

def scenarios():
    big = f"{WORK}/big.txt"
    return {
        "flood-1gb": f"for i in 1 2 3 4 5 6 7 8 9 10; do cat {big}; done",
        "churn-3M-lines": "seq 1 3000000",
        "fuzz-64mb": f"cat {WORK}/fuzz.bin",
        "glyphs-30k": f"cat {WORK}/glyphs.txt",
        "one-50mb-line": "head -c 50000000 /dev/zero | tr '\\0' 'x'",
        "sync-flood-15s": f"python3 {HERE}/gen.py sync 15",
        "tui-htop-20s": "htop -d 5 & P=$!; sleep 20; kill $P; wait $P 2>/dev/null",
        "blink-idle-20s": "printf '\\033[1 q'; sleep 20",
        "plain-idle-20s": "sleep 20",
    }

def footprint(pid):
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
    parts = [float(p) for p in out.replace("-", ":").split(":")]
    total = 0.0
    for p in parts:
        total = total * 60 + p
    return total

def idle_wakeups(pid, seconds=10):
    out = subprocess.run(["top", "-l", "2", "-s", str(seconds), "-pid", str(pid), "-stats", "pid,cpu,idlew"], capture_output=True, text=True).stdout
    rows = [l.split() for l in out.splitlines() if l.strip().startswith(str(pid))]
    if not rows:
        return None, None
    cpu, w = rows[-1][1], rows[-1][2].rstrip("+-")
    return float(cpu), int(w) / seconds

def run(name, cmd):
    marker = f"{WORK}/done-{name}"
    if os.path.exists(marker):
        os.remove(marker)
    script = f"S=$({NOW}); {cmd}; E=$({NOW}); echo $S $E > {marker}; sleep 4"
    env = dict(os.environ, DECLINE_DEBUG="1")
    p = subprocess.Popen([BIN, "-e", "sh", "-c", script], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    t0 = time.time()
    peak, samples, window = 0.0, [], None
    idle = (None, None)
    while not os.path.exists(marker):
        if p.poll() is not None:
            break
        if time.time() - t0 > 600:
            break
        f = footprint(p.pid)
        if f:
            peak = max(peak, f)
            samples.append(round(f, 1))
        if "idle" in name and idle == (None, None) and time.time() - t0 > 4:
            idle = idle_wakeups(p.pid)
        time.sleep(1)
    alive = p.poll() is None
    wall = None
    if os.path.exists(marker):
        s, e = map(float, open(marker).read().split())
        wall = e - s
    cpu = cpu_seconds(p.pid) if alive else None
    time.sleep(1)
    final = footprint(p.pid) if alive else None
    shot = None
    if alive:
        stderr_line = ""
        # The window number was printed at start.
        try:
            p.stderr and os.set_blocking(p.stderr.fileno(), False)
            stderr_line = p.stderr.read() or ""
        except Exception:
            pass
        m = re.search(r"window (\d+)", stderr_line)
        if m:
            shot = f"{WORK}/shot-{name}.png"
            subprocess.run(["screencapture", "-x", "-o", f"-l{m.group(1)}", shot])
    try:
        p.wait(timeout=15)
    except subprocess.TimeoutExpired:
        p.kill()
    code = p.returncode
    result = {
        "scenario": name, "survived": alive and code == 0, "exit": code, "wall_s": wall and round(wall, 2),
        "cpu_s": cpu and round(cpu, 2), "peak_mb": round(peak, 1), "final_mb": final and round(final, 1),
        "idle_cpu_pct": idle[0], "idle_wakeups_per_s": idle[1], "screenshot": shot, "footprint_series": samples[-30:],
    }
    print(json.dumps(result), flush=True)
    return result

if __name__ == "__main__":
    all_s = scenarios()
    names = sys.argv[3:] or list(all_s)
    for n in names:
        run(n, all_s[n])
