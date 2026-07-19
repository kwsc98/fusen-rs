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
            service::{MethodInfo, ServiceInfo},
        },
    },
};
use fusen_internal_common::{
    protocol::WireProtocol,
    resource::service::{MethodResource, ServiceResource},
    utils::uuid::uuid,
};
use fusen_register::{Register, directory::Directory};
use http::Uri;
use http_body_util::BodyExt;
use serde_json::Value;
use std::{
    collections::{HashMap, LinkedList},
    sync::Arc,
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
    /// Maximum number of response body bytes accepted from a peer.
    pub max_response_body_bytes: usize,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(10),
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
    pub fn build(self) -> FusenClientContext {
        let transport = HttpTransport {
            http_codec: FusenHttpCodec::new(self.config.max_response_body_bytes),
            http_client: protocol::http::client::HttpClient::new(
                self.config.connect_timeout,
                self.config.request_timeout,
            ),
        };
        FusenClientContext {
            register: self.register,
            handler_context: self.handler_context,
            http_client: Arc::new(Box::new(transport)),
        }
    }
}

/// Shared factory used by generated service clients.
pub struct FusenClientContext {
    register: Option<Arc<dyn Register>>,
    handler_context: HandlerContext,
    http_client: Arc<Box<dyn FusenFilter>>,
}

impl FusenClientContext {
    /// Resolves a service and creates its invocation runtime.
    pub async fn init_client(
        &mut self,
        service_info: ServiceInfo,
        options: ClientOptions,
    ) -> Result<FusenClient, FusenError> {
        let methods = service_info
            .method_infos
            .iter()
            .cloned()
            .map(|method| (method.method_name.clone(), Arc::new(method)))
            .collect::<HashMap<_, _>>();
        self.handler_context.load_controller(HandlerInfo {
            service_desc: service_info.service_desc.clone(),
            handlers: options.handlers,
        })?;
        let handler_controller = self
            .handler_context
            .get_controller(&service_info.service_desc)?
            .clone();
        let mut resource = ServiceResource {
            service_id: service_info.service_desc.service_id,
            group: service_info.service_desc.group,
            version: service_info.service_desc.version,
            methods: methods
                .values()
                .map(|method| MethodResource {
                    method_name: method.method_name.clone(),
                    path: method.path.clone(),
                    method: method.method.to_string(),
                })
                .collect(),
            addr: String::new(),
            weight: Some(1.0),
            metadata: Default::default(),
        };
        let directory = match options.endpoint {
            ClientEndpoint::Direct(uri) => {
                if uri.scheme().is_none() || uri.authority().is_none() {
                    return Err(FusenError::InvalidRequest(
                        "direct endpoint must be an absolute URI".into(),
                    ));
                }
                resource.addr = uri.to_string().trim_end_matches('/').to_owned();
                let directory = Directory::default();
                directory.replace(vec![resource]).map_err(|error| {
                    FusenError::internal("failed to initialize directory", error)
                })?;
                directory
            }
            ClientEndpoint::Discovery => self
                .register
                .as_ref()
                .ok_or_else(|| {
                    FusenError::ServiceUnavailable("discovery endpoint requires a register".into())
                })?
                .subscribe(resource, options.protocol)
                .await
                .map_err(|error| FusenError::internal("service subscription failed", error))?,
        };
        Ok(FusenClient {
            http_client: self.http_client.clone(),
            protocol: options.protocol,
            directory,
            handler_controller,
            methods,
        })
    }
}

/// Runtime owned by one generated service client.
pub struct FusenClient {
    http_client: Arc<Box<dyn FusenFilter>>,
    protocol: WireProtocol,
    directory: Directory,
    handler_controller: HandlerController,
    methods: HashMap<String, Arc<MethodInfo>>,
}

impl FusenClient {
    /// Serializes and invokes one generated service method.
    pub async fn invoke(
        &self,
        method_name: &str,
        method: &str,
        path: &str,
        field_pats: &[&str],
        request_bodies: LinkedList<Value>,
    ) -> Result<Value, FusenError> {
        let request =
            FusenRequest::init_request(self.protocol, method, path, field_pats, request_bodies)?;
        let method_info = self
            .methods
            .get(method_name)
            .ok_or_else(|| FusenError::InvalidRequest(format!("unknown method {method_name}")))?;
        let mut context = FusenContext {
            unique_identifier: uuid(),
            metadata: Default::default(),
            method_info: method_info.clone(),
            request,
            response: None,
        };
        let resources = self.directory.snapshot();
        let load_balance = self
            .handler_controller
            .load_balance
            .as_ref()
            .ok_or_else(|| FusenError::ServiceUnavailable("load balancer is missing".into()))?;
        let resource = load_balance
            .select_(&context, resources)
            .await?
            .ok_or_else(|| FusenError::ServiceUnavailable("no healthy service instances".into()))?;
        context.request.addr = Some(resource.addr.clone());
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
        if !(200..300).contains(&response.http_status.status) {
            if let Some(body) = response.body
                && let Ok(problem) = serde_json::from_value::<ProblemDetails>(body)
            {
                return Err(FusenError::Remote(Box::new(problem)));
            }
            return Err(FusenError::Application {
                status: response.http_status.status,
                code: "remote_error".into(),
                message: "remote service returned an error".into(),
            });
        }
        response
            .body
            .ok_or_else(|| FusenError::InvalidRequest("successful response body is empty".into()))
    }
}

struct HttpTransport {
    http_codec: FusenHttpCodec,
    http_client: protocol::http::client::HttpClient,
}

impl FusenFilter for HttpTransport {
    fn call<'a>(
        &'a self,
        join_point: ProceedingJoinPoint,
    ) -> fusen_internal_common::BoxFutureV2<'a, Result<FusenContext, FusenError>> {
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
