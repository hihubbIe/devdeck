#!/usr/bin/env python3
"""Measures devdeck's resource use (R11.4). Prints one line per metric.

Usage: scripts/measure.py <scenario> [options]

Scenarios
  idle      start, 20 s streaming a 50k-line log, then 10 s idle (Wayland or X11)
  kube      like idle, with the read-only cluster feed on (needs DEVDECK_KUBECONFIG,
            DEVDECK_KUBE_CONTEXT, DEVDECK_KUBE_NAMESPACE in the environment)
  latency   X11 only: sends real key presses with xdotool and reads GPUI's
            input-to-frame histogram (needs the `profile` build)
  terminal  a run's terminal under load: the demo's three servers write 2 000 lines/s
            each, one shown in a run view; CPU, and memory against the same start at
            the normal rate (the difference is three full terminals of history)
  leak      runs DEVDECK_STRESS cycles (default 1000) and prints memory every 250; the
            verdict compares 500-1000 (keep going only if that is flat)
  heapdiff  heaptrack on a 10- and a 30-cycle run (teardown skipped), prints what grows
            per cycle by call site, deletes the recordings; needs a build with symbols:
            CARGO_PROFILE_RELEASE_DEBUG=true CARGO_PROFILE_RELEASE_STRIP=false \
              cargo build --release --target-dir target/profiling

Options
  --x11            force X11 (unset WAYLAND_DISPLAY)
  --bin PATH       binary (default target/release/devdeck)
  --cycles N       leak cycles (default 1000)
"""

import argparse
import os
import shutil
import tempfile
import re
import subprocess
import sys
import time

CLK = os.sysconf("SC_CLK_TCK")


def cpu_ticks(pid):
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])  # utime + stime


def mem(pid):
    out = {}
    with open(f"/proc/{pid}/smaps_rollup") as f:
        for line in f:
            k, _, rest = line.partition(":")
            if k in ("Rss", "Pss", "Private_Clean", "Private_Dirty"):
                out[k] = int(rest.split()[0])
    return out["Rss"], out["Pss"], out["Private_Clean"] + out["Private_Dirty"]


def uses_vulkan(pid):
    with open(f"/proc/{pid}/maps") as f:
        return any("libvulkan" in l for l in f)


def isolated_env(env):
    """Fresh state and config folders: never the user's saved layouts or settings, and
    the same starting point (default layout, 100 % scale) every run."""
    for var in ("DEVDECK_STATE_DIR", "DEVDECK_CONFIG_DIR"):
        d = tempfile.mkdtemp(prefix="devdeck-measure-")
        TEMP_DIRS.append(d)
        env[var] = d
    return env


TEMP_DIRS = []


def start(args, env_extra=None):
    env = isolated_env(dict(os.environ))
    env.pop("ZED_HEADLESS", None)
    if args.x11:
        env.pop("WAYLAND_DISPLAY", None)
    env["DEVDECK_STATS"] = "10"  # rare: reading smaps_rollup itself costs CPU
    env.update(env_extra or {})
    log = open(args.log, "w")
    # The scenarios measure the demo (its fake servers and views).
    p = subprocess.Popen([args.bin, "--demo"], env=env, stdout=log, stderr=subprocess.STDOUT)
    return p


def cpu_between(pid, seconds):
    t0, w0 = cpu_ticks(pid), time.monotonic()
    time.sleep(seconds)
    t1, w1 = cpu_ticks(pid), time.monotonic()
    return 100.0 * (t1 - t0) / CLK / (w1 - w0)


def renders_in_log(log, since_line):
    with open(log) as f:
        lines = f.readlines()[since_line:]
    return sum(int(m.group(1)) for l in lines if (m := re.search(r"renders_delta=(\d+)", l)))


def log_len(log):
    with open(log) as f:
        return len(f.readlines())


def report(name, value):
    print(f"{name}: {value}")


