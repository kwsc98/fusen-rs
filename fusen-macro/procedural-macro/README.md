# fusen-procedural-macro

`fusen-procedural-macro` implements the clean-slate Fusen 0.9 interface and
message declarations. Applications normally use the `interface`, `method`, and
`RpcMessage` macros re-exported by `fusen-rs`.

```rust
use fusen_rs::{RpcError, RpcRequest, RpcResponse};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, fusen_rs::RpcMessage)]
pub struct GetUserRequest {
    #[rpc(path)]
    pub id: String,
    #[rpc(query)]
    pub expand: Option<bool>,
}

#[fusen_rs::interface(name = "user", group = "prod", version = "1")]
pub trait UserApi {
    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/users/{id}")
    )]
    async fn get(
        &self,
        request: RpcRequest<GetUserRequest>,
    ) -> Result<RpcResponse<User>, RpcError>;
}
```

The interface attribute generates `UserApiClient` and `UserApiServer<T>`.
The generated client and user handlers both implement `UserApi`; generated
clients use the runtime's generic `ClientBuilder<UserApiClient>`.

RPC methods must be `async`, take immutable `&self`, accept exactly one
`RpcRequest<T>`, and return `Result<RpcResponse<T>, RpcError>`. Idempotency is
one of `none`, `idempotent`, or `safe` and defaults to `none`.

`RpcMessage` accepts named-field structs only. Every field declares exactly one
Spring role with `#[rpc(path)]`, `#[rpc(query)]`, or `#[rpc(body)]`, plus an
optional `#[rpc(name = "...")]`; at most one body field is accepted. The built-in
`()` message represents an empty request. Safe mappings permit only GET/HEAD.
Fusen V1 always encodes every DTO field by name, independently of its Spring role.

Generated code resolves a renamed `fusen-rs` dependency and targets only its
hidden macro ABI. This package is not a runtime extension surface.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
