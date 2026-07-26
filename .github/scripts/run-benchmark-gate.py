#!/usr/bin/env python3
"""Run, archive, and optionally compare the direct invocation benchmark."""

from __future__ import annotations

import argparse
import json
import pathlib
import platform
import re
import statistics
import subprocess
import sys
from datetime import datetime, timezone


BENCHMARK = "direct/fusen-v1"
RESULT_PATTERN = re.compile(
    r"^direct/fusen-v1 iterations=(\d+) mean_ns=(\d+) "
    r"p50_ns=(\d+) p99_ns=(\d+)$",
    re.MULTILINE,
)
HOST_ID_PATTERN = re.compile(r"^[A-Za-z0-9._-]{1,64}$")


def run_command(command: list[str], cwd: pathlib.Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def parse_sample(output: str, run_number: int) -> dict[str, int]:
    matches = RESULT_PATTERN.findall(output)
    if len(matches) != 1:
        raise RuntimeError(
            f"benchmark run {run_number} produced {len(matches)} machine-readable results"
        )
    iterations, mean_ns, p50_ns, p99_ns = map(int, matches[0])
    if iterations <= 0 or min(mean_ns, p50_ns, p99_ns) <= 0:
        raise RuntimeError(f"benchmark run {run_number} reported a non-positive metric")
    if p50_ns > p99_ns:
        raise RuntimeError(f"benchmark run {run_number} reported p50 greater than p99")
    return {
        "iterations": iterations,
        "mean_ns": mean_ns,
        "p50_ns": p50_ns,
        "p99_ns": p99_ns,
    }


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("value must be positive")
    return parsed


def validate_host_id(value: str) -> str:
    if not HOST_ID_PATTERN.fullmatch(value):
        raise argparse.ArgumentTypeError(
            "host id must be 1-64 ASCII letters, digits, dot, underscore, or hyphen"
        )
    return value


def load_baseline(path: pathlib.Path) -> dict:
    try:
        baseline = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"cannot read benchmark baseline {path}: {error}") from error
    required = {
        "schema_version",
        "benchmark",
        "host_id",
        "rust_toolchain",
        "required_runs",
        "threshold_percent",
        "median_p50_ns",
        "median_p99_ns",
    }
    missing = sorted(required.difference(baseline))
    if missing:
        raise RuntimeError(f"benchmark baseline is missing: {', '.join(missing)}")
    if baseline["schema_version"] != 1 or baseline["benchmark"] != BENCHMARK:
        raise RuntimeError("benchmark baseline has an unsupported schema or benchmark name")
    for field in ("required_runs", "median_p50_ns", "median_p99_ns"):
        if not isinstance(baseline[field], int) or baseline[field] <= 0:
            raise RuntimeError(f"benchmark baseline field {field} must be a positive integer")
    threshold = baseline["threshold_percent"]
    if not isinstance(threshold, int) or not 0 < threshold <= 10:
        raise RuntimeError("benchmark threshold_percent must be an integer in 1..=10")
    return baseline


def compare(summary: dict, baseline: dict, toolchain: str) -> None:
    if summary["host_id"] != baseline["host_id"]:
        raise RuntimeError(
            "benchmark host mismatch: current "
            f"{summary['host_id']!r}, baseline {baseline['host_id']!r}"
        )
    if toolchain != baseline["rust_toolchain"]:
        raise RuntimeError(
            f"toolchain mismatch: current {toolchain}, baseline {baseline['rust_toolchain']}"
        )
    if len(summary["samples"]) < baseline["required_runs"]:
        raise RuntimeError(
            f"comparison requires {baseline['required_runs']} runs; "
            f"only {len(summary['samples'])} were collected"
        )

    threshold = baseline["threshold_percent"]
    regressions = []
    for metric in ("p50", "p99"):
        current = summary[f"median_{metric}_ns"]
        reference = baseline[f"median_{metric}_ns"]
        allowed = reference * (100 + threshold)
        if current * 100 > allowed:
            change = (current / reference - 1.0) * 100.0
            regressions.append(
                f"{metric} {current}ns vs {reference}ns ({change:+.2f}%, limit +{threshold}%)"
            )
    if regressions:
        raise RuntimeError("benchmark regression: " + "; ".join(regressions))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=pathlib.Path)
    parser.add_argument("--output-dir", type=pathlib.Path, required=True)
    parser.add_argument("--runs", type=positive_integer, default=5)
    parser.add_argument("--toolchain", default="1.97.0")
    parser.add_argument("--host-id", type=validate_host_id, required=True)
    parser.add_argument("--baseline", type=pathlib.Path)
    arguments = parser.parse_args()

    script_root = pathlib.Path(__file__).resolve().parents[2]
    repo_root = (arguments.repo_root or script_root).resolve()
    output_dir = arguments.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    samples = []
    for index in range(1, arguments.runs + 1):
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
        completed = run_command(command, repo_root)
        combined = completed.stderr + completed.stdout
        (output_dir / f"run-{index:02}.log").write_text(combined, encoding="utf-8")
        print(combined, end="")
        if completed.returncode != 0:
            raise RuntimeError(
                f"benchmark run {index} failed with exit code {completed.returncode}"
            )
        samples.append(parse_sample(combined, index))

    p50_values = [sample["p50_ns"] for sample in samples]
    p99_values = [sample["p99_ns"] for sample in samples]
    mean_values = [sample["mean_ns"] for sample in samples]
    revision = run_command(["git", "rev-parse", "HEAD"], repo_root)
    worktree = run_command(["git", "status", "--porcelain"], repo_root)
    rustc = run_command(["rustc", f"+{arguments.toolchain}", "-Vv"], repo_root)
    if revision.returncode != 0 or worktree.returncode != 0 or rustc.returncode != 0:
        raise RuntimeError("failed to collect benchmark revision, worktree, or rustc metadata")
    median_mean_ns = int(statistics.median(mean_values))
    summary = {
        "schema_version": 1,
        "benchmark": BENCHMARK,
        "host_id": arguments.host_id,
        "recorded_at": datetime.now(timezone.utc).isoformat(),
        "source_commit": revision.stdout.strip(),
        "working_tree_dirty": bool(worktree.stdout.strip()),
        "platform": platform.platform(),
        "rust_toolchain": arguments.toolchain,
        "rustc": rustc.stdout.strip(),
        "samples": samples,
        "median_mean_ns": median_mean_ns,
        "median_p50_ns": int(statistics.median(p50_values)),
        "median_p99_ns": int(statistics.median(p99_values)),
        "successful_qps": 1_000_000_000 / median_mean_ns,
    }
    summary_path = output_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    if arguments.baseline is not None:
        baseline = load_baseline(arguments.baseline.resolve())
        compare(summary, baseline, arguments.toolchain)
        summary["comparison"] = {
            "baseline": str(arguments.baseline),
            "threshold_percent": baseline["threshold_percent"],
            "passed": True,
        }
        summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    print(
        f"benchmark median: p50={summary['median_p50_ns']}ns "
        f"p99={summary['median_p99_ns']}ns"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError) as error:
        print(f"benchmark gate failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
