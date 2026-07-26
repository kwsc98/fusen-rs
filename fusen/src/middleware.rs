use crate::{RpcContext, RpcError, RpcResponse, runtime::BoxFuture};
use futures_util::FutureExt;
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

/// Result returned by middleware and generated service dispatch.
pub type RpcResult = Result<RpcResponse, RpcError>;

/// A logical-invocation middleware shared by client and server runtimes.
///
/// Client middleware runs exactly once around all physical attempts. Server middleware runs once
/// around body decoding and service dispatch. Panics are isolated to the current invocation.
pub trait Middleware: Send + Sync + 'static {
    /// Processes an invocation and optionally delegates to the rest of the chain.
    fn handle<'a>(
        &'a self,
        context: RpcContext,
        next: Next<'a>,
    ) -> impl Future<Output = RpcResult> + Send + 'a;
}

pub(crate) trait MiddlewareDyn: Send + Sync {
    fn handle_dyn<'a>(&'a self, context: RpcContext, next: Next<'a>) -> BoxFuture<'a, RpcResult>;
}

impl<T> MiddlewareDyn for T
where
    T: Middleware,
{
    fn handle_dyn<'a>(&'a self, context: RpcContext, next: Next<'a>) -> BoxFuture<'a, RpcResult> {
        Box::pin(async move {
            let future = match catch_unwind(AssertUnwindSafe(|| self.handle(context, next))) {
                Ok(future) => future,
                Err(_) => return Err(middleware_panicked()),
            };
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(result) => result,
                Err(_) => Err(middleware_panicked()),
            }
        })
    }
}

fn middleware_panicked() -> RpcError {
    tracing::error!("RPC middleware panicked");
    RpcError::framework(
        crate::RpcCategory::Internal,
        "middleware_panic",
        "middleware failed",
    )
}

pub(crate) trait Terminal: Send + Sync {
    fn call<'a>(&'a self, context: RpcContext) -> BoxFuture<'a, RpcResult>;
}

/// Consuming access to the remaining logical-invocation pipeline.
///
/// `Next` is intentionally not cloneable, preventing one middleware position from entering its
/// downstream chain more than once.
pub struct Next<'a> {
    remaining: &'a [Arc<dyn MiddlewareDyn>],
    terminal: &'a dyn Terminal,
}

impl<'a> Next<'a> {
    pub(crate) fn new(remaining: &'a [Arc<dyn MiddlewareDyn>], terminal: &'a dyn Terminal) -> Self {
        Self {
            remaining,
            terminal,
        }
    }

    /// Runs the next middleware or the framework terminal.
    pub fn run(self, context: RpcContext) -> BoxFuture<'a, RpcResult> {
        match self.remaining.split_first() {
            Some((middleware, remaining)) => middleware.handle_dyn(
                context,
                Self {
                    remaining,
                    terminal: self.terminal,
                },
            ),
            None => self.terminal.call(context),
        }
    }
}

pub(crate) fn erase_middleware(value: impl Middleware) -> Arc<dyn MiddlewareDyn> {
    Arc::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Arguments, context::RpcContextParts, runtime::deadline::Deadline};
    use fusen_contract::{
        Idempotency, MethodDescriptor, MethodId, ServiceDescriptor, ServiceSelector, WireProtocol,
    };
    use std::{future::ready, sync::OnceLock, time::Duration};

    struct SynchronousPanic;

    impl Middleware for SynchronousPanic {
        fn handle<'a>(
            &'a self,
            context: RpcContext,
            _next: Next<'a>,
        ) -> impl Future<Output = RpcResult> + Send + 'a {
            if context.request_id() == "panic" {
                panic!("private synchronous middleware panic");
            }
            let response = context.respond("unused");
            ready(response)
        }
    }

    struct UnreachableTerminal;

    impl Terminal for UnreachableTerminal {
        fn call<'a>(&'a self, _context: RpcContext) -> BoxFuture<'a, RpcResult> {
            Box::pin(async { unreachable!("panicking middleware must not call its terminal") })
        }
    }

    fn descriptor() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            ServiceDescriptor::new(
                ServiceSelector::new("middleware-test", None, None).unwrap(),
                vec![
                    MethodDescriptor::new(MethodId::new(0), "call", Idempotency::None, None)
                        .unwrap(),
                ],
            )
            .unwrap()
        })
    }

    #[tokio::test]
    async fn synchronous_future_construction_panic_is_isolated() {
        let service = descriptor();
        let context = RpcContext::new(RpcContextParts {
            request_id: "panic".to_owned(),
            protocol: WireProtocol::FusenV1,
            service,
            method: service.method(MethodId::new(0)).unwrap(),
            deadline: Deadline::after(Duration::from_secs(1)),
            attempt: 1,
            headers: http::HeaderMap::new(),
            arguments: Arguments::new(),
            response_limit: 1024,
            response_wire_overhead: 0,
            response_budget: crate::runtime::budget::ByteBudget::new(1024),
        });
        let middleware: Arc<[Arc<dyn MiddlewareDyn>]> =
            Arc::from([erase_middleware(SynchronousPanic)]);
        let error = Next::new(&middleware, &UnreachableTerminal)
            .run(context)
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "middleware_panic");
        assert_eq!(error.category(), crate::RpcCategory::Internal);
    }
}
