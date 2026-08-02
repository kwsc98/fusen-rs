# fusen-rs 0.9

`fusen-rs` is a production-oriented asynchronous microservice and service invocation framework for Rust. The 0.9 line is a clean-slate API and HTTP binding baseline: it provides generated clients and servers, explicit lifecycle ownership, bounded resource use, service discovery, retries, circuit breakers, interceptors, and structured observability.

[中文文档](README_CN.md)

## Requirements And Scope

- Rust 1.97, Edition 2024, Tokio, and JSON.
- Clients support canonical `http://` and `https://` endpoints. The stable `http-json-v1` binding is independent from HTTP transport selection; endpoints advertise supported bindings, HTTP versions, and invocation controls as capabilities.
- Client HTTPS uses Rustls Ring, TLS 1.2/1.3, bundled Mozilla WebPKI roots, and strict certificate/hostname validation. The built-in server remains plaintext; terminate inbound TLS at an ingress, sidecar, reverse proxy, or service mesh.
- The stable extension surface is limited to `Interceptor`, `Registry`, `ConfigSource`, `InstanceRouter`, `LoadBalancer`, `RetryPolicy`, `MetricsRecorder`, `Sanitizer`, and the client-side `RequestEncoder`/`ResponseDecoder`/`ErrorDecoder` binding codecs.
- HTTP transport, server codecs, acceptors, connection pools, and lifecycle state machines are runtime internals.

## Interface Contract

One trait macro defines the shared client/server interface. Every service method accepts zero or more owned, named parameters and returns `Result<Response<T>, Error>`.

```rust,no_run
use fusen_rs::{Error, Response, SensitiveFields, interface};
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
    ) -> Result<Response<User>, Error>;

    #[fusen_rs::method(
        method = "POST", path = "/users"
    )]
    async fn create(
        &self,
        user: CreateUser,
        audit: bool,
    ) -> Result<Response<User>, Error>;
}
```

The macro generates `UserApiClient` and `UserApiServer<T>`. The generated client and user handler both implement `UserApi`; all clients use the generic `ClientBuilder<UserApiClient>`. Every interface method requires `#[method(method = "...", path = "...")]`; the generated client uses it to build requests, the generated server uses it to route requests, and retry eligibility follows the standard HTTP method.

Generated code uses the doc-hidden, versioned `fusen_rs::__macro::v1` ABI. It
is not an application extension API, but all `fusen-procedural-macro` and
`fusen-rs` 0.9.x combinations permitted by Cargo remain compile-compatible,
including when the runtime dependency is renamed.

Parameter locations are inferred deterministically. A parameter whose wire name matches a `{placeholder}` is a path parameter. Other GET, HEAD, OPTIONS, and DELETE parameters are query parameters. Other POST, PUT, and PATCH parameters become fields in one synthesized JSON body object, so `create(user, audit)` sends `{"user": ..., "audit": ...}` even when only one body field exists. Use `#[param(path)]` to explicitly confirm a path parameter; its wire name must match the corresponding placeholder. Use `#[param(query)]` or `#[param(query, repeated)]` for query values, `#[param(header)]` and `#[param(cookie)]` for HTTP metadata, `#[param(query_map)]` and `#[param(header_map)]` for dynamic maps, `#[param(body_field)]` for an explicit field in the synthesized JSON object, `#[param(body)]` for one complete raw JSON body, `#[param(context)]` for a non-wire `Call`, and `#[param(name = "...")]` to rename a wire parameter. `body_field` accepts `name` but not `repeated`. Every non-context wire name must remain unique where the source is named. A raw body cannot coexist with inferred or explicit body fields. The `http-json-v1` binding sends the declared HTTP operation directly and returns raw JSON; it has no private Fusen request or response envelope. Invalid mappings fail during macro expansion, and invalid serialized values fail locally before network I/O.

Custom request and response DTOs derive `SensitiveFields`. Fields use `#[sensitive(kind = "...")]` or `#[sensitive(opaque)]`; the interface macro discovers both request and `Response<T>` schemas without a response marker. Fusen does not log payloads automatically: third-party interceptors explicitly call `sanitized_arguments` and `sanitized_body`, which fail closed without changing DTO `Debug`, wire bytes, or registry/discovery metadata. See [Interceptor and macro behavior](docs/modules/interceptor-macros.md).

## Client

`ClientRuntime` owns admission, byte budgets, interceptors, discovery subscriptions, connection pools, retry budgets, and circuit breakers.

```rust,no_run
# use fusen_rs::{ClientError, ClientRuntime};
# use crate::UserApiClient;
# async fn run() -> Result<(), ClientError> {
let runtime = ClientRuntime::builder().build()?;

let client = UserApiClient::builder(&runtime)
    .direct("http://127.0.0.1:8081")
    .connect()
    .await?;

// Generated service methods return Result<Response<T>, Error>.
runtime.shutdown().await?;
# Ok(())
# }
```

