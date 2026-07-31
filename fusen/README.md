# fusen-rs

`fusen-rs` is a production-oriented JSON RPC runtime for Rust with HTTP/HTTPS
clients and a plaintext HTTP/1.1/h2c server. It provides generated clients and
server adapters, bounded resource admission, service discovery, retries,
circuit breakers, middleware, metrics, and explicit runtime lifecycles.

Version 0.9 is a clean-slate API and wire reset. It is the first compatibility
baseline and is intentionally incompatible with releases before 0.9.

## Interface declarations

One trait declares the shared client and server contract:

```rust
use fusen_rs::{RpcError, RpcResponse, SensitiveFields};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, SensitiveFields)]
struct User {
    #[sensitive(kind = "identifier")]
    id: String,
}

#[fusen_rs::interface(name = "user", group = "prod", version = "1")]
pub trait UserApi {
    #[fusen_rs::method(
        method = "GET", path = "/users/{id}"
    )]
    async fn get(
        &self,
        #[param(path)] id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<RpcResponse<User>, RpcError>;
}
```

Every RPC requires `#[method(method = "...", path = "...")]`, accepts zero or
more owned named parameters, and returns `Result<RpcResponse<T>, RpcError>`.
Path placeholders are inferred by name, or confirmed explicitly with
`#[param(path)]`; an explicit path wire name must match its placeholder.
Remaining read-method parameters become query values, while remaining
POST/PUT/PATCH parameters become fields in one JSON body object.
`#[param(query)]`, `#[param(body)]`, `#[param(context)]`, and
`#[param(name = "...")]` provide the other explicit overrides. Non-context wire
names remain globally unique across sources. The macro generates
`UserApiClient` and `UserApiServer<T>`; both the generated client and a user
handler implement `UserApi`. Clients use the generic `ClientBuilder<UserApiClient>`.

## Sensitive projections

Request and successful-response DTOs derive the same process-local schema. Nested
DTOs recurse automatically; classify a complete field with
`#[sensitive(kind = "public")]` (or `credential`, `token`, `phone`, `email`,
`identifier`, `secret`, or a validated custom kind), and use
`#[sensitive(opaque)]` for a third-party field that must be omitted. Scalars are
opaque by default. The interface macro discovers request and `RpcResponse<T>`
schemas automatically, so there is no `#[sensitive(response)]` marker.

```rust
use fusen_rs::SensitiveFields;
use serde::Serialize;

#[derive(Serialize, SensitiveFields)]
struct LoginRequest {
    #[sensitive(kind = "public")]
    username: String,
    #[sensitive(kind = "credential")]
    password: String,
    #[sensitive(opaque)]
    vendor_payload: serde_json::Value,
}

#[derive(Serialize, SensitiveFields)]
struct LoginResponse {
    #[sensitive(kind = "identifier")]
    user_id: String,
    #[sensitive(kind = "token")]
    access_token: String,
}
```

Fusen does not log these values. Third-party middleware explicitly calls
`RpcContext::sanitized_arguments(&sanitizer)` before delegating and
`RpcResponse<RpcBody>::sanitized_body(method, &sanitizer)` on a successful
response. `PolicySanitizer` reveals only `public`, redacts predefined sensitive
kinds, and omits custom or unclassified values. Missing metadata, shape errors,
limits, sanitizer panics, and short-circuit responses fail closed to `<omitted>`
without affecting the RPC. Response projection also applies a configurable 64
KiB default input limit before constructing its JSON view.

A `kind` classifies the complete JSON value and intentionally replaces its
underlying structural schema. When a container inherits a kind, the policy sees
the complete null or array once; structured DTO containers recurse per element.
Known Serde shape-changing attributes must be classified or opaque, and
`flatten` requires classifying the complete type.

`SensitiveFields` does not change a DTO's `Debug`, either wire protocol, service
identity, registry, or discovery metadata. Only the returned `SanitizedValue`
is intended for safe `Debug`, `Display`, or structured serialization.

## Runtime lifecycle

Build a shared client runtime and then connect each generated client either to
one direct endpoint or through the configured registry:

```rust
use fusen_rs::{ClientRuntime, WireProtocol};

let runtime = ClientRuntime::builder().build()?;
let client = UserApiClient::builder(&runtime)
    .direct("http://127.0.0.1:8080")
    .protocol(WireProtocol::FusenV1)
    .connect()
    .await?;

// Use `client`, then close shared discovery and transport resources.
runtime.shutdown().await?;
```

Direct and discovered endpoints may use `http://` or `https://`. HTTPS uses
Rustls Ring, TLS 1.2/1.3, bundled Mozilla WebPKI roots, and strict certificate
and hostname verification. System roots, custom CAs, mTLS, verification bypass,
and plaintext fallback are not supported.

A successful wire response whose `result` cannot decode into the generated
method's Rust type terminates as non-retryable `DataLoss`/`invalid_result` and
is recorded as a protocol failure by both endpoint and service breakers.

A server returns only after binding, starting its accept loop, and completing
all configured registrations:

```rust
use fusen_rs::Server;

let server = Server::builder("0.0.0.0:0")
    .interface(UserApiServer::new(handler))
    .build()?;
let running = server.start().await?;
let address = running.local_addr();

// Publish or inspect `address`, then drain requests and registrations.
running.shutdown().await?;
```

`Server::serve()` adds platform shutdown-signal handling. `RunningServer` and
`ServerHandle` expose explicit state, wait, and idempotent shutdown operations.
The built-in listener remains plaintext HTTP/1.1 and h2c. An advertised HTTPS
endpoint must be served by an external TLS terminator forwarding to that listener.

## Wire protocols

| Protocol | Transport and mapping |
|---|---|
| `WireProtocol::FusenV1` | h2c over HTTP or TLS/ALPN `h2` over HTTPS; versioned Fusen JSON |
| `WireProtocol::SpringCloudV1` | HTTP/1.1 over HTTP or HTTPS; explicit method/path/query/body mapping |

The client accepts canonical `http://` and `https://` endpoints. The server does
not terminate TLS; use a sidecar, service mesh, ingress, or reverse proxy for
inbound HTTPS.

The supported extension surface is `Middleware`, `Registry`, `ConfigSource`,
`InstanceRouter`, `LoadBalancer`, `RetryPolicy`, and `MetricsRecorder`. Transports, codecs,
acceptors, connection pools, and lifecycle state machines are private runtime
implementation details.

Requires Rust 1.97 or newer. Licensed under Apache-2.0.
