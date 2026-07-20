# 0.8 到 0.9 迁移指南

> English summary: 0.9 intentionally breaks endpoint, server, error, and
> registry APIs to establish reliable runtime contracts.

## 客户端

将 `Protocol::Host(url)` 替换为 `ClientOptions::direct(url.parse()?)`；注册发现使用 `ClientOptions::discovery(WireProtocol::Fusen)`。`FusenClientContextBuilder::build()` 现在返回 `Result`，发现客户端应在结束时调用生成的 `close().await`。`ClientConfig::subscription_close_timeout` 控制调用方等待清理的最长时间，关闭开始后客户端不再接受新调用。

## 服务端

将 `FusenServerContext::new(port)` 替换为 `FusenServerBuilder::new(SocketAddr)`。启用注册时在 `ServerConfig` 中设置 `advertised_base_url`。新增连接、HTTP header/stream、注册操作 timeout 配置；默认值由 `ServerConfig::new` 提供。

## 错误与协议

删除 `ErrorMessage`、`Impossible`、`HttpError` 和不安全的任意 error variant。使用语义化 `FusenError`；应用错误改用 `FusenError::application(status, code, message)?`，不再直接构造公开字段。远端错误为 RFC 9457 Problem Details。`Protocol` 拆分为 `ClientEndpoint` 与 `WireProtocol`。

Dubbo/Triple、`Protocol::Dubbo` 和 Prost codec 已删除。调用方必须使用 JSON HTTP，或继续停留在明确支持 Triple 的旧版本。

## 注册中心

`Register` 接受 `WireProtocol`，错误源要求 `Send + Sync` 并通过 `Arc` 支持 `Clone`；register/deregister 实现必须幂等。`subscribe` 改为返回 `ServiceSubscription`。

注册 provider 使用 `directory_channel(initial)` 获得 `DirectoryWriter` 和只读 `Directory`，listener 只持有 writer。旧的 `Directory::default/get/change/replace` 已删除。订阅 cleanup 使用 `subscription_cleanup()` 获得 closer/cleanup；provider 必须在自己的 executor 上运行 `cleanup.run(unsubscribe_future)`，然后把 closer 交给 `ServiceSubscription::new`。`SubscriptionLifecycle` trait 已删除。

## 宏与参数

所有 service id/group/version 和 `asset` 只保留在 `fusen_trait`，实现上的重复属性必须删除。参数根据模板和 HTTP method 自动分类为 Path/Query/Body；公开请求字段改为 HeaderMap、结构化 query 和原始 JSON body。

## 工具链

MSRV 提升为 Rust 1.97.0，workspace 提交 Cargo.lock 并使用 `--locked` 验证。`fusen-common` 的 Nacos、OTel 和 YAML 分别通过 feature 启用。
