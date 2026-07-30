//! External-consumer compile contracts for every object-safe 0.9 SPI.

use fusen_config::{
    ConfigDocument, ConfigError, ConfigFormat, ConfigHandle, ConfigKey, ConfigSource,
    provider as config_provider,
};
use fusen_rs::{
    FailureClass, InstanceRouter, InstanceSnapshot, LoadBalancer, MetricsRecorder, Middleware,
    MiddlewareFuture, Next, RetryDecision, RetryDecisionContext, RetryPolicy, RouteRequest,
    RpcContext, RpcError,
    observability::MetricEvent,
    registry::{
        RegistrationHandle, RegistrationRequest, Registry, SubscriptionHandle, SubscriptionRequest,
        directory, error::RegistryError, provider as registry_provider,
    },
};
use std::sync::Arc;

struct ExternalMiddleware;

impl Middleware for ExternalMiddleware {
    fn call<'a>(&'a self, context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a> {
        Box::pin(async move { next.run(context).await })
    }
}

struct ExternalRouter;

impl InstanceRouter for ExternalRouter {
    fn route(&self, request: RouteRequest<'_>) -> Result<InstanceSnapshot, RpcError> {
        let _ = request.context();
        Ok(request.into_instances())
    }
}

struct ExternalLoadBalancer;

impl LoadBalancer for ExternalLoadBalancer {
    fn select(
        &self,
        _context: &RpcContext,
        _instances: &InstanceSnapshot,
    ) -> Result<usize, RpcError> {
        Ok(0)
    }
}

struct ExternalRetryPolicy;

impl RetryPolicy for ExternalRetryPolicy {
    fn decide(&self, context: &RetryDecisionContext) -> RetryDecision {
        let _ = (
            context.completed_attempts(),
            context.max_attempts(),
            context.idempotency(),
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
        let _ = (request.registration(), request.protocol());
        Ok(registry_provider::registration(
            async { Ok(()) },
            || async { Ok(()) },
        ))
    }

    fn prepare_subscription(
        &self,
        request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError> {
        let _ = (request.selector(), request.protocol());
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

fn accepts_middleware(_: impl Middleware) {}
fn accepts_router(_: impl InstanceRouter) {}
fn accepts_load_balancer(_: impl LoadBalancer) {}
fn accepts_retry_policy(_: impl RetryPolicy) {}
fn accepts_registry(_: impl Registry) {}
fn accepts_config_source(_: impl ConfigSource) {}
fn accepts_metrics(_: impl MetricsRecorder) {}

#[test]
fn external_implementations_are_object_safe_and_arc_forwarding_is_complete() {
    let middleware: Arc<dyn Middleware> = Arc::new(ExternalMiddleware);
    let router: Arc<dyn InstanceRouter> = Arc::new(ExternalRouter);
    let load_balancer: Arc<dyn LoadBalancer> = Arc::new(ExternalLoadBalancer);
    let retry_policy: Arc<dyn RetryPolicy> = Arc::new(ExternalRetryPolicy);
    let registry: Arc<dyn Registry> = Arc::new(ExternalRegistry);
    let config_source: Arc<dyn ConfigSource> = Arc::new(ExternalConfigSource);
    let metrics: Arc<dyn MetricsRecorder> = Arc::new(ExternalMetricsRecorder);

    accepts_middleware(middleware);
    accepts_router(router);
    accepts_load_balancer(load_balancer);
    accepts_retry_policy(retry_policy);
    accepts_registry(registry);
    accepts_config_source(config_source);
    accepts_metrics(metrics);

    let _ = FailureClass::Transport;
}
