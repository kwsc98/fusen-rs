# Middleware 与宏行为

> English summary: one `interface` trait macro defines the shared contract;
> one object-safe middleware SPI serves four explicit execution stages.

## Middleware

用户直接实现唯一对象安全的 `Middleware`。`Next` 消费自身且不可克隆，因此下游最多执行一次；Middleware 可以短路并返回 `RpcError`，或通过 `RpcContext::respond(value)` 创建受当前 runtime response limit 与 byte budget 约束的 `RpcResponse<RpcBody>`。

四个阶段为 `ClientCall`、`ClientAttempt`、`ServerHead` 和 `ServerCall`。`ClientCall` 包围整个 logical invocation 且不随 retry 重复；`ClientAttempt` 每个物理 attempt 执行，可读取 endpoint 与 attempt number；`ServerHead` 在 admission 后、body poll 前执行；`ServerCall` 在 decode 后、Handler 前执行。每阶段固定 global 在前、interface-local 在后。ClientCall extensions 克隆到每次 attempt 且 attempt 修改相互隔离；ServerHead extensions 原样传到 ServerCall 与 Handler。ServerHead 短路不会读取 body。

每个扩展调用有独立 panic boundary。Panic 返回隐藏细节的内部错误，并释放 admission/byte permits。Future 被取消时不保证 Middleware 后置代码执行，资源清理必须依赖 RAII。同步阻塞工作应使用 `spawn_blocking`。

## 敏感字段安全投影

请求和成功响应 DTO 都用同一个 `SensitiveFields` derive 声明进程内 schema；接口方法不需要、也不支持 `#[sensitive(response)]`：

```rust,no_run
use fusen_rs::{RpcError, RpcResponse, SensitiveFields};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, SensitiveFields)]
struct Profile {
    #[sensitive(kind = "phone")]
    phone: String,
}

#[derive(Serialize, Deserialize, SensitiveFields)]
struct LoginRequest {
    #[sensitive(kind = "public")]
    username: String,
    #[sensitive(kind = "credential")]
    password: String,
    profile: Profile,
    #[sensitive(opaque)]
    vendor_payload: serde_json::Value,
}

#[derive(Serialize, Deserialize, SensitiveFields)]
struct LoginResponse {
    #[sensitive(kind = "identifier")]
    user_id: String,
    #[sensitive(kind = "token")]
    access_token: String,
}

#[fusen_rs::interface(name = "auth")]
trait AuthApi {
    #[fusen_rs::method(method = "POST", path = "/tenants/{tenant_id}/login")]
    async fn login(
        &self,
        #[param(body)] request: LoginRequest,
        #[param(path)]
        #[sensitive(kind = "identifier")]
        tenant_id: String,
    ) -> Result<RpcResponse<LoginResponse>, RpcError>;
}
```

未标注的自定义字段会递归读取自身的 `SensitiveFields`；`Option<T>`、`Vec<T>`、数组、`Box<T>` 与 `Arc<T>` 继承 `T` 的 schema。`#[sensitive(kind = "...")]` 对完整字段或顶层参数分类，`#[sensitive(opaque)]` 用于显式省略无法实现 trait 的第三方字段。标准标量默认 opaque，只有显式分类的值才可能进入投影。预置 kind 为 `public`、`credential`、`token`、`phone`、`email`、`identifier` 和 `secret`，也可使用通过格式校验的自定义 kind。

Fusen 不自动打日志。第三方 Middleware 在真正需要记录时显式创建安全视图：

```rust,no_run
# use fusen_rs::{Next, PolicySanitizer, RpcContext, RpcError, RpcResponse};
# use fusen_rs::middleware::RpcBody;
# async fn project(context: RpcContext, next: Next<'_>) -> Result<RpcResponse<RpcBody>, RpcError> {
let sanitizer = PolicySanitizer::default();
let method = context.method();
let arguments = context.sanitized_arguments(&sanitizer);
let response = next.run(context).await?;
let body = response.sanitized_body(method, &sanitizer);

tracing::info!(arguments = %arguments, response = %body, "RPC completed");
# Ok(response)
# }
```

