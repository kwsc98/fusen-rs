# 注册发现与 Nacos 行为

> English summary: discovery publishes atomic snapshots and Nacos implements
> the workspace-owned Register contract.

`Register` 负责注册、摘除和订阅，全部错误必须可跨线程并可克隆。register/deregister 必须幂等；注册结果明确失败或因 timeout/cancellation 而不确定时，调用方可以对当前资源和已成功资源逆序补偿注销。订阅返回 `ServiceSubscription`，包含 watch Directory、幂等异步 close 和最后引用释放时的一次性关闭通知；并发 close 等待同一次后台清理并共享相同结果，取消任一等待者不取消 unsubscribe。

Nacos naming/config 均先安装 listener，再读取初始快照；初始化窗口内的事件采用 latest-wins，后续失败或取消由 setup guard 在后台移除 listener。Nacos 只发布 healthy、enabled、正权重实例，并用保留 metadata 保存 scheme、base path、service/version/group；IPv6 和 HTTPS 地址通过 URL API 重建。默认负载均衡按有效权重选择。需要验证真实 Nacos 时，显式设置 `NACOS_ADDR`，并使用 `cargo test -p fusen-common --all-features live_nacos_registration_when_configured -- --ignored` 运行手工集成测试。
