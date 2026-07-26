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

Hot configuration sources implement `ConfigSource` and return a prepared
`ConfigHandle` before starting remote work:

```rust
use fusen_config::{ConfigKey, ConfigSource};

let key = ConfigKey::builder("service.toml").group("prod").build()?;
let handle = source.prepare(key)?;
handle.activate().await?;

let mut settings = handle.typed::<Settings>()?;
let current = settings.current();
let next = settings.changed().await?;

settings.close().await?;
```

Typed updates are latest-wins and last-good: an invalid provider document is
reported by `HotConfig::last_error()` but never replaces the most recent valid
value. Revisions increase only for successfully parsed typed values.

Activation and close waiters are cancellation-safe, clones share their
terminal results, and close is idempotent. Dropping the final handle requests
cleanup without blocking; the provider worker still needs its Tokio runtime to
remain alive long enough to finish.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
