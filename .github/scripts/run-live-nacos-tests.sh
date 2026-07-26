#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rust_toolchain="${RUST_TOOLCHAIN:-1.97.0}"

cd "$repo_root"
listing="$(cargo "+$rust_toolchain" test --locked --offline \
    --package fusen-nacos --test live_nacos live_nacos_ -- \
    --ignored --list)"
test_count="$(grep -Ec ': test$' <<<"$listing" || true)"
if [[ "$test_count" -eq 0 ]]; then
    echo "Nacos release gate matched zero live_nacos_ tests" >&2
    exit 1
fi

echo "Nacos release gate discovered $test_count live test(s)"
cargo "+$rust_toolchain" test --locked --offline \
    --package fusen-nacos --test live_nacos live_nacos_ -- \
    --ignored --test-threads=1
