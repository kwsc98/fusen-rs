# fusen-rs 0.9

`fusen-rs` is an asynchronous RPC framework for Rust microservices. Procedural macros generate the client and server adapters at compile time, so services can be called through typed Rust traits without maintaining an IDL, running a code generator, or adding project scaffolding.

The framework transports JSON over HTTP/1.1 and HTTP/2 and supports direct Host addressing, Nacos registration and discovery, load balancing, Aspect middleware, configuration management, structured errors, and graceful shutdown. Version 0.9 focuses on explicit reliability contracts for deadlines, resource limits, registration rollback, and discovery cleanup.

[中文文档](README_CN.md)

## Features

- Type-safe generated clients from shared `#[fusen_trait]` interfaces
- Server adapters generated from `#[fusen_service]` implementations
- JSON services over HTTP/1.1 and HTTP/2
- Direct HTTP(S) endpoints and Nacos-based registration and discovery
- Service groups and versions plus HTTP method, path, query, and body mapping
- Pluggable load balancing and nestable Aspect middleware
- Local-file and Nacos configuration with live updates
- End-to-end client deadlines and bounded request/response bodies
- Request, connection, and HTTP/2 stream concurrency limits
- RFC 9457 Problem Details with typed remote errors
- Transactional startup registration and deadline-bounded graceful shutdown

> Dubbo Triple is intentionally unavailable in 0.9. Requests with `application/grpc` receive an RFC 9457 `415 Unsupported Media Type` response.

## Define a Service

Clients and servers share normal Rust data types and traits. `#[fusen_trait]` generates `DemoServiceClient`, while `#[asset]` can override the default route and HTTP method.

```rust
use fusen_rs::fusen_procedural_macro::fusen_trait;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestDto {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ResponseDto {
    pub message: String,
}

#[fusen_trait]
pub trait DemoService {
    async fn say_hello(&self, name: String) -> String;

    #[fusen_rs::fusen_procedural_macro::asset(path = "/hello")]
    async fn hello(&self, request: RequestDto) -> ResponseDto;

    #[fusen_rs::fusen_procedural_macro::asset(path = "/divide", method = GET)]
    async fn divide(&self, a: i32, b: i32) -> String;
}
```

Server implementations return `Result<T, FusenError>`. Errors are serialized as structured remote errors that clients can handle consistently.

```rust
use fusen_rs::{error::FusenError, fusen_procedural_macro::fusen_service};

#[derive(Default)]
struct DemoServiceImpl;

#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn say_hello(&self, name: String) -> Result<String, FusenError> {
        Ok(format!("Hello {name}"))
    }

    async fn hello(&self, request: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            message: format!("Hello {}", request.name),
        })
    }

    async fn divide(&self, a: i32, b: i32) -> Result<String, FusenError> {
        if b == 0 {
            return Err(FusenError::InvalidRequest(
                "divisor must not be zero".to_owned(),
            ));
        }
        Ok(format!("a / b = {}", a / b))
    }
}
```

## Direct Host Addressing

Direct addressing requires no registry and is useful for local development, tests, and fixed upstream services.

```rust,no_run
use fusen_rs::server::FusenServerBuilder;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let bind_addr = "0.0.0.0:8081".parse()?;
FusenServerBuilder::new(bind_addr)
    .service((Box::new(DemoServiceImpl), None))?
    .run()
    .await?;
# Ok(())
# }
```

Create a generated client with `ClientOptions::direct`. A direct URL may contain a base path, but it must be an HTTP(S) URL without a query or fragment.

```rust,no_run
use fusen_rs::client::{ClientOptions, FusenClientContextBuilder};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut context = FusenClientContextBuilder::new().build()?;
let client = DemoServiceClient::init(
    &mut context,
    ClientOptions::direct("http://127.0.0.1:8081".parse()?),
)
.await?;

println!("{}", client.say_hello("fusen".to_owned()).await?);
client.close().await?;
# Ok(())
# }
```

## Nacos Registration and Discovery

In discovery mode, servers publish each service after binding their socket. Clients subscribe using the service, group, and version declared by the trait. Directory updates are atomic snapshots containing only healthy, enabled instances with positive weights.

The server must configure both Nacos and an externally reachable `advertised_base_url`. Route validation and socket binding happen before registration. If any registration fails, completed registrations are rolled back in reverse order.

```rust,no_run
use std::sync::Arc;
use fusen_common::nacos::{NacosConfig, register::NacosRegister};
use fusen_rs::server::{FusenServerBuilder, ServerConfig};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let bind_addr = "0.0.0.0:8081".parse()?;
let mut config = ServerConfig::new(bind_addr);
config.advertised_base_url = Some("http://10.0.0.8:8081".to_owned());
let register = NacosRegister::init_nacos_register(
    "demo-server",
    Arc::new(NacosConfig {
        server_addr: "127.0.0.1:8848".to_owned(),
        ..Default::default()
    }),
)?;

FusenServerBuilder::new(bind_addr)
    .config(config)
    .register(register)
    .service((Box::new(DemoServiceImpl), None))?
    .run()
    .await?;
# Ok(())
# }
```

