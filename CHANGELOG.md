# Changelog

本项目遵循语义化版本思想；0.x 的 minor 版本可能包含迁移文档明确说明的破坏性变更。

## [Unreleased]

### Added

- RFC 9457 错误契约、客户端/服务端类型化配置和 request ID。
- body、并发、deadline 限制以及事务化注册和连接排空。
- workspace 内管理的 Nacos、日志、配置与 StrategyDebug common crate。
- 架构、模块行为、迁移、贡献、安全和发布文档。
- 具备显式关闭语义的注册发现与热配置订阅。
- Rust 1.97/stable 双工具链本地发布检查，以及可选的 `NACOS_ADDR` 手工集成验证。

### Changed

- 寻址拆分为 `ClientEndpoint`，线协议拆分为 `WireProtocol`。
- 路由和 Directory 改为确定性不可变/快照模型。
- 所有 crate 统一升级到 0.9.0 和 Rust 1.97。
- 参数元数据改为 Path/Query/Body，客户端 deadline 覆盖完整调用。
- common 的 Nacos、OTel 和 YAML 改为可选 feature。
- 应用错误改为受验证的私有字段类型，注册错误与订阅关闭结果支持并发共享。
- Nacos naming/config 改为 listener-first 初始化，路由对分段后的路径执行严格百分号解码。
- Directory 拆分只读 reader/provider writer，订阅 cleanup 改为 executor-neutral 的共享终态协调器。
- 客户端增加订阅关闭 deadline 和关闭状态，收窄 Tokio 与 internal-common 依赖 feature。

### Removed

- 不完整的 Dubbo Triple/Prost 实现和不安全的错误 Send/Sync 声明。
