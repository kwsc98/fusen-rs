#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rust_toolchain="${RUST_TOOLCHAIN:-1.97.0}"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/fusen-package-consumer.XXXXXX")"
trap 'rm -rf -- "$work_dir"' EXIT

patches=(
    --config "patch.crates-io.fusen-config.path=\"$repo_root/fusen-config\""
    --config "patch.crates-io.fusen-contract.path=\"$repo_root/fusen-contract\""
    --config "patch.crates-io.fusen-nacos.path=\"$repo_root/fusen-nacos\""
    --config "patch.crates-io.fusen-observability.path=\"$repo_root/fusen-observability\""
    --config "patch.crates-io.fusen-procedural-macro.path=\"$repo_root/fusen-macro/procedural-macro\""
    --config "patch.crates-io.fusen-register.path=\"$repo_root/fusen-register\""
    --config "patch.crates-io.fusen-rs.path=\"$repo_root/fusen\""
)

packages=(
    fusen-contract
    fusen-register
    fusen-config
    fusen-observability
    fusen-procedural-macro
    fusen-nacos
    fusen-rs
)

for package in "${packages[@]}"; do
    cargo "+$rust_toolchain" package \
        --registry crates-io \
        --locked \
        --offline \
        --allow-dirty \
        --target-dir "$work_dir/target" \
        --package "$package" \
        "${patches[@]}"
done

mkdir -p "$work_dir/unpacked"
for archive in "$work_dir"/target/package/*.crate; do
    tar -xzf "$archive" -C "$work_dir/unpacked"
done

for package in "${packages[@]}"; do
    consumer_dir="$work_dir/consumers/$package"
    mkdir -p "$consumer_dir/src"
    cat >"$consumer_dir/Cargo.toml" <<EOF
[package]
name = "${package}-package-consumer"
version = "0.0.0"
edition = "2024"
rust-version = "1.97"
publish = false

[workspace]

[dependencies]
$package = { path = "$work_dir/unpacked/$package-0.9.0" }

[patch.crates-io]
fusen-config = { path = "$work_dir/unpacked/fusen-config-0.9.0" }
fusen-contract = { path = "$work_dir/unpacked/fusen-contract-0.9.0" }
fusen-nacos = { path = "$work_dir/unpacked/fusen-nacos-0.9.0" }
fusen-observability = { path = "$work_dir/unpacked/fusen-observability-0.9.0" }
fusen-procedural-macro = { path = "$work_dir/unpacked/fusen-procedural-macro-0.9.0" }
fusen-register = { path = "$work_dir/unpacked/fusen-register-0.9.0" }
EOF

    if [[ "$package" == "fusen-rs" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_rs::{ClientBuilder, ClientRuntime, RpcError, RpcRequest, RpcResponse, interface};

#[interface(name = "package-consumer")]
pub trait PackageConsumerApi {
    #[fusen_rs::method(idempotency = "safe")]
    async fn ping(&self, request: RpcRequest<()>) -> Result<RpcResponse<String>, RpcError>;
}

pub struct PackageConsumerHandler;

impl PackageConsumerApi for PackageConsumerHandler {
    async fn ping(&self, _request: RpcRequest<()>) -> Result<RpcResponse<String>, RpcError> {
        Ok(RpcResponse::new("pong".to_owned()))
    }
}

pub fn packaged_server() -> PackageConsumerApiServer<PackageConsumerHandler> {
    PackageConsumerApiServer::new(PackageConsumerHandler)
}

pub fn packaged_client(runtime: &ClientRuntime) -> ClientBuilder<PackageConsumerApiClient> {
    PackageConsumerApiClient::builder(runtime)
}

pub fn assert_client_implements_interface(client: &PackageConsumerApiClient) {
    fn accepts_interface(_: &impl PackageConsumerApi) {}
    accepts_interface(client);
}
EOF
    else
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
pub fn packaged_dependency_compiles() {}
EOF
    fi

    cargo "+$rust_toolchain" generate-lockfile \
        --manifest-path "$consumer_dir/Cargo.toml" \
        --offline
    cargo "+$rust_toolchain" check \
        --manifest-path "$consumer_dir/Cargo.toml" \
        --target-dir "$work_dir/consumer-target" \
        --locked \
        --offline
    echo "packaged $package archive compiled in its own external consumer"
done
