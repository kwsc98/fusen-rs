# fusen-rs 0.9

`fusen-rs` 是面向 Rust 微服务的生产级异步 JSON RPC 框架。0.9 是一次 clean-slate 的 API 与 wire 基线：提供生成式客户端和服务端、明确的生命周期所有权、有界资源、服务发现、重试、熔断、中间件与结构化可观测性。

[English](README.md)

## 运行边界

- Rust 1.97、Edition 2024、Tokio 与 JSON。
- Client 支持 canonical `http://` 与 `https://` endpoint。Fusen V1 在 HTTP 上使用 h2c、在 HTTPS 上使用 TLS/ALPN `h2`；Spring Cloud V1 在两种 scheme 上都使用 HTTP/1.1。
- Client HTTPS 使用 Rustls Ring、TLS 1.2/1.3、bundled Mozilla WebPKI roots 和严格的证书/hostname 验证。内置 Server 保持明文；入站 TLS 由 ingress、sidecar、反向代理或 service mesh 终止。
- 稳定扩展面仅包括 `Middleware`、`Registry`、`ConfigSource`、`InstanceRouter`、`LoadBalancer`、`RetryPolicy` 和 `MetricsRecorder`。
- Transport、Codec、Acceptor、连接池与生命周期状态机均为 runtime 私有实现。

## 接口契约

一个 trait 宏定义 Client/Server 共享接口；每个 RPC 方法可直接接收零到多个具名的 owned 参数，并返回 `Result<RpcResponse<T>, RpcError>`。

```rust,no_run
use fusen_rs::{RpcError, RpcResponse, interface};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct User { pub id: String }

#[derive(Serialize, Deserialize)]
pub struct CreateUser {
    pub id: String,
}

#[interface(name = "user", group = "prod", version = "1")]
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

    #[fusen_rs::method(
        idempotency = "none",
        spring(method = "POST", path = "/users")
    )]
    async fn create(
        &self,
        #[rpc(body)] user: CreateUser,
    ) -> Result<RpcResponse<User>, RpcError>;
}
```

宏生成 `UserApiClient` 与 `UserApiServer<T>`。生成 Client 和用户 Handler 都实现 `UserApi`，所有 Client 统一使用 `ClientBuilder<UserApiClient>`。每个业务参数通过 `#[rpc(path)]`、`#[rpc(query)]` 或 `#[rpc(body)]` 声明 Spring wire 角色，并可用 `name = "..."` 覆盖 wire name；每个方法最多一个 body，重复 query 使用 `Vec<T>`。Path/query 必须序列化为 JSON 标量，body 可为任意 JSON 值。需要请求 headers、extensions 或框架调用信息的方法可以额外声明一个 `#[rpc(call)] call: RpcCall` 参数。无业务入参的方法无需占位参数。Fusen V1 始终按名称把全部业务参数编码到 `arguments` object；非法参数映射在宏展开阶段失败，非法序列化值在网络 I/O 前于本地失败。

## 客户端

`ClientRuntime` 统一持有 admission、字节预算、Middleware、发现订阅、连接池、重试预算和熔断器。

```rust,no_run
# use fusen_rs::{ClientError, ClientRuntime, WireProtocol};
# use crate::UserApiClient;
# async fn run() -> Result<(), ClientError> {
let runtime = ClientRuntime::builder().build()?;

let client = UserApiClient::builder(&runtime)
    .direct("http://127.0.0.1:8081")
    .protocol(WireProtocol::FusenV1)
    .connect()
    .await?;

// 生成的 RPC 方法返回 Result<RpcResponse<T>, RpcError>。
runtime.shutdown().await?;
# Ok(())
# }
```

Direct client 使用 `https://`，或从 Registry 发现 HTTPS instance，即可启用客户端
TLS。Runtime 不读取系统 trust store，也不提供自定义 CA、mTLS 或跳过证书验证；
私有 CA 与自签名 endpoint 不属于 0.9 契约。

在 runtime builder 安装一个 `Registry` 后，用 `.discover()` 替代 `.direct(...)` 即可启用发现。每个 `(ServiceSelector, WireProtocol)` 共享唯一订阅，latest-wins 快照状态为 `Initializing`、`Ready`、`Stale`、`Unavailable` 或 `Closed`。

一个绝对 deadline 覆盖 admission、Middleware、全部 attempts、退避、传输与 decode。只有声明为 `idempotent` 或 `safe` 的方法可重试；内置策略最多执行三次总 attempts，并受每服务 token budget 的硬约束。每次物理 attempt 都重新读取发现快照，并应用 endpoint/service 熔断器和 endpoint bulkhead。

如果 HTTP/wire 成功响应中的 `result` 无法反序列化为生成方法声明的 Rust 类型，调用会以 `DataLoss`/`invalid_result` 非重试终止；该 selected endpoint attempt 与 service 最终结果都会按 protocol failure 计入对应熔断器。

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

## Wire V1

Fusen V1 在 `http://` 上使用 h2c，在 `https://` 上使用 TLS/ALPN `h2`：

```text
POST /_fusen/v1/{service}/{method}
Content-Type: application/fusen+json;version=1
{"arguments":{"name":...}}
{"result":...}
```

Spring Cloud V1 按方法声明的 HTTP method/path/query/body 映射传输 `application/json`，成功响应为 raw JSON；它是明确子集，不承诺完整 Spring MVC 兼容。

两种协议共享 `x-request-id`、`x-fusen-timeout-ms` 与 `x-fusen-attempt`。错误统一为 `application/problem+json`，包含 RFC 9457 字段及 `code`、`request_id`、`retryable`；内部 source 与 panic payload 永不进入 wire。

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
| `fusen-contract` | Service、Method、Protocol、Endpoint、Instance 等纯值对象 |
| `fusen-register` | Registry SPI、生命周期 handle、Directory snapshot |
| `fusen-config` | 静态解析与 last-good 热配置 |
| `fusen-nacos` | Nacos Registry 和热配置 adapter |
| `fusen-observability` | Metrics SPI 与可选 telemetry adapter |
| `fusen-procedural-macro` | 接口声明、参数校验与客户端/服务端 wrapper 生成 |
| `fusen-rs` | HTTP/HTTPS Client、明文 HTTP Server、Middleware 与策略 runtime |

详见[架构](docs/architecture.md)、[模块契约](docs/modules/README.md)、[兼容性](docs/compatibility.md)与[示例](examples/README.md)。
