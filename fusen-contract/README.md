# fusen-contract

`fusen-contract` contains the provider-neutral, validated value objects shared
by the Fusen runtime, registries, and generated service code. It performs no
network I/O and has no runtime lifecycle.

The public contract includes:

- `HttpBindingId`, `HttpVersionSet`, `HttpVersionPolicy`, and
  `EndpointCapabilities`.
- `ServiceDescriptor`, `MethodDescriptor`, `ServiceSelector`, and required
  `HttpOperation` method/parameter metadata.
- Process-local `SensitiveFields` shapes and method sensitivity metadata for
  structured diagnostic redaction.
- `ServiceRegistration`, `ServiceInstance`, stable `InstanceId`, and bounded
  positive `ServiceWeight`.
- `ServiceEndpoint`, a canonical absolute HTTP or HTTPS endpoint.

Values use private fields plus validating constructors and read-only getters.
For example:

```rust
use fusen_contract::{ServiceEndpoint, ServiceSelector};

let selector = ServiceSelector::new("user", Some("prod".into()), Some("1".into()))?;
let endpoint: ServiceEndpoint = "http://127.0.0.1:8080".parse()?;
let secure_endpoint: ServiceEndpoint = "https://api.example.com".parse()?;

assert_eq!(selector.identity(), "user/prod@1");
assert_eq!(endpoint.as_str(), "http://127.0.0.1:8080/");
assert_eq!(secure_endpoint.as_str(), "https://api.example.com/");
```

Enable the optional `derive` feature to re-export
`#[derive(fusen_contract::SensitiveFields)]`. Structured shapes carry separate
Serde serialization and deserialization field tables, and projection selects
the table that matches the interceptor side and response origin. Shapes are
lazy, so recursive DTOs are supported. They remain local to the process and
never alter HTTP encoding, service identity, discovery, or registration.
Built-in scalar types default to the fail-closed `Opaque` shape; retaining a
value requires an explicit `public` classification.

`ServiceEndpoint` rejects credentials, queries, fragments, invalid ports, and
every scheme except `http` and `https`. It is a network-neutral value: accepting
an HTTPS server advertisement does not make the built-in server terminate TLS.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
