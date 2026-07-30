# Middleware 与宏行为

> English summary: one `interface` trait macro defines the shared contract;
> one object-safe middleware SPI serves four explicit execution stages.

## Middleware

用户直接实现唯一对象安全的 `Middleware`。`Next` 消费自身且不可克隆，因此下游最多执行一次；Middleware 可以短路并返回 `RpcError`，或通过 `RpcContext::respond(value)` 创建受当前 runtime response limit 与 byte budget 约束的 `RpcResponse<RpcBody>`。

四个阶段为 `ClientCall`、`ClientAttempt`、`ServerHead` 和 `ServerCall`。`ClientCall` 包围整个 logical invocation 且不随 retry 重复；`ClientAttempt` 每个物理 attempt 执行，可读取 endpoint 与 attempt number；`ServerHead` 在 admission 后、body poll 前执行；`ServerCall` 在 decode 后、Handler 前执行。每阶段固定 global 在前、interface-local 在后。ClientCall extensions 克隆到每次 attempt 且 attempt 修改相互隔离；ServerHead extensions 原样传到 ServerCall 与 Handler。ServerHead 短路不会读取 body。

每个扩展调用有独立 panic boundary。Panic 返回隐藏细节的内部错误，并释放 admission/byte permits。Future 被取消时不保证 Middleware 后置代码执行，资源清理必须依赖 RAII。同步阻塞工作应使用 `spawn_blocking`。

## `interface`、`method` 与 `RpcMessage`

```rust,no_run
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
# struct User;
```

RPC trait 必须是非泛型 async trait 方法集合，receiver 为 `&self`，每个方法恰好接收一个 `RpcRequest<T>`，返回值精确为 `Result<RpcResponse<T>, RpcError>`。方法 idempotency 为 `none`、`idempotent` 或 `safe`，默认 `none`，不根据 HTTP verb 推断。

`RpcMessage` 只接受具名字段 struct，空请求使用内置 `()`。每个字段恰好声明 `#[rpc(path)]`、`#[rpc(query)]` 或 `#[rpc(body)]`，可用 `#[rpc(name = "...")]` 改 wire name；最多一个 body，重复 query 只接受受支持的集合类型。DTO 内部错误在 derive 阶段失败，DTO schema 与 Spring path 的跨类型不一致在 client connect/server build 阶段返回分类错误，早于网络 I/O。

Fusen V1 始终把 DTO 的全部字段编码进 `arguments` object，与 Spring 来源无关。宏只生成 `*Client`、`*Server<T>` 和私有 dispatch；生成 Client 与用户 Handler 实现同一个 trait，Client 使用通用 `ClientBuilder<GeneratedClient>`。生成代码只依赖版本化 `fusen_rs::__macro::v1` ABI，并支持应用重命名 runtime crate。
