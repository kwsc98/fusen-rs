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
        method = "GET", path = "/users/{id}"
    )]
    async fn get(
        &self,
        id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<RpcResponse<User>, RpcError>;
}
# struct User;
```

RPC trait 必须是非泛型 async trait 方法集合，receiver 为 `&self`；每个方法可接收零到多个 owned 具名参数，返回值精确为 `Result<RpcResponse<T>, RpcError>`。每个方法必须声明 `#[method(method = "...", path = "...")]`；生成 Client 用它构造请求，生成 Server 用它匹配路由，重试资格也按标准 HTTP method 保守推导，不接受用户自报的幂等语义。

参数 wire name 与 path 中的 `{placeholder}` 同名时自动推断为 path；其余 GET、HEAD、OPTIONS、DELETE 参数默认为 query；其余 POST、PUT、PATCH 参数成为同一个 JSON body object 的字段，单字段也保持 object 形状。`#[param(query)]` 可覆盖默认位置，`#[param(body)]` 声明唯一 raw JSON body，`#[param(name = "...")]` 修改 wire name。需要 headers、extensions 或框架调用信息时，可额外声明一个类型为 `RpcCall` 的 `#[param(context)]` 参数；它不进入 wire。Raw body 不能与推断 body field 混用；重复 query 使用 `Vec<T>`，不接受 `Option<Vec<T>>`。非法映射、重复名称、非法 query 类型和 path 不匹配均在宏展开阶段失败；无法静态判断的 serde 形状在网络 I/O 前于本地失败。

Fusen V1 始终按名称把全部业务参数编码进 `arguments` object，与 HTTP 位置无关。宏只生成 `*Client`、`*Server<T>` 和私有 dispatch；生成 Client 与用户 Handler 实现同一个 trait，Client 使用通用 `ClientBuilder<GeneratedClient>`。生成代码只依赖版本化 `fusen_rs::__macro::v1` ABI，并支持应用重命名 runtime crate。
