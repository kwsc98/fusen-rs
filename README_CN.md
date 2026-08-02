# fusen-rs 0.9

`fusen-rs` 是面向 Rust 的生产级异步微服务与服务调用框架。0.9 是一次 clean-slate 的 API 与 HTTP binding 基线：提供生成式客户端和服务端、明确的生命周期所有权、有界资源、服务发现、重试、熔断、拦截器与结构化可观测性。

[English](README.md)

## 运行边界

- Rust 1.97、Edition 2024、Tokio 与 JSON。
- Client 支持 canonical `http://` 与 `https://` endpoint。稳定的 `http-json-v1` binding 与 HTTP transport 选择相互独立；endpoint 通过 capabilities 声明支持的 binding、HTTP version 与 invocation controls。
- Client HTTPS 使用 Rustls Ring、TLS 1.2/1.3、bundled Mozilla WebPKI roots 和严格的证书/hostname 验证。内置 Server 保持明文；入站 TLS 由 ingress、sidecar、反向代理或 service mesh 终止。
- 稳定扩展面仅包括 `Interceptor`、`Registry`、`ConfigSource`、`InstanceRouter`、`LoadBalancer`、`RetryPolicy`、`MetricsRecorder`，以及 Client 侧的 `RequestEncoder`/`ResponseDecoder`/`ErrorDecoder` binding codec。
- HTTP Transport、Server codec、Acceptor、连接池与生命周期状态机均为 runtime 私有实现。

## 接口契约

一个 trait 宏定义 Client/Server 共享接口；每个服务方法可直接接收零到多个具名的 owned 参数，并返回 `Result<Response<T>, Error>`。

```rust,no_run
use fusen_rs::{Error, Response, SensitiveFields, interface};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, SensitiveFields)]
pub struct User {
    #[sensitive(kind = "identifier")]
    pub id: String,
}

#[derive(Serialize, Deserialize, SensitiveFields)]
pub struct CreateUser {
    #[sensitive(kind = "identifier")]
    pub id: String,
}

#[interface(name = "user", group = "prod", version = "1")]
pub trait UserApi {
    #[fusen_rs::method(
        method = "GET", path = "/users/{id}"
    )]
    async fn get(
        &self,
        #[param(path)] id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<Response<User>, Error>;

    #[fusen_rs::method(
        method = "POST", path = "/users"
    )]
    async fn create(
        &self,
        user: CreateUser,
        audit: bool,
    ) -> Result<Response<User>, Error>;
}
```

宏生成 `UserApiClient` 与 `UserApiServer<T>`。生成 Client 和用户 Handler 都实现 `UserApi`，所有 Client 统一使用 `ClientBuilder<UserApiClient>`。每个接口方法都必须声明 `#[method(method = "...", path = "...")]`；生成的 Client 用它构造请求，生成的 Server 用它匹配路由，自动重试资格也由标准 HTTP method 推导。

参数位置采用确定性推断：wire name 与 `{placeholder}` 同名时为 path；其余 GET、HEAD、OPTIONS、DELETE 参数默认为 scalar query；其余 POST、PUT、PATCH 参数成为同一个 JSON body object 的字段，因此 `create(user, audit)` 固定发送 `{"user": ..., "audit": ...}`，即使只有一个字段也不会退化成 raw body。`#[param(path)]` 可显式确认 path 参数，且 wire name 必须匹配同名占位符；`#[param(query)]` 与 `#[param(query, repeated)]` 表示 query，`#[param(header)]` 与 `#[param(cookie)]` 表示 HTTP metadata，`#[param(query_map)]` 与 `#[param(header_map)]` 表示动态 map，`#[param(body)]` 表示唯一的完整 JSON body，`#[param(context)]` 注入不进入 wire 的 `Call`，`#[param(name = "...")]` 修改 wire name。具名来源中的非 context wire name 必须唯一。Raw body 不能与推断 body field 混用。`http-json-v1` 直接发送声明的 HTTP operation 并返回 raw JSON，不再使用 Fusen 私有 request/response envelope；非法映射在宏展开阶段失败，非法序列化值在网络 I/O 前于本地失败。

