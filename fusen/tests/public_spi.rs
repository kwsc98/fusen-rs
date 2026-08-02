//! External-consumer compile contracts for every object-safe 0.9 SPI.

use bytes::Bytes;
use fusen_config::{
    ConfigDocument, ConfigError, ConfigFormat, ConfigHandle, ConfigKey, ConfigSource,
    provider as config_provider,
};
use fusen_rs::{
    Body, BufferedResponse, ClientRuntime, Context, EncodedRequest, Error, ErrorCategory,
    ErrorDecoder, ErrorKind, ErrorOrigin, FailureClass, InstanceRouter, InstanceSnapshot,
    Interceptor, InterceptorFuture, LoadBalancer, MetricsRecorder, Next, RequestEncoder,
    RequestEncoding, Response, ResponseDecoder, RetryDecision, RetryDecisionContext, RetryPolicy,
    RouteRequest,
    contract::{HttpBindingId, MethodDescriptor},
    observability::MetricEvent,
    registry::{
        RegistrationHandle, RegistrationRequest, Registry, SubscriptionHandle, SubscriptionRequest,
        directory, error::RegistryError, provider as registry_provider,
    },
};
use http::{HeaderMap, Method};
use std::sync::Arc;

#[fusen_rs::interface(name = "public-spi-contract")]
/// Minimal generated client used to exercise interface-local SPI builders.
pub trait PublicSpiContract {
    /// Returns one probe value.
    #[fusen_rs::method(method = "GET", path = "/ping")]
    async fn ping(&self) -> Result<Response<String>, Error>;
}

struct ExternalInterceptor;

impl Interceptor for ExternalInterceptor {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        Box::pin(async move { next.run(context).await })
    }
}

struct ExternalRouter;

impl InstanceRouter for ExternalRouter {
    fn route(&self, request: RouteRequest<'_>) -> Result<InstanceSnapshot, Error> {
        let _ = request.context();
        Ok(request.into_instances())
    }
}

struct ExternalLoadBalancer;

impl LoadBalancer for ExternalLoadBalancer {
    fn select(&self, _context: &Context, _instances: &InstanceSnapshot) -> Result<usize, Error> {
        Ok(0)
    }
}

struct ExternalRetryPolicy;

impl RetryPolicy for ExternalRetryPolicy {
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

struct ExternalRegistry;

impl Registry for ExternalRegistry {
    fn prepare_registration(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        let _ = request.registration();
        Ok(registry_provider::registration(
            async { Ok(()) },
            || async { Ok(()) },
        ))
    }

