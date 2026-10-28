#!/usr/bin/env python3
"""Summarize completed interval samples; report maxima with their monotonic times.

Use alongside raw logs: interval maxima are not a per-frame trace or percentiles.
"""
import argparse
import json
from pathlib import Path


def rows(path):
    result = []
    for line in path.read_text().splitlines():
        parts = line.split()
        if not parts:
            continue
        row = dict(part.split("=", 1) for part in parts[1:] if "=" in part)
        row["kind"] = parts[0]
        for key, value in row.items():
            if value.isdecimal():
                row[key] = int(value)
        result.append(row)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    for path in sorted(args.directory.glob("*.log")):
        data = rows(path)
        print(f"\n{path.stem}")
        for name in ["operator", "left", "right"]:
            samples = [r for r in data if r["kind"] == "viewport" and
                       r.get("name") == name and r["max_gap_start_us"] >= 2_000_000]
            if samples:
                maximum = max(samples, key=lambda r: r["max_gap_us"])
                print(f"  {name}: max_gap_us={maximum['max_gap_us']} "
                      f"start_us={maximum['max_gap_start_us']}")
        for name in ["cadence", "record", "finish", "submit", "wait", "notify"]:
            samples = [r for r in data if r["kind"] == "stage" and
                       r.get("name") == name and r["count"]]
            if samples:
                maximum = max(samples, key=lambda r: r["max_us"])
                count = sum(r["count"] for r in samples)
                mean = sum(r["total_us"] for r in samples)/count
                print(f"  {name}: count={count} mean_us={mean:.1f} "
                      f"max_us={maximum['max_us']} start_us={maximum['max_start_us']}")
        # Preserve per-interval large gaps and slow stages instead of only a
        # lifetime maximum; the same event can appear in all three viewports.
        for row in data:
            if (row["kind"] == "viewport" and row["max_gap_us"] >= 100_000) or (
                    row["kind"] == "stage" and row["max_us"] >= 100_000):
                print("  slow_interval " + json.dumps(row, sort_keys=True))


if __name__ == "__main__":
    main()
