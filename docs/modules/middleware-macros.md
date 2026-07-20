# 中间件与宏行为

> English summary: generated adapters serialize arguments and compose a
> deterministic aspect chain around transport or service execution.

handler ID 在同一 context 中必须唯一，引用未知 ID 会失败。每个服务最多选择一个 load balancer，多个配置时以最后一个为准；Aspect 保持声明顺序并允许调用 `proceed`。

`fusen_trait` 是 id/group/version/path/method/参数来源的唯一元数据来源，并生成返回 `Result<_, FusenError>` 的客户端；`fusen_service` 只接受对应 trait impl 并复用隐藏元数据入口。实现侧重复 `asset` 或 service 元数据会生成编译错误。宏通过 `proc-macro-crate` 支持依赖重命名。
