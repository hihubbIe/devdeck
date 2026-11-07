#!/usr/bin/env python3
"""Measures devdeck's resource use (R11.4). Prints one line per metric.

Usage: scripts/measure.py <scenario> [options]

Scenarios
  idle      start, 20 s streaming a 50k-line log, then 10 s idle (Wayland or X11)
  latency   X11 only: sends real key presses with xdotool and reads GPUI's
            input-to-frame histogram (needs the `profile` build)
  terminal  a run's terminal under load: the demo's three servers write 2 000 lines/s
            each, one shown in a run view; CPU, and memory against the same start at
            the normal rate (the difference is three full terminals of history)
  real      the terminal scenario with a real process instead of the demo: a configured
            workspace whose project's `make run` (python) writes 2 000 coloured lines/s,
            started at launch and shown in a run view; one run, not three
  leftovers R8.2 with a real run whose recipe starts a child that leaves the process
            group (`setsid`) and an ordinary one: what SIGTERM and kill -9 to devdeck
            leave running, and what the next devdeck finds in its group log
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
import json
import os
import signal
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


def start(args, env_extra=None, fill=None, env=None):
    """Starts devdeck: the demo (its fake servers and views), unless `fill` writes a real
    workspace into the fresh config and state folders, or `env` (from an earlier real
    start) reuses them."""
    real = fill is not None or env is not None
    if env is None:
        env = isolated_env(dict(os.environ))
        if fill:
            fill(env["DEVDECK_CONFIG_DIR"], env["DEVDECK_STATE_DIR"])
    env.pop("ZED_HEADLESS", None)
    if args.x11:
        env.pop("WAYLAND_DISPLAY", None)
    env["DEVDECK_STATS"] = "10"  # rare: reading smaps_rollup itself costs CPU
    env.update(env_extra or {})
    log = open(args.log, "w")
    p = subprocess.Popen([args.bin] + ([] if real else ["--demo"]), env=env, stdout=log, stderr=subprocess.STDOUT)
    p.env = env
    return p


# Writes `rate` coloured lines per second, like a dev server's log, and how many it
# wrote so far to `out` once a second (the run's own pace: devdeck may hold it back).
STREAM = r"""
import os, sys, time
rate, out = float(sys.argv[1]), sys.argv[2]
n, t0, last = 0, time.monotonic(), 0.0
while True:
    now = time.monotonic()
    while n < int((now - t0) * rate):
        c = 31 + n % 6
        sys.stdout.write(f"\x1b[2m{now - t0:9.3f}\x1b[0m \x1b[{c}mINFO\x1b[0m GET /api/items/{n % 977} \x1b[1m200\x1b[0m in {n % 53} ms, request {n}\n")
        n += 1
    sys.stdout.flush()
    if now - last >= 1:
        with open(out + ".tmp", "w") as f:
            f.write(f"{now} {n}\n")
        os.replace(out + ".tmp", out)
        last = now
    time.sleep(0.01)