def scenario_idle(args, env_extra=None, label="idle"):
    p = start(args, env_extra)
    try:
        time.sleep(5)
        report(f"{label}.window_rendered_by_vulkan", uses_vulkan(p.pid))
        rss, pss, uss = mem(p.pid)
        report(f"{label}.start_kb", f"rss={rss} pss={pss} uss={uss}")
        report(f"{label}.cpu_streaming_pct", f"{cpu_between(p.pid, 15):.2f}")
        time.sleep(12)  # the stream ends at 25 s
        rss, pss, uss = mem(p.pid)
        report(f"{label}.after_50k_lines_kb", f"rss={rss} pss={pss} uss={uss}")
        mark = log_len(args.log)
        report(f"{label}.cpu_idle_pct", f"{cpu_between(p.pid, args.idle):.3f}")
        report(f"{label}.renders_while_idle", renders_in_log(args.log, mark))
    finally:
        p.terminate()
        p.wait(10)


def scenario_terminal(args):
    view = {"DEVDECK_OPEN": "run/entityservice/go:run"}
    base = start(args, view)
    try:
        time.sleep(8)
        _, _, uss0 = mem(base.pid)
        report("terminal.normal_rate_uss_kb", uss0)
    finally:
        base.terminate()
        base.wait(10)
    p = start(args, {**view, "DEVDECK_FAKE_RATE": "2000"})
    try:
        time.sleep(8)
        report("terminal.cpu_at_2000_lines_s_x3_pct", f"{cpu_between(p.pid, 15):.2f}")
        rss, pss, uss = mem(p.pid)
        report("terminal.full_history_kb", f"rss={rss} pss={pss} uss={uss}")
        report("terminal.history_cost_uss_kb", uss - uss0)
    finally:
        p.terminate()
        p.wait(10)


def scenario_latency(args):
    if not args.x11:
        sys.exit("latency needs --x11 (xdotool can only type into X11 windows)")
    env = {"DEVDECK_LATENCY": "1"}
    p = start(args, env)
    try:
        time.sleep(4)
        wid = None
        for _ in range(20):
            r = subprocess.run(["xdotool", "search", "--pid", str(p.pid)], capture_output=True, text=True)
            ids = r.stdout.split()
            if ids:
                wid = ids[-1]
                break
            time.sleep(0.5)
        if not wid:
            sys.exit("no X11 window found for devdeck")
        subprocess.run(["xdotool", "windowactivate", "--sync", wid])
        time.sleep(30)  # let the log stream finish so frames are input-driven
        for _ in range(args.keys):
            subprocess.run(["xdotool", "key", "--window", wid, "ctrl+Tab"])
            time.sleep(0.12)
        time.sleep(6)
        with open(args.log) as f:
            lines = [l.strip() for l in f if l.startswith("devdeck-latency")]
        report("latency.last", lines[-1] if lines else "no samples")
    finally:
        p.terminate()
        p.wait(10)


def scenario_leak(args):
    env = {"DEVDECK_STRESS": str(args.cycles)}
    env_all = isolated_env(dict(os.environ))
    env_all.update(env)
    if args.x11:
        env_all.pop("WAYLAND_DISPLAY", None)
    with open(args.log, "w") as log:
        subprocess.run([args.bin], env=env_all, stdout=log, stderr=subprocess.STDOUT, timeout=args.cycles * 1.0 + 120)
    with open(args.log) as f:
        rows = [l.strip() for l in f if l.startswith("devdeck-stress")]
    for r in rows:
        print(r)
    pts = []
    for r in rows:
        m = re.search(r"cycle=(\d+) rss_kb=(\d+) pss_kb=(\d+) uss_kb=(\d+)", r)
        if m:
            pts.append(tuple(int(x) for x in m.groups()))
    warm = [p for p in pts if p[0] >= 500]
    if len(warm) >= 2:
        (c0, _, _, u0), (c1, _, _, u1) = warm[0], warm[-1]
        report("leak.uss_bytes_per_cycle_after_warmup", f"{(u1 - u0) * 1024 / (c1 - c0):.1f}")
    if any("step failed" in l for l in open(args.log)):
        report("leak.failed_steps", sum(1 for l in open(args.log) if "step failed" in l))


