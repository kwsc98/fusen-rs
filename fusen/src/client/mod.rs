use crate::{
    error::{FusenError, ProblemDetails},
    filter::{FusenFilter, ProceedingJoinPoint},
    handler::{Handler, HandlerContext, HandlerController, HandlerInfo},
    protocol::{
        self,
        codec::{FusenHttpCodec, RequestCodec, ResponseCodec},
        fusen::{
            context::FusenContext,
            request::FusenRequest,
            service::{MethodInfo, ParameterSource, ServiceInfo},
        },
    },
};
use fusen_contract::{
    ServiceEndpoint, ServiceInstance, ServiceSelector, ServiceWeight, WireProtocol,
};
use fusen_register::{Register, ServiceSubscription};
use http::{HeaderValue, Uri, header::HeaderName};
use http_body_util::BodyExt;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug)]
/// Selects whether a client calls one absolute URI or uses service discovery.
pub enum ClientEndpoint {
    /// Calls the supplied absolute HTTP URI.
    Direct(Uri),
    /// Selects instances from the configured [`Register`] implementation.
    Discovery,
}

#[derive(Clone, Debug)]
/// Resource limits and deadlines applied by the HTTP client.
pub struct ClientConfig {
    /// Maximum time allowed to establish a connection.
    pub connect_timeout: Duration,
    /// End-to-end deadline for one HTTP request.
    pub request_timeout: Duration,
    /// Maximum time allowed for an initial discovery subscription.
    pub discovery_timeout: Duration,
    /// Maximum time a caller waits for discovery subscription cleanup.
    pub subscription_close_timeout: Duration,
    /// Maximum number of response body bytes accepted from a peer.
    pub max_response_body_bytes: usize,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(10),
            discovery_timeout: Duration::from_secs(5),
            subscription_close_timeout: Duration::from_secs(5),
            max_response_body_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug)]
/// Per-service endpoint, protocol, and middleware selection.
pub struct ClientOptions {
    /// Addressing strategy for this service.
    pub endpoint: ClientEndpoint,
    /// HTTP wire behavior used for the service.
    pub protocol: WireProtocol,
    /// Ordered handler identifiers applied to each invocation.
    pub handlers: Vec<String>,
}

impl ClientOptions {
    /// Builds options for a directly addressed Fusen HTTP/2 service.
    pub fn direct(uri: Uri) -> Self {
        Self {
            endpoint: ClientEndpoint::Direct(uri),
            protocol: WireProtocol::Fusen,
            handlers: Vec::new(),
        }
    }

    /// Builds discovery options for the requested wire protocol.
    pub fn discovery(protocol: WireProtocol) -> Self {
        Self {
            endpoint: ClientEndpoint::Discovery,
            protocol,
            handlers: Vec::new(),
        }
    }

    /// Replaces the ordered handler list.
    pub fn handlers(mut self, handlers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.handlers = handlers.into_iter().map(Into::into).collect();
        self
    }
}

/// Builder for shared client transport, discovery, and handlers.
pub struct FusenClientContextBuilder {
    register: Option<Arc<dyn Register>>,
    handler_context: HandlerContext,
    config: ClientConfig,
}

impl Default for FusenClientContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FusenClientContextBuilder {
    /// Creates a builder with conservative production defaults.
    pub fn new() -> Self {
        Self {
            register: None,
            handler_context: HandlerContext::default(),
            config: ClientConfig::default(),
        }
    }

    /// Replaces the client resource and deadline configuration.
    pub fn config(mut self, config: ClientConfig) -> Self {
        self.config = config;
        self
    }

    /// Installs the service registry used by discovery endpoints.
    pub fn register(mut self, register: impl Register + 'static) -> Self {
        self.register = Some(Arc::new(register));
        self
    }

    /// Installs one uniquely identified middleware handler.
    pub fn handler(mut self, handler: Handler) -> Result<Self, FusenError> {
        self.handler_context.load_handler(handler)?;
        Ok(self)
    }

