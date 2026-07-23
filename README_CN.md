# fusen-rs 0.9

`fusen-rs` 是一个面向 Rust 微服务的异步 RPC 框架。它通过过程宏在编译期生成客户端与服务端适配代码，不需要 IDL、代码生成脚本或额外脚手架，就能以接近本地 trait 调用的方式完成远程调用。

框架使用 JSON 作为数据格式，支持 HTTP/1.1、HTTP/2、Host 直连和 Nacos 注册发现，并提供负载均衡、Aspect 中间件、配置中心、结构化错误与优雅停机等微服务能力。0.9 版本重点完善了资源边界、超时、注册回滚和发现订阅关闭等可靠性契约。

[English](README.md)

## 功能特性

- 使用 `#[fusen_trait]` 定义共享接口，在编译期生成类型安全的客户端
- 使用 `#[fusen_service]` 实现服务，无需维护 IDL 或运行代码生成器
- 支持基于 JSON 的 HTTP/1.1 与 HTTP/2 调用
- 支持指定 URL 的 Host 直连和基于 Nacos 的服务注册与发现
- 支持服务分组、版本、HTTP method、路径、query 和 body 参数映射
- 支持自定义负载均衡与可嵌套的 Aspect 中间件
- 支持本地文件与 Nacos 配置中心，以及配置热更新
- 支持完整客户端 deadline、请求/连接并发限制和有界请求/响应 body
- 支持 RFC 9457 Problem Details 错误响应和服务端错误还原
- 支持启动失败时注册回滚、服务摘除和有总期限的优雅停机

> 0.9 暂不提供 Dubbo Triple。`application/grpc` 请求会收到 RFC 9457 格式的 `415 Unsupported Media Type` 响应。

## 快速开始

### 定义公共接口

服务端和客户端共享普通 Rust 数据结构与 trait。`#[fusen_trait]` 会生成对应的 `DemoServiceClient`；`#[asset]` 可用于覆盖默认路径和 HTTP method。

```rust
use fusen_rs::fusen_procedural_macro::fusen_trait;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestDto {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ResponseDto {
    pub message: String,
}

#[fusen_trait]
pub trait DemoService {
    async fn say_hello(&self, name: String) -> String;

    #[fusen_rs::fusen_procedural_macro::asset(path = "/hello")]
    async fn hello(&self, request: RequestDto) -> ResponseDto;

    #[fusen_rs::fusen_procedural_macro::asset(path = "/divide", method = GET)]
    async fn divide(&self, a: i32, b: i32) -> String;
}
```

### 实现服务

服务端实现的方法返回 `Result<T, FusenError>`。业务错误会转换为结构化的远端错误，客户端可以统一处理。

```rust
use fusen_rs::{error::FusenError, fusen_procedural_macro::fusen_service};

#[derive(Default)]
struct DemoServiceImpl;

#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn say_hello(&self, name: String) -> Result<String, FusenError> {
        Ok(format!("Hello {name}"))
    }

    async fn hello(&self, request: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            message: format!("Hello {}", request.name),
        })
    }

    async fn divide(&self, a: i32, b: i32) -> Result<String, FusenError> {
        if b == 0 {
            return Err(FusenError::InvalidRequest(
                "divisor must not be zero".to_owned(),
            ));
        }
        Ok(format!("a / b = {}", a / b))
    }
}
```

## Host 直连

Host 模式不依赖注册中心，适合本地开发、固定上游和测试环境。

服务端绑定端口并挂载服务：

```rust,no_run
use fusen_rs::server::FusenServerBuilder;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let bind_addr = "0.0.0.0:8081".parse()?;
FusenServerBuilder::new(bind_addr)
    .service((Box::new(DemoServiceImpl), None))?
    .run()
    .await?;
# Ok(())
# }
```

客户端通过 `ClientOptions::direct` 指定服务 URL。URL 可以包含 base path，但必须是 HTTP(S) URL，且不能包含 query 或 fragment。