默认策略只 reveal `public`，将预置敏感 kind 替换为 `<redacted>`，并省略自定义 kind 与未分类值。缺少 schema、结构 schema 与 JSON 形状不匹配、超过输入/深度/节点/数组/字符串/输出限制或 `Sanitizer` panic 时，完整视图 fail closed 为 `<omitted>`，原始 RPC 不受影响。响应在构造 JSON 投影视图前受独立的 64 KiB 默认输入上限约束，可通过 `ProjectionLimits` 调整；任意 `RpcContext::respond` 短路响应没有声明来源，因此响应投影也默认省略。

`kind` 是对完整值的显式覆盖，不保留被覆盖类型原来的 JSON 结构约束；自定义策略应把收到的只读 `Value` 当作完整分类值处理。容器继承到 `kind` 时也遵守这一规则：`Option<T>` 的完整 null/value 或 `Vec<T>`/数组的完整 JSON array 只调用一次 `Sanitizer`，默认 redaction 不会泄露数组长度。结构化 DTO 容器才逐元素递归。

derive 只生成 schema，不修改业务 DTO 的 `Debug`，也不改变 Fusen V1/Spring Cloud V1 wire bytes、service identity、注册中心或服务发现 metadata。`SanitizedValue` 才是供第三方日志组件使用的安全 `Debug`、`Display` 与 `Serialize` 载体。

`SensitiveFields` derive 会拒绝 `flatten`；字段级 `serialize_with`、`with`、`getter` 必须在该字段声明 `kind/opaque`，容器级 `into/remote` 必须使用类型级 `kind/opaque`。Rust 不会把同一列表中的其他 derive 信息传给过程宏，因此框架无法辨别手写的 `Serialize` 实现；这类实现必须保证字段名和结构与派生 schema 一致，或将整个类型声明为 `#[sensitive(kind = "...")]` / `#[sensitive(opaque)]`。

## `interface`、`method` 与多参数

```rust,no_run
use fusen_rs::{RpcError, RpcResponse, SensitiveFields};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, SensitiveFields)]
#[sensitive(opaque)]
struct User;

#[fusen_rs::interface(name = "user", group = "prod", version = "1")]
pub trait UserApi {
    #[fusen_rs::method(
        method = "GET", path = "/users/{id}"
    )]
    async fn get(
        &self,
        #[param(path)] id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<RpcResponse<User>, RpcError>;
}
```

RPC trait 必须是非泛型 async trait 方法集合，receiver 为 `&self`；每个方法可接收零到多个 owned 具名参数，返回值精确为 `Result<RpcResponse<T>, RpcError>`。每个方法必须声明 `#[method(method = "...", path = "...")]`；生成 Client 用它构造请求，生成 Server 用它匹配路由，重试资格也按标准 HTTP method 保守推导，不接受用户自报的幂等语义。

参数 wire name 与 path 中的 `{placeholder}` 同名时自动推断为 path；其余 GET、HEAD、OPTIONS、DELETE 参数默认为 query；其余 POST、PUT、PATCH 参数成为同一个 JSON body object 的字段，单字段也保持 object 形状。`#[param(path)]` 可显式确认 path 参数并要求 wire name 匹配同名占位符；`#[param(query)]` 可覆盖默认位置，`#[param(body)]` 声明唯一 raw JSON body，`#[param(name = "...")]` 修改 wire name。需要 headers、extensions 或框架调用信息时，可额外声明一个类型为 `RpcCall` 的 `#[param(context)]` 参数；它不进入 wire。所有非 context 参数的 wire name 必须全局唯一。Raw body 不能与推断 body field 混用；重复 query 使用 `Vec<T>`，不接受 `Option<Vec<T>>`。非法映射、重复名称、非法 query 类型和 path 不匹配均在宏展开阶段失败；无法静态判断的 serde 形状在网络 I/O 前于本地失败。

Fusen V1 始终按名称把全部业务参数编码进 `arguments` object，与 HTTP 位置无关。宏只生成 `*Client`、`*Server<T>` 和私有 dispatch；生成 Client 与用户 Handler 实现同一个 trait，Client 使用通用 `ClientBuilder<GeneratedClient>`。生成代码只依赖版本化 `fusen_rs::__macro::v1` ABI，并支持应用重命名 runtime crate。
