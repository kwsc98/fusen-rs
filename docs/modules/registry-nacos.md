# 注册发现与 Nacos 行为

> English summary: discovery publishes atomic snapshots and Nacos implements
> the workspace-owned Register contract.

`Register` 负责注册、摘除和订阅，全部错误必须可跨线程并可克隆。register/deregister 必须幂等；注册结果明确失败或因 timeout/cancellation 而不确定时，调用方可以对当前资源和已成功资源逆序补偿注销。

Nacos 适配器实现的是 `fusen-contract` 的 selector/registration/instance 模型，不再接收旧的 `ServiceResource`。订阅目录只暴露经过健康、启用和正权重过滤的 `ServiceInstance`。

发现快照通过 `directory_channel` 分离所有权：provider listener 独占 `DirectoryWriter`，消费者只能从 `Directory` 读取 `snapshot()` 或等待 `changed()`。最后一个 writer 释放后，`changed()` 返回 `DirectoryClosed`，消费者无法覆盖或注入地址。

订阅关闭使用 `subscription_cleanup` 创建 caller `SubscriptionCloser` 和 provider `SubscriptionCleanup`。provider 在自己的 executor 上运行 cleanup future；并发 close 共享同一结果，取消等待者不取消 cleanup，task abort/panic 或 cleanup 未启动统一返回 `CleanupAborted`。`fusen-register` 本身不启动 Tokio task，固定本地订阅可在非 Tokio executor 中关闭。

Nacos naming/config 均先安装 listener，再读取初始快照；初始化窗口内的事件采用 latest-wins，后续失败或取消由 setup guard 在后台移除 listener。unsubscribe task 持有原始 listener 和 DirectoryWriter，完成后目录进入关闭状态。Nacos 只发布 healthy、enabled、正权重实例，并用保留 metadata 保存 scheme、base path、service/version/group；IPv6 和 HTTPS 地址通过 URL API 重建。默认负载均衡按有效权重选择。需要验证真实 Nacos 时，显式设置 `NACOS_ADDR`，并使用 `cargo test -p fusen-common --all-features live_nacos_registration_when_configured -- --ignored` 运行手工集成测试。