自定义请求和响应 DTO 都 derive `SensitiveFields`，字段使用 `#[sensitive(kind = "...")]` 或 `#[sensitive(opaque)]`；接口宏自动发现请求和 `Response<T>` schema，不需要响应标记。Fusen 不会自动打印 payload；第三方 `Interceptor` 显式调用 `sanitized_arguments` 与 `sanitized_body`，失败时安全省略，且不修改 DTO `Debug`、wire bytes 或注册/发现 metadata。详见 [Interceptor 与宏行为](docs/modules/interceptor-macros.md)。

## 客户端

`ClientRuntime` 统一持有 admission、字节预算、Interceptor、发现订阅、连接池、重试预算和熔断器。

```rust,no_run
# use fusen_rs::{ClientError, ClientRuntime};
# use crate::UserApiClient;
# async fn run() -> Result<(), ClientError> {
let runtime = ClientRuntime::builder().build()?;

let client = UserApiClient::builder(&runtime)
    .direct("http://127.0.0.1:8081")
    .connect()
    .await?;

// 生成的服务方法返回 Result<Response<T>, Error>。
runtime.shutdown().await?;
# Ok(())
# }
```

Direct client 使用 `https://`，或从 Registry 发现 HTTPS instance，即可启用客户端
TLS。Runtime 不读取系统 trust store，也不提供自定义 CA、mTLS 或跳过证书验证；
私有 CA 与自签名 endpoint 不属于 0.9 契约。

在 runtime builder 安装一个 `Registry` 后，用 `.discover()` 替代 `.direct(...)` 即可启用发现。每个 `ServiceSelector` 共享唯一订阅，latest-wins 快照状态为 `Initializing`、`Ready`、`Stale`、`Unavailable` 或 `Closed`。每个发现到的 `ServiceInstance` 都携带 `EndpointCapabilities`；Client 按所需 `HttpBindingId` 过滤实例，只在连接选中 endpoint 时应用 `HttpVersionPolicy`。Registry subscription identity 因而不依赖 binding 或 transport policy。

生成 Client builder 使用 `.binding(...)` 选择表示，使用 `.http_version_policy(...)` 选择 transport policy。Direct endpoint 未设置 `.direct_capabilities(...)` 时，默认支持 Client 当前选中的 binding 且关闭 invocation controls；`http://` + `Auto` 使用 HTTP/1.1，`https://` + `Auto` 通过 ALPN 协商 HTTP/2 或 HTTP/1.1。需要用部署方声明的 binding、version 和 controls 契约取代这一推断时，使用 `.direct_capabilities(...)`。

一个绝对 deadline 覆盖 admission、Interceptor、全部 attempts、退避、传输与 decode。重试资格由声明的 HTTP method 保守推导：GET、HEAD、OPTIONS、PUT、DELETE 可重试，POST、PATCH 永不自动重试。内置策略最多执行三次总 attempts，并受每服务 token budget 的硬约束。每次物理 attempt 都重新读取发现快照，并应用 endpoint/service 熔断器和 endpoint bulkhead。

如果 HTTP 成功响应的 raw JSON body 无法反序列化为生成方法声明的 Rust 类型，调用会以 `DataLoss`/`invalid_result` 非重试终止；该 selected endpoint attempt 与 service 最终结果都会按 protocol failure 计入对应熔断器。

`ClientRuntime::shutdown()` 幂等：先关闭 admission，再在同一 deadline 内排空逻辑调用、关闭订阅与连接池。取消某个 shutdown waiter 不会取消后台 coordinator。

## 服务端

```rust,no_run
# use fusen_rs::{Server, ServerError};
# use crate::{UserApiServer, UserApiHandler};
# async fn run() -> Result<(), ServerError> {
let server = Server::builder("0.0.0.0:0")
    .interface(UserApiServer::new(UserApiHandler))
    .build()?;

let running = server.start().await?;
println!("listening on {}", running.local_addr());
running.shutdown().await?;
# Ok(())
# }
```

`start()` 先 bind，再以 not-ready 状态启动 accept loop，随后激活注册，只有进入 `Ready` 后才返回。Ready 前请求收到非 retryable 的 `503 not_ready`，且 body 不会被 poll。`RunningServer` 提供 `local_addr()`、`state()`、`handle()`、`wait()` 与 `shutdown()`；所有 handle 的 shutdown 幂等并共享唯一终态。`Server::serve()` 额外提供平台信号处理。

内置 listener 只接受明文 HTTP/1.1 与 h2c。显式
`.advertised_endpoint("https://...")` 发布由外部 TLS 终止器提供的地址，不会让本地
listener 启用 TLS。

