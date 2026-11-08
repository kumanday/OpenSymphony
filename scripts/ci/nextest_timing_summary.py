#!/usr/bin/env python3
"""Render a Markdown timing summary from a cargo-nextest JUnit report.

Usage: nextest_timing_summary.py <junit.xml> [--top N]

Prints the slowest tests, per-binary wall time and slow-test counts so CI job
summaries show where test time goes without downloading artifacts.
"""
import argparse
import sys
import xml.etree.ElementTree as ET
from collections import defaultdict
from pathlib import Path

SLOW_THRESHOLDS = (5.0, 30.0)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("junit", type=Path)
    parser.add_argument("--top", type=int, default=25)
    args = parser.parse_args()

    print("## Rust test timing\n")
    if not args.junit.is_file():
        print(f"No JUnit report at `{args.junit}`; the test step did not produce one.")
        return 0

    root = ET.parse(args.junit).getroot()
    cases = []
    for suite in root.iter("testsuite"):
        binary = suite.get("name", "?")
        for case in suite.iter("testcase"):
            failed = case.find("failure") is not None or case.find("error") is not None
            cases.append((float(case.get("time", 0) or 0), binary, case.get("name", "?"), failed))

    if not cases:
        print("The JUnit report contains no test cases.")
        return 0

    total = sum(seconds for seconds, *_ in cases)
    per_binary = defaultdict(lambda: [0.0, 0])
    for seconds, binary, _, _ in cases:
        per_binary[binary][0] += seconds
        per_binary[binary][1] += 1

    wall = root.get("time")
    wall_note = f", wall time {float(wall):.1f}s" if wall else ""
    print(f"{len(cases)} tests, {total:.1f}s summed test time{wall_note}.")
    counts = ", ".join(
        f"{sum(1 for seconds, *_ in cases if seconds >= limit)} at or over {limit:g}s"
        for limit in SLOW_THRESHOLDS
    )
    print(f"Slow tests: {counts}.\n")

    print(f"### Slowest {min(args.top, len(cases))} tests\n")
    print("| Time | Binary | Test |")
    print("|---:|---|---|")
    for seconds, binary, name, failed in sorted(cases, reverse=True)[: args.top]:
        marker = " (failed)" if failed else ""
        print(f"| {seconds:.1f}s | `{binary}` | `{name}`{marker} |")

    print("\n### Summed test time by binary\n")
    print("| Time | Tests | Binary |")
    print("|---:|---:|---|")
    for binary, (seconds, count) in sorted(per_binary.items(), key=lambda item: -item[1][0]):
        print(f"| {seconds:.1f}s | {count} | `{binary}` |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
