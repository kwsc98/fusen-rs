# fusen-procedural-macro

`fusen-procedural-macro` implements the clean-slate Fusen 0.9 interface
declaration and parameter validation. Applications normally use the `interface`
and `method` macros re-exported by `fusen-rs`.

```rust
use fusen_rs::{RpcError, RpcResponse};

#[fusen_rs::interface(name = "user", group = "prod", version = "1")]
pub trait UserApi {
    #[fusen_rs::method(
        method = "GET", path = "/users/{id}"
    )]
    async fn get(
        &self,
        id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<RpcResponse<User>, RpcError>;
}
```

The interface attribute generates `UserApiClient` and `UserApiServer<T>`.
The generated client and user handlers both implement `UserApi`; generated
clients use the runtime's generic `ClientBuilder<UserApiClient>`.

RPC methods must be `async`, take immutable `&self`, accept zero or more owned
parameters with plain identifier patterns, and return
`Result<RpcResponse<T>, RpcError>`. Every method must declare
`#[method(method = "...", path = "...")]`; generated clients use the mapping to
build requests, generated servers use it for routing, and it determines retry
eligibility.

A wire name matching a path placeholder is inferred as a path parameter. Other
GET/HEAD/OPTIONS/DELETE parameters default to query values; other POST/PUT/PATCH
parameters become fields in a synthesized JSON body object. `#[param(query)]`
overrides the default, `#[param(body)]` declares one complete raw JSON body,
`#[param(context)]` carries an unencoded `RpcCall`, and `#[param(name = "...")]`
renames the wire parameter. A raw body cannot coexist with synthesized body
fields. Retry eligibility is inferred from the standard HTTP method. Fusen V1
always encodes business parameters by name, independently of their HTTP roles.

Generated code resolves a renamed `fusen-rs` dependency and targets only its
hidden macro ABI. This package is not a runtime extension surface.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
