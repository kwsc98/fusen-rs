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

    if [[ "$package" == "fusen-contract" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_contract::{
    ContractError, EndpointCapabilities, HTTP_JSON_V1, HttpBindingId, HttpOperation,
    HttpParameter, HttpParameterCardinality, HttpParameterSource, HttpVersionPolicy,
    HttpVersionSet,
};

pub fn default_http_capabilities() -> Result<EndpointCapabilities, ContractError> {
    let binding = HttpBindingId::new(HTTP_JSON_V1)?;
    let capabilities = EndpointCapabilities::new(
        HttpVersionSet::ALL,
        [binding.clone()],
        true,
    )?;
    assert!(capabilities.supports_binding(&binding));
    assert_eq!(binding.as_str(), HTTP_JSON_V1);
    let _policy = HttpVersionPolicy::Auto;
    Ok(capabilities)
}

pub fn assert_http_operation_types(
    operation: HttpOperation,
    parameter: HttpParameter,
    source: HttpParameterSource,
    cardinality: HttpParameterCardinality,
) -> (HttpOperation, HttpParameter, HttpParameterSource, HttpParameterCardinality) {
    (operation, parameter, source, cardinality)
}
EOF
    elif [[ "$package" == "fusen-nacos" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_nacos::{NacosConvention, NacosRegistry};

pub fn use_spring_cloud_convention(registry: NacosRegistry) -> NacosRegistry {
    registry.with_convention(NacosConvention::SpringCloud)
}
EOF
    elif [[ "$package" == "fusen-rs" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_rs::{
    ClientBuilder, ClientRuntime, ClientRuntimeBuilder, EndpointCapabilities, Error, ErrorCategory,
    ConfigValidationError, ErrorConstructionError, ErrorDecoder, ErrorKind, ErrorOrigin,
    HTTP_JSON_V1, HttpBindingId,
    HttpOperation, HttpParameter, HttpParameterCardinality, HttpParameterSource,
    HttpVersionPolicy, HttpVersionSet, RequestEncoder, Response, ResponseDecoder, ServerConfig,
    interface,
};

#[interface(name = "package-consumer")]
pub trait PackageConsumerApi {
    #[fusen_rs::method(method = "GET", path = "/ping")]
    async fn ping(&self) -> Result<Response<String>, Error>;
}

pub struct PackageConsumerHandler;

impl PackageConsumerApi for PackageConsumerHandler {
    async fn ping(&self) -> Result<Response<String>, Error> {
        Ok(Response::new("pong".to_owned()))
    }
}

pub fn application_error_contract() -> Result<(ErrorKind, ErrorOrigin), ErrorConstructionError> {
    let error = Error::application(
        ErrorCategory::InvalidArgument,
        "invalid_ping",
        "ping input is invalid",
    )?;
    let _ = error.category().canonical_status();
    Ok((error.kind(), error.origin()))
}

pub fn packaged_server() -> PackageConsumerApiServer<PackageConsumerHandler> {
    PackageConsumerApiServer::new(PackageConsumerHandler)
}

pub fn packaged_client(runtime: &ClientRuntime) -> ClientBuilder<PackageConsumerApiClient> {
    PackageConsumerApiClient::builder(runtime)
        .binding(HttpBindingId::default())
        .http_version_policy(HttpVersionPolicy::Auto)
        .direct_capabilities(EndpointCapabilities::default())
}

pub fn assert_http_contract(
    operation: HttpOperation,
    parameter: HttpParameter,
    source: HttpParameterSource,
    cardinality: HttpParameterCardinality,
) -> (HttpOperation, HttpParameter, HttpParameterSource, HttpParameterCardinality) {
    assert_eq!(HttpBindingId::default().as_str(), HTTP_JSON_V1);
    assert_eq!(EndpointCapabilities::default().http_versions(), HttpVersionSet::HTTP_1_1);
    (operation, parameter, source, cardinality)
}

pub fn assert_client_codec_traits<E, R, D>(_encoder: &E, _response: &R, _error: &D)
where
    E: RequestEncoder,
    R: ResponseDecoder,
    D: ErrorDecoder,
{}

pub fn runtime_with_http_binding<E, R, D>(
    id: HttpBindingId,
    encoder: E,
    response: R,
    error: D,
) -> ClientRuntimeBuilder
where
    E: RequestEncoder,
    R: ResponseDecoder,
    D: ErrorDecoder,
{
    ClientRuntime::builder().http_binding(id, encoder, response, error)
}

pub fn server_capabilities() -> Result<ServerConfig, ConfigValidationError> {
    ServerConfig::builder()
        .capabilities(EndpointCapabilities::default())
        .build()
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
