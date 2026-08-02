use crate::Response;
pub use crate::{
    context::{Body, Context, InterceptionStage, Side},
    error::{Error, ErrorCategory, ErrorDetails, ErrorOrigin, RetryHint},
    sensitive::{
        PolicySanitizer, ProjectionLimits, Sanitization, SanitizationContext, SanitizationTarget,
        SanitizedValue, Sanitizer,
    },
};
use futures_util::FutureExt;
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::Arc,
};

/// Result returned by interceptor and generated interface dispatch.
pub type InterceptorResult = Result<Response<Body>, Error>;

/// Sendable future returned by [`Interceptor`].
pub type InterceptorFuture<'a> = Pin<Box<dyn Future<Output = InterceptorResult> + Send + 'a>>;

/// Object-safe interceptor shared by all client and server stages.
pub trait Interceptor: Send + Sync + 'static {
    /// Processes one stage and optionally delegates to the remaining chain.
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a>;
}

impl<T> Interceptor for Arc<T>
where
    T: Interceptor + ?Sized,
{
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        (**self).intercept(context, next)
    }
}

pub(crate) fn call_interceptor<'a>(
    interceptor: &'a dyn Interceptor,
    context: Context,
    next: Next<'a>,
) -> InterceptorFuture<'a> {
    Box::pin(async move {
        let future = match catch_unwind(AssertUnwindSafe(|| interceptor.intercept(context, next))) {
            Ok(future) => future,
            Err(_) => return Err(interceptor_panicked()),
        };
        match AssertUnwindSafe(future).catch_unwind().await {
            Ok(result) => result,
            Err(_) => Err(interceptor_panicked()),
        }
    })
}

fn interceptor_panicked() -> Error {
    tracing::error!("service invocation interceptor panicked");
    Error::framework(
        crate::ErrorCategory::Internal,
        "interceptor_panic",
        "interceptor failed",
    )
}

pub(crate) trait Terminal: Send + Sync {
    fn call<'a>(&'a self, context: Context) -> InterceptorFuture<'a>;
}

/// Consuming access to the remainder of an interceptor chain.
///
/// `Next` is intentionally not cloneable, so one interceptor position can enter downstream at
/// most once.
pub struct Next<'a> {
    remaining: &'a [Arc<dyn Interceptor>],
    terminal: &'a dyn Terminal,
}

impl<'a> Next<'a> {
    pub(crate) fn new(remaining: &'a [Arc<dyn Interceptor>], terminal: &'a dyn Terminal) -> Self {
        Self {
            remaining,
            terminal,
        }
    }

    /// Runs the next interceptor or the framework terminal.
    pub fn run(self, context: Context) -> InterceptorFuture<'a> {
        match self.remaining.split_first() {
            Some((interceptor, remaining)) => call_interceptor(
                interceptor.as_ref(),
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

pub(crate) fn erase_interceptor(value: impl Interceptor) -> Arc<dyn Interceptor> {
    Arc::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Arguments, InterceptionStage, Side,
        context::ContextParts,
        runtime::{budget::ByteBudget, deadline::Deadline},
    };
    use fusen_contract::{
        HttpBindingId, HttpOperation, MethodDescriptor, MethodId, ServiceDescriptor,
        ServiceSelector,
    };
    use std::{num::NonZeroU8, sync::OnceLock, time::Duration};

    struct SynchronousPanic;

    impl Interceptor for SynchronousPanic {
        fn intercept<'a>(&'a self, context: Context, _next: Next<'a>) -> InterceptorFuture<'a> {
            if context.request_id() == "panic" {
                panic!("private synchronous interceptor panic");
            }
            Box::pin(async move { context.respond("unused") })
        }
    }

    struct UnreachableTerminal;

    impl Terminal for UnreachableTerminal {
        fn call<'a>(&'a self, _context: Context) -> InterceptorFuture<'a> {
            Box::pin(async { unreachable!("panicking interceptor must not call its terminal") })
        }
    }

    fn descriptor() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            ServiceDescriptor::new(
                ServiceSelector::new("interceptor-test", None, None).unwrap(),
                vec![
                    MethodDescriptor::new(
                        MethodId::new(0),
                        "call",
                        HttpOperation::new(
                            http::Method::POST,
                            "/call",
                            Vec::new(),
                            "application/json",
                            "application/json",
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap()
        })
    }

    #[tokio::test]
    async fn synchronous_future_construction_panic_is_isolated() {
        let interface = descriptor();
        let context = Context::new(ContextParts {
            side: Side::Client,
            stage: InterceptionStage::ClientCall,
            request_id: "panic".to_owned(),
            binding_id: HttpBindingId::default(),
            http_version: None,
            interface,
            method: interface.method(MethodId::new(0)).unwrap(),
            deadline: Deadline::after(Duration::from_secs(1)),
            attempt: NonZeroU8::new(1),
            endpoint: None,
            headers: http::HeaderMap::new(),
            extensions: http::Extensions::new(),
            arguments: Some(Arguments::new()),
            response_limit: 1024,
            response_wire_overhead: 0,
            response_budget: ByteBudget::new(1024),
        });
        let interceptor: Arc<[Arc<dyn Interceptor>]> =
            Arc::from([erase_interceptor(SynchronousPanic)]);
        let error = Next::new(&interceptor, &UnreachableTerminal)
            .run(context)
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "interceptor_panic");
        assert_eq!(error.category(), crate::ErrorCategory::Internal);
    }
}
