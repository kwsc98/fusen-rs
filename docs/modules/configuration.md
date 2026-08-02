# 配置行为

> English summary: runtime configuration is immutable and private-field;
> dynamic business configuration publishes validated last-good snapshots.

## Runtime Config

`ClientConfig`、`ServerConfig` 与子配置字段均私有，只提供 `Default`、builder/setter 和 getter。它们不读取隐式环境变量。Build/start 在网络 I/O 前验证零值、预算关系、HTTP binding/capabilities 与 endpoint；`ServiceEndpoint` 只接受 canonical `http://`/`https://` URL。

Client TLS 没有公开配置面：固定使用 Rustls Ring、bundled Mozilla WebPKI roots 与 TLS 1.2/1.3，不读取系统 trust store。不支持私有 CA、自签名证书、自定义 CA、客户端证书/mTLS 或跳过验证。Server 配置不包含 TLS、证书或私钥字段；HTTPS advertisement 仅描述外部终止器。

默认请求/响应 body 各 2 MiB、全局字节预算各 64 MiB、并发请求 1024、队列关闭。Client connect 3 秒、调用 10 秒、shutdown 30 秒；Server startup/request/shutdown 上限均为 30 秒，registry operation 5 秒。Discovery initial/close 为 5 秒、max stale 30 秒、subscription 上限 1024。

Queue 只有通过 `QueueConfig::builder()` 配置非零 capacity 并安装到 admission 配置后才启用，max wait 始终受逻辑 deadline 约束。所有配置在 runtime 创建后不可变；热配置不能绕过资源与韧性硬上限。

## 静态与热配置

`fusen-config` 支持 TOML 静态解析，并在 `yaml` feature 下支持 YAML。公开且对象安全的 `ConfigSource` SPI 同步 prepare `ConfigHandle`；`provider` 模块提供 `ConfigPublisher` 与取消安全的 lifecycle 构造 API。调用方先持有 handle，再等待 `activate()`。Provider 发布带 revision 的文档，`HotConfig<T>` 只在反序列化成功时替换当前 typed snapshot，非法更新被报告但保留上一份合法值。

Activation/close waiter 的 timeout 或 drop 不取消 provider worker；late success 自动补偿。`ConfigHandle::close()` 幂等并共享终态，Drop 只请求关闭。`fusen-nacos` adapter 保持 SDK listener/channel 私有，并使用 listener-first 初始化防止丢更新。

Secret 不得出现在 Debug、错误、trace、metrics 或 Problem Details。Backend 初始化和 guard 由应用显式持有，库不安装进程级全局状态。
