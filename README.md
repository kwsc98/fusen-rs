# fusen-rs 0.9

`fusen-rs` is a production-oriented asynchronous JSON RPC framework for Rust microservices. The 0.9 line is a clean-slate API and wire baseline: it provides generated clients and servers, explicit lifecycle ownership, bounded resource use, service discovery, retries, circuit breakers, middleware, and structured observability.

[中文文档](README_CN.md)

## Requirements And Scope

- Rust 1.97, Edition 2024, Tokio, and JSON.
- Plain HTTP only: Fusen V1 uses HTTP/2 prior knowledge (h2c), and Spring Cloud V1 uses HTTP/1.1.
- `https://` endpoints are rejected during validation, before network I/O. Terminate TLS at an ingress, sidecar, reverse proxy, or service mesh.
- The stable extension surface is limited to `Middleware`, `Registry`, `Router`, `LoadBalancer`, `RetryPolicy`, and `MetricsRecorder`.
- Transport, codecs, acceptors, connection pools, and lifecycle state machines are runtime internals.

## Service Contract

One trait macro defines the service. Every RPC method declares its retry semantics and returns `Result<T, RpcError>`.

```rust,no_run
use fusen_rs::{RpcError, method, service};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct User { pub id: String }

#[derive(Serialize, Deserialize)]
pub struct CreateUser { pub id: String }

#[service(name = "user", group = "prod", version = "1")]
pub trait UserService {
    #[method(
        idempotency = "safe",
        spring(method = "GET", path = "/users/{id}", query = ["expand"])
    )]
    async fn get(&self, id: String, expand: Option<bool>) -> Result<User, RpcError>;

    #[method(
        idempotency = "none",
        spring(method = "POST", path = "/users", body = "request")
    )]
    async fn create(&self, request: CreateUser) -> Result<User, RpcError>;
}
```

The macro generates `UserServiceClient`, `UserServiceClientBuilder`, and `UserServiceServer`. Implementations directly implement the generated trait; there is no implementation macro. Idempotency defaults to `none` and is never inferred from the HTTP method. Spring path parameters come from `{name}` placeholders, while query and body sources must be listed explicitly. Spring HEAD mappings return `Result<(), RpcError>` because HEAD has no response body. Fusen V1 always encodes every argument by name.

## Client

`ClientRuntime` owns admission, byte budgets, middleware, discovery subscriptions, connection pools, retry budgets, and circuit breakers.

```rust,no_run
# use fusen_rs::{ClientError, ClientRuntime, WireProtocol};
# use crate::UserServiceClient;
# async fn run() -> Result<(), ClientError> {
let runtime = ClientRuntime::builder().build()?;

let client = UserServiceClient::builder(&runtime)
    .direct("http://127.0.0.1:8081")
    .protocol(WireProtocol::FusenV1)
    .connect()
    .await?;

// Generated RPC methods return Result<T, RpcError>.
runtime.shutdown().await?;
# Ok(())
# }
```

Use `.discover()` instead of `.direct(...)` after installing one `Registry` on the runtime builder. Discovery is shared per `(ServiceSelector, WireProtocol)` and exposes latest-wins snapshots with `Initializing`, `Ready`, `Stale`, `Unavailable`, and `Closed` states.

One absolute deadline covers admission, middleware, every attempt, backoff, transport, and decode. Only methods declared `idempotent` or `safe` can retry. The built-in policy permits at most three total attempts and is constrained by a per-service retry budget. Endpoint and service circuit breakers, endpoint bulkheads, and fresh discovery snapshots are applied on each physical attempt.

If a successful HTTP/wire response cannot decode its `result` into the generated method's Rust type, the call terminates without retry as `DataLoss` with code `invalid_result`. That selected endpoint attempt and the final service outcome are both recorded as protocol failures by their circuit breakers.

`ClientRuntime::shutdown()` is idempotent. It first closes admission, then drains logical calls while closing subscriptions and pools under one shared deadline. Cancelling a shutdown waiter does not cancel the coordinator.

## Server

```rust,no_run
# use fusen_rs::{Server, ServerError};
# use crate::{UserServiceServer, UserServiceImpl};
# async fn run() -> Result<(), ServerError> {
let server = Server::builder("0.0.0.0:0")
    .service(UserServiceServer::new(UserServiceImpl))
    .build()?;

let running = server.start().await?;
println!("listening on {}", running.local_addr());
running.shutdown().await?;
# Ok(())
# }
```

`start()` binds first, starts accepting in not-ready mode, activates registrations, and returns only after the server becomes `Ready`. Before readiness, requests receive a non-retryable `503 not_ready` without polling their body. `RunningServer` exposes `local_addr()`, `state()`, `handle()`, `wait()`, and `shutdown()`; shutdown through any handle is idempotent and shares one terminal result. `Server::serve()` adds platform signal handling.

Shutdown closes readiness and the listener first, then deregisters providers, asks Hyper connections to drain, and waits for active requests concurrently under one absolute deadline. Deadline exhaustion cancels remaining work and returns a `ServerError` without an unbounded task-reaping wait.

## Wire V1

Fusen V1 uses:

```text
POST /_fusen/v1/{service}/{method}
Content-Type: application/fusen+json;version=1
{"arguments":{"name":...}}
{"result":...}
```

Spring Cloud V1 uses each method's explicit HTTP method, path, query, and body mapping with `application/json`; successful responses contain raw JSON. It is a documented subset, not complete Spring MVC compatibility.

Both protocols use `x-request-id`, `x-fusen-timeout-ms`, and `x-fusen-attempt`. Errors use `application/problem+json` with RFC 9457 fields plus `code`, `request_id`, and `retryable`. Internal sources and panic payloads never cross the wire.

## Production Defaults

| Limit | Client | Server |
| --- | ---: | ---: |
| Request deadline | 10 s | 30 s maximum |
| Shutdown deadline | 30 s | 30 s |
| Connect/startup | 3 s | 30 s |
| Registry operation | 5 s | 5 s |
| Concurrent requests | 1024 | 1024 |
| Per-endpoint attempts | 128 | - |
| Request/response body | 2 MiB each | 2 MiB each |
| Request/response byte budget | 64 MiB each | 64 MiB each |
| TCP connections | - | 2048 |
| H2 streams per connection | - | 128 |

Queues are disabled by default. `QueueConfig::bounded(capacity)` enables a bounded queue whose wait remains part of the logical deadline. Admission and byte budgets otherwise fail fast.

Byte budgets cover decoded/encoded payload retained by the runtime and queued body chunks until Hyper consumes or cancels them. Protocol framing, HPACK/H2 codec staging, and OS socket buffers are separately bounded transport overhead and are not charged to body budgets.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `fusen-contract` | Pure service, method, protocol, endpoint, and instance values |
| `fusen-register` | Registry SPI, lifecycle handles, and directory snapshots |
| `fusen-config` | Static parsing and last-good hot configuration |
| `fusen-nacos` | Nacos registry and configuration adapters |
| `fusen-observability` | Metrics SPI and optional telemetry adapters |
| `fusen-procedural-macro` | Service declaration and generated wrappers |
| `fusen-rs` | Client/server runtimes, middleware, policy, and plaintext HTTP |

See [architecture](docs/architecture.md), [module contracts](docs/modules/README.md), [compatibility](docs/compatibility.md), and [examples](examples/README.md).
