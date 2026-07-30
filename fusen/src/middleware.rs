use crate::RpcResponse;
pub use crate::{
    context::{MiddlewareStage, RpcBody, RpcContext, RpcSide},
    rpc::{RetryHint, RpcCategory, RpcError, RpcErrorDetails, RpcOrigin},
};
use futures_util::FutureExt;
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::Arc,
};

/// Result returned by middleware and generated interface dispatch.
pub type MiddlewareResult = Result<RpcResponse<RpcBody>, RpcError>;

/// Sendable future returned by [`Middleware`].
pub type MiddlewareFuture<'a> = Pin<Box<dyn Future<Output = MiddlewareResult> + Send + 'a>>;

/// Object-safe middleware shared by all client and server stages.
pub trait Middleware: Send + Sync + 'static {
    /// Processes one stage and optionally delegates to the remaining chain.
    fn call<'a>(&'a self, context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a>;
}

impl<T> Middleware for Arc<T>
where
    T: Middleware + ?Sized,
{
    fn call<'a>(&'a self, context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a> {
        (**self).call(context, next)
    }
}

pub(crate) fn call_middleware<'a>(
    middleware: &'a dyn Middleware,
    context: RpcContext,
    next: Next<'a>,
) -> MiddlewareFuture<'a> {
    Box::pin(async move {
        let future = match catch_unwind(AssertUnwindSafe(|| middleware.call(context, next))) {
            Ok(future) => future,
            Err(_) => return Err(middleware_panicked()),
        };
        match AssertUnwindSafe(future).catch_unwind().await {
            Ok(result) => result,
            Err(_) => Err(middleware_panicked()),
        }
    })
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
    fn call<'a>(&'a self, context: RpcContext) -> MiddlewareFuture<'a>;
}

/// Consuming access to the remainder of a middleware chain.
///
/// `Next` is intentionally not cloneable, so one middleware position can enter downstream at
/// most once.
pub struct Next<'a> {
    remaining: &'a [Arc<dyn Middleware>],
    terminal: &'a dyn Terminal,
}

impl<'a> Next<'a> {
    pub(crate) fn new(remaining: &'a [Arc<dyn Middleware>], terminal: &'a dyn Terminal) -> Self {
        Self {
            remaining,
            terminal,
        }
    }

    /// Runs the next middleware or the framework terminal.
    pub fn run(self, context: RpcContext) -> MiddlewareFuture<'a> {
        match self.remaining.split_first() {
            Some((middleware, remaining)) => call_middleware(
                middleware.as_ref(),
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

pub(crate) fn erase_middleware(value: impl Middleware) -> Arc<dyn Middleware> {
    Arc::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MiddlewareStage, RpcArguments, RpcSide,
        context::RpcContextParts,
        runtime::{budget::ByteBudget, deadline::Deadline},
    };
    use fusen_contract::{
        Idempotency, MethodDescriptor, MethodId, ServiceDescriptor, ServiceSelector, WireProtocol,
    };
    use std::{num::NonZeroU8, sync::OnceLock, time::Duration};

    struct SynchronousPanic;

    impl Middleware for SynchronousPanic {
        fn call<'a>(&'a self, context: RpcContext, _next: Next<'a>) -> MiddlewareFuture<'a> {
            if context.request_id() == "panic" {
                panic!("private synchronous middleware panic");
            }
            Box::pin(async move { context.respond("unused") })
        }
    }

    struct UnreachableTerminal;

    impl Terminal for UnreachableTerminal {
        fn call<'a>(&'a self, _context: RpcContext) -> MiddlewareFuture<'a> {
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
        let interface = descriptor();
        let context = RpcContext::new(RpcContextParts {
            side: RpcSide::Client,
            stage: MiddlewareStage::ClientCall,
            request_id: "panic".to_owned(),
            protocol: WireProtocol::FusenV1,
            interface,
            method: interface.method(MethodId::new(0)).unwrap(),
            deadline: Deadline::after(Duration::from_secs(1)),
            attempt: NonZeroU8::new(1),
            endpoint: None,
            headers: http::HeaderMap::new(),
            extensions: http::Extensions::new(),
            arguments: Some(RpcArguments::new()),
            response_limit: 1024,
            response_wire_overhead: 0,
            response_budget: ByteBudget::new(1024),
        });
        let middleware: Arc<[Arc<dyn Middleware>]> =
            Arc::from([erase_middleware(SynchronousPanic)]);
        let error = Next::new(&middleware, &UnreachableTerminal)
            .run(context)
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "middleware_panic");
        assert_eq!(error.category(), crate::RpcCategory::Internal);
    }
}
