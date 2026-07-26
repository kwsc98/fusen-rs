#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rust_toolchain="${RUST_TOOLCHAIN:-1.97.0}"

python3 "$repo_root/.github/scripts/check-dependency-policy.py" \
    --repo-root "$repo_root" \
    --toolchain "$rust_toolchain"