    /// Builds a reusable client context and Hyper connection pools.
    pub fn build(self) -> Result<FusenClientContext, FusenError> {
        if self.config.connect_timeout.is_zero()
            || self.config.request_timeout.is_zero()
            || self.config.discovery_timeout.is_zero()
            || self.config.subscription_close_timeout.is_zero()
            || self.config.max_response_body_bytes == 0
        {
            return Err(FusenError::InvalidRequest(
                "client limits and timeouts must be greater than zero".into(),
            ));
        }
        let transport = HttpTransport {
            http_codec: FusenHttpCodec::new(self.config.max_response_body_bytes),
            http_client: protocol::http::client::HttpClient::new(self.config.connect_timeout),
        };
        Ok(FusenClientContext {
            register: self.register,
            handler_context: self.handler_context,
            http_client: Arc::new(transport),
            config: self.config,
        })
    }
}

/// Shared factory used by generated service clients.
pub struct FusenClientContext {
    register: Option<Arc<dyn Register>>,
    handler_context: HandlerContext,
    http_client: Arc<dyn FusenFilter>,
    config: ClientConfig,
}

impl FusenClientContext {
    /// Resolves a service and creates its invocation runtime.
    pub async fn init_client(
        &mut self,
        service_info: ServiceInfo,
        options: ClientOptions,
    ) -> Result<FusenClient, FusenError> {
        service_info.validate()?;
        let methods = service_info
            .method_infos
            .iter()
            .cloned()
            .map(|method| (method.method_name.clone(), Arc::new(method)))
            .collect::<HashMap<_, _>>();
        if options.protocol == WireProtocol::SpringCloud
            && methods.values().any(|method| {
                method
                    .parameters
                    .iter()
                    .filter(|parameter| parameter.source == ParameterSource::Body)
                    .count()
                    > 1
            })
        {
            return Err(FusenError::InvalidRequest(
                "SpringCloud services support at most one body parameter per method".into(),
            ));
        }
        self.handler_context.load_controller(HandlerInfo {
            service_desc: service_info.service_desc.clone(),
            handlers: options.handlers,
        })?;
        let handler_controller = self
            .handler_context
            .get_controller(&service_info.service_desc)?
            .clone();
        let selector = ServiceSelector::new(
            service_info.service_desc.service_id.clone(),
            service_info.service_desc.group.clone(),
            service_info.service_desc.version.clone(),
        )
        .map_err(|error| FusenError::InvalidRequest(error.to_string()))?;
        let subscription = match options.endpoint {
            ClientEndpoint::Direct(uri) => ServiceSubscription::local(vec![ServiceInstance::new(
                validate_endpoint(&uri)?,
                ServiceWeight::default(),
            )]),
            ClientEndpoint::Discovery => {
                let register = self.register.as_ref().ok_or_else(|| {
                    FusenError::ServiceUnavailable("discovery endpoint requires a register".into())
                })?;
                tokio::time::timeout(
                    self.config.discovery_timeout,
                    register.subscribe(selector, options.protocol),
                )
                .await
                .map_err(|_| FusenError::Timeout("service discovery deadline exceeded".into()))?
                .map_err(|error| FusenError::internal("service subscription failed", error))?
            }
        };
        Ok(FusenClient {
            http_client: self.http_client.clone(),
            protocol: options.protocol,
            subscription,
            handler_controller,
            methods,
            request_timeout: self.config.request_timeout,
            subscription_close_timeout: self.config.subscription_close_timeout,
            closed: AtomicBool::new(false),
        })
    }
}

/// Runtime owned by one generated service client.
pub struct FusenClient {
    http_client: Arc<dyn FusenFilter>,
    protocol: WireProtocol,
    subscription: ServiceSubscription,
    handler_controller: HandlerController,
    methods: HashMap<String, Arc<MethodInfo>>,
    request_timeout: Duration,
    subscription_close_timeout: Duration,
    closed: AtomicBool,
}

