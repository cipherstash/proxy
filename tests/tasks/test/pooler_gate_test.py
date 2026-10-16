#!/usr/bin/env python3
"""Exercise the Go result checker and matrix task through CLI exit status."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CHECKER = Path(__file__).with_name("pooler_go_result.py")


class ResultGate(unittest.TestCase):
    def test_completed_suite_and_direct_coverage_are_required(self):
        complete = [
            {"Action": "start", "Package": "suite"},
            {"Action": "run", "Package": "suite", "Test": "TestEncryptedRoundTrip"},
            {"Action": "pass", "Package": "suite", "Test": "TestEncryptedRoundTrip"},
            {"Action": "pass", "Package": "suite"},
        ]
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            manifest = Path(directory) / "direct.json"
            events.write_text("".join(json.dumps(event) + "\n" for event in complete))
            command = [sys.executable, str(CHECKER), str(events), "--exit-code", "0"]
            result = subprocess.run(command + ["--manifest", str(manifest)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(manifest.exists())
            self.assertEqual(json.loads(manifest.read_text())["tests"], [["suite", "TestEncryptedRoundTrip"]])
            result = subprocess.run(command + ["--baseline", str(manifest)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            cases = {
                "truncated": complete[:-1],
                "missing test completion": complete[:2] + complete[3:],
                "skipped": complete[:2] + [{"Action": "skip", "Package": "suite", "Test": "TestEncryptedRoundTrip"}] + complete[3:],
                "failed": complete[:2] + [{"Action": "fail", "Package": "suite", "Test": "TestEncryptedRoundTrip"}] + complete[3:],
                "cached": complete + [{"Action": "output", "Package": "suite", "Output": "ok suite (cached)"}],
                "different coverage": [dict(event, Test="TestDifferent") if "Test" in event else event for event in complete],
                "zero tests": [complete[0], complete[-1]],
            }
            for name, candidate in cases.items():
                with self.subTest(name=name):
                    events.write_text("".join(json.dumps(event) + "\n" for event in candidate))
                    result = subprocess.run(command + ["--baseline", str(manifest)], capture_output=True, text=True)
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            events.write_text("".join(json.dumps(event) + "\n" for event in complete))
            result = subprocess.run([sys.executable, str(CHECKER), str(events), "--exit-code", "125"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)
            events.write_text("not JSON\n")
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)

    def test_out_of_order_and_duplicate_events_are_rejected(self):
        start = {"Action": "start", "Package": "suite"}
        run = {"Action": "run", "Package": "suite", "Test": "TestEncryptedRoundTrip"}
        test_pass = {"Action": "pass", "Package": "suite", "Test": "TestEncryptedRoundTrip"}
        package_pass = {"Action": "pass", "Package": "suite"}
        cases = {
            "duplicate run": ([start, run, run, test_pass, package_pass], "invalid or duplicate test start"),
            "run before start": ([run, start, test_pass, package_pass], "invalid or duplicate test start"),
            "duplicate test pass": ([start, run, test_pass, test_pass, package_pass], "invalid or duplicate test completion"),
            "test pass without run": ([start, test_pass, package_pass], "invalid or duplicate test completion"),
            "duplicate package pass": ([start, run, test_pass, package_pass, package_pass], "invalid or duplicate package completion"),
            "package pass before start": ([package_pass, start, run, test_pass], "invalid or duplicate package completion"),
            "duplicate package start": ([start, start, run, test_pass, package_pass], "invalid or duplicate package start"),
        }
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            for name, (candidate, reason) in cases.items():
                with self.subTest(name=name):
                    events.write_text("".join(json.dumps(event) + "\n" for event in candidate))
                    result = subprocess.run(
                        [sys.executable, str(CHECKER), str(events), "--exit-code", "0"],
                        capture_output=True, text=True,
                    )
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertIn(reason, result.stderr)

    def test_empty_success_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            events.write_text("")
            result = subprocess.run(
                [sys.executable, str(CHECKER), str(events), "--exit-code", "0"],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("no completed tests", result.stderr)


class MatrixTaskGate(unittest.TestCase):
    def test_supported_driver_failure_changes_task_exit_status(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            commands = root / "bin"
            commands.mkdir()
            stub = commands / "stub"
            stub.write_text(f"#!{sys.executable}\n" + """
