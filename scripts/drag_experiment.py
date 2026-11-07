#!/usr/bin/env python3
"""Cross-window tab drag experiment (M0 review finding 1), X11 only.

Starts devdeck under X11, detaches the focused tab with ctrl+shift+d, then drags the
detached window's tab onto the main window's tab strip with xdotool and reports
whether the model went back to a single window.

Usage: scripts/drag_experiment.py [--bin target/release/devdeck]
"""

import argparse
import os
import re
import subprocess
import time


def xdo(*args):
    return subprocess.run(["xdotool", *args], capture_output=True, text=True).stdout.strip()


def geometry(wid):
    out = xdo("getwindowgeometry", "--shell", wid)
    g = dict(line.split("=", 1) for line in out.splitlines() if "=" in line)
    return int(g["X"]), int(g["Y"]), int(g["WIDTH"]), int(g["HEIGHT"])


def last_windows(log):
    with open(log) as f:
        found = re.findall(r"windows=(\d+)", f.read())
    return int(found[-1]) if found else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="target/release/devdeck")
    ap.add_argument("--log", default="target/drag.log")
    args = ap.parse_args()
    env = dict(os.environ)
    env.pop("WAYLAND_DISPLAY", None)
    env["DEVDECK_STATS"] = "1"
    p = subprocess.Popen([args.bin], env=env, stdout=open(args.log, "w"), stderr=subprocess.STDOUT)
    try:
        time.sleep(4)
        main_wid = xdo("search", "--pid", str(p.pid)).split()[-1]
        xdo("windowactivate", "--sync", main_wid)
        xdo("key", "ctrl+shift+d")
        time.sleep(2)
        wids = xdo("search", "--pid", str(p.pid)).split()
        detached = [w for w in wids if w != main_wid]
        print("windows after detach:", last_windows(args.log), "x11 ids:", wids)
        if not detached:
            print("result: detach did not open a window")
            return
        dx, dy, _, _ = geometry(detached[-1])
        mx, my, _, _ = geometry(main_wid)
        # Tab of the detached window: just right of the grip, in the tab strip.
        src = (dx + 80, dy + 34 + 16)
        dst = (mx + 400, my + 34 + 16)
        xdo("mousemove", "--sync", str(src[0]), str(src[1]))
        xdo("mousedown", "1")
        for i in range(1, 31):
            x = src[0] + (dst[0] - src[0]) * i // 30
            y = src[1] + (dst[1] - src[1]) * i // 30
            xdo("mousemove", "--sync", str(x), str(y))
            time.sleep(0.02)
        xdo("mouseup", "1")
        time.sleep(2)
        n = last_windows(args.log)
        print("windows after drag:", n)
        print("result:", "re-attached by dragging" if n == 1 else "drag between windows did NOT re-attach")
    finally:
        p.terminate()
        p.wait(10)


if __name__ == "__main__":
    main()