```rust,no_run
use fusen_rs::client::{ClientOptions, FusenClientContextBuilder};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut context = FusenClientContextBuilder::new().build()?;
let client = DemoServiceClient::init(
    &mut context,
    ClientOptions::direct("http://127.0.0.1:8081".parse()?),
)
.await?;

println!("{}", client.say_hello("fusen".to_owned()).await?);
client.close().await?;
# Ok(())
# }
```

## Nacos 注册与发现

Nacos 模式下，服务端启动后注册服务，客户端按 trait 中的 service/group/version 订阅实例。目录更新以不可变快照发布，客户端只读取健康、启用且权重大于零的实例。

服务端需要同时配置 Nacos 和一个可被客户端访问的 `advertised_base_url`。框架先校验路由并绑定端口，再执行注册；任一服务注册失败时会按相反顺序回滚已经完成的注册。

```rust,no_run
use std::sync::Arc;
use fusen_common::nacos::{NacosConfig, register::NacosRegister};
use fusen_rs::server::{FusenServerBuilder, ServerConfig};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let bind_addr = "0.0.0.0:8081".parse()?;
let mut config = ServerConfig::new(bind_addr);
config.advertised_base_url = Some("http://10.0.0.8:8081".to_owned());

let register = NacosRegister::init_nacos_register(
    "demo-server",
    Arc::new(NacosConfig {
        server_addr: "127.0.0.1:8848".to_owned(),
        ..Default::default()
    }),
)?;

FusenServerBuilder::new(bind_addr)
    .config(config)
    .register(register)
    .service((Box::new(DemoServiceImpl), None))?
    .run()
    .await?;
# Ok(())
# }
```

客户端把同一个 `NacosRegister` 装配到 context，并使用 `ClientOptions::discovery`。发现客户端结束使用时应调用 `close().await`，以取消 Nacos 订阅并释放 listener。

```rust,no_run
use std::sync::Arc;
use fusen_common::nacos::{NacosConfig, register::NacosRegister};
use fusen_rs::{
    client::{ClientOptions, FusenClientContextBuilder},
    contract::WireProtocol,
};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let register = NacosRegister::init_nacos_register(
    "demo-client",
    Arc::new(NacosConfig {
        server_addr: "127.0.0.1:8848".to_owned(),
        ..Default::default()
    }),
)?;
let mut context = FusenClientContextBuilder::new()
    .register(register)
    .build()?;
let client = DemoServiceClient::init(
    &mut context,
    ClientOptions::discovery(WireProtocol::Fusen),
)
.await?;

println!("{}", client.say_hello("nacos".to_owned()).await?);
client.close().await?;
# Ok(())
# }
```

## 自定义组件

客户端和服务端都可以通过 handler 扩展调用链。handler 需要先装配到 context 或 server，再通过字符串 ID 按顺序绑定到指定服务。

### LoadBalance

`LoadBalance` 从当前发现快照中选择服务实例，可实现随机、轮询、一致性哈希或业务路由等策略。

```rust,no_run
use std::sync::Arc;
use fusen_rs::{
    contract::ServiceInstance,
    error::FusenError,
    fusen_procedural_macro::handler,
    handler::loadbalance::LoadBalance,
    protocol::fusen::context::FusenContext,
};

struct FirstAvailable;

#[handler(id = "FirstAvailable")]
impl LoadBalance for FirstAvailable {
    async fn select(
        &self,
        _context: &FusenContext,
        invokers: Arc<Vec<Arc<ServiceInstance>>>,
    ) -> Result<Option<Arc<ServiceInstance>>, FusenError> {
        Ok(invokers.first().cloned())
    }
}
```

### Aspect

`Aspect` 提供环绕调用模型，可以用于日志、鉴权、限流、熔断、指标和链路追踪，并支持多个 Aspect 嵌套执行。

```rust,no_run
use std::time::Instant;
use fusen_rs::{
    error::FusenError,
    filter::ProceedingJoinPoint,
    fusen_procedural_macro::handler,
    handler::aspect::Aspect,
    protocol::fusen::context::FusenContext,
};

struct TimeAspect;

#[handler(id = "TimeAspect")]
impl Aspect for TimeAspect {
    async fn around(
        &self,
        join_point: ProceedingJoinPoint,
    ) -> Result<FusenContext, FusenError> {
        let started = Instant::now();
        let result = join_point.proceed().await;
        tracing::info!(elapsed_ms = started.elapsed().as_millis(), "request completed");
        result
    }
}
```

