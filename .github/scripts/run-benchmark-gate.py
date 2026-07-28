#!/usr/bin/env python3
"""Run, archive, and compare the 0.9 direct invocation benchmark matrix."""

from __future__ import annotations

import argparse
import copy
import json
import math
import os
import pathlib
import platform
import re
import statistics
import subprocess
import sys
from datetime import datetime, timezone


SCHEMA_VERSION = 2
BENCHMARK_SUITE = "direct-invocation-matrix-v1"
REQUIRED_BASELINE_RUNS = 5
DEFAULT_THRESHOLD_PERCENT = 10
HOST_ID_PATTERN = re.compile(r"^[A-Za-z0-9._-]{1,64}$")
COMMIT_PATTERN = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")
PARAMETERS_PATTERN = re.compile(
    r"^benchmark-parameters warmup_iterations=(\d+) "
    r"small_iterations=(\d+) large_iterations=(\d+)$",
    re.MULTILINE,
)
RESULT_PATTERN = re.compile(
    r"^benchmark-result case=(\S+) protocol=(\S+) transport=(\S+) "
    r"concurrency=(\d+) payload=(\S+) payload_bytes=(\d+) "
    r"iterations=(\d+) bytes=(\d+) errors=(\d+) duration_ns=(\d+) "
    r"qps=([0-9]+(?:\.[0-9]+)?) p50_ns=(\d+) p99_ns=(\d+)$",
    re.MULTILINE,
)
PROTOCOLS = (
    ("fusen", "fusen-v1", "h2c", True),
    ("spring", "spring-cloud-v1", "http1", False),
)
CONCURRENCIES = (1, 100)
PAYLOADS = (
    ("small", 7, "small_iterations"),
    ("64k", 64 * 1024, "large_iterations"),
)


