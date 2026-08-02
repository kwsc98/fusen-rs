# fusen-config

`fusen-config` provides typed static parsing and cancellation-safe hot
configuration with an explicit provider lifecycle.

Static configuration supports TOML by default:

```rust
#[derive(serde::Deserialize)]
struct Settings {
    workers: usize,
}

let settings: Settings = fusen_config::load("settings.toml")?;
```

Enable the `yaml` feature for YAML parsing and `.yaml`/`.yml` files. Without
that feature, YAML entry points return a classified unsupported-format error.

`ConfigSource` is the stable, object-safe configuration provider SPI for the
0.9 line. Provider adapters synchronously return a prepared `ConfigHandle`
before starting remote work; `fusen_config::provider::lifecycle` and
`ConfigPublisher` let third-party adapters implement that ownership contract
without exposing Tokio channels or provider SDK values. The supported Nacos
adapter implements the same SPI:

```rust
use fusen_config::{ConfigKey, ConfigSource};
use fusen_nacos::{NacosConfig, NacosConfigSource};

let nacos_source = NacosConfigSource::connect("orders-api", NacosConfig::default()).await?;
let key = ConfigKey::builder("service.toml").group("prod").build()?;
let handle = nacos_source.prepare(key)?;
handle.activate().await?;

let mut settings = handle.typed::<Settings>()?;
let current = settings.current();
let next = settings.changed().await?;

settings.close().await?;
```

Across compatible 0.9.x releases, provider implementations may rely on
`ConfigSource`, `ConfigKey`, `ConfigHandle`, `ConfigError`, and the public
`provider` constructors. The internal worker, channel, waiter, and cleanup
state-machine types are not extension APIs. `prepare` must not start remote side
effects; those begin when the returned handle is activated.

Typed updates are latest-wins and last-good: an invalid provider document is
reported by `HotConfig::last_error()` but never replaces the most recent valid
value. Revisions increase only for successfully parsed typed values.

Activation and close waiters are cancellation-safe, clones share their
terminal results, and close is idempotent. Dropping the final handle requests
cleanup without blocking; the provider worker still needs its Tokio runtime to
remain alive long enough to finish.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