impl FusenClient {
    /// Serializes and invokes one generated service method.
    pub async fn invoke(
        &self,
        method_name: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, FusenError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(FusenError::ServiceUnavailable("client is closed".into()));
        }
        tokio::time::timeout(
            self.request_timeout,
            self.invoke_inner(method_name, arguments),
        )
        .await
        .map_err(|_| FusenError::Timeout("client request deadline exceeded".into()))?
    }

    async fn invoke_inner(
        &self,
        method_name: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, FusenError> {
        let method_info = self
            .methods
            .get(method_name)
            .ok_or_else(|| FusenError::InvalidRequest(format!("unknown method {method_name}")))?;
        let mut request = FusenRequest::init_request(self.protocol, method_info, arguments)?;
        let request_id = crate::request_id::new_request_id();
        request.headers.insert(
            HeaderName::from_static("x-request-id"),
            HeaderValue::from_str(&request_id).map_err(|error| {
                FusenError::internal("failed to create request ID header", error)
            })?,
        );
        let mut context = FusenContext {
            unique_identifier: request_id,
            metadata: Default::default(),
            method_info: method_info.clone(),
            request,
            response: None,
        };
        let resources = self.subscription.directory().snapshot();
        let load_balance = self
            .handler_controller
            .load_balance
            .as_ref()
            .ok_or_else(|| FusenError::ServiceUnavailable("load balancer is missing".into()))?;
        let resource = load_balance
            .select_dyn(&context, resources)
            .await?
            .ok_or_else(|| FusenError::ServiceUnavailable("no healthy service instances".into()))?;
        context.request.endpoint = Some(resource.endpoint().as_str().to_owned());
        let context = ProceedingJoinPoint::new(
            self.handler_controller.aspect.clone(),
            self.http_client.clone(),
            context,
        )
        .proceed()
        .await?;
        let response = context.response.ok_or_else(|| {
            FusenError::ServiceUnavailable("transport returned no response".into())
        })?;
        if !response.http_status.status.is_success() {
            if let Some(body) = response.body
                && let Ok(mut problem) = serde_json::from_value::<ProblemDetails>(body)
            {
                problem.status = response.http_status.status.as_u16();
                return Err(FusenError::Remote(Box::new(problem)));
            }
            return Err(FusenError::application(
                response.http_status.status,
                "remote_error",
                "remote service returned an error",
            )
            .unwrap_or_else(|_| {
                FusenError::InvalidResponse(format!(
                    "remote service returned unexpected HTTP status {}",
                    response.http_status.status
                ))
            }));
        }
        response
            .body
            .ok_or_else(|| FusenError::InvalidResponse("successful response body is empty".into()))
    }

    /// Closes a discovery subscription. Direct clients complete immediately.
    pub async fn close(&self) -> Result<(), FusenError> {
        self.closed.store(true, Ordering::Release);
        tokio::time::timeout(self.subscription_close_timeout, self.subscription.close())
            .await
            .map_err(|_| FusenError::Timeout("subscription cleanup deadline exceeded".into()))?
            .map_err(|error| FusenError::internal("failed to close service subscription", error))
    }
}

