# ADR 0001: 0.9 clean-slate 重构范围

- 状态：已接受
- 日期：2026-07-26
- 决策者：fusen-rs 维护者

## 背景

0.9 尚未发布，已有开发实现的公开 API、宏语法、wire body 和生命周期模型彼此牵制。为这些未发布入口增加兼容层，会把过渡设计永久变成用户契约。

## 决策

在保留 crate 名称与职责的前提下，将 0.9 作为全新项目原地重写：

- Rust API、宏语法、配置和 wire format 可以同时破坏；
- 不提供 alias、deprecated facade、legacy decoder、公开版本过渡模块或迁移 adapter；
- 保留 Tokio、JSON、Nacos、Rust 1.97、Edition 2024 与现有 crate 边界；
- 每阶段直接迁移 workspace、examples、fixtures 与文档，阶段结束必须恢复绿色；
- `v0.9.0` tag 是第一个兼容性 baseline，此前开发提交不构成兼容承诺；
- Fusen V1 与 Spring Cloud V1 从本次重构的 golden fixtures 开始形成明确 wire 契约。

`v0.9.0` 发布后才以该 tag 启用 API semver 检查。后续 wire 破坏必须定义新协议版本，而不是让现有 v1 decoder 猜测格式。

## 后果

旧用户代码、旧配置和旧流量不能直接迁移，也不会得到 runtime fallback。作为交换，0.9 不携带双生命周期、双错误体系或双 wire decoder，公开扩展面可以一次性收紧并作为稳定起点。

## 备选方案

- 新建平行 workspace/crate：拒绝，会复制依赖、测试与发布链路。
- 保留旧 API 并逐项弃用：拒绝，尚未发布的设计不值得形成长期维护成本。
- 只重写 Rust API、保留旧 wire：拒绝，旧 wire 的参数来源与错误格式无法满足新资源和重试契约。
