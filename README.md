# fusen-rs 0.9

`fusen-rs` is a production-oriented asynchronous JSON RPC framework for Rust microservices. The 0.9 line is a clean-slate API and wire baseline: it provides generated clients and servers, explicit lifecycle ownership, bounded resource use, service discovery, retries, circuit breakers, middleware, and structured observability.

[中文文档](README_CN.md)

## Requirements And Scope

- Rust 1.97, Edition 2024, Tokio, and JSON.
- Clients support canonical `http://` and `https://` endpoints. Fusen V1 uses h2c over HTTP and TLS/ALPN `h2` over HTTPS; Spring Cloud V1 uses HTTP/1.1 over either scheme.
- Client HTTPS uses Rustls Ring, TLS 1.2/1.3, bundled Mozilla WebPKI roots, and strict certificate/hostname validation. The built-in server remains plaintext; terminate inbound TLS at an ingress, sidecar, reverse proxy, or service mesh.
- The stable extension surface is limited to `Middleware`, `Registry`, `ConfigSource`, `InstanceRouter`, `LoadBalancer`, `RetryPolicy`, and `MetricsRecorder`.
- Transport, codecs, acceptors, connection pools, and lifecycle state machines are runtime internals.

## Interface Contract

One trait macro defines the shared client/server interface. Every RPC method accepts zero or more owned, named parameters and returns `Result<RpcResponse<T>, RpcError>`.

```rust,no_run
use fusen_rs::{RpcError, RpcResponse, SensitiveFields, interface};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, SensitiveFields)]
pub struct User {
    #[sensitive(kind = "identifier")]
    pub id: String,
}

#[derive(Serialize, Deserialize, SensitiveFields)]
pub struct CreateUser {
    #[sensitive(kind = "identifier")]
    pub id: String,
}

#[interface(name = "user", group = "prod", version = "1")]
pub trait UserApi {
    #[fusen_rs::method(
        method = "GET", path = "/users/{id}"
    )]
    async fn get(
        &self,
        #[param(path)] id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<RpcResponse<User>, RpcError>;

    #[fusen_rs::method(
        method = "POST", path = "/users"
    )]
    async fn create(
        &self,
        user: CreateUser,
        audit: bool,
    ) -> Result<RpcResponse<User>, RpcError>;
}
```

The macro generates `UserApiClient` and `UserApiServer<T>`. The generated client and user handler both implement `UserApi`; all clients use the generic `ClientBuilder<UserApiClient>`. Every interface method requires `#[method(method = "...", path = "...")]`; the generated client uses it to build requests, the generated server uses it to route requests, and retry eligibility follows the standard HTTP method.

Parameter locations are inferred deterministically. A parameter whose wire name matches a `{placeholder}` is a path parameter. Other GET, HEAD, OPTIONS, and DELETE parameters are query parameters. Other POST, PUT, and PATCH parameters become fields in one synthesized JSON body object, so `create(user, audit)` sends `{"user": ..., "audit": ...}` even when only one body field exists. Use `#[param(path)]` to explicitly confirm a path parameter; its wire name must match the corresponding placeholder. Use `#[param(query)]` to override the default, `#[param(query, repeated)]` for a repeated-key query array, `#[param(body)]` for one complete raw JSON body, `#[param(context)]` for a non-wire `RpcCall`, and `#[param(name = "...")]` to rename a wire parameter. Every non-context wire name must remain unique, even across different sources. A raw body cannot coexist with inferred body fields. Fusen V1 always encodes every business parameter by name in its `arguments` object. Invalid mappings fail during macro expansion, and invalid serialized values fail locally before network I/O.

Custom request and response DTOs derive `SensitiveFields`. Fields use `#[sensitive(kind = "...")]` or `#[sensitive(opaque)]`; the interface macro discovers both request and `RpcResponse<T>` schemas without a response marker. Fusen does not log payloads automatically: third-party middleware explicitly calls `sanitized_arguments` and `sanitized_body`, which fail closed without changing DTO `Debug`, wire bytes, or registry/discovery metadata. See [Middleware and macro behavior](docs/modules/middleware-macros.md).

## Client

`ClientRuntime` owns admission, byte budgets, middleware, discovery subscriptions, connection pools, retry budgets, and circuit breakers.

