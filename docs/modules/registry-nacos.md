# 注册发现与 Nacos

> English summary: synchronous prepare returns an owned lifecycle handle;
> activate/close waiters are cancellation-safe and directory updates are latest-wins.

## Registry SPI

`Registry` 只提供同步 `prepare_registration(RegistrationRequest) -> RegistrationHandle` 与 `prepare_subscription(SubscriptionRequest) -> SubscriptionHandle`。Prepare 返回前，handle 已拥有 provider worker、远端资源身份和补偿状态；runtime 必须先追踪 handle，再等待 `activate().await`。

取消或 timeout 只取消当前 waiter，不取消 provider worker。Activation late success 且已无人等待时，worker 自动请求一次补偿 close。`close()` 幂等，并发调用共享唯一终态；Drop 只请求关闭。Provider activate/close panic 转为 `RegistryError`，补偿继续处理其他 handle。公开 API 不泄漏 Tokio channel、cleanup coordinator 或 Nacos SDK listener 类型。

## Directory

`DirectorySnapshot` 包含严格递增的 `revision`、`observed_at`、`DirectoryState` 与 `Arc<[ServiceInstance]>`。状态为：

- `Initializing`：尚无可用初始快照；
- `Ready`：provider 当前可用；
- `Stale`：使用有期限的 last-good 实例；
- `Unavailable`：不可路由并 fail fast；
- `Closed`：subscription 已到达终态。

更新使用 watch/latest-wins；状态或实例变化才递增 revision。每个 selector/protocol 由唯一 supervisor 管理，旧 generation 的迟到更新会被隔离。关闭超时进入 quarantined，旧 close 未终止前禁止重叠订阅。

## Nacos Adapter

`NacosRegistry` 实现 Registry SPI；`NacosConfigSource` 是内置的 `ConfigSource` adapter，第三方 provider 可通过 `fusen-config::provider` 的安全 lifecycle API 实现同一 SPI。Adapter 将 provider SDK 类型、listener 和执行器保持私有，并把 `FusenV1`/`SpringCloudV1`、稳定 `InstanceId`、HTTP/HTTPS endpoint、group/version 与 metadata 映射到 Nacos。Registration 将实际 scheme 写入 `fusen.scheme`；discovery 保留 `http`/`https` 并过滤未知 scheme，不执行降级或 scheme 重写。

Naming 与 config setup 都先安装 listener，再读取初始值，消除查询与监听之间的丢更新窗口；初始化窗口内采用 latest-wins。Setup waiter 取消后，late success 自动移除 listener。Nacos 只发布 healthy、enabled、正权重实例。

`NacosConfig` 字段私有，仅通过 builder/getter 访问；Debug 永远脱敏 password。Nacos provider 自身的控制面连接安全由 SDK/部署负责，与 RPC Client 的 Rustls/bundled-roots 数据面相互独立。Server 发布 HTTPS endpoint 时，该地址必须由外部 TLS 终止器实际提供。

真实 Nacos 验证使用唯一资源名并显式执行 ignored release-gate tests；日常单元测试使用 fake adapter 覆盖每个 await 点的取消与 finally cleanup。