fn validate_endpoint(uri: &Uri) -> Result<ServiceEndpoint, FusenError> {
    let url = url::Url::parse(&uri.to_string())
        .map_err(|error| FusenError::InvalidRequest(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(FusenError::InvalidRequest(
            "direct endpoint must be an absolute HTTP(S) URI".into(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(FusenError::InvalidRequest(
            "direct endpoint must not contain a query or fragment".into(),
        ));
    }
    ServiceEndpoint::new(url).map_err(|error| FusenError::InvalidRequest(error.to_string()))
}

struct HttpTransport {
    http_codec: FusenHttpCodec,
    http_client: protocol::http::client::HttpClient,
}

impl FusenFilter for HttpTransport {
    fn call<'a>(
        &'a self,
        join_point: ProceedingJoinPoint,
    ) -> fusen_contract::BoxFuture<'a, Result<FusenContext, FusenError>> {
        Box::pin(async move {
            let mut context = join_point.context;
            let request = RequestCodec::encode(&self.http_codec, &mut context.request)?;
            let response = self.http_client.send_http_request(request).await?;
            context.response = Some(
                ResponseCodec::decode(&self.http_codec, response.map(|body| body.boxed())).await?,
            );
            Ok(context)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        filter::ProceedingJoinPoint,
        handler::{Handler, HandlerInvoker, loadbalance::LoadBalanceDyn},
        protocol::fusen::service::{ParameterInfo, ParameterSource, ServiceDesc},
    };
    use fusen_contract::{BoxFuture, ServiceRegistration, StaticBoxFuture};
    use fusen_register::{error::RegisterError, subscription_cleanup};
    use http::Method;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Notify,
    };

    fn service(method: MethodInfo) -> ServiceInfo {
        ServiceInfo::new(method.service_desc.clone(), vec![method])
    }

    async fn spring_client(
        endpoint: String,
        request_timeout: Duration,
        method: MethodInfo,
    ) -> FusenClient {
        let mut context = FusenClientContextBuilder::new()
            .config(ClientConfig {
                request_timeout,
                ..ClientConfig::default()
            })
            .build()
            .unwrap();
        context
            .init_client(
                service(method),
                ClientOptions {
                    endpoint: ClientEndpoint::Direct(endpoint.parse().unwrap()),
                    protocol: WireProtocol::SpringCloud,
                    handlers: Vec::new(),
                },
            )
            .await
            .unwrap()
    }

    async fn read_headers(stream: &mut tokio::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        String::from_utf8(request).unwrap()
    }

    #[test]
    fn rejects_zero_client_limits() {
        let config = ClientConfig {
            request_timeout: Duration::ZERO,
            ..ClientConfig::default()
        };
        assert!(
            FusenClientContextBuilder::new()
                .config(config)
                .build()
                .is_err()
        );
    }

    #[test]
    fn rejects_zero_subscription_close_timeout() {
        let config = ClientConfig {
            subscription_close_timeout: Duration::ZERO,
            ..ClientConfig::default()
        };
        assert!(
            FusenClientContextBuilder::new()
                .config(config)
                .build()
                .is_err()
        );
    }

    #[tokio::test]
    async fn spring_http1_preserves_path_and_query_arguments() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_headers(&mut stream).await;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\nConnection: close\r\n\r\n\"ok\"",
                )
                .await
                .unwrap();
            request
        });
        let desc = ServiceDesc::new("spring", None, None);
        let client = spring_client(
            format!("http://{addr}/api"),
            Duration::from_secs(1),
            MethodInfo::new(
                desc,
                "find".into(),
                Method::GET,
                "/users/{id}".into(),
                vec![
                    ParameterInfo::new("id", ParameterSource::Path),
                    ParameterInfo::new("filter", ParameterSource::Query),
                ],
            ),
        )
        .await;
        let value = client
            .invoke(
                "find",
                vec![serde_json::json!("a/b"), serde_json::json!("x y")],
            )
            .await
            .unwrap();
        assert_eq!(value, serde_json::json!("ok"));
        let request = server.await.unwrap();
        assert!(request.starts_with("GET /api/users/a%2Fb?filter=x+y HTTP/1.1\r\n"));
    }

    #[tokio::test]
    async fn request_timeout_includes_streaming_response_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_headers(&mut stream).await;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\n\r\n",
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = stream.write_all(b"\"ok\"").await;
        });
        let desc = ServiceDesc::new("slow", None, None);
        let client = spring_client(
            format!("http://{addr}"),
            Duration::from_millis(30),
            MethodInfo::new(desc, "slow".into(), Method::GET, "/slow".into(), Vec::new()),
        )
        .await;
        let error = client.invoke("slow", Vec::new()).await.unwrap_err();
        assert!(matches!(error, FusenError::Timeout(_)));
        server.abort();
    }

    #[tokio::test]
    async fn request_timeout_includes_response_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_headers(&mut stream).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\n\r\n\"ok\"",
                )
                .await;
        });
        let desc = ServiceDesc::new("slow-headers", None, None);
        let client = spring_client(
            format!("http://{addr}"),
            Duration::from_millis(30),
            MethodInfo::new(desc, "slow".into(), Method::GET, "/slow".into(), Vec::new()),
        )
        .await;
        assert!(matches!(
            client.invoke("slow", Vec::new()).await,
            Err(FusenError::Timeout(_))
        ));
        server.abort();
    }

    struct SlowLoadBalance;

    impl LoadBalanceDyn for SlowLoadBalance {
        fn select_dyn<'a>(
            &'a self,
            _context: &'a FusenContext,
            _invokers: Arc<Vec<Arc<ServiceInstance>>>,
        ) -> BoxFuture<'a, Result<Option<Arc<ServiceInstance>>, FusenError>> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(None)
            })
        }
    }

    struct SlowAspect;

    impl FusenFilter for SlowAspect {
        fn call<'a>(
            &'a self,
            join_point: ProceedingJoinPoint,
        ) -> BoxFuture<'a, Result<FusenContext, FusenError>> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                join_point.proceed().await
            })
        }
    }

    async fn client_with_handler(handler: Handler) -> FusenClient {
        let desc = ServiceDesc::new("slow-handler", None, None);
        let mut context = FusenClientContextBuilder::new()
            .config(ClientConfig {
                request_timeout: Duration::from_millis(30),
                ..ClientConfig::default()
            })
            .handler(handler)
            .unwrap()
            .build()
            .unwrap();
        context
            .init_client(
                service(MethodInfo::new(
                    desc,
                    "slow".into(),
                    Method::GET,
                    "/slow".into(),
                    Vec::new(),
                )),
                ClientOptions::direct("http://127.0.0.1:1".parse().unwrap()).handlers(["slow"]),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn request_timeout_includes_load_balancing() {
        let client = client_with_handler(Handler {
            id: "slow".into(),
            handler_invoker: HandlerInvoker::LoadBalance(Arc::new(SlowLoadBalance)),
        })
        .await;
        assert!(matches!(
            client.invoke("slow", Vec::new()).await,
            Err(FusenError::Timeout(_))
        ));
    }

    #[tokio::test]
    async fn request_timeout_includes_aspects() {
        let client = client_with_handler(Handler {
            id: "slow".into(),
            handler_invoker: HandlerInvoker::Aspect(Arc::new(SlowAspect)),
        })
        .await;
        assert!(matches!(
            client.invoke("slow", Vec::new()).await,
            Err(FusenError::Timeout(_))
        ));
    }

    struct HangingRegister;

    impl Register for HangingRegister {
        fn register(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn deregister(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn subscribe(
            &self,
            _resource: ServiceSelector,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
            Box::pin(std::future::pending())
        }
    }

    #[derive(Clone)]
    struct HangingCleanupRegister {
        release: Arc<Notify>,
        completed: Arc<Notify>,
    }

    impl Register for HangingCleanupRegister {
        fn register(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn deregister(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn subscribe(
            &self,
            _resource: ServiceSelector,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
            let release = self.release.clone();
            let completed = self.completed.clone();
            Box::pin(async move {
                let (closer, cleanup) = subscription_cleanup();
                tokio::spawn(cleanup.run(async move {
                    release.notified().await;
                    completed.notify_one();
                    Ok(())
                }));
                Ok(ServiceSubscription::new(
                    fusen_register::directory::Directory::fixed(Vec::new()),
                    closer,
                ))
            })
        }
    }

    #[tokio::test]
    async fn discovery_timeout_bounds_initial_subscription() {
        let desc = ServiceDesc::new("slow-discovery", None, None);
        let mut context = FusenClientContextBuilder::new()
            .config(ClientConfig {
                discovery_timeout: Duration::from_millis(30),
                ..ClientConfig::default()
            })
            .register(HangingRegister)
            .build()
            .unwrap();
        let result = context
            .init_client(
                service(MethodInfo::new(
                    desc,
                    "find".into(),
                    Method::GET,
                    "/find".into(),
                    Vec::new(),
                )),
                ClientOptions::discovery(WireProtocol::Fusen),
            )
            .await;
        assert!(matches!(result, Err(FusenError::Timeout(_))));
    }

    #[tokio::test]
    async fn subscription_close_timeout_marks_client_closed() {
        let desc = ServiceDesc::new("hanging-cleanup", None, None);
        let release = Arc::new(Notify::new());
        let completed = Arc::new(Notify::new());
        let mut context = FusenClientContextBuilder::new()
            .config(ClientConfig {
                subscription_close_timeout: Duration::from_millis(20),
                ..ClientConfig::default()
            })
            .register(HangingCleanupRegister {
                release: release.clone(),
                completed: completed.clone(),
            })
            .build()
            .unwrap();
        let client = context
            .init_client(
                service(MethodInfo::new(
                    desc,
                    "find".into(),
                    Method::GET,
                    "/find".into(),
                    Vec::new(),
                )),
                ClientOptions::discovery(WireProtocol::Fusen),
            )
            .await
            .unwrap();
        assert!(matches!(client.close().await, Err(FusenError::Timeout(_))));
        assert!(matches!(
            client.invoke("find", Vec::new()).await,
            Err(FusenError::ServiceUnavailable(message)) if message == "client is closed"
        ));
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), completed.notified())
            .await
            .expect("background subscription cleanup did not complete");
    }

    #[tokio::test]
    async fn spring_http1_sends_one_body_argument_as_raw_json() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let headers = read_headers(&mut stream).await;
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap();
            let mut body = vec![0; content_length];
            stream.read_exact(&mut body).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\nConnection: close\r\n\r\n\"ok\"",
                )
                .await
                .unwrap();
            (headers, body)
        });
        let desc = ServiceDesc::new("spring", None, None);
        let client = spring_client(
            format!("http://{addr}/api"),
            Duration::from_secs(1),
            MethodInfo::new(
                desc,
                "create".into(),
                Method::POST,
                "/orders".into(),
                vec![ParameterInfo::new("order", ParameterSource::Body)],
            ),
        )
        .await;
        client
            .invoke("create", vec![serde_json::json!({ "name": "demo" })])
            .await
            .unwrap();
        let (headers, body) = server.await.unwrap();
        assert!(headers.starts_with("POST /api/orders HTTP/1.1\r\n"));
        assert_eq!(body, br#"{"name":"demo"}"#);
    }

    #[tokio::test]
    async fn spring_http1_decodes_problem_details() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_headers(&mut stream).await;
            let body = br#"{"type":"https://example.test/problems/rejected","title":"Conflict","status":409,"detail":"rejected","code":"rejected","request_id":"remote-1"}"#;
            let response = format!(
                "HTTP/1.1 422 Unprocessable Entity\r\nContent-Type: application/problem+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
        });
        let desc = ServiceDesc::new("spring-error", None, None);
        let client = spring_client(
            format!("http://{addr}"),
            Duration::from_secs(1),
            MethodInfo::new(desc, "find".into(), Method::GET, "/find".into(), Vec::new()),
        )
        .await;
        let error = client.invoke("find", Vec::new()).await.unwrap_err();
        assert!(matches!(
            error,
            FusenError::Remote(problem) if problem.status == 422 && problem.code == "rejected"
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn successful_response_without_json_body_is_invalid_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_headers(&mut stream).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let desc = ServiceDesc::new("empty-success", None, None);
        let client = spring_client(
            format!("http://{addr}"),
            Duration::from_secs(1),
            MethodInfo::new(desc, "find".into(), Method::GET, "/find".into(), Vec::new()),
        )
        .await;
        assert!(matches!(
            client.invoke("find", Vec::new()).await,
            Err(FusenError::InvalidResponse(_))
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn spring_rejects_multiple_body_parameters_during_initialization() {
        let desc = ServiceDesc::new("spring", None, None);
        let method = MethodInfo::new(
            desc,
            "invalid".into(),
            Method::POST,
            "/invalid".into(),
            vec![
                ParameterInfo::new("left", ParameterSource::Body),
                ParameterInfo::new("right", ParameterSource::Body),
            ],
        );
        let mut context = FusenClientContextBuilder::new().build().unwrap();
        let result = context
            .init_client(
                service(method),
                ClientOptions {
                    endpoint: ClientEndpoint::Direct("http://127.0.0.1:1".parse().unwrap()),
                    protocol: WireProtocol::SpringCloud,
                    handlers: Vec::new(),
                },
            )
            .await;
        assert!(matches!(result, Err(FusenError::InvalidRequest(_))));
    }
}
