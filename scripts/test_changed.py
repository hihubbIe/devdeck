#!/usr/bin/env python3
"""Runs clippy and the tests of the crates a change touches, and of the crates built on
them (docs/WORKFLOW.md, "Keep the step loop fast"). The full suite runs at the end of a
milestone.

Usage: scripts/test_changed.py [--since REV] [--dry-run] [--no-clippy]

  (default)     what changed in the working tree since the last commit, untracked
                files included
  --since REV   what changed since REV (e.g. `--since HEAD~3`), plus the working tree
  --dry-run     prints the crates and commands without running them
  --no-clippy   tests only

Editing `devdeck-ui` tests it and `devdeck-app` (built on it); editing only the app tests
only the app; editing `devdeck-core` or the workspace's Cargo files tests everything; a
change to docs or scripts alone tests nothing.
"""

import argparse
import json
import subprocess
import sys

ROOT_FILES = ("Cargo.toml", "Cargo.lock", "clippy.toml", "rust-toolchain.toml")


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, text=True, capture_output=True, **kw).stdout


def changed_files(since):
    files = set(run(["git", "diff", "--name-only", "HEAD"]).split())
    files |= set(run(["git", "ls-files", "--others", "--exclude-standard"]).split())
    if since:
        files |= set(run(["git", "diff", "--name-only", f"{since}..HEAD"]).split())
    return sorted(files)


def workspace():
    """Member crates: name -> (folder relative to the root, names of member crates it depends on)."""
    meta = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1"]))
    root = meta["workspace_root"].rstrip("/") + "/"
    names = {p["name"] for p in meta["packages"]}
    crates = {}
    for p in meta["packages"]:
        folder = p["manifest_path"][len(root):].rsplit("/", 1)[0] + "/"
        deps = {d["name"] for d in p["dependencies"] if d["name"] in names}
        crates[p["name"]] = (folder, deps)
    return crates


def to_test(files, crates):
    if any(f in ROOT_FILES for f in files):
        return sorted(crates)
    touched = {name for name, (folder, _) in crates.items() if any(f.startswith(folder) for f in files)}
    # Add every crate built on a touched one, until nothing changes.
    while True:
        more = {name for name, (_, deps) in crates.items() if deps & touched} - touched
        if not more:
            return sorted(touched)
        touched |= more


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--since")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--no-clippy", action="store_true")
    args = ap.parse_args()

    files = changed_files(args.since)
    crates = to_test(files, workspace())
    if not crates:
        print(f"{len(files)} changed file(s), none in a crate: nothing to test.")
        return 0
    packages = [arg for c in crates for arg in ("-p", c)]
    commands = []
    if not args.no_clippy:
        commands.append(["cargo", "clippy", *packages, "--all-targets", "--", "-D", "warnings"])
    commands.append(["cargo", "test", *packages])
    print(f"{len(files)} changed file(s) → testing {', '.join(crates)}")
    for cmd in commands:
        print("$ " + " ".join(cmd))
        if not args.dry_run and subprocess.run(cmd).returncode != 0:
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
