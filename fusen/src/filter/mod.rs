use crate::{
    error::FusenError,
    protocol::fusen::{context::RpcContext, response::RpcResponse},
};
use fusen_contract::BoxFuture;
use std::{future::Future, sync::Arc};

/// Result returned by middleware and RPC terminals.
pub type RpcResult = Result<RpcResponse, FusenError>;

/// User-defined middleware shared by client and provider pipelines.
///
/// Implementations can use `async fn handle` directly. The explicit future bound lets the runtime
/// erase middleware internally while keeping user code free of boxed futures and adapter macros.
pub trait Middleware: Send + Sync + 'static {
    /// Processes one invocation and optionally delegates to the remaining chain.
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
        Box::pin(self.handle(context, next))
    }
}

pub(crate) trait Terminal: Send + Sync {
    fn call<'a>(&'a self, context: RpcContext) -> BoxFuture<'a, RpcResult>;
}

/// Consuming access to the rest of an RPC middleware pipeline.
///
/// `Next` is intentionally not cloneable, so one pipeline position cannot enter its downstream
/// middleware or terminal more than once.
pub struct Next<'a> {
    remaining: &'a [Arc<dyn MiddlewareDyn>],
    terminal: &'a dyn Terminal,
}

impl<'a> Next<'a> {
    pub(crate) fn new(
        middleware: &'a [Arc<dyn MiddlewareDyn>],
        terminal: &'a dyn Terminal,
    ) -> Self {
        Self {
            remaining: middleware,
            terminal,
        }
    }

    /// Runs the next middleware or the framework terminal when the chain is exhausted.
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

pub(crate) fn erase_middleware(middleware: impl Middleware) -> Arc<dyn MiddlewareDyn> {
    Arc::new(middleware)
}

/// Runs the framework-owned no-op middleware microbenchmark.
#[doc(hidden)]
pub async fn __benchmark_middleware(count: usize, iterations: u64) -> std::time::Duration {
    use crate::protocol::fusen::{
        request::{FusenRequest, Path},
        response::RpcResponse,
    };
    use fusen_contract::{MethodDescriptor, MethodId, ServiceDescriptor, WireProtocol};
    use http::Method;
    use std::sync::OnceLock;

    struct Noop;
    impl Middleware for Noop {
        async fn handle<'a>(&'a self, context: RpcContext, next: Next<'a>) -> RpcResult {
            next.run(context).await
        }
    }
    struct NoopTerminal;
    impl Terminal for NoopTerminal {
        fn call<'a>(&'a self, _context: RpcContext) -> BoxFuture<'a, RpcResult> {
            Box::pin(async { Ok(RpcResponse::default()) })
        }
    }
    fn service() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            ServiceDescriptor::__new(
                "benchmark",
                None,
                None,
                vec![
                    MethodDescriptor::__new(
                        MethodId::__new(0),
                        "call",
                        Method::POST,
                        "/benchmark",
                        Vec::new(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap()
        })
    }
    fn context(index: u64) -> RpcContext {
        RpcContext::new(
            index.to_string(),
            service(),
            &service().methods()[0],
            FusenRequest {
                protocol: WireProtocol::Fusen,
                path: Path {
                    method: Method::POST,
                    path: "/benchmark".into(),
                },
                endpoint: None,
                path_parameters: Default::default(),
                query_parameters: Default::default(),
                headers: Default::default(),
                body: None,
            },
            tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        )
    }

    let middleware = (0..count)
        .map(|_| erase_middleware(Noop))
        .collect::<Vec<_>>();
    let terminal = NoopTerminal;
    let started = std::time::Instant::now();
    for index in 0..iterations {
        std::hint::black_box(
            Next::new(&middleware, &terminal)
                .run(context(index))
                .await
                .expect("no-op benchmark invocation must succeed"),
        );
    }
    started.elapsed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::fusen::{
        request::{FusenRequest, Path},
        response::RpcResponse,
    };
    use fusen_contract::{MethodDescriptor, MethodId, ServiceDescriptor, WireProtocol};
    use http::{Method, StatusCode};
    use std::sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::time::{Duration, Instant};

    fn service() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            ServiceDescriptor::__new(
                "demo",
                None,
                None,
                vec![
                    MethodDescriptor::__new(
                        MethodId::__new(0),
                        "call",
                        Method::POST,
                        "/call",
                        Vec::new(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap()
        })
    }

    fn context() -> RpcContext {
        RpcContext::new(
            "request-1".into(),
            service(),
            &service().methods()[0],
            FusenRequest {
                protocol: WireProtocol::Fusen,
                path: Path {
                    method: Method::POST,
                    path: "/call".into(),
                },
                endpoint: None,
                path_parameters: Default::default(),
                query_parameters: Default::default(),
                headers: Default::default(),
                body: None,
            },
            Instant::now() + Duration::from_secs(1),
        )
    }

    struct Recording {
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl Middleware for Recording {
        async fn handle<'a>(&'a self, context: RpcContext, next: Next<'a>) -> RpcResult {
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:in", self.name));
            let result = next.run(context).await;
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:out", self.name));
            result
        }
    }

    struct CountingTerminal {
        calls: Arc<AtomicUsize>,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl Terminal for CountingTerminal {
        fn call<'a>(&'a self, _context: RpcContext) -> BoxFuture<'a, RpcResult> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.events.lock().unwrap().push("terminal".into());
                Ok(RpcResponse::default())
            })
        }
    }

    #[tokio::test]
    async fn middleware_enters_in_order_and_exits_in_reverse() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let middleware = vec![
            erase_middleware(Recording {
                name: "global",
                events: events.clone(),
            }),
            erase_middleware(Recording {
                name: "local",
                events: events.clone(),
            }),
        ];
        let terminal = CountingTerminal {
            calls: calls.clone(),
            events: events.clone(),
        };
        Next::new(&middleware, &terminal)
            .run(context())
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *events.lock().unwrap(),
            [
                "global:in",
                "local:in",
                "terminal",
                "local:out",
                "global:out"
            ]
        );
    }

    struct ShortCircuit;

    impl Middleware for ShortCircuit {
        async fn handle<'a>(&'a self, _context: RpcContext, _next: Next<'a>) -> RpcResult {
            Ok(RpcResponse::new(StatusCode::ACCEPTED))
        }
    }

    #[tokio::test]
    async fn short_circuit_does_not_enter_terminal() {
        let calls = Arc::new(AtomicUsize::new(0));
        let terminal = CountingTerminal {
            calls: calls.clone(),
            events: Arc::new(Mutex::new(Vec::new())),
        };
        let response = Next::new(&[erase_middleware(ShortCircuit)], &terminal)
            .run(context())
            .await
            .unwrap();
        assert_eq!(response.status, StatusCode::ACCEPTED);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    struct Failure;

    impl Middleware for Failure {
        async fn handle<'a>(&'a self, _context: RpcContext, _next: Next<'a>) -> RpcResult {
            Err(FusenError::InvalidRequest("rejected".into()))
        }
    }

    #[tokio::test]
    async fn middleware_error_is_returned_without_terminal_execution() {
        let calls = Arc::new(AtomicUsize::new(0));
        let terminal = CountingTerminal {
            calls: calls.clone(),
            events: Arc::new(Mutex::new(Vec::new())),
        };
        let error = Next::new(&[erase_middleware(Failure)], &terminal)
            .run(context())
            .await
            .unwrap_err();
        assert!(matches!(error, FusenError::InvalidRequest(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
