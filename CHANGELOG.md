# Changelog

本项目遵循语义化版本思想；0.x 的 minor 版本可能包含迁移文档明确说明的破坏性变更。

## [Unreleased]

## [0.9.0] - 2026-07-19

### Added

- RFC 9457 错误契约、客户端/服务端类型化配置和 request ID。
- body、并发、deadline 限制以及事务化注册和连接排空。
- workspace 内管理的 Nacos、日志、配置与 StrategyDebug common crate。
- 架构、模块行为、迁移、贡献、安全和发布文档。

### Changed

- 寻址拆分为 `ClientEndpoint`，线协议拆分为 `WireProtocol`。
- 路由和 Directory 改为确定性不可变/快照模型。
- 所有 crate 统一升级到 0.9.0 和 Rust 1.85。

### Removed

- 不完整的 Dubbo Triple/Prost 实现和不安全的错误 Send/Sync 声明。
