from __future__ import annotations

import copy
import importlib.util
import pathlib
import tempfile
import unittest
from unittest import mock


SCRIPT = pathlib.Path(__file__).with_name("run-benchmark-gate.py")
SPEC = importlib.util.spec_from_file_location("run_benchmark_gate", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


def benchmark_output(run: int, *, omit: str | None = None, errors: str | None = None) -> str:
    parameters = gate.benchmark_parameters(500, 10_000, 1_000)
    lines = [
        "benchmark-parameters warmup_iterations=500 "
        "small_iterations=10000 large_iterations=1000"
    ]
    for index, (case, spec) in enumerate(
        gate.expected_case_specs(parameters).items(), start=1
    ):
        if case == omit:
            continue
        duration_ns = spec["iterations"] * 100_000
        p50_ns = 1_000 + index * 100 + run * 10
        p99_ns = p50_ns + 500
        error_count = 1 if case == errors else 0
        byte_count = spec["iterations"] * spec["payload_bytes"] * 2
        lines.append(
            f"benchmark-result case={case} binding={spec['binding']} "
            f"transport={spec['transport']} concurrency={spec['concurrency']} "
            f"payload={spec['payload']} payload_bytes={spec['payload_bytes']} "
            f"iterations={spec['iterations']} bytes={byte_count} "
            f"errors={error_count} duration_ns={duration_ns} qps=10000.000 "
            f"p50_ns={p50_ns} p99_ns={p99_ns}"
        )
    return "\n".join(lines) + "\n"


def ready_summary() -> dict:
    samples = [gate.parse_sample(benchmark_output(run), run) for run in range(1, 6)]
    return gate.build_summary(
        samples,
        {
            "source_commit": "a" * 40,
            "working_tree_dirty": False,
            "host": {
                "id": "fusen-0.9-reference-macos-arm64",
                "cpu": "Reference CPU",
                "os": "Reference OS",
            },
            "rust": {"toolchain": "1.97.0", "rustc": "rustc 1.97.0"},
        },
        {"argv": ["cargo", "bench"], "environment": {}},
    )


class BenchmarkGateTests(unittest.TestCase):
    def test_output_directory_must_be_new_or_empty(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            output_dir = pathlib.Path(temporary_directory) / "benchmark-output"
            gate.prepare_output_directory(output_dir)
            self.assertTrue(output_dir.is_dir())

            gate.prepare_output_directory(output_dir)
            (output_dir / "stale-summary.json").write_text("{}", encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "must be empty"):
                gate.prepare_output_directory(output_dir)

    def test_baseline_source_commit_must_be_an_ancestor_of_head(self) -> None:
        source_commit = "a" * 40
        present = mock.Mock(returncode=0)
        unrelated = mock.Mock(returncode=1)
        repo_root = pathlib.Path("/repo")
        baseline_path = repo_root / ".github/benchmarks/reference.json"

        with mock.patch.object(
            gate, "run_command", side_effect=[present, unrelated]
        ) as run_command:
            with self.assertRaisesRegex(RuntimeError, "not an ancestor of HEAD"):
                gate.verify_baseline_commit(repo_root, source_commit, baseline_path)

        self.assertEqual(
            run_command.call_args_list,
            [
                mock.call(
                    ["git", "cat-file", "-e", f"{source_commit}^{{commit}}"],
                    repo_root,
                ),
                mock.call(
                    [
                        "git",
                        "merge-base",
                        "--is-ancestor",
                        source_commit,
                        "HEAD",
                    ],
                    repo_root,
                ),
            ],
        )

    def test_final_candidate_may_only_change_the_committed_baseline(self) -> None:
        source_commit = "a" * 40
        repo_root = pathlib.Path("/repo")
        baseline_path = repo_root / ".github/benchmarks/reference.json"
        success = mock.Mock(returncode=0, stdout="")
        one_commit = mock.Mock(returncode=0, stdout="1\n")
        direct_parent = mock.Mock(
            returncode=0,
            stdout=f"{'c' * 40} {source_commit}\n",
        )
        baseline_delta = mock.Mock(
            returncode=0,
            stdout=".github/benchmarks/reference.json\0",
        )

        with mock.patch.object(
            gate,
            "run_command",
            side_effect=[
                success,
                success,
                one_commit,
                direct_parent,
                success,
                success,
                baseline_delta,
            ],
        ) as run_command:
            gate.verify_baseline_commit(repo_root, source_commit, baseline_path)

        self.assertEqual(
            run_command.call_args_list[-1],
            mock.call(
                [
                    "git",
                    "diff",
                    "--name-only",
                    "--no-renames",
                    "-z",
                    f"{source_commit}..HEAD",
                    "--",
                ],
                repo_root,
            ),
        )

    def test_final_candidate_must_be_one_commit_after_calibration(self) -> None:
        source_commit = "a" * 40
        repo_root = pathlib.Path("/repo")
        baseline_path = repo_root / ".github/benchmarks/reference.json"
        success = mock.Mock(returncode=0, stdout="")
        two_commits = mock.Mock(returncode=0, stdout="2\n")

        with mock.patch.object(
            gate,
            "run_command",
            side_effect=[success, success, two_commits],
        ):
            with self.assertRaisesRegex(RuntimeError, "exactly one commit"):
                gate.verify_baseline_commit(repo_root, source_commit, baseline_path)

    def test_final_candidate_must_have_only_the_calibration_parent(self) -> None:
        source_commit = "a" * 40
        repo_root = pathlib.Path("/repo")
        baseline_path = repo_root / ".github/benchmarks/reference.json"
        success = mock.Mock(returncode=0, stdout="")
        one_commit = mock.Mock(returncode=0, stdout="1\n")
        merge_parents = mock.Mock(
            returncode=0,
            stdout=f"{'c' * 40} {source_commit} {'b' * 40}\n",
        )

        with mock.patch.object(
            gate,
            "run_command",
            side_effect=[success, success, one_commit, merge_parents],
        ):
            with self.assertRaisesRegex(RuntimeError, "only its calibration source as parent"):
                gate.verify_baseline_commit(repo_root, source_commit, baseline_path)

    def test_final_candidate_rejects_non_baseline_changes(self) -> None:
        source_commit = "a" * 40
        repo_root = pathlib.Path("/repo")
        baseline_path = repo_root / ".github/benchmarks/reference.json"
        success = mock.Mock(returncode=0, stdout="")
        one_commit = mock.Mock(returncode=0, stdout="1\n")
        direct_parent = mock.Mock(
            returncode=0,
            stdout=f"{'c' * 40} {source_commit}\n",
        )
        extra_delta = mock.Mock(
            returncode=0,
            stdout=(
                ".github/benchmarks/reference.json\0"
                "fusen/src/lib.rs\0"
            ),
        )

        with mock.patch.object(
            gate,
            "run_command",
            side_effect=[
                success,
                success,
                one_commit,
                direct_parent,
                success,
                success,
                extra_delta,
            ],
        ):
            with self.assertRaisesRegex(
                RuntimeError,
                "must differ from its calibration source only",
            ):
                gate.verify_baseline_commit(repo_root, source_commit, baseline_path)

    def test_final_candidate_requires_a_baseline_delta(self) -> None:
        source_commit = "a" * 40
        repo_root = pathlib.Path("/repo")
        baseline_path = repo_root / ".github/benchmarks/reference.json"
        success = mock.Mock(returncode=0, stdout="")
        one_commit = mock.Mock(returncode=0, stdout="1\n")
        direct_parent = mock.Mock(
            returncode=0,
            stdout=f"{'c' * 40} {source_commit}\n",
        )
        no_delta = mock.Mock(returncode=0, stdout="")

        with mock.patch.object(
            gate,
            "run_command",
            side_effect=[
                success,
                success,
                one_commit,
                direct_parent,
                success,
                success,
                no_delta,
            ],
        ):
            with self.assertRaisesRegex(RuntimeError, "changed paths: <none>"):
                gate.verify_baseline_commit(repo_root, source_commit, baseline_path)

    def test_parse_requires_the_complete_eight_case_matrix(self) -> None:
        sample = gate.parse_sample(benchmark_output(1), 1)
        self.assertEqual(len(sample["cases"]), 8)
        self.assertEqual(
            {
                case
                for case, result in sample["cases"].items()
                if result["transport"] == "h2c"
            },
            {
                "h2c-c1-small",
                "h2c-c1-64k",
                "h2c-c100-small",
                "h2c-c100-64k",
            },
        )
        self.assertEqual(
            {result["binding"] for result in sample["cases"].values()},
            {"http-json-v1"},
        )
        with self.assertRaisesRegex(RuntimeError, "case mismatch"):
            gate.parse_sample(benchmark_output(1, omit="http1-c100-64k"), 1)

    def test_parse_rejects_any_request_error(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "reported 1 error"):
            gate.parse_sample(benchmark_output(1, errors="h2c-c1-small"), 1)

    def test_baseline_schema_requires_five_raw_samples_and_full_sha(self) -> None:
        baseline = gate.baseline_from_summary(ready_summary(), 10)
        gate.validate_baseline(baseline)
        self.assertEqual(
            len(baseline["cases"]["h2c-c100-64k"]["samples"]), 5
        )

        invalid_sha = copy.deepcopy(baseline)
        invalid_sha["source_commit"] = "candidate"
        with self.assertRaisesRegex(RuntimeError, "full Git object id"):
            gate.validate_baseline(invalid_sha)

        missing_sample = copy.deepcopy(baseline)
        missing_sample["cases"]["h2c-c100-64k"]["samples"].pop()
        with self.assertRaisesRegex(RuntimeError, "five raw samples"):
            gate.validate_baseline(missing_sample)

        inconsistent_qps = copy.deepcopy(baseline)
        inconsistent_qps["cases"]["h2c-c100-64k"]["samples"][0]["qps"] *= 2
        with self.assertRaisesRegex(RuntimeError, "inconsistent sample QPS"):
            gate.validate_baseline(inconsistent_qps)

        wrong_threshold = copy.deepcopy(baseline)
        wrong_threshold["threshold_percent"] = 9
        with self.assertRaisesRegex(RuntimeError, "exactly 10"):
            gate.validate_baseline(wrong_threshold)

        dirty = ready_summary()
        dirty["working_tree_dirty"] = True
        with self.assertRaisesRegex(RuntimeError, "clean working tree"):
            gate.baseline_from_summary(dirty, 10)
        with self.assertRaisesRegex(RuntimeError, "clean working tree"):
            gate.compare(dirty, baseline)

    def test_exact_ten_percent_transport_regression_passes_and_qps_is_not_enforced(
        self,
    ) -> None:
        summary = ready_summary()
        baseline = gate.baseline_from_summary(summary, 10)
        current = copy.deepcopy(summary)
        for case, reference in baseline["cases"].items():
            if reference["blocking"]:
                current["cases"][case]["median_p50_ns"] = (
                    reference["median_p50_ns"] * 110 // 100
                )
                current["cases"][case]["median_p99_ns"] = (
                    reference["median_p99_ns"] * 110 // 100
                )
                current["cases"][case]["median_qps"] = 1.0
        comparison = gate.compare(current, baseline)
        self.assertTrue(comparison["passed"])
        self.assertFalse(
            comparison["cases"]["h2c-c1-small"]["qps"]["enforced"]
        )
        self.assertTrue(
            comparison["cases"]["http1-c1-small"]["p99_ns"]["enforced"]
        )

    def test_more_than_ten_percent_in_any_transport_latency_metric_fails(self) -> None:
        summary = ready_summary()
        baseline = gate.baseline_from_summary(summary, 10)
        current = copy.deepcopy(summary)
        reference = baseline["cases"]["h2c-c100-64k"]["median_p99_ns"]
        current["cases"]["h2c-c100-64k"]["median_p99_ns"] = (
            reference * 110 // 100 + 1
        )
        with self.assertRaisesRegex(RuntimeError, "h2c-c100-64k p99"):
            gate.compare(current, baseline)

    def test_calibration_marker_fails_closed(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "requires fixed-host calibration"):
            gate.validate_baseline(
                {"schema_version": gate.SCHEMA_VERSION, "status": "calibration-required"}
            )

    def test_release_metadata_must_remain_clean_and_stable(self) -> None:
        initial = {
            "source_commit": "a" * 40,
            "working_tree_dirty": False,
            "host": {"id": "reference", "cpu": "cpu", "os": "os"},
            "rust": {"toolchain": "1.97.0", "rustc": "rustc 1.97.0"},
        }
        gate.ensure_stable_metadata(initial, copy.deepcopy(initial), require_clean=True)

        changed_revision = copy.deepcopy(initial)
        changed_revision["source_commit"] = "b" * 40
        with self.assertRaisesRegex(RuntimeError, "source revision changed"):
            gate.ensure_stable_metadata(
                initial, changed_revision, require_clean=True
            )

        dirty = copy.deepcopy(initial)
        dirty["working_tree_dirty"] = True
        with self.assertRaisesRegex(RuntimeError, "clean working tree"):
            gate.ensure_stable_metadata(initial, dirty, require_clean=True)


if __name__ == "__main__":
    unittest.main()
