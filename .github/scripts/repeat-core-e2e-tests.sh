#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
repeat_count="${1:-${CORE_E2E_REPEAT_COUNT:-100}}"
rust_toolchain="${RUST_TOOLCHAIN:-1.97.0}"

if ! [[ "$repeat_count" =~ ^[1-9][0-9]*$ ]]; then
    echo "repeat count must be a positive integer" >&2
    exit 2
fi

cd "$repo_root"
for ((iteration = 1; iteration <= repeat_count; iteration++)); do
    echo "core E2E repetition $iteration/$repeat_count"
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-rs \
        --test runtime_e2e \
        --test wire_v1_contract \
        -- --test-threads=1
done
