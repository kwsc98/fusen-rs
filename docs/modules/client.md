# 客户端行为

> English summary: `ClientRuntime` owns logical admission, discovery, HTTP/HTTPS pools,
> retries, circuit breakers, and one cancellation-safe shutdown result.

## 构建与所有权

`ClientRuntime::builder()` 接受私有字段的 `ClientConfig`、一个可选 `Registry`、全局 `Middleware`、`RetryPolicy` 与 `MetricsRecorder`。生成的服务 Builder 通过 `.direct("http://...")`、`.direct("https://...")` 或 `.discover()` 选择寻址，并通过 `.protocol(...)` 选择明确 wire 版本。

Runtime 必须在正在运行的 Tokio runtime 内构建。Endpoint 只接受 canonical absolute `http://`/`https://`，含凭据/query/fragment 或其他 scheme 的值在 connect/validation 阶段失败。Direct client 不创建订阅；discovery client 按 `(ServiceSelector, WireProtocol)` 共享 supervisor。

Fusen V1 在 HTTP endpoint 上使用 h2c，在 HTTPS endpoint 上要求 TLS 1.2/1.3 与 ALPN `h2`。Spring Cloud V1 在两种 scheme 上都使用 HTTP/1.1。HTTPS 使用 Rustls Ring 和 bundled Mozilla WebPKI roots 验证证书链、有效期与 hostname；不读取系统 trust store，也没有自定义 CA、mTLS、跳过验证或明文 fallback。私有 CA 和自签名证书不受支持。

## 逻辑调用

全局与 interface-local `ClientCall` Middleware 每次逻辑调用各执行一次，位于 InstanceRouter、LoadBalancer 与全部物理 attempts 之外。成功通过 Middleware 后，runtime 冻结可重放请求模板，并在每次 attempt 重新读取最新 Directory snapshot；global/local `ClientAttempt` Middleware 则包围每个物理 attempt。

选择顺序为 InstanceRouter -> open endpoint 过滤 -> LoadBalancer -> endpoint bulkhead。只要有尚未尝试的 endpoint，就不会重复选择本次调用已失败的 endpoint。无实例、非法 LB 结果、序列化、本地 admission 与调用方取消均不进入 circuit breaker。

## Deadline、Retry 与 Breaker

默认调用 deadline 为 10 秒，一个 absolute deadline 覆盖 admission/queue、Middleware、所有 attempts、退避、传输与 decode。调用方取消立即取消当前 attempt。

重试资格由接口声明的 HTTP method 保守推导：GET、HEAD、OPTIONS、PUT、DELETE 可重试，POST、PATCH 永不自动重试。内置策略最多三次总 attempts，使用 10 ms 到 200 ms 的 full-jitter 指数退避，并由每服务容量 100、每秒补充 10 的 token bucket 限制 retry。`Retry-After` 支持 delta-seconds 与 HTTP-date，并作为最小等待；剩余 deadline 不足时直接结束。自定义 policy 不能放宽这些硬上限。

Endpoint breaker 使用 10 秒窗口、最少 20 样本、50% 失败比例；service breaker 使用 30 秒窗口、最少 50 样本、60% 失败比例。Endpoint 记录每个真实 attempt，service 仅记录最终逻辑结果。Endpoint entry 上限 10,000，缺失或空闲 10 分钟后淘汰。

HTTP/wire 成功但 typed `result` 无法反序列化为生成方法的 Rust 类型时，不执行 retry；调用以 `DataLoss`/`invalid_result` 终止，selected endpoint attempt 与 service final outcome 均按 `Protocol` failure 计入 breaker。

## Admission 与预算

默认最多 1024 个逻辑调用、每 endpoint 128 个 attempts，单请求和响应各 2 MiB，全局请求和响应 byte budget 各 64 MiB。默认 fail-fast；只有通过 `QueueConfig::builder()` 设置非零 capacity 并安装到 admission 配置后才允许排队，max wait 始终计入逻辑 deadline。

请求不会按 2 MiB 上限预分配。可重放请求模板在序列化写入前增量申请 byte permit，同一份 `Bytes` 在全部 attempts 与 backoff 期间只计费一次，并由 queued body chunk 持有到 Hyper transport 消费或取消。响应 body permit 持有到 decode 完成或取消，panic、timeout 和 cancellation 都必须归还 admission 与 byte permits。协议 framing、codec staging 与 socket buffer 属于独立有界的 transport overhead，不计入 body budget。

## Discovery

连接要求 Directory 在 initial timeout 内进入 `Ready`。最近一次有效实例在 provider 断开后可短暂以 `Stale` 状态继续路由，默认最长 30 秒；之后进入 `Unavailable` 并 fail fast。Revision 对状态或实例变化严格递增，旧 subscription generation 的迟到更新不能覆盖新状态。

Subscription close 超时会隔离该 selector/protocol；在旧 worker 到达终态前，新的 discover connect 立即失败，不等待也不创建重叠 listener。

## Shutdown

状态为 `Running -> Draining -> Closed`。`shutdown()` 先原子关闭 admission，再在共享 30 秒 deadline 内并行排空逻辑调用、关闭 subscriptions 与连接池；期限到达后广播 cancellation 并 drop pool，有界返回 `ClientError`。

并发 shutdown 调用共享同一终态。取消某个 waiter 不取消 coordinator；Drop 只请求关闭，显式 `shutdown().await` 才是应用生命周期契约。
