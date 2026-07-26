# fusen-rs 0.9

`fusen-rs` is an asynchronous JSON RPC framework for Rust microservices. `#[fusen_trait]` generates a typed client, a service-specific client builder, static method descriptors, and a server wrapper. `#[fusen_service]` binds a Rust implementation to declaration-order `MethodId` dispatch.

The runtime supports HTTP/1.1, HTTP/2, direct endpoints, Nacos registration and discovery, pluggable cluster selection, typed middleware, complete lifecycle observation, bounded bodies, deadlines, Problem Details, and graceful shutdown.

[中文文档](README_CN.md)

## Core Model

```text
ClientRuntime -> Middleware -> Router -> LoadBalancer -> HTTP
Server        -> admission/decode/route -> Middleware -> MethodId dispatch -> encode
```

- One logical RPC performs one HTTP attempt by default. There is no implicit retry.
- `Next` is consuming and cannot be cloned, so downstream executes at most once.
- `InvocationObserver` covers errors, timeouts, and cancellation outside middleware.
- Nacos publishes immutable snapshots. The no-Router path reuses the current snapshot allocation.
- JSON HTTP wire behavior remains identical for Fusen HTTP/2 and SpringCloud HTTP/1.1.

## Define And Implement

```rust,no_run
use fusen_rs::{FusenError, fusen_service, fusen_trait};

#[fusen_trait]
pub trait DemoService {
    async fn say_hello(&self, name: String) -> String;
}

pub struct DemoServiceImpl;

#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn say_hello(&self, name: String) -> Result<String, FusenError> {
        Ok(format!("Hello {name}"))
    }
}
```

Trait declaration order assigns stable `MethodId(u16)` values. Service implementation methods may be written in any order; generated dispatch remains O(1) and does not compare method-name strings.

## Client

One `ClientRuntime` owns connection pools, global middleware, observers, discovery subscriptions, and shutdown state. Generated clients contain only service-local configuration and shared runtime ownership.

```rust,no_run
use fusen_rs::{ClientRuntime, FusenError, fusen_trait};

#[fusen_trait]
trait DemoService {
    async fn say_hello(&self, name: String) -> String;
}

async fn example() -> Result<(), FusenError> {
let runtime = ClientRuntime::builder()
    .build()?;

let client = DemoServiceClient::builder(&runtime)
    .direct("http://127.0.0.1:8081")
    .connect()
    .await?;

let value = client.say_hello("fusen".into()).await?;
runtime.shutdown().await?;
Ok(())
}
```

Discovery uses the same generated builder:

```rust,no_run
use fusen_rs::{ClientRuntime, FusenError, fusen_trait};

#[fusen_trait]
trait DemoService {
    async fn say_hello(&self, name: String) -> String;
}

async fn example() -> Result<(), FusenError> {
let runtime = ClientRuntime::builder().build()?;
let client = DemoServiceClient::builder(&runtime)
    .discover()
    .connect()
    .await?;
runtime.shutdown().await?;
Ok(())
}
```

`shutdown()` is idempotent, closes all runtime-owned subscriptions, and rejects new clients and RPCs. Drop provides best-effort background cleanup, but explicit shutdown is the application contract.

HTTP/1.1 and HTTP/2 pools can be tuned independently on the runtime:

```rust,no_run
use fusen_rs::{ClientRuntime, FusenError, Http1PoolConfig, Http2PoolConfig};

fn example() -> Result<(), FusenError> {
let runtime = ClientRuntime::builder()
    .http1_pool(Http1PoolConfig {
        max_idle_per_host: 256,
        ..Http1PoolConfig::default()
    })
    .http2_pool(Http2PoolConfig {
        connections_per_host: 4,
        ..Http2PoolConfig::default()
    })
    .build()?;
Ok(())
}
```

`max_idle_per_host` limits retained idle HTTP/1.1 connections, not concurrent in-use connections; zero disables HTTP/1.1 reuse. HTTP/2 connection shards are opened lazily per endpoint and selected by a lock-free stable hash of the endpoint and request ID.

## Server

