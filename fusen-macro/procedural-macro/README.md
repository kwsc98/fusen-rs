# fusen-procedural-macro

`fusen-procedural-macro` implements the clean-slate Fusen 0.9 interface
declaration and parameter validation. Applications normally use the `interface`
and `method` macros re-exported by `fusen-rs`.

```rust
use fusen_rs::{RpcError, RpcResponse};

#[fusen_rs::interface(name = "user", group = "prod", version = "1")]
pub trait UserApi {
    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/users/{id}")
    )]
    async fn get(
        &self,
        #[rpc(path)] id: String,
        #[rpc(query)] expand: Option<bool>,
    ) -> Result<RpcResponse<User>, RpcError>;
}
```

The interface attribute generates `UserApiClient` and `UserApiServer<T>`.
The generated client and user handlers both implement `UserApi`; generated
clients use the runtime's generic `ClientBuilder<UserApiClient>`.

RPC methods must be `async`, take immutable `&self`, accept zero or more owned
parameters with plain identifier patterns, and return
`Result<RpcResponse<T>, RpcError>`. Idempotency is one of `none`, `idempotent`,
or `safe` and defaults to `none`.

Every business parameter declares exactly one Spring role with `#[rpc(path)]`,
`#[rpc(query)]`, or `#[rpc(body)]`, plus an optional `name = "..."`; at most one
body parameter is accepted. One optional `#[rpc(call)] RpcCall` parameter carries
headers, extensions, and framework call information but is not serialized.
Path and query parameters must serialize as JSON scalars; body parameters may
contain any JSON value. Safe mappings permit only GET/HEAD. Fusen V1 always
encodes business parameters by name, independently of their Spring roles.

Generated code resolves a renamed `fusen-rs` dependency and targets only its
hidden macro ABI. This package is not a runtime extension surface.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