def run_command(
    command: list[str],
    cwd: pathlib.Path,
    *,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("value must be positive")
    return parsed


def percentile_iterations(value: str) -> int:
    parsed = positive_integer(value)
    if parsed < 100:
        raise argparse.ArgumentTypeError("measured iterations must be at least 100")
    return parsed


def threshold_percent(value: str) -> int:
    parsed = positive_integer(value)
    if parsed != DEFAULT_THRESHOLD_PERCENT:
        raise argparse.ArgumentTypeError("0.9 threshold must be exactly 10 percent")
    return parsed


def validate_host_id(value: str) -> str:
    if not HOST_ID_PATTERN.fullmatch(value):
        raise argparse.ArgumentTypeError(
            "host id must be 1-64 ASCII letters, digits, dot, underscore, or hyphen"
        )
    return value


def prepare_output_directory(output_dir: pathlib.Path) -> None:
    if output_dir.exists():
        if not output_dir.is_dir():
            raise RuntimeError(
                f"benchmark output path is not a directory: {output_dir}"
            )
        if any(output_dir.iterdir()):
            raise RuntimeError(
                f"benchmark output directory must be empty: {output_dir}"
            )
        return
    output_dir.mkdir(parents=True)


def benchmark_parameters(
    warmup_iterations: int,
    small_iterations: int,
    large_iterations: int,
) -> dict:
    return {
        "warmup_iterations": warmup_iterations,
        "measured_iterations": {
            "small": small_iterations,
            "64k": large_iterations,
        },
        "concurrencies": list(CONCURRENCIES),
        "payload_bytes": {label: size for label, size, _ in PAYLOADS},
    }


def expected_case_specs(parameters: dict) -> dict[str, dict]:
    specs = {}
    for prefix, protocol, transport, blocking in PROTOCOLS:
        for concurrency in CONCURRENCIES:
            for payload, payload_bytes, iteration_key in PAYLOADS:
                case = f"{prefix}-c{concurrency}-{payload}"
                specs[case] = {
                    "protocol": protocol,
                    "transport": transport,
                    "concurrency": concurrency,
                    "payload": payload,
                    "payload_bytes": payload_bytes,
                    "iterations": parameters["measured_iterations"][
                        "small" if iteration_key == "small_iterations" else "64k"
                    ],
                    "blocking": blocking,
                }
    return specs


def parse_sample(output: str, run_number: int) -> dict:
    parameter_matches = PARAMETERS_PATTERN.findall(output)
    if len(parameter_matches) != 1:
        raise RuntimeError(
            f"benchmark run {run_number} produced {len(parameter_matches)} parameter records"
        )
    warmup, small, large = map(int, parameter_matches[0])
    if warmup <= 0 or min(small, large) < 100:
        raise RuntimeError(f"benchmark run {run_number} reported invalid parameters")
    parameters = benchmark_parameters(warmup, small, large)
    expected = expected_case_specs(parameters)

    cases = {}
    for match in RESULT_PATTERN.finditer(output):
        (
            case,
            protocol,
            transport,
            concurrency,
            payload,
            payload_bytes,
            iterations,
            byte_count,
            errors,
            duration_ns,
            qps,
            p50_ns,
            p99_ns,
        ) = match.groups()
        if case in cases:
            raise RuntimeError(f"benchmark run {run_number} duplicated case {case}")
        cases[case] = {
            "protocol": protocol,
            "transport": transport,
            "concurrency": int(concurrency),
            "payload": payload,
            "payload_bytes": int(payload_bytes),
            "iterations": int(iterations),
            "bytes": int(byte_count),
            "errors": int(errors),
            "duration_ns": int(duration_ns),
            "qps": float(qps),
            "p50_ns": int(p50_ns),
            "p99_ns": int(p99_ns),
        }

    if set(cases) != set(expected):
        missing = sorted(set(expected).difference(cases))
        unexpected = sorted(set(cases).difference(expected))
        raise RuntimeError(
            f"benchmark run {run_number} case mismatch; "
            f"missing={missing}, unexpected={unexpected}"
        )

    for case, spec in expected.items():
        sample = cases[case]
        for field in (
            "protocol",
            "transport",
            "concurrency",
            "payload",
            "payload_bytes",
            "iterations",
        ):
            if sample[field] != spec[field]:
                raise RuntimeError(
                    f"benchmark run {run_number} case {case} has unexpected {field}: "
                    f"{sample[field]!r}, expected {spec[field]!r}"
                )
        if sample["errors"] != 0:
            raise RuntimeError(
                f"benchmark run {run_number} case {case} reported "
                f"{sample['errors']} error(s)"
            )
        expected_bytes = sample["iterations"] * sample["payload_bytes"] * 2
        if sample["bytes"] != expected_bytes:
            raise RuntimeError(
                f"benchmark run {run_number} case {case} reported {sample['bytes']} bytes; "
                f"expected {expected_bytes}"
            )
        if min(
            sample["duration_ns"],
            sample["qps"],
            sample["p50_ns"],
            sample["p99_ns"],
        ) <= 0:
            raise RuntimeError(
                f"benchmark run {run_number} case {case} reported a non-positive metric"
            )
        if sample["p50_ns"] > sample["p99_ns"]:
            raise RuntimeError(
                f"benchmark run {run_number} case {case} reported p50 greater than p99"
            )
        calculated_qps = sample["iterations"] * 1_000_000_000 / sample["duration_ns"]
        if not math.isclose(sample["qps"], calculated_qps, rel_tol=1e-6, abs_tol=0.001):
            raise RuntimeError(
                f"benchmark run {run_number} case {case} reported inconsistent QPS"
            )

    return {"run": run_number, "parameters": parameters, "cases": cases}


def aggregate_samples(run_samples: list[dict]) -> tuple[dict, dict[str, dict]]:
    if not run_samples:
        raise RuntimeError("benchmark produced no samples")
    parameters = run_samples[0]["parameters"]
    if any(sample["parameters"] != parameters for sample in run_samples[1:]):
        raise RuntimeError("benchmark parameters changed between runs")
    expected = expected_case_specs(parameters)
    cases = {}
    for case, spec in expected.items():
        raw_samples = []
        for run_sample in run_samples:
            sample = run_sample["cases"][case]
            raw_samples.append(
                {
                    "run": run_sample["run"],
                    "iterations": sample["iterations"],
                    "bytes": sample["bytes"],
                    "errors": sample["errors"],
                    "duration_ns": sample["duration_ns"],
                    "qps": sample["qps"],
                    "p50_ns": sample["p50_ns"],
                    "p99_ns": sample["p99_ns"],
                }
            )
        cases[case] = {
            **spec,
            "samples": raw_samples,
            "median_qps": statistics.median(
                sample["qps"] for sample in raw_samples
            ),
            "median_p50_ns": int(
                statistics.median(sample["p50_ns"] for sample in raw_samples)
            ),
            "median_p99_ns": int(
                statistics.median(sample["p99_ns"] for sample in raw_samples)
            ),
        }
    return parameters, cases


def _require_mapping(value: object, field: str) -> dict:
    if not isinstance(value, dict):
        raise RuntimeError(f"benchmark baseline field {field} must be an object")
    return value


def _require_non_empty_string(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise RuntimeError(f"benchmark baseline field {field} must be non-empty")
    return value


def validate_baseline(baseline: dict) -> None:
    if baseline.get("status") == "calibration-required":
        raise RuntimeError(
            "benchmark baseline requires fixed-host calibration from a clean committed SHA"
        )
    required = {
        "schema_version",
        "status",
        "benchmark_suite",
        "source_commit",
        "recorded_at",
        "host",
        "rust",
        "parameters",
        "required_runs",
        "threshold_percent",
        "cases",
    }
    missing = sorted(required.difference(baseline))
    if missing:
        raise RuntimeError(f"benchmark baseline is missing: {', '.join(missing)}")
    if baseline["schema_version"] != SCHEMA_VERSION:
        raise RuntimeError("benchmark baseline has an unsupported schema version")
    if baseline["status"] != "ready" or baseline["benchmark_suite"] != BENCHMARK_SUITE:
        raise RuntimeError("benchmark baseline has an unsupported status or suite")
    source_commit = _require_non_empty_string(
        baseline["source_commit"], "source_commit"
    )
    if not COMMIT_PATTERN.fullmatch(source_commit):
        raise RuntimeError("benchmark baseline source_commit must be a full Git object id")
    _require_non_empty_string(baseline["recorded_at"], "recorded_at")

    host = _require_mapping(baseline["host"], "host")
    for field in ("id", "cpu", "os"):
        _require_non_empty_string(host.get(field), f"host.{field}")
    if not HOST_ID_PATTERN.fullmatch(host["id"]):
        raise RuntimeError("benchmark baseline host.id is invalid")

    rust = _require_mapping(baseline["rust"], "rust")
    for field in ("toolchain", "rustc"):
        _require_non_empty_string(rust.get(field), f"rust.{field}")

    if baseline["required_runs"] != REQUIRED_BASELINE_RUNS:
        raise RuntimeError("benchmark baseline must contain exactly five runs")
    threshold = baseline["threshold_percent"]
    if threshold != DEFAULT_THRESHOLD_PERCENT:
        raise RuntimeError("benchmark threshold_percent must be exactly 10")

    parameters = _require_mapping(baseline["parameters"], "parameters")
    warmup = parameters.get("warmup_iterations")
    measured = _require_mapping(parameters.get("measured_iterations"), "parameters.measured_iterations")
    if not isinstance(warmup, int) or warmup <= 0:
        raise RuntimeError("benchmark warmup_iterations must be positive")
    if any(
        not isinstance(measured.get(payload), int) or measured[payload] < 100
        for payload in ("small", "64k")
    ):
        raise RuntimeError("benchmark measured iterations must be at least 100")
    expected_parameters = benchmark_parameters(
        warmup, measured["small"], measured["64k"]
    )
    if parameters != expected_parameters:
        raise RuntimeError("benchmark baseline parameters do not match the fixed matrix")

    expected = expected_case_specs(parameters)
    cases = _require_mapping(baseline["cases"], "cases")
    if set(cases) != set(expected):
        raise RuntimeError("benchmark baseline cases do not match the fixed matrix")
    for case, spec in expected.items():
        record = _require_mapping(cases[case], f"cases.{case}")
        for field, expected_value in spec.items():
            if record.get(field) != expected_value:
                raise RuntimeError(
                    f"benchmark baseline case {case} has invalid {field}"
                )
        samples = record.get("samples")
        if not isinstance(samples, list) or len(samples) != REQUIRED_BASELINE_RUNS:
            raise RuntimeError(
                f"benchmark baseline case {case} must contain five raw samples"
            )
        if [sample.get("run") for sample in samples] != list(
            range(1, REQUIRED_BASELINE_RUNS + 1)
        ):
            raise RuntimeError(
                f"benchmark baseline case {case} samples must be runs 1 through 5"
            )
        for sample in samples:
            if sample.get("iterations") != spec["iterations"] or sample.get("errors") != 0:
                raise RuntimeError(
                    f"benchmark baseline case {case} has invalid iterations or errors"
                )
            expected_bytes = spec["iterations"] * spec["payload_bytes"] * 2
            if sample.get("bytes") != expected_bytes:
                raise RuntimeError(
                    f"benchmark baseline case {case} has invalid byte count"
                )
            integer_metrics = [
                sample.get("duration_ns"),
                sample.get("p50_ns"),
                sample.get("p99_ns"),
            ]
            if any(
                isinstance(value, bool) or not isinstance(value, int) or value <= 0
                for value in integer_metrics
            ):
                raise RuntimeError(
                    f"benchmark baseline case {case} has a non-positive sample metric"
                )
            qps = sample.get("qps")
            if (
                isinstance(qps, bool)
                or not isinstance(qps, (int, float))
                or qps <= 0
            ):
                raise RuntimeError(
                    f"benchmark baseline case {case} has invalid sample QPS"
                )
            if sample["p50_ns"] > sample["p99_ns"]:
                raise RuntimeError(
                    f"benchmark baseline case {case} has p50 greater than p99"
                )
            calculated_qps = (
                spec["iterations"] * 1_000_000_000 / sample["duration_ns"]
            )
            if not math.isclose(qps, calculated_qps, rel_tol=1e-6, abs_tol=0.001):
                raise RuntimeError(
                    f"benchmark baseline case {case} has inconsistent sample QPS"
                )
        median_qps = statistics.median(sample["qps"] for sample in samples)
        median_p50 = int(statistics.median(sample["p50_ns"] for sample in samples))
        median_p99 = int(statistics.median(sample["p99_ns"] for sample in samples))
        if not math.isclose(record.get("median_qps", -1), median_qps, rel_tol=1e-12):
            raise RuntimeError(f"benchmark baseline case {case} has invalid median_qps")
        if record.get("median_p50_ns") != median_p50:
            raise RuntimeError(f"benchmark baseline case {case} has invalid median_p50_ns")
        if record.get("median_p99_ns") != median_p99:
            raise RuntimeError(f"benchmark baseline case {case} has invalid median_p99_ns")


def load_baseline(path: pathlib.Path) -> dict:
    try:
        baseline = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"cannot read benchmark baseline {path}: {error}") from error
    if not isinstance(baseline, dict):
        raise RuntimeError("benchmark baseline root must be an object")
    validate_baseline(baseline)
    return baseline


def compare(summary: dict, baseline: dict) -> dict:
    validate_baseline(baseline)
    if summary["working_tree_dirty"]:
        raise RuntimeError("benchmark comparison requires a clean working tree")
    if summary["host"] != baseline["host"]:
        raise RuntimeError(
            f"benchmark host mismatch: current {summary['host']!r}, "
            f"baseline {baseline['host']!r}"
        )
    if summary["rust"] != baseline["rust"]:
        raise RuntimeError("benchmark Rust toolchain or rustc metadata does not match baseline")
    if summary["parameters"] != baseline["parameters"]:
        raise RuntimeError("benchmark parameters do not match baseline")
    if summary["runs_collected"] != baseline["required_runs"]:
        raise RuntimeError(
            f"comparison requires exactly {baseline['required_runs']} runs; "
            f"{summary['runs_collected']} were collected"
        )

    threshold = baseline["threshold_percent"]
    regressions = []
    comparisons = {}
    for case, reference in baseline["cases"].items():
        current = summary["cases"][case]
        case_comparison = {
            "blocking": reference["blocking"],
            "qps": {
                "current": current["median_qps"],
                "baseline": reference["median_qps"],
                "enforced": False,
            },
        }
        for metric in ("p50_ns", "p99_ns"):
            current_value = current[f"median_{metric}"]
            reference_value = reference[f"median_{metric}"]
            change = (current_value / reference_value - 1.0) * 100.0
            enforced = reference["blocking"]
            case_comparison[metric] = {
                "current": current_value,
                "baseline": reference_value,
                "change_percent": change,
                "enforced": enforced,
            }
            if enforced and current_value * 100 > reference_value * (100 + threshold):
                regressions.append(
                    f"{case} {metric.removesuffix('_ns')} {current_value}ns vs "
                    f"{reference_value}ns ({change:+.2f}%, limit +{threshold}%)"
                )
        comparisons[case] = case_comparison
    if regressions:
        raise RuntimeError("benchmark regression: " + "; ".join(regressions))
    return {
        "baseline_source_commit": baseline["source_commit"],
        "threshold_percent": threshold,
        "passed": True,
        "cases": comparisons,
    }


def detect_cpu(repo_root: pathlib.Path) -> str:
    commands = []
    if platform.system() == "Darwin":
        commands.append(["sysctl", "-n", "machdep.cpu.brand_string"])
    elif platform.system() == "Linux":
        commands.append(["lscpu"])
    for command in commands:
        completed = run_command(command, repo_root)
        if completed.returncode == 0 and completed.stdout.strip():
            output = completed.stdout.strip()
            if command[0] == "lscpu":
                for line in output.splitlines():
                    if line.startswith("Model name:"):
                        return line.partition(":")[2].strip()
            else:
                return output
    return platform.processor().strip() or platform.machine().strip()


def collect_metadata(
    repo_root: pathlib.Path, host_id: str, toolchain: str
) -> dict:
    revision = run_command(["git", "rev-parse", "HEAD"], repo_root)
    worktree = run_command(["git", "status", "--porcelain"], repo_root)
    rustc = run_command(["rustc", f"+{toolchain}", "-Vv"], repo_root)
    if revision.returncode != 0 or worktree.returncode != 0 or rustc.returncode != 0:
        raise RuntimeError("failed to collect benchmark revision, worktree, or rustc metadata")
    source_commit = revision.stdout.strip()
    if not COMMIT_PATTERN.fullmatch(source_commit):
        raise RuntimeError("benchmark source revision is not a full Git object id")
    cpu = detect_cpu(repo_root)
    operating_system = platform.platform()
    if not cpu or not operating_system:
        raise RuntimeError("failed to collect benchmark CPU or operating system metadata")
    return {
        "source_commit": source_commit,
        "working_tree_dirty": bool(worktree.stdout.strip()),
        "host": {"id": host_id, "cpu": cpu, "os": operating_system},
        "rust": {"toolchain": toolchain, "rustc": rustc.stdout.strip()},
    }


def ensure_stable_metadata(initial: dict, final: dict, *, require_clean: bool) -> None:
    if initial["source_commit"] != final["source_commit"]:
        raise RuntimeError("benchmark source revision changed while samples were running")
    if initial["host"] != final["host"] or initial["rust"] != final["rust"]:
        raise RuntimeError("benchmark host or Rust metadata changed while samples were running")
    if require_clean and (
        initial["working_tree_dirty"] or final["working_tree_dirty"]
    ):
        raise RuntimeError("benchmark release evidence requires a clean working tree")
    if not initial["working_tree_dirty"] and final["working_tree_dirty"]:
        raise RuntimeError("benchmark working tree became dirty while samples were running")


def build_summary(run_samples: list[dict], metadata: dict, command: dict) -> dict:
    parameters, cases = aggregate_samples(run_samples)
    return {
        "schema_version": SCHEMA_VERSION,
        "benchmark_suite": BENCHMARK_SUITE,
        "recorded_at": datetime.now(timezone.utc).isoformat(),
        **metadata,
        "runs_collected": len(run_samples),
        "parameters": parameters,
        "command": command,
        "cases": cases,
    }


def baseline_from_summary(summary: dict, threshold: int) -> dict:
    if summary["working_tree_dirty"]:
        raise RuntimeError("baseline generation requires a clean working tree")
    if summary["runs_collected"] != REQUIRED_BASELINE_RUNS:
        raise RuntimeError("baseline generation requires exactly five runs")
    baseline = {
        "schema_version": SCHEMA_VERSION,
        "status": "ready",
        "benchmark_suite": BENCHMARK_SUITE,
        "source_commit": summary["source_commit"],
        "recorded_at": summary["recorded_at"],
        "host": copy.deepcopy(summary["host"]),
        "rust": copy.deepcopy(summary["rust"]),
        "parameters": copy.deepcopy(summary["parameters"]),
        "required_runs": REQUIRED_BASELINE_RUNS,
        "threshold_percent": threshold,
        "cases": copy.deepcopy(summary["cases"]),
    }
    validate_baseline(baseline)
    return baseline


def verify_baseline_commit(repo_root: pathlib.Path, source_commit: str) -> None:
    completed = run_command(
        ["git", "cat-file", "-e", f"{source_commit}^{{commit}}"], repo_root
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"baseline source commit {source_commit} is not present in this checkout"
        )
    completed = run_command(
        ["git", "merge-base", "--is-ancestor", source_commit, "HEAD"], repo_root
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"baseline source commit {source_commit} is not an ancestor of HEAD"
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=pathlib.Path)
    parser.add_argument("--output-dir", type=pathlib.Path, required=True)
    parser.add_argument("--runs", type=positive_integer, default=REQUIRED_BASELINE_RUNS)
    parser.add_argument("--toolchain", default="1.97.0")
    parser.add_argument("--host-id", type=validate_host_id, required=True)
    parser.add_argument("--baseline", type=pathlib.Path)
    parser.add_argument("--write-baseline", type=pathlib.Path)
    parser.add_argument(
        "--threshold-percent",
        type=threshold_percent,
        default=DEFAULT_THRESHOLD_PERCENT,
    )
    parser.add_argument(
        "--warmup-iterations", type=positive_integer, default=500
    )
    parser.add_argument(
        "--small-iterations", type=percentile_iterations, default=10_000
    )
    parser.add_argument(
        "--large-iterations", type=percentile_iterations, default=1_000
    )
    arguments = parser.parse_args()
    if arguments.baseline is not None and arguments.write_baseline is not None:
        parser.error("--baseline and --write-baseline are mutually exclusive")
    if (
        arguments.baseline is not None or arguments.write_baseline is not None
    ) and arguments.runs != REQUIRED_BASELINE_RUNS:
        parser.error("baseline comparison and generation require exactly five runs")

    script_root = pathlib.Path(__file__).resolve().parents[2]
    repo_root = (arguments.repo_root or script_root).resolve()
    output_dir = arguments.output_dir.resolve()
    prepare_output_directory(output_dir)

    comparison_baseline = None
    if arguments.baseline is not None:
        comparison_baseline = load_baseline(arguments.baseline.resolve())
        verify_baseline_commit(repo_root, comparison_baseline["source_commit"])
    metadata = collect_metadata(repo_root, arguments.host_id, arguments.toolchain)
    if arguments.write_baseline is not None and metadata["working_tree_dirty"]:
        raise RuntimeError("baseline generation requires a clean working tree")
    if comparison_baseline is not None and metadata["working_tree_dirty"]:
        raise RuntimeError("benchmark comparison requires a clean working tree")

    command = [
        "cargo",
        f"+{arguments.toolchain}",
        "bench",
        "--locked",
        "--offline",
        "--package",
        "fusen-rs",
        "--bench",
        "invocation",
    ]
    benchmark_environment = {
        "FUSEN_BENCH_WARMUP_ITERATIONS": str(arguments.warmup_iterations),
        "FUSEN_BENCH_SMALL_ITERATIONS": str(arguments.small_iterations),
        "FUSEN_BENCH_LARGE_ITERATIONS": str(arguments.large_iterations),
    }
    command_environment = os.environ.copy()
    command_environment.update(benchmark_environment)

    run_samples = []
    for index in range(1, arguments.runs + 1):
        completed = run_command(command, repo_root, env=command_environment)
        combined = completed.stderr + completed.stdout
        (output_dir / f"run-{index:02}.log").write_text(combined, encoding="utf-8")
        print(combined, end="")
        if completed.returncode != 0:
            raise RuntimeError(
                f"benchmark run {index} failed with exit code {completed.returncode}"
            )
        run_samples.append(parse_sample(combined, index))

    final_metadata = collect_metadata(repo_root, arguments.host_id, arguments.toolchain)
    ensure_stable_metadata(
        metadata,
        final_metadata,
        require_clean=(
            comparison_baseline is not None or arguments.write_baseline is not None
        ),
    )
    metadata = final_metadata
    summary = build_summary(
        run_samples,
        metadata,
        {"argv": command, "environment": benchmark_environment},
    )
    summary_path = output_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    if comparison_baseline is not None:
        try:
            summary["comparison"] = compare(summary, comparison_baseline)
        except RuntimeError as error:
            summary["comparison"] = {
                "baseline_source_commit": comparison_baseline["source_commit"],
                "passed": False,
                "error": str(error),
            }
            summary_path.write_text(
                json.dumps(summary, indent=2) + "\n", encoding="utf-8"
            )
            raise
        summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    if arguments.write_baseline is not None:
        baseline = baseline_from_summary(summary, arguments.threshold_percent)
        baseline_path = arguments.write_baseline.resolve()
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        baseline_path.write_text(json.dumps(baseline, indent=2) + "\n", encoding="utf-8")
        print(f"wrote benchmark baseline to {baseline_path}")

    blocking = [case for case in summary["cases"].values() if case["blocking"]]
    print(
        "benchmark matrix medians: "
        + "; ".join(
            f"{case['protocol']}/c{case['concurrency']}/{case['payload']} "
            f"p50={case['median_p50_ns']}ns p99={case['median_p99_ns']}ns "
            f"qps={case['median_qps']:.3f}"
            for case in blocking
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError) as error:
        print(f"benchmark gate failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
