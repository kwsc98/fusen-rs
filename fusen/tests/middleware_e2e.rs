//! End-to-end ordering and isolation contracts for all four middleware stages.

use fusen_rs::{
    ClientConfig, ClientRuntime, Middleware, MiddlewareFuture, MiddlewareStage, Next, RetryConfig,
    RpcCall, RpcCategory, RpcContext, RpcError, RpcResponse, RpcSide, Server, ServerConfig,
    WireProtocol, contract::ProtocolSet, interface,
};
use http::HeaderValue;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[interface(name = "middleware-contract")]
trait MiddlewareContract {
    #[fusen_rs::method(
        idempotency = "idempotent",
        spring(method = "POST", path = "/middleware")
    )]
    async fn execute(
        &self,
        #[rpc(call)] call: RpcCall,
        #[rpc(body)] value: String,
    ) -> Result<RpcResponse<String>, RpcError>;
}

#[derive(Clone, Copy, Debug)]
struct ClientExtension(u8);

#[derive(Clone, Copy, Debug)]
struct ServerExtension(u8);

struct OrderedHandler {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl MiddlewareContract for OrderedHandler {
    async fn execute(&self, call: RpcCall, value: String) -> Result<RpcResponse<String>, RpcError> {
        assert_eq!(
            call.extensions()
                .get::<ServerExtension>()
                .map(|value| value.0),
            Some(23)
        );
        assert_eq!(call.headers().get("x-client-chain").unwrap(), "ready");
        self.events.lock().unwrap().push("handler");
        Ok(RpcResponse::new(value))
    }
}

#[derive(Clone)]
struct OrderedMiddleware {
    before: &'static str,
    after: &'static str,
    stage: MiddlewareStage,
    side: RpcSide,
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl Middleware for OrderedMiddleware {
    fn call<'a>(&'a self, mut context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a> {
        assert_eq!(context.stage(), self.stage);
        assert_eq!(context.side(), self.side);
        assert!(!context.request_id().is_empty());
        assert!(context.remaining() > Duration::ZERO);
        assert_eq!(
            context.interface().selector().service_id(),
            "middleware-contract"
        );
        assert_eq!(context.method().fusen_identity(), "execute");

        match self.stage {
            MiddlewareStage::ClientCall if self.before == "client-call-global:before" => {
                context.extensions_mut().insert(ClientExtension(11));
                context
                    .headers_mut()
                    .insert("x-client-chain", HeaderValue::from_static("ready"));
            }
            MiddlewareStage::ClientCall => {
                assert_eq!(
                    context
                        .extensions()
                        .get::<ClientExtension>()
                        .map(|value| value.0),
                    Some(11)
                );
            }
            MiddlewareStage::ClientAttempt => {
                assert_eq!(context.attempt().map(|value| value.get()), Some(1));
                assert!(context.endpoint().is_some());
                assert_eq!(
                    context
                        .extensions()
                        .get::<ClientExtension>()
                        .map(|value| value.0),
                    Some(11)
                );
            }
            MiddlewareStage::ServerHead if self.before == "server-head-global:before" => {
                assert!(context.arguments().is_none());
                context.extensions_mut().insert(ServerExtension(23));
            }
            MiddlewareStage::ServerHead => {
                assert!(context.arguments().is_none());
                assert_eq!(
                    context
                        .extensions()
                        .get::<ServerExtension>()
                        .map(|value| value.0),
                    Some(23)
                );
            }
            MiddlewareStage::ServerCall => {
                assert!(context.arguments().is_some());
                assert_eq!(
                    context
                        .extensions()
                        .get::<ServerExtension>()
                        .map(|value| value.0),
                    Some(23)
                );
            }
            _ => panic!("unexpected middleware stage"),
        }

        self.events.lock().unwrap().push(self.before);
        Box::pin(async move {
            let mut response = next.run(context).await?;
            response
                .headers_mut()
                .append("x-middleware-count", HeaderValue::from_static("1"));
            self.events.lock().unwrap().push(self.after);
            Ok(response)
        })
    }
}

fn ordered(
    before: &'static str,
    after: &'static str,
    stage: MiddlewareStage,
    side: RpcSide,
    events: &Arc<Mutex<Vec<&'static str>>>,
) -> OrderedMiddleware {
    OrderedMiddleware {
        before,
        after,
        stage,
        side,
        events: events.clone(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn four_stages_run_global_then_local_with_shared_server_extensions() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let server = Server::builder("127.0.0.1:0")
        .config(
            ServerConfig::builder()
                .protocols(ProtocolSet::ALL)
                .build()
                .unwrap(),
        )
        .head_middleware(ordered(
            "server-head-global:before",
            "server-head-global:after",
            MiddlewareStage::ServerHead,
            RpcSide::Server,
            &events,
        ))
        .middleware(ordered(
            "server-call-global:before",
            "server-call-global:after",
            MiddlewareStage::ServerCall,
            RpcSide::Server,
            &events,
        ))
        .interface(
            MiddlewareContractServer::new(OrderedHandler {
                events: events.clone(),
            })
            .head_middleware(ordered(
                "server-head-local:before",
                "server-head-local:after",
                MiddlewareStage::ServerHead,
                RpcSide::Server,
                &events,
            ))
            .middleware(ordered(
                "server-call-local:before",
                "server-call-local:after",
                MiddlewareStage::ServerCall,
                RpcSide::Server,
                &events,
            )),
        )
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    let runtime = ClientRuntime::builder()
        .middleware(ordered(
            "client-call-global:before",
            "client-call-global:after",
            MiddlewareStage::ClientCall,
            RpcSide::Client,
            &events,
        ))
        .attempt_middleware(ordered(
            "client-attempt-global:before",
            "client-attempt-global:after",
            MiddlewareStage::ClientAttempt,
            RpcSide::Client,
            &events,
        ))
        .build()
        .unwrap();
    let client = MiddlewareContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .protocol(WireProtocol::SpringCloudV1)
        .middleware(ordered(
            "client-call-local:before",
            "client-call-local:after",
            MiddlewareStage::ClientCall,
            RpcSide::Client,
            &events,
        ))
        .attempt_middleware(ordered(
            "client-attempt-local:before",
            "client-attempt-local:after",
            MiddlewareStage::ClientAttempt,
            RpcSide::Client,
            &events,
        ))
        .connect()
        .await
        .unwrap();

    let response = client
        .execute(RpcCall::new(), "complete".to_owned())
        .await
        .unwrap();
    assert_eq!(response.body(), "complete");
    assert_eq!(response.attempts(), 1);
    assert_eq!(
        response
            .headers()
            .get_all("x-middleware-count")
            .iter()
            .count(),
        8
    );
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "client-call-global:before",
            "client-call-local:before",
            "client-attempt-global:before",
            "client-attempt-local:before",
            "server-head-global:before",
            "server-head-local:before",
            "server-call-global:before",
            "server-call-local:before",
            "handler",
            "server-call-local:after",
            "server-call-global:after",
            "server-head-local:after",
            "server-head-global:after",
            "client-attempt-local:after",
            "client-attempt-global:after",
            "client-call-local:after",
            "client-call-global:after",
        ]
    );

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

struct RetryHandler(AtomicUsize);

impl MiddlewareContract for RetryHandler {
    async fn execute(
        &self,
        _call: RpcCall,
        value: String,
    ) -> Result<RpcResponse<String>, RpcError> {
        if self.0.fetch_add(1, Ordering::AcqRel) == 0 {
            return Err(
                RpcError::new(RpcCategory::Unavailable, "retry_once", "retry once").unwrap(),
            );
        }
        Ok(RpcResponse::new(value))
    }
}

#[derive(Clone)]
struct LogicalExtension;

impl Middleware for LogicalExtension {
    fn call<'a>(&'a self, mut context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a> {
        context.extensions_mut().insert(ClientExtension(41));
        Box::pin(async move { next.run(context).await })
    }
}

#[derive(Clone)]
struct AttemptIsolation {
    seen: Arc<Mutex<Vec<(u8, u8)>>>,
}

impl Middleware for AttemptIsolation {
    fn call<'a>(&'a self, mut context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a> {
        let attempt = context.attempt().unwrap().get();
        let inherited = context.extensions().get::<ClientExtension>().unwrap().0;
        self.seen.lock().unwrap().push((attempt, inherited));
        context.extensions_mut().insert(ClientExtension(attempt));
        Box::pin(async move { next.run(context).await })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_attempt_gets_an_isolated_clone_and_success_reports_final_attempts() {
    let server = Server::builder("127.0.0.1:0")
        .config(
            ServerConfig::builder()
                .protocols(ProtocolSet::ALL)
                .build()
                .unwrap(),
        )
        .interface(MiddlewareContractServer::new(RetryHandler(
            AtomicUsize::new(0),
        )))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let retry = RetryConfig::builder()
        .max_attempts(2)
        .backoff_base(Duration::from_nanos(1))
        .backoff_cap(Duration::from_nanos(1))
        .build()
        .unwrap();
    let config = ClientConfig::builder().retry(retry).build().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let runtime = ClientRuntime::builder()
        .config(config)
        .middleware(LogicalExtension)
        .attempt_middleware(AttemptIsolation { seen: seen.clone() })
        .build()
        .unwrap();
    let client = MiddlewareContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .unwrap();

    let response = client
        .execute(RpcCall::new(), "retried".to_owned())
        .await
        .unwrap();
    assert_eq!(response.body(), "retried");
    assert_eq!(response.attempts(), 2);
    assert_eq!(seen.lock().unwrap().as_slice(), [(1, 41), (2, 41)]);

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[derive(Clone)]
struct RejectHead;

impl Middleware for RejectHead {
    fn call<'a>(&'a self, _context: RpcContext, _next: Next<'a>) -> MiddlewareFuture<'a> {
        Box::pin(async {
            let mut error = RpcError::new(
                RpcCategory::PermissionDenied,
                "authentication_required",
                "authentication required",
            )
            .unwrap();
            error
                .headers_mut()
                .insert("www-authenticate", HeaderValue::from_static("Bearer"));
            Err(error)
        })
    }
}

struct UnreachableHandler;

impl MiddlewareContract for UnreachableHandler {
    async fn execute(
        &self,
        _call: RpcCall,
        _value: String,
    ) -> Result<RpcResponse<String>, RpcError> {
        panic!("ServerHead rejection must not invoke the handler")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_head_rejection_does_not_poll_the_request_body() {
    let server = Server::builder("127.0.0.1:0")
        .config(
            ServerConfig::builder()
                .protocols(ProtocolSet::SPRING_CLOUD_V1)
                .build()
                .unwrap(),
        )
        .interface(MiddlewareContractServer::new(UnreachableHandler).head_middleware(RejectHead))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    let mut stream = TcpStream::connect(server.local_addr()).await.unwrap();
    let head = concat!(
        "POST /middleware HTTP/1.1\r\n",
        "Host: localhost\r\n",
        "Content-Type: application/json\r\n",
        "Content-Length: 1048576\r\n",
        "Expect: 100-continue\r\n",
        "Connection: close\r\n",
        "\r\n"
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .expect("ServerHead must respond without waiting for the unsent body")
        .unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    assert!(!response.contains("100 Continue"), "{response}");
    assert!(
        response
            .to_ascii_lowercase()
            .contains("www-authenticate: bearer"),
        "{response}"
    );

    server.shutdown().await.unwrap();
}
