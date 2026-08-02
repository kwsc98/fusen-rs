# fusen-nacos

`fusen-nacos` adapts the Nacos SDK to the lifecycle contracts in
`fusen-register` and `fusen-config`. It exports `NacosRegistry` for service
registration/discovery and `NacosConfigSource` for hot configuration.

```rust
use fusen_nacos::{NacosConfig, NacosConfigSource, NacosConvention, NacosRegistry};

let config = NacosConfig::builder()
    .server_addr("127.0.0.1:8848")
    .namespace("prod")
    .username("service")
    .password("secret")
    .build()?;

let registry = NacosRegistry::connect("orders-api", config.clone()).await?;
let spring_registry = NacosRegistry::connect("orders-api", config.clone())
    .await?
    .with_convention(NacosConvention::SpringCloud);
let source = NacosConfigSource::connect("orders-api", config).await?;
```

Both adapters validate their configuration and application name before the
SDK performs network I/O. Credentials must be configured as a pair, and
`NacosConfig` redacts its password in `Debug` output.

The registry maps every selector to its `service_id` and uses the selector group,
or Nacos `DEFAULT_GROUP` when absent. Discovery strictly matches
`fusen.version`, so versioned subscriptions never accept unversioned or differently
versioned instances. This naming is independent from the endpoint's HTTP binding.

Canonical registrations advertise `fusen.http.bindings`, `fusen.http.versions`,
and, when supported, `fusen.invocation-controls=v1`. Bindings use sorted,
comma-separated IDs and versions use `1.1`, `2`, or `1.1,2`; the controls key is
optional. Canonical discovery requires the bindings and versions keys and rejects
partial, empty, duplicate, or invalid capability metadata. It also requires
`fusen.service_id` to match the selector. Spring Cloud instances may omit that
identity key, but it must match when present.
`NacosConvention::SpringCloud` accepts an instance when all three capability keys
are absent, in which case it synthesizes HTTP/1.1,
`http-json-v1`, and no invocation controls. It applies the same strict version
filter and does not dual-read legacy service names or protocol metadata.

The adapter preserves stable provider instance identities and validated HTTP and
HTTPS service endpoints. Registration stores the actual scheme in `fusen.scheme`;
discovery accepts `http`/`https` and filters unknown schemes without rewriting or
downgrade. Prepared registration and subscription handles retain provider cleanup
ownership across cancelled waiters and late activation results.

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
