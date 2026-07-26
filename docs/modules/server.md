# 服务端行为

> English summary: `Server::start` returns only at Ready and `RunningServer`
> owns a cancellation-safe, deadline-bounded terminal result.

## 构建与启动

`Server::builder(address)` 收集私有字段 `ServerConfig`、按插入顺序命名的 registries、全局 Middleware、MetricsRecorder 与宏生成的 `*Server` wrapper。`build()` 完成静态服务、协议与路由校验；`start()` 执行 bind、启动 not-ready accept loop、准备并激活 registration handles，只有 Ready 后才返回 `RunningServer`。

状态为：

```text
Constructed -> Validated -> Bound -> AcceptingNotReady
            -> Registering -> Ready -> Draining -> Stopped
```

Ready 前请求返回非 retryable `503 not_ready`，且不读取 body。注册按 registry 插入顺序、protocol、service identity 确定性排序，并以窗口 8 激活。Runtime 总是先追踪 handle 再等待 activation；失败时所有已追踪资源按确定性逆序分批关闭，late success 由 handle 自动补偿。

`RunningServer` 提供 `local_addr()`、`state()`、`handle()`、`wait()` 与 `shutdown()`。`ServerHandle::shutdown()` 可跨任务调用，幂等且等待共享终态。`Server::serve()` 是带平台信号监听的 convenience API。

## 请求入口

处理顺序固定为 protocol/request-id/deadline/state -> route head -> admission -> content-type/content-length -> body budget/read/decode -> Middleware/service -> bounded response encode。

未知 route、not-ready、draining、head 非法或已知 Content-Length 超限时不 poll body。默认限制为：1024 个在途请求、2048 条 TCP 连接、每 H2 连接 128 streams、单请求/响应 2 MiB、全局请求/响应预算各 64 MiB、URI 8 KiB、query 128 pairs、headers 32 KiB。H1 header timeout 为 10 秒；H2 keepalive 为 30 秒 interval / 10 秒 timeout。

Admission 与 byte budget 默认 fail-fast。Response 使用 bounded writer 单次序列化，permit 跟随 queued body chunk 到 Hyper transport 消费或取消；超限返回非 retryable `500 response_too_large`。协议 framing、codec staging 与 socket buffer 是独立有界且不计入 body budget 的 transport overhead。框架错误走独立、最大 4 KiB 的应急 Problem Details encoder。

## Accept 与故障

`Interrupted` accept error 立即重试。其他可恢复错误从 10 ms 指数退避到 1 秒，成功一次即清零；连续 16 次失败才升级为 fatal accept error。Shutdown 可立即中断 backoff。

Middleware 或 handler panic 只让当前请求返回隐藏细节的 500；单个 H2 stream panic 不关闭同连接的并发或后续 streams。连接级协议错误和 task panic 会记录结构化事件，但不会自动升级为 server 生命周期错误。

## Shutdown

停机先将 readiness 置为 draining 并关闭 listener，然后立即通知 Hyper 连接 graceful shutdown；registration close 与连接排空在同一个 absolute deadline 下并行执行。期限到达后 abort 剩余连接并有界返回，不无限等待 JoinSet 回收。

生命周期错误优先级固定为：共享 graceful deadline、fatal accept、registry aggregate。被更高优先级覆盖的错误仍写入结构化日志。Registry close 会逆序尝试全部 handle，provider panic/error 不阻止后续补偿。

`serve()` 在 Unix 监听 SIGINT 与 SIGTERM，其他平台监听 Ctrl-C；signal listener 失败会触发 fail-safe shutdown。取消 startup waiter 或 shutdown waiter不取消后台 coordinator，前提是承载 supervisor 的 Tokio runtime 在清理期间仍运行。
