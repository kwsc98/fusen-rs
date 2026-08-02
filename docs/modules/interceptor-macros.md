# Interceptor 与宏行为

> English summary: one `interface` trait macro defines the shared contract;
> one object-safe interceptor SPI serves four explicit execution stages.

## Interceptor

用户直接实现唯一对象安全的 `Interceptor`。`Next` 消费自身且不可克隆，因此下游最多执行一次；Interceptor 可以短路并返回 `Error`，或通过 `Context::respond(value)` 创建受当前 runtime response limit 与 byte budget 约束的 `Response<Body>`。

四个阶段为 `ClientCall`、`ClientAttempt`、`ServerHead` 和 `ServerCall`。`ClientCall` 包围整个 logical invocation 且不随 retry 重复；`ClientAttempt` 每个物理 attempt 执行，可读取 endpoint 与 attempt number；`ServerHead` 在 admission 后、body poll 前执行；`ServerCall` 在 decode 后、Handler 前执行。每阶段固定 global 在前、interface-local 在后。ClientCall extensions 克隆到每次 attempt 且 attempt 修改相互隔离；ServerHead extensions 原样传到 ServerCall 与 Handler。ServerHead 短路不会读取 body。

每个扩展调用有独立 panic boundary。Panic 返回隐藏细节的内部错误，并释放 admission/byte permits。Future 被取消时不保证 Interceptor 后置代码执行，资源清理必须依赖 RAII。同步阻塞工作应使用 `spawn_blocking`。

## 敏感字段安全投影

请求和成功响应 DTO 都用 `SensitiveFields` derive 声明进程内 schema；每个结构 shape 分别保存 Serde serialization/deserialization 字段表，接口方法不需要、也不支持 `#[sensitive(response)]`：

```rust,no_run
use fusen_rs::{Error, Response, SensitiveFields};
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
    ) -> Result<Response<LoginResponse>, Error>;
}
```

未标注的自定义字段会递归读取自身的 `SensitiveFields`；`Option<T>`、`Vec<T>`、数组、`Box<T>` 与 `Arc<T>` 继承 `T` 的 schema。`#[sensitive(kind = "...")]` 对完整字段或顶层参数分类，`#[sensitive(opaque)]` 用于显式省略无法实现 trait 的第三方字段。标准标量默认 opaque，只有显式分类的值才可能进入投影。预置 kind 为 `public`、`credential`、`token`、`phone`、`email`、`identifier` 和 `secret`，也可使用通过格式校验的自定义 kind。

Fusen 不自动打日志。第三方 Interceptor 在真正需要记录时显式创建安全视图：

```rust,no_run
# use fusen_rs::{Body, Context, Error, Next, PolicySanitizer, Response};
# async fn project(context: Context, next: Next<'_>) -> Result<Response<Body>, Error> {
let sanitizer = PolicySanitizer::default();
let method = context.method();
let arguments = context.sanitized_arguments(&sanitizer);
let response = next.run(context).await?;
let body = response.sanitized_body(method, &sanitizer);

tracing::info!(arguments = %arguments, response = %body, "service invocation completed");
# Ok(response)
# }
```

默认策略只 reveal `public`，将预置敏感 kind 替换为 `<redacted>`，并省略自定义 kind 与未分类值。投影按声明字段白名单遍历，输入中未出现在对应方向 schema 的未知 JSON 字段一律忽略，绝不会进入安全视图或日志。Client arguments 与服务端生成的 response 使用 serialization 表，Server arguments 与客户端收到的 response 使用 deserialization 表；方向性 `rename`、`alias` 和 `skip` 因此按实际 Serde 入口解释。缺少 schema、结构 schema 与 JSON 形状不匹配、超过输入/深度/节点/数组/字符串/输出限制或 `Sanitizer` panic 时，完整视图 fail closed 为 `<omitted>`，原始 service invocation 不受影响。响应在构造 JSON 投影视图前受独立的 64 KiB 默认输入上限约束，可通过 `ProjectionLimits` 调整；任意 `Context::respond` 短路响应没有声明来源，因此响应投影也默认省略。

`kind` 是对完整值的显式覆盖，不保留被覆盖类型原来的 JSON 结构约束；自定义策略应把收到的只读 `Value` 当作完整分类值处理。容器继承到 `kind` 时也遵守这一规则：`Option<T>` 的完整 null/value 或 `Vec<T>`/数组的完整 JSON array 只调用一次 `Sanitizer`，默认 redaction 不会泄露数组长度。结构化 DTO 容器才逐元素递归。

