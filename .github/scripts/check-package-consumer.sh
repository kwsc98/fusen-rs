#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rust_toolchain="${RUST_TOOLCHAIN:-1.97.0}"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/fusen-package-consumer.XXXXXX")"
trap 'rm -rf -- "$work_dir"' EXIT

caller_home="${HOME:?HOME must be set}"
cargo_home="$work_dir/cargo-home"
cargo_user_home="$work_dir/home"
rustup_home="${RUSTUP_HOME:-$caller_home/.rustup}"
# Cargo and every build process it launches receive only this explicit allowlist.
cargo_environment=(
    env
    -i
    "PATH=${PATH:?PATH must be set}"
    "HOME=$cargo_user_home"
    "CARGO_HOME=$cargo_home"
    "CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse"
    "RUSTUP_HOME=$rustup_home"
    "TMPDIR=${TMPDIR:-/tmp}"
)

append_cargo_environment() {
    if [[ -n "$2" ]]; then
        cargo_environment+=("$1=$2")
    fi
}

append_cargo_environment LANG "${LANG:-}"
append_cargo_environment LC_ALL "${LC_ALL:-}"

fetch_environment=("${cargo_environment[@]}")

# Only the online fetch may inherit network routing and trust-store settings.
append_fetch_environment() {
    if [[ -n "$2" ]]; then
        fetch_environment+=("$1=$2")
    fi
}

append_fetch_environment HTTP_PROXY "${HTTP_PROXY:-}"
append_fetch_environment HTTPS_PROXY "${HTTPS_PROXY:-}"
append_fetch_environment NO_PROXY "${NO_PROXY:-}"
append_fetch_environment ALL_PROXY "${ALL_PROXY:-}"
append_fetch_environment http_proxy "${http_proxy:-}"
append_fetch_environment https_proxy "${https_proxy:-}"
append_fetch_environment no_proxy "${no_proxy:-}"
append_fetch_environment all_proxy "${all_proxy:-}"
append_fetch_environment SSL_CERT_FILE "${SSL_CERT_FILE:-}"
append_fetch_environment SSL_CERT_DIR "${SSL_CERT_DIR:-}"

run_cargo() {
    "${cargo_environment[@]}" cargo "+$rust_toolchain" "$@"
}

fetch_cargo() {
    "${fetch_environment[@]}" cargo "+$rust_toolchain" "$@"
}

if [[ -n "$(git -C "$repo_root" status --porcelain=v1)" ]]; then
    echo "package archive verification requires a clean Git worktree" >&2
    exit 1
fi

mkdir -p "$cargo_home" "$cargo_user_home"

fetch_cargo fetch \
    --locked \
    --manifest-path "$repo_root/Cargo.toml"

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
    run_cargo package \
        --registry crates-io \
        --locked \
        --offline \
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
    optional_feature=""
    extra_dependency=""
    case "$package" in
        fusen-contract)
            optional_feature="derive"
            ;;
        fusen-config)
            optional_feature="yaml"
            ;;
        fusen-observability)
            optional_feature="otel"
            ;;
        fusen-procedural-macro)
            extra_dependency="fusen-contract = { path = \"$work_dir/unpacked/fusen-contract-0.9.0\" }"
            ;;
        fusen-nacos)
            optional_feature="yaml"
            extra_dependency="fusen-config = { path = \"$work_dir/unpacked/fusen-config-0.9.0\" }