def scenario_heapdiff(args):
    """What grows per cycle, by call site: heaptrack on two run lengths past warm-up,
    teardown skipped so everything alive counts, diffed."""
    if not shutil.which("heaptrack"):
        sys.exit("heaptrack is not installed")
    out_dir = tempfile.mkdtemp(prefix="devdeck-heapdiff-")
    TEMP_DIRS.append(out_dir)
    files = {}
    for n in (args.short, args.long):
        env = isolated_env(dict(os.environ))
        env.update({"DEVDECK_STRESS": str(n), "DEVDECK_STRESS_EXIT": "hard", "DEVDECK_STRESS_REPORT": str(n)})
        if args.x11:
            env.pop("WAYLAND_DISPLAY", None)
        base = os.path.join(out_dir, f"heap{n}")
        with open(args.log, "w") as log:
            subprocess.run(["heaptrack", "-o", base, args.bin], env=env, stdout=log, stderr=subprocess.STDOUT, timeout=600)
        files[n] = base + ".zst"
    diff = subprocess.run(
        ["heaptrack_print", "-f", files[args.long], "-d", files[args.short], "-p", "0", "-a", "0", "-T", "0", "-l", "1", "-n", "40", "-s", "1"],
        capture_output=True, text=True,
    ).stdout.splitlines()
    cycles = args.long - args.short
    groups = []
    for i, line in enumerate(diff):
        m = re.match(r"^(-?[\d.]+)([KMG]?)B? leaked over (\d+) calls from", line)
        if not m:
            continue
        kb = float(m.group(1)) * {"": 1 / 1024, "K": 1, "M": 1024, "G": 1024 * 1024}[m.group(2)]
        # Symbols are v0-mangled (heaptrack can't demangle them): name the place by the
        # first source lines of our crates in the backtrace.
        here = os.path.abspath("crates") + "/"
        places = [f.strip().removeprefix("at ").replace(here, "") for f in diff[i + 1 : i + 80] if f.strip().startswith("at " + here)]
        groups.append((kb, places[:3]))
    total = sum(kb for kb, _ in groups if kb > 0)
    report("heapdiff.cycles", f"{args.short} -> {args.long}")
    report("heapdiff.growth_bytes_per_cycle", f"{total * 1024 / cycles:.0f}")
    for kb, frames in sorted(groups, key=lambda g: -g[0])[:8]:
        if kb <= 0:
            break
        where = " < ".join(frames) if frames else "(gpui or a library, no devdeck frame)"
        print(f"  {kb * 1024 / cycles:8.0f} B/cycle  {where}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scenario", choices=["idle", "kube", "latency", "leak", "heapdiff", "terminal"])
    ap.add_argument("--x11", action="store_true")
    ap.add_argument("--bin", default="target/release/devdeck")
    ap.add_argument("--cycles", type=int, default=1000)
    ap.add_argument("--short", type=int, default=10)
    ap.add_argument("--long", type=int, default=30)
    ap.add_argument("--keys", type=int, default=60)
    ap.add_argument("--idle", type=float, default=20.0)
    ap.add_argument("--log", default="target/measure.log")
    args = ap.parse_args()
    report("session", "x11" if args.x11 else os.environ.get("XDG_SESSION_TYPE", "?"))
    if args.scenario == "idle":
        scenario_idle(args)
    elif args.scenario == "kube":
        scenario_idle(args, label="kube")
    elif args.scenario == "latency":
        scenario_latency(args)
    elif args.scenario == "terminal":
        scenario_terminal(args)
    elif args.scenario == "heapdiff":
        if args.bin == "target/release/devdeck":
            args.bin = "target/profiling/release/devdeck"
        scenario_heapdiff(args)
    else:
        scenario_leak(args)
    for d in TEMP_DIRS:
        shutil.rmtree(d, ignore_errors=True)


if __name__ == "__main__":
    main()
