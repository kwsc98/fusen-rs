use super::{
    builder::ServiceClientBuilder,
    invocation::HttpTransport,
    subscription::{ManagerShutdownError, SubscriptionManager},
};
use crate::{
    error::FusenError,
    filter::{Middleware, MiddlewareDyn, erase_middleware},
    invocation::InvocationObserver,
    protocol::{self, codec::FusenHttpCodec},
};
use fusen_contract::ServiceDescriptor;
use fusen_register::{Register, error::RegisterError};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub use crate::protocol::http::client::{Http1PoolConfig, Http2PoolConfig};

/// Resource limits and deadlines shared by all clients in one runtime.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Maximum time allowed to establish a connection.
    pub connect_timeout: Duration,
    /// End-to-end deadline for one logical RPC invocation.
    pub request_timeout: Duration,
    /// Maximum time allowed for an initial discovery subscription.
    pub discovery_timeout: Duration,
    /// Maximum time allowed to clean up one subscription.
    pub subscription_close_timeout: Duration,
    /// Maximum response body size accepted from a peer.
    pub max_response_body_bytes: usize,
    /// HTTP/1.1 connection-pool settings.
    pub http1_pool: Http1PoolConfig,
    /// HTTP/2 connection-pool and keep-alive settings.
    pub http2_pool: Http2PoolConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(10),
            discovery_timeout: Duration::from_secs(5),
            subscription_close_timeout: Duration::from_secs(5),
            max_response_body_bytes: 2 * 1024 * 1024,
            http1_pool: Http1PoolConfig::default(),
            http2_pool: Http2PoolConfig::default(),
        }
    }
}

/// Builds a shared client runtime.
pub struct ClientRuntimeBuilder {
    pub(super) registry: Option<Arc<dyn Register>>,
    middleware: Vec<Arc<dyn MiddlewareDyn>>,
    observers: Vec<Arc<dyn InvocationObserver>>,
    pub(super) config: ClientConfig,
}

impl ClientRuntimeBuilder {
    /// Replaces client resource limits and deadlines.
    pub fn config(mut self, config: ClientConfig) -> Self {
        self.config = config;
        self
    }

    /// Replaces the HTTP/1.1 connection-pool settings.
    pub fn http1_pool(mut self, config: Http1PoolConfig) -> Self {
        self.config.http1_pool = config;
        self
    }

    /// Replaces the HTTP/2 connection-pool and keep-alive settings.
    pub fn http2_pool(mut self, config: Http2PoolConfig) -> Self {
        self.config.http2_pool = config;
        self
    }

    /// Installs the registry used by generated clients in discovery mode.
    pub fn registry(mut self, registry: impl Register + 'static) -> Self {
        self.registry = Some(Arc::new(registry));
        self
    }

    /// Appends global client middleware in execution order.
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }

    /// Appends a synchronous complete-invocation observer.
    pub fn observer(mut self, observer: impl InvocationObserver + 'static) -> Self {
        self.observers.push(Arc::new(observer));
        self
    }

    /// Builds the reusable runtime and HTTP connection pools.
    pub fn build(self) -> Result<ClientRuntime, FusenError> {
        validate_config(&self.config)?;
        Ok(ClientRuntime {
            inner: Arc::new(ClientRuntimeInner {
                registry: self.registry,
                transport: HttpTransport {
                    codec: FusenHttpCodec::new(self.config.max_response_body_bytes),
                    client: protocol::http::client::HttpClient::new(
                        self.config.connect_timeout,
                        &self.config.http1_pool,
                        &self.config.http2_pool,
                    ),
                },
                middleware: Arc::from(self.middleware),
                observers: Arc::from(self.observers),
                subscriptions: SubscriptionManager::new(self.config.subscription_close_timeout),
                shutdown_failure: Mutex::new(None),
                shutdown_lock: tokio::sync::Mutex::new(()),
                closed: AtomicBool::new(false),
                shutdown_complete: AtomicBool::new(false),
                config: self.config,
            }),
        })
    }
}

