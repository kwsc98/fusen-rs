# fusen-rs

`fusen-rs` is a production-oriented JSON RPC runtime for Rust over plaintext
HTTP/1.1 and HTTP/2 prior knowledge (h2c). It provides generated clients and
server adapters, bounded resource admission, service discovery, retries,
circuit breakers, middleware, metrics, and explicit runtime lifecycles.

Version 0.9 is a clean-slate API and wire reset. It is the first compatibility
baseline and is intentionally incompatible with releases before 0.9.

## Service declarations

One trait declares the shared client and server contract:

```rust
use fusen_rs::RpcError;

#[fusen_rs::service(name = "user", group = "prod", version = "1")]
pub trait UserService {
    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/users/{id}", query = ["expand"])
    )]
    async fn get(
        &self,
        id: String,
        expand: Option<bool>,
    ) -> Result<User, RpcError>;
}
```

Every RPC must explicitly return `Result<T, RpcError>`. The macro generates
`UserServiceClient`, `UserServiceClientBuilder`, and `UserServiceServer`.
Server implementations implement `UserService` directly. Spring HEAD mappings
must return `Result<(), RpcError>` because HEAD responses have no body.

## Runtime lifecycle

Build a shared client runtime and then connect each generated client either to
one direct endpoint or through the configured registry:

```rust
use fusen_rs::{ClientRuntime, WireProtocol};

let runtime = ClientRuntime::builder().build()?;
let client = UserServiceClient::builder(&runtime)
    .direct("http://127.0.0.1:8080")
    .protocol(WireProtocol::FusenV1)
    .connect()
    .await?;

// Use `client`, then close shared discovery and transport resources.
runtime.shutdown().await?;
```

A successful wire response whose `result` cannot decode into the generated
method's Rust type terminates as non-retryable `DataLoss`/`invalid_result` and
is recorded as a protocol failure by both endpoint and service breakers.

A server returns only after binding, starting its accept loop, and completing
all configured registrations:

```rust
use fusen_rs::Server;

let server = Server::builder("0.0.0.0:0")
    .service(UserServiceServer::new(service))
    .build()?;
let running = server.start().await?;
let address = running.local_addr();

// Publish or inspect `address`, then drain requests and registrations.
running.shutdown().await?;
```

`Server::serve()` adds platform shutdown-signal handling. `RunningServer` and
`ServerHandle` expose explicit state, wait, and idempotent shutdown operations.

## Wire protocols

| Protocol | Transport and mapping |
|---|---|
| `WireProtocol::FusenV1` | h2c, `POST /_fusen/v1/{service}/{method}`, versioned Fusen JSON |
| `WireProtocol::SpringCloudV1` | HTTP/1.1, explicit method/path/query/body mapping, raw success JSON |

The core runtime accepts only canonical `http://` endpoints. An `https://`
endpoint is rejected during validation, before network I/O. Terminate TLS in a
sidecar, service mesh, ingress, or reverse proxy.

The supported extension surface is `Middleware`, `Registry`, `Router`,
`LoadBalancer`, `RetryPolicy`, and `MetricsRecorder`. Transports, codecs,
acceptors, connection pools, and lifecycle state machines are private runtime
implementation details.

Requires Rust 1.97 or newer. Licensed under Apache-2.0.
