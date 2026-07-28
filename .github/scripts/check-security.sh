#!/usr/bin/env bash

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
deny_config="$repo_root/deny.toml"
failures=0

workspace_names=(root fuzz-support fuzz)
deny_checks=(advisories bans licenses sources)
workspace_manifests=(
    "$repo_root/Cargo.toml"
    "$repo_root/fuzz-support/Cargo.toml"
    "$repo_root/fuzz/Cargo.toml"
)
workspace_locks=(
    "$repo_root/Cargo.lock"
    "$repo_root/fuzz-support/Cargo.lock"
    "$repo_root/fuzz/Cargo.lock"
)

record_failure() {
    local workspace="$1"
    local check="$2"

    echo "security check failed: $workspace ($check)" >&2
    failures=$((failures + 1))
}

for index in "${!workspace_names[@]}"; do
    workspace="${workspace_names[$index]}"
    manifest="${workspace_manifests[$index]}"
    lockfile="${workspace_locks[$index]}"

    echo "::group::security metadata: $workspace"
    metadata_files_ready=true
    if [[ ! -s "$manifest" ]]; then
        echo "missing or empty workspace manifest: $manifest" >&2
        record_failure "$workspace" "manifest"
        metadata_files_ready=false
    fi
    if [[ ! -s "$lockfile" ]]; then
        echo "missing or empty workspace lockfile: $lockfile" >&2
        record_failure "$workspace" "lockfile"
        metadata_files_ready=false
    fi
    if [[ "$metadata_files_ready" == true ]]; then
        if ! cargo_metadata="$(cargo metadata \
            --manifest-path "$manifest" \
            --locked \
            --offline \
            --no-deps \
        --format-version 1)"; then
            record_failure "$workspace" "cargo metadata"
        elif ! python3 -c \
            'import json, sys; raise SystemExit(not json.load(sys.stdin)["packages"])' \
            <<<"$cargo_metadata"; then
            echo "workspace contains no packages: $manifest" >&2
            record_failure "$workspace" "empty workspace"
        fi
        unset cargo_metadata
    fi
    echo "::endgroup::"

    for deny_check in "${deny_checks[@]}"; do
        echo "::group::cargo deny $deny_check: $workspace"
        if ! cargo deny \
            --manifest-path "$manifest" \
            --config "$deny_config" \
            --locked \
            check "$deny_check"; then
            record_failure "$workspace" "cargo deny $deny_check"
        fi
        echo "::endgroup::"
    done

    echo "::group::cargo audit: $workspace"
    if ! cargo audit --file "$lockfile"; then
        record_failure "$workspace" "cargo audit"
    fi
    echo "::endgroup::"
done

if ((failures > 0)); then
    echo "$failures security check(s) failed" >&2
    exit 1
fi

echo "security checks passed for root, fuzz-support, and fuzz"
