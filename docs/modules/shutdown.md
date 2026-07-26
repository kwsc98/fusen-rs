# 停机行为

> English summary: client and server shutdown are idempotent background
> coordinators with one shared absolute deadline and a stable terminal result.

## Client

```text
Running
-> linearize admission closed
-> Draining(deadline)
   -> logical invocations
   -> subscription handles
   -> connection pools
-> Closed(result)
```

三类工作并行共享默认 30 秒预算，不分别获得完整 timeout。期限到达后 coordinator 广播 cancellation、drop pool 并返回 `ClientError`。并发 `shutdown()` waiter 读取同一终态；取消一个 waiter 不取消实际关闭。

## Server

```text
Ready or startup/accept failure
-> compute one absolute deadline
-> readiness = draining; close listener
-> notify every HTTP connection graceful shutdown
-> concurrently close registration handles and drain requests
-> Stopped(result), or cancel remaining work at deadline
```

关闭 listener 和 readiness 发生在任何可能阻塞的 cleanup await 之前。Registration 按确定性逆序全部尝试；单项失败不短路其他项。Deadline 到达后剩余连接被 abort，runtime 不为 task reaping 无界等待。

错误优先级为 deadline > fatal accept > registry aggregate。所有被覆盖的次要错误仍产生结构化事件。普通连接协议错误与单请求 panic 不属于 server 生命周期错误。

`Server::serve()` 在 Unix 监听 SIGINT/SIGTERM，在其他平台监听 Ctrl-C。监听器失败或关闭时进入同一 fail-safe shutdown。需要由应用控制信号时，调用 `start()` 并通过 `RunningServer`/`ServerHandle` 关闭。

## 取消补偿边界

Registry 与 Config 的 handle 在同步 prepare 返回前已拥有 worker 与补偿状态。Runtime 先追踪 handle，再等待 `activate()`；activation waiter 被 drop 不会取消 provider worker，late success 会请求一次 close。`close()` 幂等，并发 waiter 共享终态；Drop 只请求关闭，不在析构中执行 async cleanup。

这些保证依赖承载 supervisor 的 Tokio runtime 在后台 coordinator 到达终态前继续运行。应用退出 runtime 之前应显式等待 client/server/config shutdown。