/// Shared ownership root for connection pools, discovery subscriptions, middleware and observers.
#[derive(Clone)]
pub struct ClientRuntime {
    pub(super) inner: Arc<ClientRuntimeInner>,
}

impl ClientRuntime {
    /// Creates an empty runtime builder with bounded defaults.
    pub fn builder() -> ClientRuntimeBuilder {
        ClientRuntimeBuilder {
            registry: None,
            middleware: Vec::new(),
            observers: Vec::new(),
            config: ClientConfig::default(),
        }
    }

    /// Idempotently rejects new work and closes every runtime-owned subscription.
    pub async fn shutdown(&self) -> Result<(), FusenError> {
        self.inner.closed.store(true, Ordering::Release);
        let _guard = self.inner.shutdown_lock.lock().await;
        if self.inner.shutdown_complete.load(Ordering::Acquire) {
            return self.inner.shutdown_result();
        }
        match self.inner.subscriptions.shutdown().await {
            Ok(()) => {}
            Err(ManagerShutdownError::Timeout) => {
                return Err(FusenError::Timeout(
                    "subscription cleanup deadline exceeded".into(),
                ));
            }
            Err(ManagerShutdownError::Terminal(error)) => {
                self.inner.record_shutdown_failure(error);
            }
        }
        self.inner.shutdown_complete.store(true, Ordering::Release);
        self.inner.shutdown_result()
    }

    #[doc(hidden)]
    pub fn __client_builder(&self, service: &'static ServiceDescriptor) -> ServiceClientBuilder {
        ServiceClientBuilder::new(self.clone(), service)
    }
}

pub(super) struct ClientRuntimeInner {
    pub(super) registry: Option<Arc<dyn Register>>,
    pub(super) transport: HttpTransport,
    pub(super) middleware: Arc<[Arc<dyn MiddlewareDyn>]>,
    pub(super) observers: Arc<[Arc<dyn InvocationObserver>]>,
    pub(super) subscriptions: Arc<SubscriptionManager>,
    shutdown_failure: Mutex<Option<ShutdownFailure>>,
    shutdown_lock: tokio::sync::Mutex<()>,
    pub(super) closed: AtomicBool,
    shutdown_complete: AtomicBool,
    pub(super) config: ClientConfig,
}

#[derive(Clone)]
struct ShutdownFailure {
    source: RegisterError,
}

impl ClientRuntimeInner {
    fn record_shutdown_failure(&self, source: RegisterError) {
        let mut failure = self
            .shutdown_failure
            .lock()
            .expect("client shutdown failure lock poisoned");
        if failure.is_none() {
            *failure = Some(ShutdownFailure { source });
        }
    }

    fn shutdown_result(&self) -> Result<(), FusenError> {
        match self
            .shutdown_failure
            .lock()
            .expect("client shutdown failure lock poisoned")
            .clone()
        {
            Some(failure) => Err(FusenError::internal(
                "failed to close service subscription",
                failure.source,
            )),
            None => Ok(()),
        }
    }
}

impl Drop for ClientRuntimeInner {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let subscriptions = self.subscriptions.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = subscriptions.shutdown().await;
            });
        }
    }
}

fn validate_config(config: &ClientConfig) -> Result<(), FusenError> {
    if config.connect_timeout.is_zero()
        || config.request_timeout.is_zero()
        || config.discovery_timeout.is_zero()
        || config.subscription_close_timeout.is_zero()
        || config.max_response_body_bytes == 0
        || config
            .http1_pool
            .idle_timeout
            .is_some_and(|timeout| timeout.is_zero())
        || config.http2_pool.connections_per_host == 0
        || config
            .http2_pool
            .idle_timeout
            .is_some_and(|timeout| timeout.is_zero())
        || config
            .http2_pool
            .keep_alive_interval
            .is_some_and(|interval| interval.is_zero())
        || config.http2_pool.keep_alive_timeout.is_zero()
    {
        Err(FusenError::InvalidRequest(
            "client limits, pool sizes, and configured timeouts must be greater than zero".into(),
        ))
    } else {
        Ok(())
    }
}
