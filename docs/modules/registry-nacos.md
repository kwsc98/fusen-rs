# 注册发现与 Nacos 行为

> English summary: discovery publishes atomic snapshots and Nacos implements
> the workspace-owned Register contract.

`Register` 负责注册、摘除和订阅，全部错误必须可跨线程。`Directory` 基于 Tokio watch，读取是无等待快照，replace 原子替换全部实例并通知订阅者。

Nacos 使用 advertised URL 构建实例，拒绝缺少 host/port 的地址。Fusen 服务名为 `providers:{service}:{version}:{group}`；SpringCloud 优先读取 `spring.application.name` metadata。真实 Nacos 测试仅在提供 `NACOS_ADDR` 时启用。
