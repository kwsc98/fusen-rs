use super::{
    cluster::{InstanceSnapshot, LoadBalancer, Router, ensure_not_empty, validate_selection},
    runtime::ClientRuntimeInner,
    subscription::SubscriptionLease,
};
use crate::{
    error::{FusenError, ProblemDetails},
    filter::{MiddlewareDyn, Next, RpcResult, Terminal},
    invocation::{InvocationGuard, InvocationPhase, InvocationSide, PhaseTracker},
    protocol::{
        self,
        codec::{FusenHttpCodec, RequestCodec, ResponseCodec},
        fusen::{context::RpcContext, request::FusenRequest, response::RpcResponse},
    },
};
use fusen_contract::{MethodDescriptor, MethodId, ServiceDescriptor, WireProtocol};
use fusen_register::directory::Directory;
use http::{HeaderValue, header::HeaderName};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::{Arc, atomic::Ordering};

/// Runtime used internally by a macro-generated service client.
#[doc(hidden)]
pub struct ServiceClient {
    pub(super) runtime: Arc<ClientRuntimeInner>,
    pub(super) service: &'static ServiceDescriptor,
    pub(super) protocol: WireProtocol,
    pub(super) directory: Directory,
    pub(super) _subscription_lease: Option<SubscriptionLease>,
    pub(super) middleware: Arc<[Arc<dyn MiddlewareDyn>]>,
    pub(super) routers: Arc<[Arc<dyn Router>]>,
    pub(super) load_balancer: Arc<dyn LoadBalancer>,
}

impl ServiceClient {
    /// Invokes one statically described method.
    #[doc(hidden)]
    pub async fn __invoke(
        &self,
        method_id: MethodId,
        arguments: Vec<Value>,
    ) -> Result<Value, FusenError> {
        let method = self
            .service
            .method(method_id)
            .ok_or_else(|| FusenError::InvalidRequest("unknown client method".into()))?;
        let request_id = crate::request_id::new_request_id();
        let deadline = tokio::time::Instant::now() + self.runtime.config.request_timeout;
        let mut guard = InvocationGuard::start(
            &self.runtime.observers,
            InvocationSide::Client,
            &request_id,
            Some(self.service.selector().service_id()),
            Some(method.name()),
        );
        let tracker = guard.tracker();
        let result = tokio::time::timeout_at(
            deadline,
            self.invoke_inner(method, arguments, request_id, deadline, tracker),
        )
        .await;
        match result {
            Ok(Ok((value, status))) => {
                guard.finish_response(status);
                Ok(value)
            }
            Ok(Err(error)) => {
                guard.finish_error(&error);
                Err(error)
            }
            Err(_) => {
                guard.finish_timeout();
                Err(FusenError::Timeout(
                    "client request deadline exceeded".into(),
                ))
            }
        }
    }

    async fn invoke_inner(
        &self,
        method: &'static MethodDescriptor,
        arguments: Vec<Value>,
        request_id: String,
        deadline: tokio::time::Instant,
        tracker: PhaseTracker,
    ) -> Result<(Value, http::StatusCode), FusenError> {
        if self.runtime.closed.load(Ordering::Acquire) {
            return Err(FusenError::ServiceUnavailable(
                "client runtime is shut down".into(),
            ));
        }
        tracker.set(InvocationPhase::BuildRequest);
        let mut request = FusenRequest::init_request(self.protocol, method, arguments)?;
        request.headers.insert(
            HeaderName::from_static("x-request-id"),
            HeaderValue::from_str(&request_id).map_err(|error| {
                FusenError::internal("failed to create request ID header", error)
            })?,
        );
        let context = RpcContext::new(request_id, self.service, method, request, deadline);
        tracker.set(InvocationPhase::Middleware);
        let terminal = ClusterInvoker {
            client: self,
            tracker,
        };
        let response = Next::new(&self.middleware, &terminal).run(context).await?;
        let status = response.status;
        let value = response.body.ok_or_else(|| {
            FusenError::InvalidResponse("successful response body is empty".into())
        })?;
        Ok((value, status))
    }
}

struct ClusterInvoker<'a> {
    client: &'a ServiceClient,
    tracker: PhaseTracker,
}

impl Terminal for ClusterInvoker<'_> {
    fn call<'a>(&'a self, mut context: RpcContext) -> fusen_contract::BoxFuture<'a, RpcResult> {
        Box::pin(async move {
            self.tracker.set(InvocationPhase::Cluster);
            let mut instances = InstanceSnapshot::from_shared(self.client.directory.snapshot());
            for router in self.client.routers.iter() {
                instances = ensure_not_empty(router.route(&context, instances)?)?;
            }
            instances = ensure_not_empty(instances)?;
            let index = self.client.load_balancer.select(&context, &instances)?;
            let instance = validate_selection(&instances, index)?;
            context.request.endpoint = Some(instance.endpoint().clone());
            self.tracker.set(InvocationPhase::Transport);
            self.client
                .runtime
                .transport
                .call_tracked(context, &self.tracker)
                .await
        })
    }
}

pub(super) struct HttpTransport {
    pub(super) codec: FusenHttpCodec,
    pub(super) client: protocol::http::client::HttpClient,
}

impl HttpTransport {
    fn call_tracked<'a>(
        &'a self,
        mut context: RpcContext,
        tracker: &'a PhaseTracker,
    ) -> fusen_contract::BoxFuture<'a, RpcResult> {
        Box::pin(async move {
            let request = RequestCodec::encode(&self.codec, &mut context.request)?;
            let response = self.client.send_http_request(request).await?;
            tracker.set(InvocationPhase::Decode);
            let response =
                ResponseCodec::decode(&self.codec, response.map(|body| body.boxed())).await?;
            response_error(&response)?;
            Ok(response)
        })
    }
}

fn response_error(response: &RpcResponse) -> Result<(), FusenError> {
    if response.status.is_success() {
        return Ok(());
    }
    if let Some(body) = &response.body
        && let Ok(mut problem) = serde_json::from_value::<ProblemDetails>(body.clone())
    {
        problem.status = response.status.as_u16();
        return Err(FusenError::Remote(Box::new(problem)));
    }
    Err(FusenError::application(
        response.status,
        "remote_error",
        "remote service returned an error",
    )
    .unwrap_or_else(|_| {
        FusenError::InvalidResponse(format!(
            "remote service returned unexpected HTTP status {}",
            response.status
        ))
    }))
}
