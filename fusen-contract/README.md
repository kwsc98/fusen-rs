# fusen-contract

`fusen-contract` contains the provider-neutral, validated value objects shared
by the Fusen runtime, registries, and generated service code. It performs no
network I/O and has no runtime lifecycle.

The public contract includes:

- `WireProtocol::{FusenV1, SpringCloudV1}`, `ProtocolSet`, and explicit
  `Idempotency` semantics.
- `ServiceDescriptor`, `MethodDescriptor`, `ServiceSelector`, and Spring Cloud
  method/parameter metadata.
- `ServiceRegistration`, `ServiceInstance`, stable `InstanceId`, and bounded
  positive `ServiceWeight`.
- `ServiceEndpoint`, a canonical absolute plaintext HTTP endpoint.

Values use private fields plus validating constructors and read-only getters.
For example:

```rust
use fusen_contract::{ServiceEndpoint, ServiceSelector};

let selector = ServiceSelector::new("user", Some("prod".into()), Some("1".into()))?;
let endpoint: ServiceEndpoint = "http://127.0.0.1:8080".parse()?;

assert_eq!(selector.identity(), "user/prod@1");
assert_eq!(endpoint.as_str(), "http://127.0.0.1:8080/");
```

`ServiceEndpoint` rejects credentials, queries, fragments, invalid ports, and
every non-HTTP scheme. In particular, `https://` is rejected before any runtime
can perform network I/O.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