停机先关闭 readiness 和 listener，再在同一个绝对 deadline 内并行注销 provider、通知 Hyper graceful shutdown 并排空在途请求。deadline 到达后会取消剩余工作并有界返回 `ServerError`，不会无限等待 task 回收。

## HTTP Binding

`http-json-v1` 将每个服务方法映射到其声明的 HTTP operation。JSON request field 与 raw JSON 成功响应默认使用 `application/json`；方法声明可通过 `consumes` 与 `produces` 覆盖 media type。

```text
POST /users
Content-Type: application/json
Accept: application/json

{"user":{"id":"42"},"audit":true}
{"id":"42"}
```

`HttpBindingId` 是该表示在 registry metadata 与 telemetry 中的标识，`HTTP_JSON_V1` 是其稳定字符串；它不负责选择 HTTP/1.1 或 HTTP/2。`HttpVersionPolicy::{Auto, Http1, Http2, H2c}` 表示 Client 的 transport 偏好，`EndpointCapabilities` 声明 endpoint 实际支持的 version 与 binding。`EndpointCapabilities::default()` 为 HTTP/1.1、`http-json-v1` 且关闭 Fusen invocation controls，Registry convention 可显式用它处理缺失 metadata。未声明 capabilities 的 direct endpoint 则使用当前选中的 binding 并关闭 controls；`http://` + `Auto` 使用 HTTP/1.1，`https://` + `Auto` 通过 ALPN 协商 HTTP/2 或 HTTP/1.1。

额外的 Client-only binding 通过 `ClientRuntimeBuilder::http_binding(...)` 注册 `RequestEncoder`、`ResponseDecoder` 与 `ErrorDecoder`，再由生成 Client builder 选择同一个 `HttpBindingId`。Codec 只接收受限的 HTTP semantic data，不拥有 transport 或 lifecycle 资源；built-in Server 有意只提供 `http-json-v1`。

内置错误使用 `application/problem+json`，包含 RFC 9457 字段及 `code`、`request_id`、`retryable`；Client 也接受合法的外部 RFC Problem type。只有选中的 endpoint 声明支持 invocation controls 时才发送 Fusen timeout 与 attempt headers；内部 source 与 panic payload 永不进入 wire。

## 生产默认值

| 限制 | Client | Server |
| --- | ---: | ---: |
| 请求 deadline | 10 秒 | 最大 30 秒 |
| 停机 deadline | 30 秒 | 30 秒 |
| connect/startup | 3 秒 | 30 秒 |
| registry operation | 5 秒 | 5 秒 |
| 并发请求 | 1024 | 1024 |
| endpoint 并发 attempt | 128 | - |
| 单请求/响应 body | 各 2 MiB | 各 2 MiB |
| 请求/响应全局字节预算 | 各 64 MiB | 各 64 MiB |
| TCP 连接 | - | 2048 |
| 单 H2 连接 stream | - | 128 |

队列默认关闭。通过 `QueueConfig::builder().capacity(...).max_wait(...).build()?` 构建队列配置，再由 `ClientAdmissionConfigBuilder` 安装即可启用有界队列；等待时间仍计入逻辑 deadline，其他 admission 与 byte budget 均 fail-fast。

Byte budget 覆盖 runtime 持有的 decoded/encoded payload，以及 Hyper 消费或取消前的排队 body chunk。协议 framing、HPACK/H2 codec staging 和 OS socket buffer 是独立有界的 transport overhead，不计入 body budget。

## Workspace

| Crate | 职责 |
| --- | --- |
| `fusen-contract` | Service、HTTP binding、capability、Endpoint、Instance 等纯值对象 |
| `fusen-register` | Registry SPI、生命周期 handle、Directory snapshot |
| `fusen-config` | 静态解析与 last-good 热配置 |
| `fusen-nacos` | Nacos Registry 和热配置 adapter |
| `fusen-observability` | Metrics SPI 与可选 telemetry adapter |
| `fusen-procedural-macro` | 接口声明、参数校验与客户端/服务端 wrapper 生成 |
| `fusen-rs` | HTTP/HTTPS Client、明文 HTTP Server、Interceptor 与策略 runtime |

详见[架构](docs/architecture.md)、[模块契约](docs/modules/README.md)、[兼容性](docs/compatibility.md)与[示例](examples/README.md)。
