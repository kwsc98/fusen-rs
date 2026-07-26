# fusen-procedural-macro

`fusen-procedural-macro` implements the clean-slate Fusen 0.9 service
declaration. Applications normally use the `service` and `method` attributes
re-exported by `fusen-rs`.

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

The service attribute generates `UserServiceClient`,
`UserServiceClientBuilder`, and `UserServiceServer`. Implementations directly
implement `UserService`; no second implementation attribute is required.

RPC methods must be `async`, take an immutable `&self`, use owned parameter and
success types, and explicitly return `Result<T, RpcError>`. Idempotency is one
of `none`, `idempotent`, or `safe` and defaults to `none`.

Spring Cloud mappings are optional. Path sources are inferred from complete
`{name}` segments, while every query or body source is explicit and at most one
body parameter is accepted. Safe mappings permit only GET/HEAD. Fusen V1 always
encodes every argument by its Rust parameter name, independently of Spring
mapping sources.

Generated code resolves a renamed `fusen-rs` dependency and targets only its
hidden macro ABI. This package is not a runtime extension surface.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
