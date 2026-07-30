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
use fusen_rs::{RpcError, RpcRequest, RpcResponse};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, fusen_rs::RpcMessage)]
pub struct GetUserRequest {
    #[rpc(path)]
    pub id: String,
    #[rpc(query)]
    pub expand: Option<bool>,
}

#[fusen_rs::interface(name = "user", group = "prod", version = "1")]
pub trait UserApi {
    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/users/{id}")
    )]
    async fn get(
        &self,
        request: RpcRequest<GetUserRequest>,
    ) -> Result<RpcResponse<User>, RpcError>;
}
```

Every RPC accepts exactly one `RpcRequest<T>` and returns
`Result<RpcResponse<T>, RpcError>`. The macro generates `UserApiClient` and
`UserApiServer<T>`; both the generated client and a user handler implement
`UserApi`. Clients use the generic `ClientBuilder<UserApiClient>`.

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