Register an implementation directly. Use the generated `*Server` wrapper only when a service needs local middleware.

```rust,no_run
use fusen_rs::{FusenError, Server, fusen_service, fusen_trait};

#[fusen_trait]
trait DemoService {
    async fn ping(&self) -> String;
}

struct DemoServiceImpl;

#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn ping(&self) -> Result<String, FusenError> {
        Ok("pong".into())
    }
}

async fn example() -> Result<(), FusenError> {
Server::bind("0.0.0.0:8081")
    .service(DemoServiceImpl)
    .run()
    .await?;
Ok(())
}
```

Server startup validates services and routes, binds the listener, then registers providers transactionally. Routes are pre-bound to a static descriptor, immutable middleware slice, and service invoker. Admission is fail-fast and one absolute deadline covers decode, route, middleware, service dispatch, and response encode.

`Server::run` shuts down on SIGINT or SIGTERM on Unix and on Ctrl-C elsewhere. It closes the listener first, then deregisters providers in reverse order while active Hyper connections drain; both operations share the single `graceful_shutdown_timeout` budget. Accept, deregistration, and shutdown timeout failures are returned to the caller. `run_with_shutdown` is controlled only by its supplied future. If that Server future is cancelled after registration is tracked, bounded background deregistration runs while its Tokio runtime remains available.

## Middleware

Users implement one trait on both sides. No registration macro, string ID, boxed future, or terminal implementation is required.

```rust,no_run
use fusen_rs::{Middleware, Next, RpcContext, RpcResult};

struct AuthMiddleware;

impl Middleware for AuthMiddleware {
    async fn handle<'a>(&'a self, mut context: RpcContext, next: Next<'a>) -> RpcResult {
        context.metadata_mut().insert("tenant".into(), "acme".into());
        next.run(context).await
    }
}
```

Global middleware executes before service-local middleware and unwinds after it. Client middleware runs before routing and load balancing, so it can set tenant, gray-release, or consistent-hash metadata. Provider middleware runs after HTTP route matching. Middleware may return an error or an explicit `RpcResponse` without calling `next`.

Use `InvocationObserver` for complete request logs and metrics. Observer callbacks are synchronous, ordered, exactly-once at finish, and intentionally omit bodies, credentials, and complete headers. Middleware post-processing is not guaranteed after cancellation; use RAII for resource cleanup.

## Cluster Extensions

Advanced client extensions live under `client::cluster`:

- `Router` filters or reorders an `InstanceSnapshot`.
- `LoadBalancer` returns one validated snapshot index.
- The default load balancer uses validated provider weights.

Empty snapshots, Router results with no instances, and invalid selected indexes return `FusenError::ServiceUnavailable`. This phase intentionally provides no retry, backoff, circuit breaker, or multi-attempt API.

## Defaults

- Client connect timeout: 3 seconds; invocation timeout: 10 seconds
- Discovery and subscription cleanup timeout: 5 seconds
- HTTP/1.1: 128 idle connections per host; 90-second idle eviction
- HTTP/2: one connection per host; 90-second idle eviction; keep-alive pings disabled
- Maximum request and response body: 2 MiB
- Server request timeout: 30 seconds
- Maximum requests: 1024; connections: 2048; HTTP/2 streams per connection: 128
- Graceful shutdown total shared budget: 30 seconds; registry operation: 5 seconds

Configure these through `ClientConfig` and `ServerConfig`. Configured durations and required pool sizes must be greater than zero; HTTP/1.1 `max_idle_per_host` may be zero to disable reuse. Non-2xx Problem Details responses become `FusenError::Remote`.

## Examples And Benchmarks

```bash
cargo run -p examples --bin host-server
cargo run -p examples --bin host-client

cargo run --release -p examples --bin host-server-pt
PT_PROTOCOL=both PT_CONCURRENCY=1,100 PT_ROUNDS=5 \
cargo run --release -p examples --bin host-client-pt
```

Nacos examples use `NACOS_ADDR`; the server also uses `FUSEN_ADVERTISED_URL`. See [examples/README.md](examples/README.md) and [the performance baseline](docs/performance-baseline.md).
