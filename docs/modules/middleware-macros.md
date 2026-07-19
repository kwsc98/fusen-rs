# 中间件与宏行为

> English summary: generated adapters serialize arguments and compose a
> deterministic aspect chain around transport or service execution.

handler ID 在同一 context 中必须唯一，引用未知 ID 会失败。每个服务最多选择一个 load balancer，多个配置时以最后一个为准；Aspect 保持声明顺序并允许调用 `proceed`。

`fusen_trait` 生成返回 `Result<_, FusenError>` 的客户端；`fusen_service` 只接受 trait impl，并把参数错误映射为 400。宏输入错误必须生成编译诊断而不是过程宏 panic。