import json, os, pathlib, sys
command = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
if command == "docker" and args[:2] == ["network", "ls"]:
    print("fixture-network")
elif command == "docker" and args[:1] == ["run"]:
    if os.environ.get("INDUCE_DOCKER_WARNING"):
        print("WARNING: fixture Docker diagnostic", file=sys.stderr)
    failed = os.environ.get("INDUCE_POOLER_FAILURE") and any("pgbouncer-1-19:6432/app_session" in arg for arg in args)
    for event in [
        {"Action": "start", "Package": "suite"},
        {"Action": "run", "Package": "suite", "Test": "TestEncryption"},
        {"Action": "fail" if failed else "pass", "Package": "suite", "Test": "TestEncryption"},
        {"Action": "fail" if failed else "pass", "Package": "suite"},
    ]:
        print(json.dumps(event))
    sys.exit(1 if failed else 0)
elif command == "cargo":
    print("MATRIX | fixture | works")
    if os.environ.get("INDUCE_MATRIX_FAILURE") and "--nocapture" in args:
        sys.exit(1)
""")
            stub.chmod(0o755)
            for command in ["mise", "docker", "psql", "cargo"]:
                (commands / command).symlink_to(stub)
            env = dict(os.environ, PATH=str(commands) + os.pathsep + os.environ["PATH"],
                       POOLERS_GOLANG="supported", POOLERS_RESULTS_DIR=str(root / "results"),
                       CS_DEFAULT_KEYSET_ID="fixture", CS_CLIENT_ID="fixture", CS_CLIENT_KEY="fixture",
                       CS_CLIENT_ACCESS_KEY="fixture", CS_WORKSPACE_CRN="fixture",
                       CS_DATABASE__PASSWORD="fixture", CS_DATABASE__PASSWORD_ESCAPED_FOR_TESTS="fixture",
                       CS_DATABASE__USERNAME="fixture", CS_DATABASE__NAME="fixture", CI="true")
            for key in ["POOLERS_TEST_FILTER", "POOLERS_SKIP_BUILD", "POOLERS_KEEP_RUNNING"]:
                env.pop(key, None)
            task = ["bash", str(CHECKER.with_name("poolers.sh"))]
            result = subprocess.run(task, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(result.stdout.count("GOLANG |"), 16)  # eight results and their summary
            result = subprocess.run(task, env=dict(env, INDUCE_POOLER_FAILURE="1"), capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("FAIL", result.stdout)
            result = subprocess.run(task, env=dict(env, INDUCE_MATRIX_FAILURE="1"), capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            for setting in [{"POOLERS_TEST_FILTER": "test(none)"}, {"POOLERS_SKIP_BUILD": "1"}, {"POOLERS_GOLANG": "off"}, {"CS_CLIENT_KEY": ""}]:
                with self.subTest(setting=setting):
                    result = subprocess.run(task, env=dict(env, **setting), capture_output=True, text=True)
                    self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            with self.subTest("Docker stderr must not corrupt passing Go JSON events"):
                result = subprocess.run(task, env=dict(env, INDUCE_DOCKER_WARNING="1"), capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(result.stdout.count("GOLANG |"), 16)
                diagnostics = list((root / "results").glob("go-*.stderr.log"))
                self.assertEqual(len(diagnostics), 8)
                for diagnostic in diagnostics:
                    self.assertIn("WARNING: fixture Docker diagnostic", diagnostic.read_text())
                for events in (root / "results").glob("go-*.jsonl"):
                    self.assertTrue(events.read_text())
                    for line in events.read_text().splitlines():
                        self.assertIsInstance(json.loads(line), dict)


if __name__ == "__main__":
    unittest.main()
