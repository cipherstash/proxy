#!/usr/bin/env python3
"""Require a fresh, completed Go suite and the direct control's exact coverage."""
import argparse
import json
from pathlib import Path
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("events", type=Path)
    parser.add_argument("--exit-code", required=True, type=int)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--manifest", type=Path)
    args = parser.parse_args()
    if args.exit_code != 0:
        raise ValueError(f"container/test process exited {args.exit_code}")

    started_packages, passed_packages = set(), set()
    started_tests, passed_tests = set(), set()
    for number, line in enumerate(args.events.read_text().splitlines(), 1):
        event = json.loads(line)
        if not isinstance(event, dict):
            raise ValueError(f"line {number}: expected a Go test event object")
        action, package, test = (event.get(key) for key in ("Action", "Package", "Test"))
        if action not in {"start", "run", "pause", "cont", "pass", "fail", "skip", "output", "build-output"}:
            raise ValueError(f"line {number}: unknown action {action!r}")
        if action == "build-output":
            continue
        if not isinstance(package, str) or not package:
            raise ValueError(f"line {number}: missing package")
        if test is not None and (not isinstance(test, str) or not test):
            raise ValueError(f"line {number}: invalid test identity")
        if action in {"fail", "skip"}:
            raise ValueError(f"{package}/{test or '<package>'}: {action}")
        output = event.get("Output", "")
        if not isinstance(output, str):
            raise ValueError(f"line {number}: invalid output")
        if "(cached)" in output:
            raise ValueError("cached results cannot establish current compatibility")
        identity = (package, test)
        if action == "start":
            if test is not None or package in started_packages:
                raise ValueError(f"invalid or duplicate package start: {package}")
            started_packages.add(package)
        elif action == "run":
            if test is None or package not in started_packages or identity in started_tests:
                raise ValueError(f"invalid or duplicate test start: {identity}")
            started_tests.add(identity)
        elif action == "pass":
            if test is None:
                if package not in started_packages or package in passed_packages:
                    raise ValueError(f"invalid or duplicate package completion: {package}")
                passed_packages.add(package)
            else:
                if identity not in started_tests or identity in passed_tests:
                    raise ValueError(f"invalid or duplicate test completion: {identity}")
                passed_tests.add(identity)

    if not passed_tests:
        raise ValueError("no completed tests")
    if started_tests != passed_tests or started_packages != passed_packages:
        raise ValueError("incomplete test/package execution")
    manifest = {"packages": sorted(passed_packages), "tests": sorted(map(list, passed_tests))}
    if args.baseline and json.loads(args.baseline.read_text()) != manifest:
        raise ValueError("test/package identities differ from the successful direct control")
    if args.manifest:
        args.manifest.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"passed {len(passed_tests)} tests in {len(passed_packages)} packages; fresh complete execution")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as error:
        print(f"Go gate failed: {error}", file=sys.stderr)
        sys.exit(1)
