#!/usr/bin/env python3
"""
build_summary.py
Reads per-target status JSON files (one per build job, downloaded as
artifacts named status-<platform>, each containing a status.json) and
prints a single markdown summary for $GITHUB_STEP_SUMMARY.
"""

import argparse
import json
import os


def parse_args():
    p = argparse.ArgumentParser()
    p.add_argument(
        "--artifacts-dir",
        required=True,
        help="Directory containing one subfolder per downloaded artifact",
    )
    return p.parse_args()


def collect_statuses(artifacts_dir):
    rows = []
    if not os.path.isdir(artifacts_dir):
        return rows

    for entry in sorted(os.listdir(artifacts_dir)):
        entry_path = os.path.join(artifacts_dir, entry)
        if not os.path.isdir(entry_path):
            continue
        status_file = os.path.join(entry_path, "status.json")
        if not os.path.isfile(status_file):
            continue
        try:
            with open(status_file) as f:
                rows.append(json.load(f))
        except (json.JSONDecodeError, OSError):
            continue
    return rows


def group_label(platform):
    if platform.startswith("android-"):
        return "Android"
    if platform.startswith("ios-"):
        return "iOS"
    return "Desktop"


def render_table(rows):
    if not rows:
        print("No build status artifacts found.")
        return

    groups = {}
    for row in rows:
        label = group_label(row.get("platform", "unknown"))
        groups.setdefault(label, []).append(row)

    total = len(rows)
    passed = sum(1 for r in rows if r.get("status") == "success")
    failed = total - passed

    print(f"## Cross-Platform Build — {passed}/{total} targets passed\n")

    for group_name in ("Desktop", "Android", "iOS"):
        group_rows = groups.get(group_name)
        if not group_rows:
            continue
        print(f"### {group_name}")
        print("| Target | Status |")
        print("|---|---|")
        for row in sorted(group_rows, key=lambda r: r.get("target", "")):
            ok = row.get("status") == "success"
            icon = "✅" if ok else "❌"
            print(f"| `{row.get('target', 'unknown')}` | {icon} {row.get('status', 'unknown')} |")
        print()

    if failed > 0:
        print(f"⚠️ {failed} target(s) failed — check the matching job log for details.")


def main():
    args = parse_args()
    rows = collect_statuses(args.artifacts_dir)
    render_table(rows)


if __name__ == "__main__":
    main()
