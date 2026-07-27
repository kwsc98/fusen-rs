#!/usr/bin/env python3
"""Enforce the approved client TLS and backend boundaries in resolved Cargo graphs."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys
from collections import deque


CORE_FORBIDDEN = {
    "aws-lc-rs",
    "aws-lc-sys",
    "fusen-nacos",
    "opentelemetry",
    "opentelemetry-otlp",
    "opentelemetry_sdk",
    "tracing-opentelemetry",
    "tracing-subscriber",
}

SUPPORTED_CLIENT_TARGETS = (
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)

APPROVED_CLIENT_TLS = {
    "hyper-rustls",
    "rustls",
    "rustls-pki-types",
    "rustls-webpki",
    "tokio-rustls",
    "webpki-roots",
}

REQUIRED_CLIENT_STACK = APPROVED_CLIENT_TLS | {"ring"}

EXPECTED_TLS_FEATURES = {
    "hyper-rustls": {
        "http1",
        "http2",
        "ring",
        "tls12",
        "webpki-roots",
        "webpki-tokio",
    },
    "rustls": {"ring", "std", "tls12"},
    "tokio-rustls": {"ring", "tls12"},
}

FORBIDDEN_CRYPTO_PACKAGES = {
    "aws-lc-rs",
    "native-tls",
    "openssl",
    "rustls-native-certs",
    "rustls-platform-verifier",
    "rustls-platform-verifier-android",
    "tokio-native-tls",
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
            "webpki-root-certs",
            "webpki-roots",
        }
        or name == "openssl"
        or name.startswith("openssl-")
        or name == "rustls"
        or name.startswith("rustls-")
        or name.endswith("-rustls")
    )


def is_forbidden_crypto_package(name: str) -> bool:
    return (
        name in FORBIDDEN_CRYPTO_PACKAGES
        or name.startswith("aws-lc-")
        or name.startswith("openssl-")
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


def resolved_features(document: dict) -> dict[str, set[str]]:
    packages = {package["id"]: package["name"] for package in document["packages"]}
    features: dict[str, set[str]] = {}
    for node in document["resolve"]["nodes"]:
        name = packages.get(node["id"])
        if name is not None:
            features.setdefault(name, set()).update(node["features"])
    return features


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
    targets = tuple(dict.fromkeys((host, *SUPPORTED_CLIENT_TARGETS)))
    root_documents = {
        target: metadata(repo_root, arguments.toolchain, target, manifests[0])
        for target in targets
    }
    documents = [root_documents[host]] + [
        metadata(repo_root, arguments.toolchain, host, manifest)
        for manifest in manifests[1:]
    ]

    failures: list[str] = []
    for target, document in root_documents.items():
        root_names = resolved_names(document)
        unexpected_root_tls = sorted(
            name
            for name in root_names
            if is_tls_package(name) and name not in APPROVED_CLIENT_TLS
        )
        if unexpected_root_tls:
            failures.append(
                f"root graph for {target} contains unapproved TLS dependencies: "
                + ", ".join(unexpected_root_tls)
            )
        forbidden_crypto = sorted(filter(is_forbidden_crypto_package, root_names))
        if forbidden_crypto:
            failures.append(
                f"root graph for {target} contains forbidden crypto backends: "
                + ", ".join(forbidden_crypto)
            )
        actual_features = resolved_features(document)
        for package, expected in EXPECTED_TLS_FEATURES.items():
            actual = actual_features.get(package, set())
            if actual != expected:
                failures.append(
                    f"{package} features for {target} are {sorted(actual)}, "
                    f"expected {sorted(expected)}"
                )

    for manifest, document in zip(manifests[1:], documents[1:]):
        forbidden_dependencies = sorted(
            name
            for name in resolved_names(document)
            if is_tls_package(name) or is_forbidden_crypto_package(name)
        )
        if forbidden_dependencies:
            failures.append(
                f"non-runtime graph contains TLS or forbidden crypto dependencies for "
                f"{manifest}: " + ", ".join(forbidden_dependencies)
            )

    lockfiles = [
        repo_root / "Cargo.lock",
        repo_root / "fuzz-support" / "Cargo.lock",
        repo_root / "fuzz" / "Cargo.lock",
    ]
    root_lock_tls = {
        name for name in lockfile_package_names(lockfiles[0]) if is_tls_package(name)
    }
    unexpected_lock_tls = sorted(root_lock_tls - APPROVED_CLIENT_TLS)
    if unexpected_lock_tls:
        failures.append(
            "root lockfile contains unapproved TLS dependencies: "
            + ", ".join(unexpected_lock_tls)
        )
    forbidden_lock_crypto = sorted(
        filter(is_forbidden_crypto_package, lockfile_package_names(lockfiles[0]))
    )
    if forbidden_lock_crypto:
        failures.append(
            "root lockfile contains forbidden crypto backends: "
            + ", ".join(forbidden_lock_crypto)
        )

    for lockfile in lockfiles[1:]:
        forbidden_dependencies = sorted(
            name
            for name in lockfile_package_names(lockfile)
            if is_tls_package(name) or is_forbidden_crypto_package(name)
        )
        if forbidden_dependencies:
            failures.append(
                f"TLS or forbidden crypto dependencies retained in non-runtime "
                f"{lockfile}: " + ", ".join(forbidden_dependencies)
            )

    for target, document in root_documents.items():
        core_names, _ = core_closure(document)
        forbidden_core = sorted(name for name in core_names if name in CORE_FORBIDDEN)
        if forbidden_core:
            failures.append(
                f"fusen-rs dependency closure for {target} contains forbidden backends: "
                + ", ".join(forbidden_core)
            )
        missing_client_stack = sorted(REQUIRED_CLIENT_STACK - core_names)
        if missing_client_stack:
            failures.append(
                f"fusen-rs dependency closure for {target} is missing required client "
                "TLS packages: " + ", ".join(missing_client_stack)
            )

    if failures:
        for failure in failures:
            print(f"dependency policy violation: {failure}", file=sys.stderr)
        return 1

    print(
        "dependency policy passed: fusen-rs uses Ring with bundled WebPKI roots, "
        "native roots/TLS, OpenSSL, platform verifiers, and AWS-LC are absent, "
        "and fuzz graphs remain TLS-free"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(f"dependency policy check failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