    fn prepare_subscription(
        &self,
        request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError> {
        let _ = request.selector();
        let (_, directory) = directory::directory();
        Ok(registry_provider::subscription(
            directory,
            async { Ok(()) },
            || async { Ok(()) },
        ))
    }
}

struct ExternalConfigSource;

impl ConfigSource for ExternalConfigSource {
    fn prepare(&self, key: ConfigKey) -> Result<ConfigHandle, ConfigError> {
        let _ = (key.name(), key.group());
        Ok(config_provider::lifecycle(|_publisher| {
            (
                async { Ok(ConfigDocument::new(ConfigFormat::Toml, "enabled = true")) },
                || async { Ok(()) },
            )
        }))
    }
}

struct ExternalMetricsRecorder;

impl MetricsRecorder for ExternalMetricsRecorder {
    fn record(&self, event: &MetricEvent<'_>) {
        let _ = event;
    }
}

struct ExternalRequestEncoder;

impl RequestEncoder for ExternalRequestEncoder {
    fn encode(&self, request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
        let _ = (
            request.service(),
            request.method(),
            request.arguments(),
            request.headers(),
        );
        Ok(EncodedRequest::new(
            Method::POST,
            "/external-binding",
            HeaderMap::new(),
            Bytes::new(),
        ))
    }
}

struct ExternalResponseDecoder;

impl ResponseDecoder for ExternalResponseDecoder {
    fn decode(
        &self,
        _method: &'static MethodDescriptor,
        response: BufferedResponse,
    ) -> Result<Response<Body>, Error> {
        let _ = response.version();
        let mut decoded = Response::new(Body::from_bytes(response.body().clone()));
        decoded.set_status(response.status())?;
        *decoded.headers_mut() = response.headers().clone();
        Ok(decoded)
    }
}

struct ExternalErrorDecoder;

impl ErrorDecoder for ExternalErrorDecoder {
    fn decode(&self, _method: &'static MethodDescriptor, response: BufferedResponse) -> Error {
        let _ = (response.status(), response.headers(), response.body());
        Error::local(
            ErrorCategory::Unavailable,
            "external_binding_error",
            "external binding rejected the response",
        )
        .unwrap()
    }
}

fn accepts_interceptor(_: impl Interceptor) {}
fn accepts_router(_: impl InstanceRouter) {}
fn accepts_load_balancer(_: impl LoadBalancer) {}
fn accepts_retry_policy(_: impl RetryPolicy) {}
fn accepts_registry(_: impl Registry) {}
fn accepts_config_source(_: impl ConfigSource) {}
fn accepts_metrics(_: impl MetricsRecorder) {}
fn accepts_request_encoder(_: Arc<dyn RequestEncoder>) {}
fn accepts_response_decoder(_: Arc<dyn ResponseDecoder>) {}
fn accepts_error_decoder(_: Arc<dyn ErrorDecoder>) {}

#[tokio::test]
async fn external_implementations_are_object_safe_and_arc_forwarding_is_complete() {
    let interceptor: Arc<dyn Interceptor> = Arc::new(ExternalInterceptor);
    let router: Arc<dyn InstanceRouter> = Arc::new(ExternalRouter);
    let load_balancer: Arc<dyn LoadBalancer> = Arc::new(ExternalLoadBalancer);
    let retry_policy: Arc<dyn RetryPolicy> = Arc::new(ExternalRetryPolicy);
    let registry: Arc<dyn Registry> = Arc::new(ExternalRegistry);
    let config_source: Arc<dyn ConfigSource> = Arc::new(ExternalConfigSource);
    let metrics: Arc<dyn MetricsRecorder> = Arc::new(ExternalMetricsRecorder);
    let request_encoder: Arc<dyn RequestEncoder> = Arc::new(ExternalRequestEncoder);
    let response_decoder: Arc<dyn ResponseDecoder> = Arc::new(ExternalResponseDecoder);
    let error_decoder: Arc<dyn ErrorDecoder> = Arc::new(ExternalErrorDecoder);

    accepts_interceptor(interceptor.clone());
    accepts_router(router.clone());
    accepts_load_balancer(load_balancer.clone());
    accepts_retry_policy(retry_policy.clone());
    accepts_registry(registry.clone());
    accepts_config_source(config_source);
    accepts_metrics(metrics.clone());

    let binding = HttpBindingId::new("external-v1").unwrap();
    let runtime = ClientRuntime::builder()
        .registry(registry)
        .interceptor(interceptor.clone())
        .attempt_interceptor(interceptor.clone())
        .retry_policy(retry_policy)
        .metrics(metrics)
        .http_binding(
            binding.clone(),
            request_encoder.clone(),
            response_decoder.clone(),
            error_decoder.clone(),
        )
        .build()
        .unwrap();
    let _client_builder = PublicSpiContractClient::builder(&runtime)
        .binding(binding)
        .interceptor(interceptor)
        .instance_router(router)
        .load_balancer(load_balancer);

    accepts_request_encoder(request_encoder);
    accepts_response_decoder(response_decoder);
    accepts_error_decoder(error_decoder);

    let _ = FailureClass::Transport;
    runtime.shutdown().await.unwrap();
}

#[test]
fn invocation_error_dimensions_and_constructors_are_public() {
    let application = Error::application(
        ErrorCategory::Conflict,
        "already_exists",
        "the resource already exists",
    )
    .unwrap();
    assert_eq!(application.kind(), ErrorKind::Application);
    assert_eq!(application.origin(), ErrorOrigin::Local);
    assert_eq!(application.status(), http::StatusCode::CONFLICT);
    assert_eq!(application.request_id(), None);
    assert_eq!(application.attempts(), 0);

    let custom = Error::application_status(
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_entity",
        "the entity is invalid",
    )
    .unwrap();
    assert_eq!(custom.category(), ErrorCategory::Unknown);

    let framework = Error::local(
        ErrorCategory::Unavailable,
        "dependency_unavailable",
        "the dependency is unavailable",
    )
    .unwrap();
    assert_eq!(framework.kind(), ErrorKind::Framework);
    assert_eq!(framework.origin(), ErrorOrigin::Local);
}
