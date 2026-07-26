#!/usr/bin/env python3
"""Enforce plaintext and backend-neutral boundaries from resolved Cargo graphs."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys
from collections import deque


CORE_FORBIDDEN = {
    "fusen-nacos",
    "opentelemetry",
    "opentelemetry-otlp",
    "opentelemetry_sdk",
    "tracing-opentelemetry",
    "tracing-subscriber",
}


def is_tls_package(name: str) -> bool:
    return (
        name in {
            "hyper-tls",
            "native-tls",
            "tokio-native-tls",
            "tokio-rustls",
            "hyper-rustls",
            "webpki",
            "webpki-roots",
        }
        or name == "openssl"
        or name.startswith("openssl-")
        or name == "rustls"
        or name.startswith("rustls-")
        or name.endswith("-rustls")
    )


def command_output(command: list[str], cwd: pathlib.Path) -> str:
    completed = subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"command failed ({completed.returncode}): {' '.join(command)}")
    return completed.stdout


def host_triple(repo_root: pathlib.Path, toolchain: str) -> str:
    version = command_output(["rustc", f"+{toolchain}", "-vV"], repo_root)
    for line in version.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise RuntimeError("rustc -vV did not report a host triple")


def metadata(
    repo_root: pathlib.Path,
    toolchain: str,
    host: str,
    manifest: pathlib.Path,
) -> dict:
    raw = command_output(
        [
            "cargo",
            f"+{toolchain}",
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--filter-platform",
            host,
            "--manifest-path",
            str(manifest),
        ],
        repo_root,
    )
    return json.loads(raw)


def resolved_names(document: dict) -> set[str]:
    packages = {package["id"]: package["name"] for package in document["packages"]}
    return {
        packages[node["id"]]
        for node in document["resolve"]["nodes"]
        if node["id"] in packages
    }


def core_closure(document: dict) -> tuple[set[str], dict[str, str | None]]:
    packages = {package["id"]: package["name"] for package in document["packages"]}
    workspace_members = set(document["workspace_members"])
    roots = [
        package_id
        for package_id in workspace_members
        if packages.get(package_id) == "fusen-rs"
    ]
    if len(roots) != 1:
        raise RuntimeError(f"expected one fusen-rs workspace package, found {len(roots)}")

    edges: dict[str, list[str]] = {}
    for node in document["resolve"]["nodes"]:
        edges[node["id"]] = [
            dependency["pkg"]
            for dependency in node["deps"]
            if any(
                kind["kind"] in (None, "build")
                for kind in dependency["dep_kinds"]
            )
        ]

    root = roots[0]
    queue = deque([root])
    seen: set[str] = set()
    parent: dict[str, str | None] = {root: None}
    while queue:
        package_id = queue.popleft()
        if package_id in seen:
            continue
        seen.add(package_id)
        for dependency in edges.get(package_id, []):
            if dependency not in parent:
                parent[dependency] = package_id
            queue.append(dependency)
    return {packages[package_id] for package_id in seen}, parent


def lockfile_package_names(lockfile: pathlib.Path) -> set[str]:
    if not lockfile.exists():
        raise RuntimeError(f"missing lockfile: {lockfile}")
    return set(
        re.findall(
            r'^name = "([^"]+)"$',
            lockfile.read_text(encoding="utf-8"),
            flags=re.MULTILINE,
        )
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=pathlib.Path, required=True)
    parser.add_argument("--toolchain", default="1.97.0")
    arguments = parser.parse_args()

    repo_root = arguments.repo_root.resolve()
    host = host_triple(repo_root, arguments.toolchain)
    manifests = [
        repo_root / "Cargo.toml",
        repo_root / "fuzz-support" / "Cargo.toml",
        repo_root / "fuzz" / "Cargo.toml",
    ]
    documents = [
        metadata(repo_root, arguments.toolchain, host, manifest)
        for manifest in manifests
    ]

    failures: list[str] = []
    for manifest, document in zip(manifests, documents):
        forbidden = sorted(filter(is_tls_package, resolved_names(document)))
        if forbidden:
            failures.append(
                f"resolved TLS dependencies for {manifest}: {', '.join(forbidden)}"
            )

    lockfiles = [
        repo_root / "Cargo.lock",
        repo_root / "fuzz-support" / "Cargo.lock",
        repo_root / "fuzz" / "Cargo.lock",
    ]
    for lockfile in lockfiles:
        forbidden = sorted(filter(is_tls_package, lockfile_package_names(lockfile)))
        if forbidden:
            failures.append(
                f"TLS dependencies retained in {lockfile}: {', '.join(forbidden)}"
            )

    core_names, _ = core_closure(documents[0])
    forbidden_core = sorted(
        name for name in core_names if name in CORE_FORBIDDEN or is_tls_package(name)
    )
    if forbidden_core:
        failures.append(
            "fusen-rs resolved dependency closure contains forbidden backends: "
            + ", ".join(forbidden_core)
        )

    if failures:
        for failure in failures:
            print(f"dependency policy violation: {failure}", file=sys.stderr)
        return 1

    print(
        "dependency policy passed: resolved root/fuzz graphs are TLS-free and "
        "fusen-rs is backend-neutral"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(f"dependency policy check failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
