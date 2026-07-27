# fusen-nacos

`fusen-nacos` adapts the Nacos SDK to the lifecycle contracts in
`fusen-register` and `fusen-config`. It exports `NacosRegistry` for service
registration/discovery and `NacosConfigSource` for hot configuration.

```rust
use fusen_nacos::{NacosConfig, NacosConfigSource, NacosRegistry};

let config = NacosConfig::builder()
    .server_addr("127.0.0.1:8848")
    .namespace("prod")
    .credentials("service", "secret")
    .build();

let registry = NacosRegistry::connect("orders-api", config.clone()).await?;
let source = NacosConfigSource::connect("orders-api", config).await?;
```

Both adapters validate their configuration and application name before the
SDK performs network I/O. Credentials must be configured as a pair, and
`NacosConfig` redacts its password in `Debug` output.

The registry supports `WireProtocol::FusenV1` and
`WireProtocol::SpringCloudV1`, preserves stable provider instance identities,
and preserves validated HTTP and HTTPS service endpoints. Registration stores
the actual scheme in `fusen.scheme`; discovery accepts `http`/`https` and filters
unknown schemes without rewriting or downgrade. Its prepared
registration and subscription handles retain provider cleanup ownership across
cancelled waiters and late activation results.

An HTTPS endpoint is callable by the fusen client using Rustls and bundled
Mozilla WebPKI roots. When the built-in server advertises HTTPS, an external TLS
terminator must actually serve that address and forward to the plaintext listener.

The configuration adapter installs the listener before fetching the initial
document, preventing a setup gap. It returns a `ConfigHandle` that can produce
the last-good typed views provided by `fusen-config`. The `yaml` feature forwards
YAML support to that crate.

Nacos SDK clients, listeners, and channels remain private implementation
details. Version 0.9 defines the first compatibility baseline. Requires Rust
1.97 or newer. Licensed under Apache-2.0.