Install a `NacosRegister` in the client context and select discovery addressing. Discovery clients should be closed explicitly to unsubscribe and release the provider listener.

```rust,no_run
use std::sync::Arc;
use fusen_common::nacos::{NacosConfig, register::NacosRegister};
use fusen_rs::{
    client::{ClientOptions, FusenClientContextBuilder},
    contract::WireProtocol,
};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let register = NacosRegister::init_nacos_register(
    "demo-client",
    Arc::new(NacosConfig {
        server_addr: "127.0.0.1:8848".to_owned(),
        ..Default::default()
    }),
)?;
let mut context = FusenClientContextBuilder::new()
    .register(register)
    .build()?;
let client = DemoServiceClient::init(
    &mut context,
    ClientOptions::discovery(WireProtocol::Fusen),
)
.await?;

println!("{}", client.say_hello("nacos".to_owned()).await?);
client.close().await?;
# Ok(())
# }
```

## Middleware

Clients and servers can extend their invocation pipelines with handlers. Register handlers on a context or server first, then attach their string IDs to individual services in execution order.

`LoadBalance` selects an instance from the current discovery snapshot. It can implement random, round-robin, consistent-hash, or application-specific routing. `Aspect` wraps an invocation and is suitable for logging, authorization, limits, circuit breaking, metrics, and tracing. Multiple Aspects may be nested.

The repository includes complete [logging and tracing Aspects](examples/src/handler/aspect) and a [custom load balancer](examples/src/handler/loadbalance/custom.rs).

## Reliability Defaults

- Client connect timeout: 3 seconds
- End-to-end client invocation timeout: 10 seconds
- Discovery and subscription-close timeouts: 5 seconds
- Maximum response body: 2 MiB
- Server request timeout: 30 seconds
- Maximum concurrent requests: 1024
- Maximum connections: 2048
- Maximum HTTP/2 streams per connection: 128
- Maximum request body: 2 MiB
- Total graceful-shutdown timeout: 30 seconds

These defaults can be changed through `ClientConfig` and `ServerConfig`. All limits and timeouts must be greater than zero. Non-2xx Problem Details responses are restored as `FusenError::Remote`; server registration operations are bounded and shutdown deregisters services before draining connections.

## Run the Examples

Examples are separated by addressing mode:

```text
examples/src/
├── host/
│   ├── server.rs
│   ├── client.rs
│   ├── server_pt.rs
│   └── client_pt.rs
└── nacos/
    ├── server.rs
    ├── client.rs
    └── hot_config.rs
```

Run the direct Host example in two terminals:

```bash
cargo run -p examples --bin host-server
cargo run -p examples --bin host-client
```

For a benchmark without per-request logging or tracing, run the dedicated release-mode server and client. Set `PT_PROTOCOL=both` to run HTTP/1.1 and HTTP/2 sequentially with the same workload and report their QPS and throughput ratio:

```bash
cargo run --release -p examples --bin host-server-pt
PT_PROTOCOL=both PT_CONCURRENCY=100 PT_REQUESTS_PER_TASK=10000 \
cargo run --release -p examples --bin host-client-pt
```

HTTP/1.1 scales concurrent work through the connection pool, while HTTP/2 multiplexes streams on a connection and compresses headers with HPACK. Byte counts cover serialized JSON bodies, excluding HTTP framing, TCP/IP, and TLS overhead. See [examples/README.md](examples/README.md) for all benchmark settings and protocol details.

For Nacos, start a Nacos server and then run. `NACOS_ADDR` defaults to `127.0.0.1:8848`, and `FUSEN_ADVERTISED_URL` defaults to `http://127.0.0.1:8081`; the environment variables below override those defaults:

```bash
NACOS_ADDR=127.0.0.1:8848 \
FUSEN_ADVERTISED_URL=http://127.0.0.1:8081 \
cargo run -p examples --bin nacos-server

NACOS_ADDR=127.0.0.1:8848 \
cargo run -p examples --bin nacos-client
```

`FUSEN_ADVERTISED_URL` must be reachable by clients in container or multi-host deployments. See [examples/README.md](examples/README.md) for all example commands.

## Documentation

- [Architecture and invocation pipeline](docs/architecture.md)
- [Module behavior index](docs/modules/README.md)
- [Client behavior](docs/modules/client.md)
- [Server behavior](docs/modules/server.md)
- [Nacos registration and discovery](docs/modules/registry-nacos.md)
- [Middleware and macros](docs/modules/middleware-macros.md)
- [Configuration](docs/modules/configuration.md)
- [Routing](docs/modules/routing.md)
- [Error model](docs/modules/errors.md)
- [Graceful shutdown](docs/modules/shutdown.md)
- [0.8 to 0.9 migration guide](docs/migration-0.9.md)
- [Compatibility policy](docs/compatibility.md)
- [Contributing](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security policy](SECURITY.md)
- [Release process](docs/releasing.md)
