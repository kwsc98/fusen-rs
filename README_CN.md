# fusen-rs 0.9

`fusen-rs` 是一个面向 Rust 微服务的异步 JSON RPC 框架。`#[fusen_trait]` 会生成类型安全客户端、服务专属 Builder、静态方法描述和服务端 wrapper；`#[fusen_service]` 将 Rust 实现绑定到按声明顺序生成的 `MethodId` 分派。

框架支持 HTTP/1.1、HTTP/2、直连、Nacos 注册发现、可插拔 Cluster 选择、类型化 Middleware、完整生命周期观察、有界 body、绝对 deadline、Problem Details 和优雅停机。

[English](README.md)

## 核心模型

```text
ClientRuntime -> Middleware -> Router -> LoadBalancer -> HTTP
Server        -> admission/decode/route -> Middleware -> MethodId dispatch -> encode
```

- 默认一次逻辑 RPC 只执行一次 HTTP attempt，不做隐式重试。
- `Next` 消费自身且不可克隆，下游最多执行一次。
- `InvocationObserver` 覆盖 Middleware 外的错误、timeout 和 cancellation。
- Nacos 使用不可变快照；未配置 Router 时直接复用当前快照，不复制实例列表。
- Fusen HTTP/2 与 SpringCloud HTTP/1.1 的 JSON wire format 保持不变。

## 定义与实现服务

```rust,no_run
use fusen_rs::{FusenError, fusen_service, fusen_trait};

#[fusen_trait]
pub trait DemoService {
    async fn say_hello(&self, name: String) -> String;
}

pub struct DemoServiceImpl;

#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn say_hello(&self, name: String) -> Result<String, FusenError> {
        Ok(format!("Hello {name}"))
    }
}
```

`MethodId(u16)` 按 trait 声明顺序稳定生成。实现方法可以用任意顺序书写，生成的 O(1) dispatch 不做方法名字符串比较。

## 客户端

一个 `ClientRuntime` 统一持有连接池、全局 Middleware、Observer、发现订阅与关闭状态。生成客户端只保存服务局部配置和共享 runtime 引用。

```rust,no_run
use fusen_rs::{ClientRuntime, FusenError, fusen_trait};

#[fusen_trait]
trait DemoService {
    async fn say_hello(&self, name: String) -> String;
}

async fn example() -> Result<(), FusenError> {
let runtime = ClientRuntime::builder()
    .build()?;

let client = DemoServiceClient::builder(&runtime)
    .direct("http://127.0.0.1:8081")
    .connect()
    .await?;

let value = client.say_hello("fusen".into()).await?;
runtime.shutdown().await?;
Ok(())
}
```

注册发现仍使用同一个生成 Builder：

```rust,no_run
use fusen_rs::{ClientRuntime, FusenError, fusen_trait};

#[fusen_trait]
trait DemoService {
    async fn say_hello(&self, name: String) -> String;
}

async fn example() -> Result<(), FusenError> {
let runtime = ClientRuntime::builder().build()?;
let client = DemoServiceClient::builder(&runtime)
    .discover()
    .connect()
    .await?;
runtime.shutdown().await?;
Ok(())
}
```

`shutdown()` 幂等关闭 runtime 持有的所有订阅，并拒绝新客户端和 RPC。Drop 只提供尽力而为的后台兜底，应用仍应显式 shutdown。

H1 与 H2 连接池可以在 runtime 上分别配置：

```rust,no_run
use fusen_rs::{ClientRuntime, FusenError, Http1PoolConfig, Http2PoolConfig};

fn example() -> Result<(), FusenError> {
let runtime = ClientRuntime::builder()
    .http1_pool(Http1PoolConfig {
        max_idle_per_host: 256,
        ..Http1PoolConfig::default()
    })
    .http2_pool(Http2PoolConfig {
        connections_per_host: 4,
        ..Http2PoolConfig::default()
    })
    .build()?;
Ok(())
}
```