fusen-register = { path = \"$work_dir/unpacked/fusen-register-0.9.0\" }"
            ;;
    esac

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
$extra_dependency

[patch.crates-io]
fusen-config = { path = "$work_dir/unpacked/fusen-config-0.9.0" }
fusen-contract = { path = "$work_dir/unpacked/fusen-contract-0.9.0" }
fusen-nacos = { path = "$work_dir/unpacked/fusen-nacos-0.9.0" }
fusen-observability = { path = "$work_dir/unpacked/fusen-observability-0.9.0" }
fusen-procedural-macro = { path = "$work_dir/unpacked/fusen-procedural-macro-0.9.0" }
fusen-register = { path = "$work_dir/unpacked/fusen-register-0.9.0" }
EOF

    if [[ -n "$optional_feature" ]]; then
        cat >>"$consumer_dir/Cargo.toml" <<EOF

[features]
archive-all-features = ["$package/$optional_feature"]
EOF
    fi

    if [[ "$package" == "fusen-contract" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_contract::{
    ContractError, EndpointCapabilities, HTTP_JSON_V1, HttpBindingId, HttpOperation,
    HttpParameter, HttpParameterCardinality, HttpParameterSource, HttpVersionPolicy,
    HttpVersionSet, SensitiveFields, SensitiveShape,
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

#[cfg(feature = "archive-all-features")]
#[derive(fusen_contract::SensitiveFields)]
pub struct PackagedSensitiveDto {
    #[sensitive(kind = "token")]
    pub token: String,
}

#[cfg(feature = "archive-all-features")]
pub fn derived_sensitive_shape() -> SensitiveShape {
    PackagedSensitiveDto::sensitive_shape()
}
EOF
    elif [[ "$package" == "fusen-register" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_register::{
    RegistrationHandle, RegistrationRequest, Registry, SubscriptionHandle,
    SubscriptionRequest,
    directory::{Directory, directory},
    error::RegistryError,
    provider,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub struct PackagedRegistry;

impl Registry for PackagedRegistry {
    fn prepare_registration(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        let _ = request.registration();
        Ok(provider::registration(
            async { Ok::<(), RegistryError>(()) },
            || async { Ok::<(), RegistryError>(()) },
        ))
    }

    fn prepare_subscription(
        &self,
        request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError> {
        let _ = request.selector();
        let (_, directory) = directory();
        Ok(provider::subscription(
            directory,
            async { Ok::<(), RegistryError>(()) },
            || async { Ok::<(), RegistryError>(()) },
        ))
    }
}

pub fn assert_registry_arc_forwarding(registry: Arc<dyn Registry>) {
    fn accepts_registry(_registry: impl Registry) {}
    accepts_registry(registry);
}

pub fn packaged_directory() -> Directory {
    directory().1
}
EOF
    elif [[ "$package" == "fusen-config" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_config::{
    ConfigDocument, ConfigError, ConfigFormat, ConfigHandle, ConfigKey, ConfigSource,
    provider,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub struct PackagedConfigSource;

impl ConfigSource for PackagedConfigSource {
    fn prepare(&self, _key: ConfigKey) -> Result<ConfigHandle, ConfigError> {
        Ok(provider::lifecycle(|_publisher| {
            (
                async {
                    Ok::<_, ConfigError>(ConfigDocument::new(
                        ConfigFormat::Toml,
                        "workers = 1",
                    ))
                },
                || async { Ok::<(), ConfigError>(()) },
            )
        }))
    }
}

pub fn prepare_arc_config_source(
    source: Arc<dyn ConfigSource>,
    key: ConfigKey,
) -> Result<ConfigHandle, ConfigError> {
    fn accepts_config_source(_source: impl ConfigSource) {}
    accepts_config_source(source.clone());
    source.prepare(key)
}

#[cfg(feature = "archive-all-features")]
pub fn parse_packaged_yaml() -> Result<String, ConfigError> {
    fusen_config::parse_yaml("package-consumer")
}
EOF
    elif [[ "$package" == "fusen-observability" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_observability::{MetricEvent, MetricsRecorder};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub struct PackagedMetricsRecorder;

impl MetricsRecorder for PackagedMetricsRecorder {
    fn record(&self, _event: &MetricEvent<'_>) {}
}

pub fn assert_metrics_recorder_arc_forwarding(recorder: Arc<dyn MetricsRecorder>) {
    fn accepts_metrics_recorder(_recorder: impl MetricsRecorder) {}
    accepts_metrics_recorder(recorder);
}

#[cfg(feature = "archive-all-features")]
pub fn assert_otel_adapter(
    recorder: &fusen_observability::otel::OpenTelemetryMetricsRecorder,
) {
    fn accepts_metrics_recorder(_recorder: impl MetricsRecorder) {}
    accepts_metrics_recorder((*recorder).clone());
}
EOF
    elif [[ "$package" == "fusen-procedural-macro" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_contract::{SensitiveFields as _, SensitiveShape};

#[derive(fusen_procedural_macro::SensitiveFields)]
pub struct PackagedMacroDto {
    #[sensitive(opaque)]
    pub value: String,
}

pub fn packaged_derive_expands() -> SensitiveShape {
    PackagedMacroDto::sensitive_shape()
}
EOF
    elif [[ "$package" == "fusen-nacos" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_config::ConfigSource;
use fusen_nacos::{
    NacosConfig, NacosConfigSource, NacosConfigValidationError, NacosConvention, NacosRegistry,
};
use fusen_register::Registry;
use std::sync::Arc;

pub fn use_spring_cloud_convention(registry: NacosRegistry) -> NacosRegistry {
    registry.with_convention(NacosConvention::SpringCloud)
}

pub fn validated_nacos_config() -> Result<NacosConfig, NacosConfigValidationError> {
    NacosConfig::builder().server_addr("127.0.0.1:8848").build()
}

pub fn packaged_adapters_are_object_safe(
    registry: NacosRegistry,
    config: NacosConfigSource,
) -> (Arc<dyn Registry>, Arc<dyn ConfigSource>) {
    (Arc::new(registry), Arc::new(config))
}

#[cfg(feature = "archive-all-features")]
pub fn parse_forwarded_yaml() -> Result<String, fusen_config::ConfigError> {
    fusen_config::parse_yaml("package-consumer")
}
EOF
    elif [[ "$package" == "fusen-rs" ]]; then
        cat >"$consumer_dir/src/lib.rs" <<'EOF'
use fusen_rs::{
    Body, BufferedResponse, ClientBuilder, ClientRuntime, ClientRuntimeBuilder,
    ConfigValidationError, Context, EncodedRequest, EndpointCapabilities, Error, ErrorCategory,
    ErrorConstructionError, ErrorDecoder, ErrorKind, ErrorOrigin, HTTP_JSON_V1, HttpBindingId,
    HttpOperation, HttpParameter, HttpParameterCardinality, HttpParameterSource, HttpVersionPolicy,
    HttpVersionSet, InstanceRouter, InstanceSnapshot, Interceptor, InterceptorFuture, LoadBalancer,
    MetricsRecorder, Next, Registry, RequestEncoder, RequestEncoding, Response, ResponseDecoder,
    RetryDecision, RetryDecisionContext, RetryPolicy, RouteRequest, Sanitization,
    SanitizationContext, Sanitizer, ServerConfig,
    contract::MethodDescriptor, interface, observability::MetricEvent,
};
use std::sync::Arc;

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

pub fn packaged_client(
    runtime: &ClientRuntime,
    router: Arc<dyn InstanceRouter>,
    load_balancer: Arc<dyn LoadBalancer>,
) -> ClientBuilder<PackageConsumerApiClient> {
    PackageConsumerApiClient::builder(runtime)
        .binding(HttpBindingId::default())
        .http_version_policy(HttpVersionPolicy::Auto)
        .direct_capabilities(EndpointCapabilities::default())
        .instance_router(router)
        .load_balancer(load_balancer)
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

pub struct PackagedInterceptor;

impl Interceptor for PackagedInterceptor {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        Box::pin(async move { next.run(context).await })
    }
}

pub struct PackagedRouter;

impl InstanceRouter for PackagedRouter {
    fn route(&self, request: RouteRequest<'_>) -> Result<InstanceSnapshot, Error> {
        let _ = request.context();
        Ok(request.into_instances())
    }
}

pub struct PackagedLoadBalancer;

impl LoadBalancer for PackagedLoadBalancer {
    fn select(&self, _context: &Context, instances: &InstanceSnapshot) -> Result<usize, Error> {
        if instances.is_empty() {
            return Err(local_extension_error());
        }
        Ok(0)
    }
}

pub struct PackagedRetryPolicy;

impl RetryPolicy for PackagedRetryPolicy {
    fn decide(&self, context: &RetryDecisionContext) -> RetryDecision {
        let _ = (
            context.completed_attempts(),
            context.max_attempts(),
            context.method_allows_retries(),
            context.failure(),
            context.remaining(),
        );
        RetryDecision::Stop
    }
}

pub struct PackagedMetricsRecorder;

impl MetricsRecorder for PackagedMetricsRecorder {
    fn record(&self, _event: &MetricEvent<'_>) {}
}

pub struct PackagedSanitizer;

impl Sanitizer for PackagedSanitizer {
    fn sanitize(&self, _context: SanitizationContext<'_>) -> Sanitization {
        Sanitization::Omit
    }
}

pub struct PackagedRequestEncoder;

impl RequestEncoder for PackagedRequestEncoder {
    fn encode(&self, request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
        let _ = (
            request.service(),
            request.method(),
            request.arguments(),
            request.headers(),
        );
        Err(local_extension_error())
    }
}

pub struct PackagedResponseDecoder;

impl ResponseDecoder for PackagedResponseDecoder {
    fn decode(
        &self,
        _method: &'static MethodDescriptor,
        response: BufferedResponse,
    ) -> Result<Response<Body>, Error> {
        let (status, _version, headers, body) = response.into_parts();
        let mut decoded = Response::new(Body::from_bytes(body));
        decoded.set_status(status)?;
        *decoded.headers_mut() = headers;
        Ok(decoded)
    }
}

pub struct PackagedErrorDecoder;

impl ErrorDecoder for PackagedErrorDecoder {
    fn decode(&self, _method: &'static MethodDescriptor, response: BufferedResponse) -> Error {
        let _ = response.into_parts();
        local_extension_error()
    }
}

fn local_extension_error() -> Error {
    Error::local(
        ErrorCategory::Internal,
        "package_consumer_extension",
        "package consumer extension stopped the invocation",
    )
    .unwrap()
}

pub fn runtime_with_extensions(
    id: HttpBindingId,
    interceptor: Arc<dyn Interceptor>,
    registry: Arc<dyn Registry>,
    retry_policy: Arc<dyn RetryPolicy>,
    metrics: Arc<dyn MetricsRecorder>,
    request_encoder: Arc<dyn RequestEncoder>,
    response_decoder: Arc<dyn ResponseDecoder>,
    error_decoder: Arc<dyn ErrorDecoder>,
) -> ClientRuntimeBuilder {
    ClientRuntime::builder()
        .registry(registry)
        .interceptor(interceptor.clone())
        .attempt_interceptor(interceptor)
        .retry_policy(retry_policy)
        .metrics(metrics)
        .http_binding(id, request_encoder, response_decoder, error_decoder)
}

pub fn packaged_sanitizer() -> Arc<dyn Sanitizer> {
    Arc::new(PackagedSanitizer)
}

pub fn assert_sanitizer_arc_forwarding(sanitizer: Arc<dyn Sanitizer>) {
    fn accepts_sanitizer(_sanitizer: impl Sanitizer) {}
    accepts_sanitizer(sanitizer);
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
    fi

    run_cargo generate-lockfile \
        --manifest-path "$consumer_dir/Cargo.toml" \
        --offline
    run_cargo check \
        --manifest-path "$consumer_dir/Cargo.toml" \
        --target-dir "$work_dir/consumer-target" \
        --locked \
        --offline
    echo "packaged $package archive compiled with default features"

    if [[ -n "$optional_feature" ]]; then
        run_cargo check \
            --manifest-path "$consumer_dir/Cargo.toml" \
            --target-dir "$work_dir/consumer-target" \
            --locked \
            --offline \
            --features archive-all-features
        echo "packaged $package archive compiled with optional feature $optional_feature"
    fi
done
