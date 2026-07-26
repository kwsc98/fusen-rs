# Changelog

本项目遵循语义化版本思想；当前 `0.9.0` 仍处于开发阶段，Unreleased API 可以直接破坏性调整。

## [Unreleased]

### Added

- 新增客户端和服务端共用的 `InvocationObserver`，完整报告阶段、耗时、HTTP 状态、错误 code、timeout 与 cancellation。
- 新增 `ClientRuntime`、生成式服务 Client Builder/Server wrapper，以及 Router/LoadBalancer 扩展层。
- 新增独立的 H1/H2 客户端连接池配置，以及 H2 每地址多连接分片和无锁轮询。
- 新增调用链、Observer、客户端分派和 1/64 KiB codec 微基准，以及多轮中位数 HTTP 压测矩阵。
- RFC 9457 错误契约、客户端/服务端类型化配置和 request ID。
- body、并发、deadline 限制以及事务化注册和连接排空。
- workspace 内职责独立的 `fusen-nacos`、`fusen-config`、`fusen-observability` 与 `StrategyDebug` 配置宏。
- 架构、模块行为、贡献、安全和发布文档。
- 具备显式关闭语义的注册发现与热配置订阅。
- Rust 1.97/stable 双工具链本地发布检查，以及可选的 `NACOS_ADDR` 手工集成验证。

### Changed

- workspace 第三方依赖升级到当前最新版本并统一集中管理；Nacos 0.8 的客户端构造改为异步初始化。
- 调用链改为 Observer、统一 Middleware、ClusterInvoker 和 MethodId ServiceInvoker 分层。
- 请求/响应 API 收敛为 `RpcContext`、`RpcResponse` 与 `RpcResult`，统一使用绝对 deadline。
- 客户端按 `MethodId` 分派并传递类型化 endpoint；服务端启动时预绑定路由、middleware 与 service invoker。
- codec 移除 `Bytes` clone，并按受限 `size_hint` 预分配 body buffer。
- 寻址拆分为 `ClientEndpoint`，线协议拆分为 `WireProtocol`。
- 路由和 Directory 改为确定性不可变/快照模型。
- 所有 crate 统一升级到 0.9.0 和 Rust 1.97。
- 参数元数据改为 Path/Query/Body，客户端 deadline 覆盖完整调用。
- Nacos、配置与 OTel 从 common 拆分为独立 crate，YAML/OTel 保持可选 feature。
- 应用错误改为受验证的私有字段类型，注册错误与订阅关闭结果支持并发共享。
- Nacos naming/config 改为 listener-first 初始化，路由对分段后的路径执行严格百分号解码。
- Directory 拆分只读 reader/provider writer，订阅 cleanup 改为 executor-neutral 的共享终态协调器。
- 客户端增加订阅关闭 deadline 和关闭状态，收窄 Tokio 依赖 feature。
- RPC 宏生成静态描述、服务专属 Client Builder、Server wrapper 和 O(1) dispatch。
- 服务描述统一为 `fusen-contract::ServiceDescriptor`，客户端、服务端与注册中心复用同一静态对象。
- 相同 selector/protocol 的 discovery client 复用订阅，Direct client 不再进入 shutdown 清理集合。
- 服务端优雅停机改为先关闭 listener，再在一个共享 deadline 内并行逆序注销与排空连接；Unix 默认同时响应 SIGINT/SIGTERM，停机失败向调用方返回错误，并为 Server future 取消提供有界的后台注销补偿。

### Removed

- 删除 `FusenFilter`、`ProceedingJoinPoint`、Aspect、Handler、HandlerLoad、字符串 handler ID、`ClientOptions`、逐客户端 close 和公开 terminal。
- 不完整的 Dubbo Triple/Prost 实现和不安全的错误 Send/Sync 声明。
- `fusen-internal-common`、`ServiceResource` 和 `BoxFutureV2`；稳定共享契约迁移到 `fusen-contract`。
- `ClusterPolicy`、`SingleAttempt` 和职责过宽的 `fusen-common` 兼容入口。
