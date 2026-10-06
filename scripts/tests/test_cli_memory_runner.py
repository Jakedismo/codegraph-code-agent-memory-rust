# ABOUTME: Validates memory-before-agent ordering, exact recall checks, and scoped cleanup.
# ABOUTME: All subprocesses use a stateful fixture; no installed CodeGraph or providers run.
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from cli_memory_tests import SEEDS, memory_check


class MemoryRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.project = self.root / "project with spaces"
        self.project.mkdir()
        for path, symbol, _ in SEEDS.values():
            source = self.project / path
            source.parent.mkdir(parents=True, exist_ok=True)
            # The server and executor fixtures carry several symbols.
            source.write_text(
                "\n".join(
                    sorted({value[1] for value in SEEDS.values() if value[0] == path})
                )
            )
        self.binary = self.root / "mock codegraph"
        self.binary.write_text((ROOT / "scripts/tests/mock_memory_cli.py").read_text())
        self.binary.chmod(0o755)
        self.record = self.root / "calls.jsonl"
        self.output = self.root / "results"

    def execute(self, *arguments, mode="valid"):
        return subprocess.run(
            [
                sys.executable,
                str(ROOT / "test_cli_agentic.py"),
                "--binary",
                str(self.binary),
                "--project",
                str(self.project),
                "--output-dir",
                str(self.output),
                "--summary-only",
                *arguments,
            ],
            cwd=self.root,
            env=dict(os.environ, MOCK_RECORD=str(self.record), MOCK_MODE=mode),
            text=True,
            capture_output=True,
            timeout=30,
            check=False,
        )

    def report(self):
        path = next(self.output.glob("*/summary.json"))
        return path.parent, json.loads(path.read_text())

    def calls(self):
        return [json.loads(line) for line in self.record.read_text().splitlines()]

    def test_memory_contracts_then_all_seeds_then_all_agent_cases_and_cleanup(self):
        process = self.execute("--project-id", "$(literal); custom project")
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        directory, report = self.report()
        calls = self.calls()
        first_agent = next(
            index for index, call in enumerate(calls) if "agent" in call["argv"]
        )
        memory_results = report["memory"]["results"]
        labels = [result["test"] for result in memory_results]
        self.assertTrue(all(result["status"] == "OK" for result in memory_results))
        for label in (
            "unanchored_write",
            "semantic_unanchored_read",
            "update_semantic_selection",
            "delete_semantic_selection",
            "correction",
            "update_stale_revision",
            "delete_stale_revision",
            "confirm_evidence",
            "idempotent_feedback",
            "persistent_recall_after_restart",
            "delete_exact_revision",
            "forgotten_write_replay",
            "session_async_write",
            "other_session_isolation",
            "complete_session",
            "archive_explicit_read",
            "user_semantic_read",
        ):
            self.assertIn(label, labels)
        for number in range(1, 9):
            index = labels.index(f"seed_{number:02}_ready")
            self.assertLess(index, first_agent)
            self.assertEqual(
                report["memory"]["seeds"][str(number)]["probe"]["status"], "OK"
            )
        agents = [call for call in calls if "agent" in call["argv"]]
        self.assertEqual(len(agents), 8)
        self.assertEqual(report["counts"], {"OK": 8})
        for call in agents:
            self.assertEqual(call["argv"][call["argv"].index("--memory") + 1], "on")
            self.assertEqual(call["project_id"], "$(literal); custom project")
        self.assertEqual(len({call["home"] for call in calls}), 1)
        self.assertEqual(
            Path(calls[0]["home"]).resolve(), (directory / "memory-home").resolve()
        )
        state = json.loads((directory / "memory-home/mock-state.json").read_text())
        self.assertFalse(state["running"])
        self.assertEqual(state["claims"], {})
        for result in report["results"]:
            self.assertEqual(result["memory_check"]["status"], "OK")
            self.assertTrue(result["memory_check"]["matched"])
            saved = json.loads((directory / result["json_file"]).read_text())
            self.assertTrue(
                saved["response"]["memory_context"]["retrieved_memory_refs"]
            )
        self.assertEqual(
            report["results"][0]["memory_check"]["matched"][0]["section"],
            "needs_verification",
        )
        self.assertTrue((directory / "memory_manifest.json").exists())

    def test_memory_only_runs_contracts_and_no_agent_or_seeding(self):
        process = self.execute("--memory-only")
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        _, report = self.report()
        self.assertEqual(report["results"], [])
        self.assertEqual(report["memory"]["seeds"], {})
        self.assertFalse(any("agent" in call["argv"] for call in self.calls()))

    def test_vanilla_or_failed_background_processing_blocks_agents(self):
        for mode in ("vanilla", "classification-failed", "seed-failed", "no-grounding"):
            with self.subTest(mode=mode):
                self.output = self.root / mode
                self.record = self.root / f"{mode}.jsonl"
                process = self.execute("--case", "1", mode=mode)
                self.assertEqual(process.returncode, 1, process.stdout + process.stderr)
                _, report = self.report()
                self.assertTrue(report["memory"]["error"])
                self.assertFalse(report["results"])
                self.assertFalse(any("agent" in call["argv"] for call in self.calls()))
                self.assertTrue(
                    any(
                        result["status"] != "OK"
                        for result in report["memory"]["results"]
                    )
                )

    def test_missing_seed_wrong_revision_partial_and_absent_context_fail_agent_checks(
        self,
    ):
        for mode in (
            "missing-memory",
            "wrong-revision",
            "partial-memory",
            "no-context",
        ):
            with self.subTest(mode=mode):
                self.output = self.root / mode
                self.record = self.root / f"{mode}.jsonl"
                process = self.execute("--case", "1", "--case", "2", mode=mode)
                self.assertEqual(process.returncode, 1, process.stdout + process.stderr)
                directory, report = self.report()
                self.assertEqual(report["counts"], {"MEMORY_FAILED": 2})
                self.assertEqual(report["memory"]["error"], None)
                self.assertEqual(
                    json.loads((directory / "memory-home/mock-state.json").read_text())[
                        "claims"
                    ],
                    {},
                )

    def test_filters_seed_only_selected_cases_without_changing_questions(self):
        process = self.execute("--case", "7", "--tool", "architecture")
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        _, report = self.report()
        self.assertEqual(list(report["memory"]["seeds"]), ["7"])
        self.assertEqual([result["case"] for result in report["results"]], [7])

    def test_memory_dry_run_and_list_have_no_side_effects(self):
        for flag in ("--dry-run", "--list"):
            process = self.execute(flag, "--binary", "not-installed", "--case", "3")
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertIn("FIRST: memory", process.stdout)
            self.assertIn("Memory seed 03", process.stdout)
            self.assertFalse(self.output.exists())
            self.assertFalse(self.record.exists())

    def test_malformed_context_is_a_reported_failure(self):
        for context in (None, {}, {"memories": None}, {"memories": [{}]}):
            self.assertEqual(
                memory_check(context, ["memory:claim@1"])["status"], "FAIL"
            )

    def test_equivalent_seed_merges_refresh_earlier_expected_revisions(self):
        process = self.execute("--case", "2", "--case", "4", mode="merged-seeds")
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        _, report = self.report()
        first = report["memory"]["seeds"]["2"]["expected_refs"]
        second = report["memory"]["seeds"]["4"]["expected_refs"]
        self.assertEqual(first, second)
        self.assertTrue(int(first[0].rsplit("@", 1)[1]) > 1)

    def test_cleanup_preserves_unrelated_claims(self):
        process = self.execute("--case", "1", mode="foreign-memory")
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        directory, _ = self.report()
        state = json.loads((directory / "memory-home/mock-state.json").read_text())
        self.assertEqual(
            state["claims"],
            {"foreign": {"id": "foreign", "foreign": True, "statement": "Keep me."}},
        )

    def test_failed_operation_is_reported_once_and_its_provisional_claim_is_cleaned(self):
        process = self.execute("--memory-only", mode="classification-failed")
        self.assertEqual(process.returncode, 1, process.stdout + process.stderr)
        directory, report = self.report()
        errors = [r for r in report["memory"]["results"] if r["status"] != "OK"]
        self.assertEqual([r["test"] for r in errors], ["unanchored_write_wait"])
        self.assertNotIn("cleanup_status", report["memory"]["error"])
        cleanup = next(r for r in report["memory"]["results"] if r["test"] == "cleanup_status")
        self.assertEqual(cleanup["response"]["state"], "failed")
        self.assertTrue(cleanup["response"]["error"])
        self.assertEqual(cleanup["status"], "OK")
        state = json.loads((directory / "memory-home/mock-state.json").read_text())
        self.assertEqual(state["claims"], {})
        self.assertFalse(state["running"])

    def test_memory_timeout_blocks_agents_and_saves_diagnostics(self):
        process = self.execute(
            "--case", "1", "--memory-timeout-secs", "1", mode="memory-timeout"
        )
        self.assertEqual(process.returncode, 1, process.stdout + process.stderr)
        directory, report = self.report()
        wait = next(
            result
            for result in report["memory"]["results"]
            if result["status"] == "TIMEOUT"
        )
        self.assertIn("waiting on mock classifier", wait["stdout"])
        self.assertIn("mock wait diagnostics", wait["stderr"])
        self.assertLess(wait["duration"], 10)
        self.assertFalse(report["results"])
        state = json.loads((directory / "memory-home/mock-state.json").read_text())
        self.assertFalse(state["running"])
        self.assertEqual(state["claims"], {})

    def test_cleanup_failure_is_reported_with_manifest_and_owner_shutdown(self):
        process = self.execute("--case", "1", mode="cleanup-failed")
        self.assertEqual(process.returncode, 1, process.stdout + process.stderr)
        directory, report = self.report()
        self.assertEqual(report["counts"], {"OK": 1})
        self.assertIn("cleanup", report["memory"]["error"].lower())
        manifest = json.loads((directory / "memory_manifest.json").read_text())
        self.assertTrue(manifest["seeds"])
        self.assertTrue(manifest["operations"])
        self.assertFalse(
            json.loads((directory / "memory-home/mock-state.json").read_text())[
                "running"
            ]
        )

    def test_memory_only_replay_needs_no_binary_or_provider(self):
        process = self.execute("--memory-only")
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        directory, _ = self.report()
        invocations = len(self.calls())
        process = self.execute(
            "--replay", str(directory), "--memory-only", "--binary", "missing-codegraph"
        )
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        self.assertIn("Memory unanchored_write: OK", process.stdout)
        self.assertEqual(len(self.calls()), invocations)


if __name__ == "__main__":
    unittest.main()
