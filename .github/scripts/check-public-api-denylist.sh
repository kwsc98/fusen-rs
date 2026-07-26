#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rust_toolchain="${RUST_TOOLCHAIN:-1.97.0}"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/fusen-public-api.XXXXXX")"
trap 'rm -rf -- "$work_dir"' EXIT

cd "$repo_root"
CARGO_TARGET_DIR="$work_dir/target" \
    cargo "+$rust_toolchain" doc --locked --offline --workspace --all-features --no-deps
python3 "$repo_root/.github/scripts/check-public-api-denylist.py" \
    "$work_dir/target/doc" \
    --source-root "$repo_root"
