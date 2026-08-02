//! End-to-end ordering and isolation contracts for all four interceptor stages.

use fusen_rs::{
    Call, ClientConfig, ClientRuntime, Context, Error, ErrorCategory, InterceptionStage,
    Interceptor, InterceptorFuture, Next, PolicySanitizer, Response, RetryConfig, SanitizedValue,
    Server, Side, interface,
};
use http::HeaderValue;
use serde_json::{Value, json};
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

#[interface(name = "interceptor-contract")]
trait InterceptorContract {
    #[fusen_rs::method(method = "PUT", path = "/interceptor")]
    async fn execute(
        &self,
        #[param(context)] call: Call,
        #[param(body)] value: String,
    ) -> Result<Response<String>, Error>;
}

#[derive(Clone, Copy, Debug)]
struct ClientExtension(u8);

#[derive(Clone, Copy, Debug)]
struct ServerExtension(u8);

struct OrderedHandler {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl InterceptorContract for OrderedHandler {
    async fn execute(&self, call: Call, value: String) -> Result<Response<String>, Error> {
        assert_eq!(
            call.extensions()
                .get::<ServerExtension>()
                .map(|value| value.0),
            Some(23)
        );
        assert_eq!(call.headers().get("x-client-chain").unwrap(), "ready");
        self.events.lock().unwrap().push("handler");
        Ok(Response::new(value))
    }
}

#[derive(Clone)]
struct OrderedInterceptor {
    before: &'static str,
    after: &'static str,
    stage: InterceptionStage,
    side: Side,
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl Interceptor for OrderedInterceptor {
    fn intercept<'a>(&'a self, mut context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        assert_eq!(context.stage(), self.stage);
        assert_eq!(context.side(), self.side);
        assert!(!context.request_id().is_empty());
        assert!(context.remaining() > Duration::ZERO);
        assert_eq!(
            context.interface().selector().service_id(),
            "interceptor-contract"
        );
        assert_eq!(context.method().invocation_name(), "execute");

        match self.stage {
            InterceptionStage::ClientCall if self.before == "client-call-global:before" => {
                context.extensions_mut().insert(ClientExtension(11));
                context
                    .headers_mut()
                    .insert("x-client-chain", HeaderValue::from_static("ready"));
            }
            InterceptionStage::ClientCall => {
                assert_eq!(
                    context
                        .extensions()
                        .get::<ClientExtension>()
                        .map(|value| value.0),
                    Some(11)
                );
            }
            InterceptionStage::ClientAttempt => {
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
            InterceptionStage::ServerHead if self.before == "server-head-global:before" => {
                assert!(context.arguments().is_none());
                context.extensions_mut().insert(ServerExtension(23));
            }
            InterceptionStage::ServerHead => {
                assert!(context.arguments().is_none());
                assert_eq!(
                    context
                        .extensions()
                        .get::<ServerExtension>()
                        .map(|value| value.0),
                    Some(23)
                );
            }
            InterceptionStage::ServerCall => {
                assert!(context.arguments().is_some());
                assert_eq!(
                    context
                        .extensions()
                        .get::<ServerExtension>()
                        .map(|value| value.0),
                    Some(23)
                );
            }
            _ => panic!("unexpected interceptor stage"),
        }

        self.events.lock().unwrap().push(self.before);
        Box::pin(async move {
            let mut response = next.run(context).await?;
            response
                .headers_mut()
                .append("x-interceptor-count", HeaderValue::from_static("1"));
            self.events.lock().unwrap().push(self.after);
            Ok(response)
        })
    }
}

fn ordered(
    before: &'static str,
    after: &'static str,
    stage: InterceptionStage,
    side: Side,
    events: &Arc<Mutex<Vec<&'static str>>>,
) -> OrderedInterceptor {
    OrderedInterceptor {
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
        .head_interceptor(ordered(
            "server-head-global:before",
            "server-head-global:after",
            InterceptionStage::ServerHead,
            Side::Server,
            &events,
        ))
        .interceptor(ordered(
            "server-call-global:before",
            "server-call-global:after",
            InterceptionStage::ServerCall,
            Side::Server,
            &events,
        ))
        .interface(
            InterceptorContractServer::new(OrderedHandler {
                events: events.clone(),
            })
            .head_interceptor(ordered(
                "server-head-local:before",
                "server-head-local:after",
                InterceptionStage::ServerHead,
                Side::Server,
                &events,
            ))
            .interceptor(ordered(
                "server-call-local:before",
                "server-call-local:after",
                InterceptionStage::ServerCall,
                Side::Server,
                &events,
            )),
        )
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    let runtime = ClientRuntime::builder()
        .interceptor(ordered(
            "client-call-global:before",
            "client-call-global:after",
            InterceptionStage::ClientCall,
            Side::Client,
            &events,
        ))
        .attempt_interceptor(ordered(
            "client-attempt-global:before",
            "client-attempt-global:after",
            InterceptionStage::ClientAttempt,
            Side::Client,
            &events,
        ))
        .build()
        .unwrap();
    let client = InterceptorContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .interceptor(ordered(
            "client-call-local:before",
            "client-call-local:after",
            InterceptionStage::ClientCall,
            Side::Client,
            &events,
        ))
        .attempt_interceptor(ordered(
            "client-attempt-local:before",
            "client-attempt-local:after",
            InterceptionStage::ClientAttempt,
            Side::Client,
            &events,
        ))
        .connect()
        .await
        .unwrap();

    let response = client
        .execute(Call::new(), "complete".to_owned())
        .await
        .unwrap();
    assert_eq!(response.body(), "complete");
    assert_eq!(response.attempts(), 1);
    assert_eq!(
        response
            .headers()
            .get_all("x-interceptor-count")
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

impl InterceptorContract for RetryHandler {
    async fn execute(&self, _call: Call, value: String) -> Result<Response<String>, Error> {
        if self.0.fetch_add(1, Ordering::AcqRel) == 0 {
            return Err(
                Error::local(ErrorCategory::Unavailable, "retry_once", "retry once").unwrap(),
            );
        }
        Ok(Response::new(value))
    }
}

#[derive(Clone)]
struct LogicalExtension;

impl Interceptor for LogicalExtension {
    fn intercept<'a>(&'a self, mut context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        context.extensions_mut().insert(ClientExtension(41));
        Box::pin(async move { next.run(context).await })
    }
}

#[derive(Clone)]
struct AttemptIsolation {
    seen: Arc<Mutex<Vec<(u8, u8)>>>,
}

impl Interceptor for AttemptIsolation {
    fn intercept<'a>(&'a self, mut context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
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
        .interface(InterceptorContractServer::new(RetryHandler(
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
        .interceptor(LogicalExtension)
        .attempt_interceptor(AttemptIsolation { seen: seen.clone() })
        .build()
        .unwrap();
    let client = InterceptorContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .connect()
        .await
        .unwrap();

    let response = client
        .execute(Call::new(), "retried".to_owned())
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

impl Interceptor for RejectHead {
    fn intercept<'a>(&'a self, _context: Context, _next: Next<'a>) -> InterceptorFuture<'a> {
        Box::pin(async {
            let mut error = Error::local(
                ErrorCategory::PermissionDenied,
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

impl InterceptorContract for UnreachableHandler {
    async fn execute(&self, _call: Call, _value: String) -> Result<Response<String>, Error> {
        panic!("ServerHead rejection must not invoke the handler")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_head_rejection_does_not_poll_the_request_body() {
    let server = Server::builder("127.0.0.1:0")
        .interface(InterceptorContractServer::new(UnreachableHandler).head_interceptor(RejectHead))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    let mut stream = TcpStream::connect(server.local_addr()).await.unwrap();
    let head = concat!(
        "PUT /interceptor HTTP/1.1\r\n",
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

#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, fusen_rs::SensitiveFields,
)]
struct ProjectionReply {
    #[sensitive(kind = "public")]
    visible: String,
    #[sensitive(kind = "secret")]
    secret: String,
}

#[interface(name = "interceptor-projection-contract")]
trait InterceptorProjectionContract {
    #[fusen_rs::method(method = "PUT", path = "/interceptor/projection")]
    async fn project(
        &self,
        #[param(context)] call: Call,
        #[param(body)]
        #[sensitive(kind = "public")]
        value: String,
    ) -> Result<Response<ProjectionReply>, Error>;
}

struct ProjectionHandler;

impl InterceptorProjectionContract for ProjectionHandler {
    async fn project(
        &self,
        _call: Call,
        value: String,
    ) -> Result<Response<ProjectionReply>, Error> {
        Ok(Response::new(ProjectionReply {
            visible: value,
            secret: "server-secret".to_owned(),
        }))
    }
}

#[derive(Default)]
struct ProjectionObservations {
    server: Vec<Option<Value>>,
    client_remote: Vec<Option<Value>>,
    client_short: Vec<Option<Value>>,
}

fn observed_value(value: SanitizedValue) -> Option<Value> {
    if value.is_omitted() {
        None
    } else {
        Some(serde_json::to_value(value).expect("sanitized projections serialize safely"))
    }
}

#[derive(Clone)]
struct ServerProjectionCapture(Arc<Mutex<ProjectionObservations>>);

impl Interceptor for ServerProjectionCapture {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        assert_eq!(context.stage(), InterceptionStage::ServerCall);
        let method = context.method();
        let observations = self.0.clone();

        Box::pin(async move {
            let response = next.run(context).await?;
            let projected =
                observed_value(response.sanitized_body(method, &PolicySanitizer::default()));
            observations.lock().unwrap().server.push(projected);
            Ok(response)
        })
    }
}

#[derive(Clone)]
struct ClientProjectionCapture(Arc<Mutex<ProjectionObservations>>);

impl Interceptor for ClientProjectionCapture {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        assert_eq!(context.stage(), InterceptionStage::ClientCall);
        let method = context.method();
        let short_circuit = context.headers().contains_key("x-projection-short");
        let observations = self.0.clone();

        Box::pin(async move {
            if short_circuit {
                let response = context.respond(ProjectionReply {
                    visible: "local".to_owned(),
                    secret: "local-secret".to_owned(),
                })?;
                let projected =
                    observed_value(response.sanitized_body(method, &PolicySanitizer::default()));
                observations.lock().unwrap().client_short.push(projected);
                return Ok(response);
            }

            let response = next.run(context).await?;
            let projected =
                observed_value(response.sanitized_body(method, &PolicySanitizer::default()));
            observations.lock().unwrap().client_remote.push(projected);
            Ok(response)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_response_origins_are_projectable_but_context_responses_are_not() {
    let observations = Arc::new(Mutex::new(ProjectionObservations::default()));
    let server = Server::builder("127.0.0.1:0")
        .interceptor(ServerProjectionCapture(observations.clone()))
        .interface(InterceptorProjectionContractServer::new(ProjectionHandler))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    let runtime = ClientRuntime::builder().build().unwrap();
    let client = InterceptorProjectionContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .interceptor(ClientProjectionCapture(observations.clone()))
        .connect()
        .await
        .unwrap();

    let remote = client
        .project(Call::new(), "safe".to_owned())
        .await
        .unwrap();
    assert_eq!(remote.body().visible, "safe");
    assert_eq!(remote.body().secret, "server-secret");

    let mut call = Call::new();
    call.headers_mut()
        .insert("x-projection-short", HeaderValue::from_static("1"));
    let local = client.project(call, "unused".to_owned()).await.unwrap();
    assert_eq!(local.body().visible, "local");

    let expected = Some(json!({
        "visible": "safe",
        "secret": "<redacted>",
    }));
    {
        let observed = observations.lock().unwrap();
        assert_eq!(observed.server, vec![expected.clone()]);
        assert_eq!(observed.client_remote, vec![expected]);
        assert_eq!(observed.client_short, vec![None]);
    }

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, fusen_rs::SensitiveFields,
)]
struct DirectionalPayload {
    #[serde(default, rename(serialize = "display", deserialize = "public_input"))]
    #[sensitive(kind = "public")]
    public_value: String,
    #[serde(rename(serialize = "password", deserialize = "display"))]
    #[sensitive(kind = "secret")]
    secret_value: String,
}

#[interface(name = "directional-projection-contract")]
trait DirectionalProjectionContract {
    #[fusen_rs::method(method = "PUT", path = "/interceptor/directional")]
    async fn directional(
        &self,
        #[param(body)] value: DirectionalPayload,
    ) -> Result<Response<DirectionalPayload>, Error>;
}

struct DirectionalHandler;

impl DirectionalProjectionContract for DirectionalHandler {
    async fn directional(
        &self,
        value: DirectionalPayload,
    ) -> Result<Response<DirectionalPayload>, Error> {
        assert!(value.public_value.is_empty());
        assert_eq!(value.secret_value, "client-visible");
        Ok(Response::new(DirectionalPayload {
            public_value: value.secret_value,
            secret_value: "server-secret".to_owned(),
        }))
    }
}

#[derive(Default)]
struct DirectionalObservations {
    client_arguments: Vec<Option<Value>>,
    server_arguments: Vec<Option<Value>>,
    server_responses: Vec<Option<Value>>,
    client_responses: Vec<Option<Value>>,
}

#[derive(Clone)]
struct DirectionalProjectionCapture(Arc<Mutex<DirectionalObservations>>);

impl Interceptor for DirectionalProjectionCapture {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        let side = context.side();
        let method = context.method();
        let arguments = observed_value(context.sanitized_arguments(&PolicySanitizer::default()));
        let observations = self.0.clone();
        Box::pin(async move {
            let response = next.run(context).await?;
            let body = observed_value(response.sanitized_body(method, &PolicySanitizer::default()));
            let mut observed = observations.lock().unwrap();
            match side {
                Side::Client => {
                    observed.client_arguments.push(arguments);
                    observed.client_responses.push(body);
                }
                Side::Server => {
                    observed.server_arguments.push(arguments);
                    observed.server_responses.push(body);
                }
                _ => panic!("unexpected service invocation side"),
            }
            Ok(response)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn directional_schemas_protect_all_four_projection_paths() {
    let observations = Arc::new(Mutex::new(DirectionalObservations::default()));
    let server = Server::builder("127.0.0.1:0")
        .interceptor(DirectionalProjectionCapture(observations.clone()))
        .interface(DirectionalProjectionContractServer::new(DirectionalHandler))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = DirectionalProjectionContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .interceptor(DirectionalProjectionCapture(observations.clone()))
        .connect()
        .await
        .unwrap();

    let response = client
        .directional(DirectionalPayload {
            public_value: "client-visible".to_owned(),
            secret_value: "client-secret".to_owned(),
        })
        .await
        .unwrap()
        .into_body();
    assert!(response.public_value.is_empty());
    assert_eq!(response.secret_value, "client-visible");

    {
        let observed = observations.lock().unwrap();
        assert_eq!(
            observed.client_arguments,
            [Some(json!({
                "value": {
                    "display": "client-visible",
                    "password": "<redacted>"
                }
            }))]
        );
        assert_eq!(
            observed.server_arguments,
            [Some(json!({"value": {"display": "<redacted>"}}))]
        );
        assert_eq!(
            observed.server_responses,
            [Some(json!({
                "display": "client-visible",
                "password": "<redacted>"
            }))]
        );
        assert_eq!(
            observed.client_responses,
            [Some(json!({"display": "<redacted>"}))]
        );
    }

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}
