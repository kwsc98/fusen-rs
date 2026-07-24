use crate::{
    filter::{MiddlewareDyn, Next, RpcResult, Terminal},
    invocation::{InvocationPhase, PhaseTracker},
    protocol::fusen::context::RpcContext,
};
use fusen_contract::{BoxFuture, ServiceDescriptor};
use std::sync::Arc;

/// Generated declaration-order dispatch implemented by one service object.
pub trait RpcService: Send + Sync {
    /// Dispatches the [`RpcContext::method_id`](crate::protocol::fusen::context::RpcContext::method_id).
    fn call<'a>(&'a self, context: RpcContext) -> BoxFuture<'a, RpcResult>;
}

/// Static service metadata generated alongside [`RpcService`].
pub trait RpcServiceInfo: Send + Sync {
    /// Returns the process-lifetime service descriptor.
    fn service_descriptor(&self) -> &'static ServiceDescriptor;
}

/// A service that can be registered by [`Server`](crate::server::Server).
pub trait RegisteredRpcService: RpcService + RpcServiceInfo {}

impl<T> RegisteredRpcService for T where T: RpcService + RpcServiceInfo {}

#[derive(Clone)]
pub(crate) struct RouteDispatch {
    middleware: Arc<[Arc<dyn MiddlewareDyn>]>,
    service: Arc<dyn RegisteredRpcService>,
}

impl RouteDispatch {
    pub(crate) fn new(
        middleware: Arc<[Arc<dyn MiddlewareDyn>]>,
        service: Arc<dyn RegisteredRpcService>,
    ) -> Self {
        Self {
            middleware,
            service,
        }
    }

    pub(crate) async fn call(&self, context: RpcContext, tracker: PhaseTracker) -> RpcResult {
        tracker.set(InvocationPhase::Middleware);
        let terminal = ServiceInvoker {
            service: self.service.as_ref(),
            tracker,
        };
        Next::new(&self.middleware, &terminal).run(context).await
    }
}

struct ServiceInvoker<'a> {
    service: &'a dyn RegisteredRpcService,
    tracker: PhaseTracker,
}

impl Terminal for ServiceInvoker<'_> {
    fn call<'a>(&'a self, context: RpcContext) -> BoxFuture<'a, RpcResult> {
        self.tracker.set(InvocationPhase::Service);
        self.service.call(context)
    }
}