完整的日志、OpenTelemetry tracing 和自定义负载均衡实现位于 [`examples/src/handler`](examples/src/handler)。

## 0.9 可靠性约束

- 客户端默认连接超时 3 秒、完整调用超时 10 秒、发现超时 5 秒、响应 body 上限 2 MiB
- 服务端默认请求超时 30 秒、请求并发 1024、连接数 2048、单连接 HTTP/2 stream 数 128
- 服务端请求 body 默认上限 2 MiB，HTTP/1 header 默认最多读取 10 秒
- 注册操作默认超时 5 秒；停机时先摘除服务，再在总期限内排空连接
- Direct、Nacos endpoint 和路由在启动阶段完成校验，避免把配置错误推迟到第一次请求
- 非 2xx 响应使用 Problem Details 表达，并在客户端还原为 `FusenError::Remote`

这些默认值都可以通过 `ClientConfig` 和 `ServerConfig` 调整，所有资源限制与 timeout 必须大于零。

## 运行示例

示例已经按寻址方式分开：

```text
examples/src/
├── host/
│   ├── server.rs
│   ├── client.rs
│   ├── server_pt.rs
│   └── client_pt.rs
└── nacos/
    ├── server.rs
    ├── client.rs
    └── hot_config.rs
```

Host 直连示例不需要外部组件：

```bash
cargo run -p examples --bin host-server
cargo run -p examples --bin host-client
```

压测请使用关闭了逐请求日志和 tracing 的专用服务端。客户端默认执行 HTTP/2 压测；设置 `PT_PROTOCOL=both` 会用相同负载依次测试 HTTP/1.1 和 HTTP/2，并输出 QPS 与吞吐倍率：

```bash
cargo run --release -p examples --bin host-server-pt
PT_PROTOCOL=both PT_CONCURRENCY=100 PT_REQUESTS_PER_TASK=10000 \
cargo run --release -p examples --bin host-client-pt
```

HTTP/1.1 通过连接池增加连接承载并发；HTTP/2 可在单连接中多路复用 stream，并使用 HPACK 压缩 header。字节统计为应用层 JSON body 的实际序列化长度，不包含 HTTP 帧头、TCP/IP 与 TLS 开销。完整参数与协议差异见 [`examples/README.md`](examples/README.md)。

Nacos 示例需要先启动 Nacos，然后在两个终端分别运行。`NACOS_ADDR` 默认使用 `127.0.0.1:8848`，`FUSEN_ADVERTISED_URL` 默认使用 `http://127.0.0.1:8081`，下面的环境变量用于覆盖默认值：

```bash
NACOS_ADDR=127.0.0.1:8848 \
FUSEN_ADVERTISED_URL=http://127.0.0.1:8081 \
cargo run -p examples --bin nacos-server

NACOS_ADDR=127.0.0.1:8848 \
cargo run -p examples --bin nacos-client
```

容器或多机环境中的 `FUSEN_ADVERTISED_URL` 必须填写客户端实际可访问的地址。更多说明见 [`examples/README.md`](examples/README.md)。

## 文档导航

- [架构与调用链](docs/architecture.md)
- [模块行为索引](docs/modules/README.md)
- [客户端](docs/modules/client.md)
- [服务端](docs/modules/server.md)
- [注册发现与 Nacos](docs/modules/registry-nacos.md)
- [中间件与过程宏](docs/modules/middleware-macros.md)
- [配置](docs/modules/configuration.md)
- [路由](docs/modules/routing.md)
- [错误模型](docs/modules/errors.md)
- [优雅停机](docs/modules/shutdown.md)
- [0.8 到 0.9 迁移指南](docs/migration-0.9.md)
- [兼容性策略](docs/compatibility.md)
- [贡献指南](CONTRIBUTING.md)
- [行为准则](CODE_OF_CONDUCT.md)
- [安全策略](SECURITY.md)
- [发布流程](docs/releasing.md)