"""

REAL_RUN = "run/stream/make:run"


def real_workspace(rate, recipe="python3 stream.py {rate} {out}"):
    """A `fill` for `start`: one workspace, `measure`, whose folder holds one project,
    `stream`, whose `make run` runs `recipe`. Its run view was open with the run
    running, so the run starts again when devdeck opens it (`DEVDECK_OPEN`). Returns
    the fill and the file the stream counts its lines in (outside the watched folders)."""
    work = tempfile.mkdtemp(prefix="devdeck-measure-")
    TEMP_DIRS.append(work)
    out = os.path.join(work, "lines")

    def fill(config, state):
        proj = os.path.join(work, "projects", "stream")
        os.makedirs(proj)
        os.makedirs(os.path.join(config, "workspaces"))
        with open(os.path.join(config, "workspaces", "measure.toml"), "w") as f:
            f.write(f'name = "measure"\nfolders = ["{os.path.dirname(proj)}"]\n')
        with open(os.path.join(proj, "Makefile"), "w") as f:
            f.write(f"run:\n\t@{recipe.format(rate=rate, out=out)}\n")
        with open(os.path.join(proj, "stream.py"), "w") as f:
            f.write(STREAM)
        views = os.path.join(state, "workspaces", "measure")
        os.makedirs(views)
        with open(os.path.join(views, "views.json"), "w") as f:
            json.dump({REAL_RUN: {"follow": True, "running": True, "args": ""}}, f)

    return fill, out


def lines_written(out):
    with open(out) as f:
        t, n = f.read().split()
    return float(t), int(n)


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


def scenario_real(args):
    """`terminal` with a real process (one run, not the demo's three): devdeck's CPU and
    memory, and the pace the stream kept (devdeck holds back a run it can't follow)."""
    view = {"DEVDECK_OPEN": REAL_RUN}
    fill, _ = real_workspace(0.5)  # the demo's normal rate: a line every 2 s
    base = start(args, view, fill=fill)
    try:
        time.sleep(8)
        _, _, uss0 = mem(base.pid)
        report("real.normal_rate_uss_kb", uss0)
    finally:
        base.terminate()
        base.wait(10)
    fill, out = real_workspace(2000)
    p = start(args, view, fill=fill)
    try:
        time.sleep(8)
        t0, n0 = lines_written(out)
        report("real.cpu_at_2000_lines_s_pct", f"{cpu_between(p.pid, 15):.2f}")
        t1, n1 = lines_written(out)
        report("real.lines_per_s_written", f"{(n1 - n0) / (t1 - t0):.0f}")
        rss, pss, uss = mem(p.pid)
        report("real.full_history_kb", f"rss={rss} pss={pss} uss={uss}")
        report("real.history_cost_uss_kb", uss - uss0)
    finally:
        p.terminate()
        p.wait(10)
    time.sleep(1)
    report("real.left_running_after_sigterm", len(marked([out])))


def marked(marks):
    """pid -> command line of the live processes whose command line holds one of
    `marks`."""
    found = {}
    for d in filter(str.isdigit, os.listdir("/proc")):
        try:
            with open(f"/proc/{d}/cmdline", "rb") as f:
                cmd = f.read().replace(b"\0", b" ").decode().strip()
            with open(f"/proc/{d}/stat") as f:
                state = f.read().rsplit(")", 1)[1].split()[0]
        except OSError:
            continue
        if state not in "ZX" and any(m in cmd for m in marks):
            found[int(d)] = cmd
    return found


def session_of(pid):
    with open(f"/proc/{pid}/stat") as f:
        return int(f.read().rsplit(")", 1)[1].split()[3])


def alive_since(pid, start_):
    """`pid` runs and started at `start_` (clock ticks), as `GroupLog` checks."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            fields = f.read().rsplit(")", 1)[1].split()
        return fields[19] == str(start_) and fields[0] not in "ZX"
    except OSError:
        return False


def scope_pids(unit):
    """The processes in a run's scope (devdeck-<instance>-<n>), none once it's gone."""
    uid = os.getuid()
    root = f"/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service"
    for dirpath, dirnames, _ in os.walk(root):
        if f"{unit}.scope" in dirnames:
            with open(os.path.join(dirpath, f"{unit}.scope", "cgroup.procs")) as f:
                return [int(l) for l in f.read().split()]
    return []


def log_lines(state):
    try:
        return open(os.path.join(state, "processes")).read().split("\n")
    except OSError:
        return None


def group_log(state):
    """The state folder's group log, as `GroupLog::leftovers` reads it: `pid start` lines
    (a run's group, running or ended) and `scope <unit> <instance> <label>` lines (whether
    that devdeck still runs, and how many processes the scope holds)."""
    lines = log_lines(state)
    if lines is None:
        return "no file"
    rows = []
    for line in lines:
        if line.startswith("scope "):
            unit, instance = line.split(" ", 3)[1:3]
            devdeck = "devdeck running" if alive_since(*instance.split("-")) else "devdeck gone"
            rows.append(f"{unit}({devdeck}, {len(scope_pids(unit))} processes)")
        elif line.strip():
            pid, start_ = line.split()
            rows.append(f"{pid}({'running' if alive_since(pid, start_) else 'ended'})")
    return " ".join(rows) or "empty"


def offered(state):
    """How many scopes the next devdeck offers: of a devdeck that's gone, not empty."""
    n = 0
    for line in log_lines(state) or []:
        if line.startswith("scope "):
            unit, instance = line.split(" ", 3)[1:3]
            if not alive_since(*instance.split("-")) and scope_pids(unit):
                n += 1
    return n


def scenario_leftovers(args):
    """R8.2. The run's recipe starts `sleep 30101` as a daemon does (a double fork: out
    of the run's tree at once, in its own session) and `sleep 30102` in the group, then
    streams. Three devdecks on the same folders: one gets SIGTERM, one kill -9, and the
    next one starts (the run was running, so it starts again), offers what the killed one
    left (counted from the log: the dialog isn't clicked here) and gets SIGTERM, which
    leaves the offered ones alone (only the user decides). Kills whatever is left."""
    fill, out = real_workspace(20, "( setsid sleep 30101 & ) ; sleep 30102 & python3 stream.py {rate} {out}")
    marks = ["sleep 30101", "sleep 30102", out]
    view = {"DEVDECK_OPEN": REAL_RUN}

    def show(label):
        procs = marked(marks)
        report(f"{label}.processes", "; ".join(f"{pid} sid={session_of(pid)} {cmd}" for pid, cmd in sorted(procs.items())) or "none")
        report(f"{label}.group_log", group_log(state))
        return procs

    p = start(args, view, fill=fill)
    env, state = p.env, p.env["DEVDECK_STATE_DIR"]
    try:
        time.sleep(6)
        show("sigterm.before")
        t = time.monotonic()
        p.send_signal(signal.SIGTERM)
        p.wait(10)
        report("sigterm.exit_s", f"{time.monotonic() - t:.2f}")
        time.sleep(1)
        show("sigterm.after")

        p = start(args, view, env=env)
        time.sleep(6)
        show("kill9.before")
        p.kill()
        p.wait(10)
        time.sleep(2)
        left = show("kill9.after")

        report("kill9.offered_at_next_start", offered(state))
        p = start(args, view, env=env)
        time.sleep(6)
        report("next_start.kill9_survivors_alive", sum(1 for pid in left if pid in marked(marks)))
        show("next_start")
        p.send_signal(signal.SIGTERM)
        p.wait(10)
        time.sleep(1)
        report("next_start_sigterm.kill9_survivors_alive", sum(1 for pid in left if pid in marked(marks)))
        show("next_start_sigterm.after")
    finally:
        if p.poll() is None:
            p.kill()
            p.wait(10)
        rest = marked(marks)
        for pid in rest:
            os.kill(pid, signal.SIGKILL)
        report("killed_by_this_script", len(rest))


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
    ap.add_argument("scenario", choices=["idle", "latency", "leak", "heapdiff", "terminal", "real", "leftovers"])
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
    elif args.scenario == "latency":
        scenario_latency(args)
    elif args.scenario == "terminal":
        scenario_terminal(args)
    elif args.scenario == "real":
        scenario_real(args)
    elif args.scenario == "leftovers":
        scenario_leftovers(args)
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