```rust,no_run
# use fusen_rs::{ClientError, ClientRuntime, WireProtocol};
# use crate::UserApiClient;
# async fn run() -> Result<(), ClientError> {
let runtime = ClientRuntime::builder().build()?;

let client = UserApiClient::builder(&runtime)
    .direct("http://127.0.0.1:8081")
    .protocol(WireProtocol::FusenV1)
    .connect()
    .await?;

// Generated RPC methods return Result<RpcResponse<T>, RpcError>.
runtime.shutdown().await?;
# Ok(())
# }
```

Use an `https://` direct endpoint, or discover an HTTPS instance from a registry,
to enable client TLS. The runtime does not read the system trust store and does
not expose custom CA, mTLS, or certificate-verification bypasses; private CA and
self-signed endpoints are outside the 0.9 contract.

Use `.discover()` instead of `.direct(...)` after installing one `Registry` on the runtime builder. Discovery is shared per `(ServiceSelector, WireProtocol)` and exposes latest-wins snapshots with `Initializing`, `Ready`, `Stale`, `Unavailable`, and `Closed` states.

One absolute deadline covers admission, middleware, every attempt, backoff, transport, and decode. Retry eligibility is derived conservatively from the declared HTTP method: GET, HEAD, OPTIONS, PUT, and DELETE may retry; POST and PATCH never retry automatically. The built-in policy permits at most three total attempts and is constrained by a per-service retry budget. Endpoint and service circuit breakers, endpoint bulkheads, and fresh discovery snapshots are applied on each physical attempt.

If a successful HTTP/wire response cannot decode its `result` into the generated method's Rust type, the call terminates without retry as `DataLoss` with code `invalid_result`. That selected endpoint attempt and the final service outcome are both recorded as protocol failures by their circuit breakers.

`ClientRuntime::shutdown()` is idempotent. It first closes admission, then drains logical calls while closing subscriptions and pools under one shared deadline. Cancelling a shutdown waiter does not cancel the coordinator.

## Server

```rust,no_run
# use fusen_rs::{Server, ServerError};
# use crate::{UserApiServer, UserApiHandler};
# async fn run() -> Result<(), ServerError> {
let server = Server::builder("0.0.0.0:0")
    .interface(UserApiServer::new(UserApiHandler))
    .build()?;

let running = server.start().await?;
println!("listening on {}", running.local_addr());
running.shutdown().await?;
# Ok(())
# }
```

`start()` binds first, starts accepting in not-ready mode, activates registrations, and returns only after the server becomes `Ready`. Before readiness, requests receive a non-retryable `503 not_ready` without polling their body. `RunningServer` exposes `local_addr()`, `state()`, `handle()`, `wait()`, and `shutdown()`; shutdown through any handle is idempotent and shares one terminal result. `Server::serve()` adds platform signal handling.

The built-in listener accepts plaintext HTTP/1.1 and h2c only. An explicit
`.advertised_endpoint("https://...")` publishes an address served by an external
TLS terminator; it does not enable TLS on the local listener.

Shutdown closes readiness and the listener first, then deregisters providers, asks Hyper connections to drain, and waits for active requests concurrently under one absolute deadline. Deadline exhaustion cancels remaining work and returns a `ServerError` without an unbounded task-reaping wait.

## Wire V1

Fusen V1 uses h2c for `http://` and TLS/ALPN `h2` for `https://`:

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

Queues are disabled by default. Configure `QueueConfig::builder().capacity(...).max_wait(...).build()?` and install it through `ClientAdmissionConfigBuilder` to enable a bounded queue; its wait remains part of the logical deadline. Admission and byte budgets otherwise fail fast.

Byte budgets cover decoded/encoded payload retained by the runtime and queued body chunks until Hyper consumes or cancels them. Protocol framing, HPACK/H2 codec staging, and OS socket buffers are separately bounded transport overhead and are not charged to body budgets.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `fusen-contract` | Pure service, method, protocol, endpoint, and instance values |
| `fusen-register` | Registry SPI, lifecycle handles, and directory snapshots |
| `fusen-config` | Static parsing and last-good hot configuration |
| `fusen-nacos` | Nacos registry and configuration adapters |
| `fusen-observability` | Metrics SPI and optional telemetry adapters |
| `fusen-procedural-macro` | Interface declaration, parameter validation, and generated wrappers |
| `fusen-rs` | HTTP/HTTPS client, plaintext HTTP server, middleware, and policy runtimes |

See [architecture](docs/architecture.md), [module contracts](docs/modules/README.md), [compatibility](docs/compatibility.md), and [examples](examples/README.md).
