#!/usr/bin/env python3
"""Measures devdeck's resource use (R11.4). Prints one line per metric.

Usage: scripts/measure.py <scenario> [options]

Scenarios
  idle      start, 20 s streaming a 50k-line log, then 10 s idle (Wayland or X11)
  kube      like idle, with the read-only cluster feed on (needs DEVDECK_KUBECONFIG,
            DEVDECK_KUBE_CONTEXT, DEVDECK_KUBE_NAMESPACE in the environment)
  latency   X11 only: sends real key presses with xdotool and reads GPUI's
            input-to-frame histogram (needs the `profile` build)
  leak      runs DEVDECK_STRESS cycles and prints memory every 500 cycles

Options
  --x11            force X11 (unset WAYLAND_DISPLAY)
  --bin PATH       binary (default target/release/devdeck)
  --cycles N       leak cycles (default 5000)
"""

import argparse
import os
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


def start(args, env_extra=None):
    env = dict(os.environ)
    env.pop("ZED_HEADLESS", None)
    if args.x11:
        env.pop("WAYLAND_DISPLAY", None)
    env["DEVDECK_STATS"] = "10"  # rare: reading smaps_rollup itself costs CPU
    env.update(env_extra or {})
    log = open(args.log, "w")
    p = subprocess.Popen([args.bin], env=env, stdout=log, stderr=subprocess.STDOUT)
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
    env_all = dict(os.environ)
    env_all.update(env)
    if args.x11:
        env_all.pop("WAYLAND_DISPLAY", None)
    with open(args.log, "w") as log:
        subprocess.run([args.bin], env=env_all, stdout=log, stderr=subprocess.STDOUT, timeout=args.cycles * 0.5 + 120)
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scenario", choices=["idle", "kube", "latency", "leak"])
    ap.add_argument("--x11", action="store_true")
    ap.add_argument("--bin", default="target/release/devdeck")
    ap.add_argument("--cycles", type=int, default=5000)
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
    else:
        scenario_leak(args)


if __name__ == "__main__":
    main()
