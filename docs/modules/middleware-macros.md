# Middleware 与宏行为

> English summary: one `interface` trait macro defines the shared contract;
> one object-safe middleware SPI serves four explicit execution stages.

## Middleware

用户直接实现唯一对象安全的 `Middleware`。`Next` 消费自身且不可克隆，因此下游最多执行一次；Middleware 可以短路并返回 `RpcError`，或通过 `RpcContext::respond(value)` 创建受当前 runtime response limit 与 byte budget 约束的 `RpcResponse<RpcBody>`。

四个阶段为 `ClientCall`、`ClientAttempt`、`ServerHead` 和 `ServerCall`。`ClientCall` 包围整个 logical invocation 且不随 retry 重复；`ClientAttempt` 每个物理 attempt 执行，可读取 endpoint 与 attempt number；`ServerHead` 在 admission 后、body poll 前执行；`ServerCall` 在 decode 后、Handler 前执行。每阶段固定 global 在前、interface-local 在后。ClientCall extensions 克隆到每次 attempt 且 attempt 修改相互隔离；ServerHead extensions 原样传到 ServerCall 与 Handler。ServerHead 短路不会读取 body。

每个扩展调用有独立 panic boundary。Panic 返回隐藏细节的内部错误，并释放 admission/byte permits。Future 被取消时不保证 Middleware 后置代码执行，资源清理必须依赖 RAII。同步阻塞工作应使用 `spawn_blocking`。

## `interface`、`method` 与多参数

```rust,no_run
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
# struct User;
```

RPC trait 必须是非泛型 async trait 方法集合，receiver 为 `&self`；每个方法可接收零到多个 owned 具名参数，返回值精确为 `Result<RpcResponse<T>, RpcError>`。方法 idempotency 为 `none`、`idempotent` 或 `safe`，默认 `none`，不根据 HTTP verb 推断。

每个业务参数恰好声明 `#[rpc(path)]`、`#[rpc(query)]` 或 `#[rpc(body)]`，可用 `name = "..."` 改 wire name；每个方法最多一个 body，重复 query 使用 `Vec<T>`，不接受 `Option<Vec<T>>`。Path/query 必须序列化为 JSON 标量，body 可为任意 JSON 值。需要 headers、extensions 或框架调用信息时，可额外声明一个类型为 `RpcCall` 的 `#[rpc(call)]` 参数；它不进入 wire。无业务入参的方法直接省略参数。非法角色、重复名称、重复 body/call、非法 query 类型和 Spring path 不匹配均在宏展开阶段失败；无法静态判断的 serde 形状在网络 I/O 前于本地失败。

Fusen V1 始终按名称把全部业务参数编码进 `arguments` object，与 Spring 来源无关。宏只生成 `*Client`、`*Server<T>` 和私有 dispatch；生成 Client 与用户 Handler 实现同一个 trait，Client 使用通用 `ClientBuilder<GeneratedClient>`。生成代码只依赖版本化 `fusen_rs::__macro::v1` ABI，并支持应用重命名 runtime crate。