Use an `https://` direct endpoint, or discover an HTTPS instance from a registry,
to enable client TLS. The runtime does not read the system trust store and does
not expose custom CA, mTLS, or certificate-verification bypasses; private CA and
self-signed endpoints are outside the 0.9 contract.

Use `.discover()` instead of `.direct(...)` after installing one `Registry` on the runtime builder. Discovery is shared per `ServiceSelector` and exposes latest-wins snapshots with `Initializing`, `Ready`, `Stale`, `Unavailable`, and `Closed` states. Each discovered `ServiceInstance` carries `EndpointCapabilities`; the client filters instances by the required `HttpBindingId` and applies `HttpVersionPolicy` only when opening the selected endpoint. Registry subscription identity is therefore independent from binding and transport policy.

Generated client builders use `.binding(...)` to select a representation and `.http_version_policy(...)` to select the transport policy. Without `.direct_capabilities(...)`, a direct endpoint is assumed to support the client-selected binding with invocation controls disabled; `http://` plus `Auto` uses HTTP/1.1, while `https://` plus `Auto` negotiates HTTP/2 or HTTP/1.1 through ALPN. Set `.direct_capabilities(...)` when the operator needs to replace that inference with an explicit binding, version, and controls contract.

One absolute deadline covers admission, interceptors, every attempt, backoff, transport, and decode. Retry eligibility is derived conservatively from the declared HTTP method: GET, HEAD, OPTIONS, PUT, and DELETE may retry; POST and PATCH never retry automatically. The built-in policy permits at most three total attempts and is constrained by a per-service retry budget. Endpoint and service circuit breakers, endpoint bulkheads, and fresh discovery snapshots are applied on each physical attempt.

If a successful HTTP response cannot decode its raw JSON body into the generated method's Rust type, the call terminates without retry as `DataLoss` with code `invalid_result`. That selected endpoint attempt and the final service outcome are both recorded as protocol failures by their circuit breakers.

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

## HTTP Binding

`HttpOperation` accepts any syntactically valid MIME value so custom bindings
can define their own representation families. `http-json-v1` maps every service
method to its declared HTTP operation and preflights both `consumes` and
`produces` when a client or server is built: it accepts `application/json` and
concrete `application/<subtype>+json` media types, including parameters, and
rejects other MIME families locally before network I/O. JSON request fields and
raw JSON success responses use `application/json` by default.

```text
POST /users
Content-Type: application/json
Accept: application/json

{"user":{"id":"42"},"audit":true}
{"id":"42"}
```

`HttpBindingId` identifies this representation in registry metadata and telemetry; `HTTP_JSON_V1` is its stable string identifier. It does not select HTTP/1.1 versus HTTP/2. `HttpVersionPolicy::{Auto, Http1, Http2, H2c}` expresses the client transport preference, while `EndpointCapabilities` advertises the versions and bindings an endpoint actually supports. `EndpointCapabilities::default()` is HTTP/1.1, `http-json-v1`, and no Fusen invocation controls; registry conventions may explicitly use it for missing metadata. An endpoint passed directly without declared capabilities instead uses the selected binding with controls disabled, HTTP/1.1 for `http://` plus `Auto`, and ALPN-negotiated HTTP/2 or HTTP/1.1 for `https://` plus `Auto`.

Additional client-only bindings register their `RequestEncoder`, `ResponseDecoder`, and `ErrorDecoder` through `ClientRuntimeBuilder::http_binding(...)`, then select the same `HttpBindingId` on the generated client builder. These codecs receive bounded HTTP semantic data and do not own transport or lifecycle resources. The built-in Server intentionally serves only `http-json-v1`.

`ConfigSource` is likewise a stable 0.9 provider SPI. Third-party providers use
`fusen-config`'s public key, handle, error, and safe lifecycle constructor types;
their SDK clients, channels, workers, and cleanup coordinators stay private.

Built-in errors use `application/problem+json` with RFC 9457 fields plus `code`, `request_id`, and `retryable`; the client also accepts valid external RFC Problem types. Fusen timeout and attempt headers are sent only when the selected endpoint advertises invocation controls. Internal sources and panic payloads never cross the wire.

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
| `fusen-contract` | Pure service, HTTP binding, capability, endpoint, and instance values |
| `fusen-register` | Registry SPI, lifecycle handles, and directory snapshots |
| `fusen-config` | Static parsing and last-good hot configuration |
| `fusen-nacos` | Nacos registry and configuration adapters |
| `fusen-observability` | Metrics SPI and optional telemetry adapters |
| `fusen-procedural-macro` | Interface declaration, parameter validation, and generated wrappers |
| `fusen-rs` | HTTP/HTTPS client, plaintext HTTP server, interceptor, and policy runtimes |

See [architecture](docs/architecture.md), [module contracts](docs/modules/README.md), [compatibility](docs/compatibility.md), and [examples](examples/README.md).