`max_idle_per_host` 限制 H1 每个地址保留的空闲连接数，不限制正在使用的并发连接；设为 0 会关闭 H1 连接复用。H2 分片按地址懒建连接，并根据 endpoint 和 request ID 做无锁稳定哈希选择。

## 服务端

普通服务直接注册实现对象；只有需要服务局部 Middleware 时才使用宏生成的 `*Server` wrapper。

```rust,no_run
use fusen_rs::{FusenError, Server, fusen_service, fusen_trait};

#[fusen_trait]
trait DemoService {
    async fn ping(&self) -> String;
}

struct DemoServiceImpl;

#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn ping(&self) -> Result<String, FusenError> {
        Ok("pong".into())
    }
}

async fn example() -> Result<(), FusenError> {
Server::bind("0.0.0.0:8081")
    .service(DemoServiceImpl)
    .run()
    .await?;
Ok(())
}
```

启动过程先校验服务与路由、绑定监听器，再事务化注册 provider。每条路由会预绑定静态方法描述、不可变 Middleware slice 和 service invoker。并发准入保持 fail-fast，同一个绝对 deadline 覆盖 decode、route、Middleware、service dispatch 与 response encode。

## Middleware

客户端和服务端共用一个用户 trait，不需要注册宏、字符串 ID、`BoxFuture` 或自定义 terminal。

```rust,no_run
use fusen_rs::{Middleware, Next, RpcContext, RpcResult};

struct AuthMiddleware;

impl Middleware for AuthMiddleware {
    async fn handle<'a>(&'a self, mut context: RpcContext, next: Next<'a>) -> RpcResult {
        context.metadata_mut().insert("tenant".into(), "acme".into());
        next.run(context).await
    }
}
```

全局 Middleware 先进入，服务局部 Middleware 后进入，退出顺序相反。客户端 Middleware 在 Router 与负载均衡之前执行，可以设置租户、灰度或一致性哈希 metadata；服务端 Middleware 在 HTTP 路由后执行。Middleware 可以不调用 `next`，直接返回错误或显式 `RpcResponse`。

完整请求日志和指标应使用 `InvocationObserver`。Observer 同步、按注册顺序执行，并恰好产生一次 finish；事件不会暴露 body、凭据或完整 headers。Future 取消时不保证 Middleware 后置代码执行，资源清理应使用 RAII。

## Cluster 扩展

高级客户端接口集中在 `client::cluster`：

- `Router` 过滤或重排 `InstanceSnapshot`。
- `LoadBalancer` 返回快照中的一个索引。
- 默认负载均衡按已校验的 provider 权重随机选择。

空快照、Router 清空结果和非法索引统一返回 `FusenError::ServiceUnavailable`。当前阶段不提供 retry、退避、熔断或多 attempt API。

## 默认约束

- 客户端连接超时 3 秒，完整调用超时 10 秒
- 发现和订阅清理超时 5 秒
- H1 每个地址最多保留 128 条空闲连接，空闲 90 秒后回收
- H2 每个地址默认一条连接，空闲 90 秒后回收，默认关闭 ping 保活
- 请求与响应 body 上限 2 MiB
- 服务端请求超时 30 秒
- 请求并发 1024、连接 2048、单连接 HTTP/2 stream 128
- 优雅停机 30 秒、注册操作 5 秒

这些值通过 `ClientConfig` 和 `ServerConfig` 配置。配置的 duration 和必要 pool size 必须大于零；H1 的 `max_idle_per_host` 可以为 0，表示关闭复用。非 2xx Problem Details 会还原为 `FusenError::Remote`。

## 示例与压测

```bash
cargo run -p examples --bin host-server
cargo run -p examples --bin host-client

cargo run --release -p examples --bin host-server-pt
PT_PROTOCOL=both PT_CONCURRENCY=1,100 PT_ROUNDS=5 \
cargo run --release -p examples --bin host-client-pt
```

Nacos 示例使用 `NACOS_ADDR`，服务端还需要 `FUSEN_ADVERTISED_URL`。详见 [examples/README.md](examples/README.md) 和 [性能基线](docs/performance-baseline.md)。
