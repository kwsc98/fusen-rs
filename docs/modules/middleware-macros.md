# Middleware 与宏行为

> English summary: one `service` trait macro defines the contract; logical
> middleware executes exactly once around all physical attempts.

## Middleware

用户直接实现 `Middleware`。`Next` 消费自身且不可克隆，因此下游最多执行一次；Middleware 可以短路并返回 `RpcError`，或通过 `RpcContext::respond(value)` 创建受当前 runtime response limit 与 byte budget 约束的 `RpcResponse`。客户端全局/服务局部 Middleware 在整个 logical invocation 外执行一次，不随 retry 重复；服务端 Middleware 在 route、admission 和 decode 后包围 service dispatch。

每个扩展调用有独立 panic boundary。Panic 返回隐藏细节的内部错误，并释放 admission/byte permits。Future 被取消时不保证 Middleware 后置代码执行，资源清理必须依赖 RAII。同步阻塞工作应使用 `spawn_blocking`。

## `service` 与 `method`

```rust,no_run
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
    ) -> Result<User, fusen_rs::RpcError>;
}
# struct User;
```

RPC trait 必须是非泛型 async trait 方法集合，receiver 为 `&self`，参数为拥有所有权的具体类型，返回值必须精确为 `Result<T, RpcError>`。方法 idempotency 为 `none`、`idempotent` 或 `safe`，默认 `none`，不根据 HTTP verb 推断。

`safe` 的 Spring 映射只允许 GET/HEAD。由于 HTTP HEAD 不传输响应 body，HEAD 映射必须返回 `Result<(), RpcError>`；客户端把成功的空响应还原为 unit。`idempotent` 可用于 PUT/DELETE 或显式声明的 POST。Path 参数从 `{name}` 推导；query/body 参数必须在属性中列出且不能重叠，最多一个 body。重复或等价动态 route、未知 placeholder、未映射参数与非法组合在宏展开时失败。

Fusen V1 始终按参数名编码全部参数，与 Spring 来源无关。宏生成且只生成 `*Client`、`*ClientBuilder`、`*Server` 和私有 dispatch；实现类型直接实现 trait。生成代码通过 `fusen_rs::__macro` 隐藏 ABI 定位依赖，支持应用重命名 runtime crate。