derive 只生成 schema，不修改业务 DTO 的 `Debug`，也不改变 `http-json-v1` bytes、service identity、endpoint capabilities 或注册发现 metadata。`SanitizedValue` 才是供第三方日志组件使用的安全 `Debug`、`Display` 与 `Serialize` 载体。

`SensitiveFields` derive 会拒绝结构化 `flatten/tag/content/untagged`；字段级 `serialize_with`、`deserialize_with`、`with`、`getter` 必须在该字段声明 `kind/opaque`，容器级 `into/from/try_from/remote` 必须使用类型级 `kind/opaque`。`#[serde(transparent)]` 可带 skipped/default marker 或 `PhantomData`，但两个 Serde 方向必须选择同一个有效字段。递归泛型通常自动推导；过程宏无法解析的递归 type alias 可用类型级 `#[sensitive(bound = "...")]` 覆盖自动 bound。Rust 不会把同一列表中的其他 derive 信息传给过程宏，因此框架无法辨别手写的 `Serialize`/`Deserialize` 实现；手写 `SensitiveFields` 必须提供与两种实际表示一致的字段表，否则应将完整类型声明为 `kind/opaque`。这些手写实现属于受信任代码。

## `interface`、`method` 与多参数

```rust,no_run
use fusen_rs::{Error, Response, SensitiveFields};
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
    ) -> Result<Response<User>, Error>;
}
```

Service interface trait 必须是非泛型 async trait 方法集合，receiver 为 `&self`；每个方法可接收零到多个 owned 具名参数，返回值精确为 `Result<Response<T>, Error>`。每个方法必须声明 `#[method(method = "...", path = "...")]`；可选 `consumes`/`produces` 接受语法合法的 MIME，并覆盖缺省 `application/json`。`HttpOperation` 本身保持 binding-neutral；内置 `http-json-v1` 在 Client/Server build 阶段预检并只接受 `application/json` 或具体的 `application/<subtype>+json`（可带参数），其他 MIME 在网络 I/O 前失败。生成 Client 用该 `HttpOperation` 构造请求，生成 Server 用它匹配路由，重试资格也按标准 HTTP method 保守推导，不接受用户自报的幂等语义。

参数 wire name 与 path 中的 `{placeholder}` 同名时自动推断为 path；其余 GET、HEAD、OPTIONS、DELETE 参数默认为 scalar query；其余 POST、PUT、PATCH 参数成为同一个 JSON body object 的字段，单字段也保持 object 形状。`#[param(path)]` 可显式确认 path 参数并要求 wire name 匹配同名占位符；`#[param(query)]` 可覆盖默认位置，`#[param(query, repeated)]` 声明序列化为 JSON array 的重复 query；`#[param(header)]`、`#[param(cookie)]`、`#[param(query_map)]` 与 `#[param(header_map)]` 显式映射其他 HTTP 来源；每个方法最多声明一个 query map 和一个 header map。`#[param(body_field)]` 显式声明 synthesized JSON object 中的字段，可用 `name` 改名但禁止 `repeated`；`#[param(body)]` 声明唯一 raw JSON body。GET、HEAD、OPTIONS 禁止两种 body，DELETE 默认 query 但允许显式 body/body_field，HEAD 必须返回 `Response<()>`。需要 headers、extensions 或框架调用信息时，可额外声明一个类型为 `Call` 的 `#[param(context)]` 参数；它不进入 wire。具名来源中的 wire name 必须唯一；map 来源不接受 `name`。Raw body 不能与 inferred 或 explicit body field 混用；非法映射、重复名称、非规范 route 和 path 不匹配均在宏展开阶段失败；serialized value 与声明 cardinality 不一致时在网络 I/O 前本地失败。

`http-json-v1` 直接按声明的 HTTP 来源编码参数，成功响应为 raw body，不使用私有 `arguments`/`result` envelope。宏只生成 `*Client`、`*Server<T>` 和私有 dispatch；生成 Client 与用户 Handler 实现同一个 trait，Client 使用通用 `ClientBuilder<GeneratedClient>`。生成代码只依赖版本化 `fusen_rs::__macro::v1` ABI，并支持应用重命名 runtime crate。

`__macro::v1` 是 doc-hidden 的 macro/runtime ABI，不是用户扩展 SPI；但 Cargo 允许组合的
任意 `fusen-procedural-macro` 与 `fusen-rs` 0.9.x 版本必须保持编译兼容。Patch 版本
不能原地删除或改变生成代码依赖的 `v1` item。需要不兼容形状时新增版本化 ABI，
并在新的 minor 版本协调 macro/runtime 依赖范围；renamed-runtime package consumer
持续验证 crate rename 与 ABI 路径。
