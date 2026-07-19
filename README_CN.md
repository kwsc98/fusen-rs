# fusen-rs 0.9

`fusen-rs` 是面向 Rust 微服务的异步 RPC 框架，通过过程宏生成客户端和服务端适配代码，支持 JSON over HTTP/1.1、HTTP/2、Direct 寻址、Nacos 注册发现、中间件与优雅停机。

0.9 的重点是可靠性契约：有界请求体、客户端和服务端 deadline、并发限制、启动注册回滚、RFC 9457 错误、确定性路由以及可等待的连接排空。Dubbo Triple 在本版本明确禁用，不再声明不完整的兼容能力。

## 文档导航

- [架构与调用链](docs/architecture.md)
- [模块行为索引](docs/modules/README.md)
- [0.8 到 0.9 迁移指南](docs/migration-0.9.md)
- [兼容性策略](docs/compatibility.md)
- [贡献指南](CONTRIBUTING.md)
- [安全策略](SECURITY.md)
- [发布流程](docs/releasing.md)

## 客户端

```rust,no_run
use fusen_rs::client::{ClientOptions, FusenClientContextBuilder};

# async fn demo() -> Result<(), Box<dyn std::error::Error>> {
let mut context = FusenClientContextBuilder::new().build();
let options = ClientOptions::direct("http://127.0.0.1:8081".parse()?);
// let client = DemoServiceClient::init(&mut context, options).await?;
# let _ = (context, options);
# Ok(())
# }
```

注册发现使用 `ClientOptions::discovery(WireProtocol::Fusen)`，并在 `FusenClientContextBuilder` 上配置 workspace 内的 `NacosRegister`。

## 服务端

```rust,no_run
use fusen_rs::server::{FusenServerBuilder, ServerConfig};

# async fn demo() -> Result<(), Box<dyn std::error::Error>> {
let bind = "0.0.0.0:8081".parse()?;
let mut config = ServerConfig::new(bind);
config.advertised_base_url = Some("http://10.0.0.8:8081".into());
let server = FusenServerBuilder::new(bind).config(config);
// let server = server.service((Box::new(DemoServiceImpl), None))?;
// server.run().await?;
# let _ = server;
# Ok(())
# }
```

启用注册中心时必须提供外部可访问的 `advertised_base_url`。框架先完成路由校验和端口绑定，再注册服务；任何注册失败都会逆序回滚。
