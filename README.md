# fusen-rs

`fusen-rs` 0.9 is an asynchronous Rust RPC framework for JSON services over
HTTP/1.1 and HTTP/2. It provides compile-time client/server generation,
service discovery, middleware, bounded request processing, structured errors,
and graceful shutdown.

The detailed documentation is maintained in Chinese. Start with the
[architecture](docs/architecture.md), [module index](docs/modules/README.md),
and [0.9 migration guide](docs/migration-0.9.md).

## Quick start

```rust,no_run
use fusen_rs::{
    client::{ClientOptions, FusenClientContextBuilder},
    server::FusenServerBuilder,
};

# async fn demo() -> Result<(), Box<dyn std::error::Error>> {
let mut context = FusenClientContextBuilder::new().build()?;
let options = ClientOptions::direct("http://127.0.0.1:8081".parse()?);
// Generated clients use: DemoServiceClient::init(&mut context, options).await?;

let server = FusenServerBuilder::new("0.0.0.0:8081".parse()?);
// Add handlers and services, then call server.run().await?;
# let _ = (context, options, server);
# Ok(())
# }
```

Dubbo Triple is intentionally disabled in 0.9. Requests with
`application/grpc` receive an RFC 9457 `415` response.

See [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md), and the
[compatibility policy](docs/compatibility.md) before contributing.
