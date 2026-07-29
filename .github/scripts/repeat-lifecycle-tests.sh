#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
repeat_count="${1:-${LIFECYCLE_REPEAT_COUNT:-20}}"
rust_toolchain="${RUST_TOOLCHAIN:-1.97.0}"

if ! [[ "$repeat_count" =~ ^[1-9][0-9]*$ ]]; then
    echo "repeat count must be a positive integer" >&2
    exit 2
fi

cd "$repo_root"

require_test() {
    local listing="$1"
    local test_name="$2"
    local suite="$3"

    if ! grep -Fqx "$test_name: test" <<<"$listing"; then
        echo "required lifecycle test is missing from $suite: $test_name" >&2
        exit 1
    fi
}

runtime_tests="$(cargo "+$rust_toolchain" test --locked --offline \
    --package fusen-rs --test runtime_e2e -- --list)"
server_tests="$(cargo "+$rust_toolchain" test --locked --offline \
    --package fusen-rs --test server_registry -- --list)"
startup_tests="$(cargo "+$rust_toolchain" test --locked --offline \
    --package fusen-rs --test server_startup -- --list)"
register_tests="$(cargo "+$rust_toolchain" test --locked --offline \
    --package fusen-register --lib -- --list)"
config_tests="$(cargo "+$rust_toolchain" test --locked --offline \
    --package fusen-config --lib -- --list)"
nacos_tests="$(cargo "+$rust_toolchain" test --locked --offline \
    --package fusen-nacos --lib -- --list)"

require_test "$runtime_tests" \
    "graceful_shutdown_aborts_a_permanently_pending_stream_at_deadline" \
    "fusen-rs/runtime_e2e"
require_test "$server_tests" \
    "shutdown_closes_listener_before_registry_and_connection_drain_in_parallel" \
    "fusen-rs/server_registry"
require_test "$startup_tests" \
    "aborting_start_compensates_a_late_registration_success_exactly_once" \
    "fusen-rs/server_startup"
require_test "$register_tests" \
    "tests::cancelling_last_activation_waiter_requests_late_success_cleanup" \
    "fusen-register/lib"
require_test "$register_tests" \
    "tests::cancelling_one_of_two_activation_waiters_keeps_the_shared_activation_alive" \
    "fusen-register/lib"
require_test "$config_tests" \
    "hot::tests::cancelling_activation_waiter_does_not_cancel_provider_work" \
    "fusen-config/lib"
require_test "$config_tests" \
    "hot::tests::dropping_the_last_waiter_compensates_a_late_success" \
    "fusen-config/lib"
require_test "$nacos_tests" \
    "config::tests::cancelled_activation_waiter_still_removes_the_listener_once" \
    "fusen-nacos/lib"
require_test "$nacos_tests" \
    "register::tests::cancelled_nacos_activation_waiter_is_compensated_once" \
    "fusen-nacos/lib"

for ((iteration = 1; iteration <= repeat_count; iteration++)); do
    echo "lifecycle repetition $iteration/$repeat_count"
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-rs --test runtime_e2e -- --test-threads=1
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-rs --test server_registry -- --test-threads=1
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-rs --test server_startup -- --test-threads=1
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-register --lib \
        tests::cancelling_last_activation_waiter_requests_late_success_cleanup -- \
        --exact --test-threads=1
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-register --lib \
        tests::cancelling_one_of_two_activation_waiters_keeps_the_shared_activation_alive -- \
        --exact --test-threads=1
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-config --lib \
        hot::tests::cancelling_activation_waiter_does_not_cancel_provider_work -- \
        --exact --test-threads=1
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-config --lib \
        hot::tests::dropping_the_last_waiter_compensates_a_late_success -- \
        --exact --test-threads=1
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-nacos --lib \
        config::tests::cancelled_activation_waiter_still_removes_the_listener_once -- \
        --exact --test-threads=1
    cargo "+$rust_toolchain" test --locked --offline \
        --package fusen-nacos --lib \
        register::tests::cancelled_nacos_activation_waiter_is_compensated_once -- \
        --exact --test-threads=1
done
