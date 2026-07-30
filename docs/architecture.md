# Fusen 0.9 架构

## 总体模型

0.9 将客户端和服务端视为拥有明确终态的 runtime，而不是一组可独立拼装的 transport helper。

```text
ClientRuntime / Server runtime
├── Lifecycle supervisor
├── Control plane
│   ├── Registry handles
│   ├── Subscription supervisor
│   └── Readiness / stale state
├── Data plane
│   ├── Admission / byte budgets
│   ├── Middleware / interface Handler
│   └── HTTP transport (client HTTP/HTTPS, server plaintext)
└── LogicalInvocation
    └── AttemptExecutor
        ├── InstanceRouter / LoadBalancer
        ├── Retry budget
        └── Endpoint + service circuit breaker
```

生命周期 supervisor 是资源的唯一所有者。调用者等待 activation、close 或 shutdown 时可以取消自己的 waiter，但不会夺走 provider worker 或 coordinator 的所有权。Transport、Codec、Acceptor、连接池和内部失败分类均不属于公开扩展面。

## Crate 边界

依赖方向固定为纯值对象 -> SPI -> adapter/runtime -> application：

| Crate | 职责 |
| --- | --- |
| `fusen-contract` | 无 executor 依赖的 service/method/protocol/endpoint/instance 值对象 |
| `fusen-register` | `Registry`、registration/subscription handle 与 `DirectorySnapshot` |
| `fusen-config` | 静态解析、last-good 热配置及显式关闭 |
| `fusen-nacos` | Nacos naming/config provider adapter |
| `fusen-observability` | 同步非阻塞 `MetricsRecorder` 及可选 backend adapter |
| `fusen-procedural-macro` | `interface`/`method`/`RpcMessage` 解析和 wrapper 生成 |
| `fusen-rs` | HTTP/HTTPS Client、明文 HTTP Server、策略与 Middleware runtime |

Core 不依赖 Nacos、OpenSSL/native-tls、系统证书加载器、进程级 tracing subscriber 或 OTel backend。Client 内部使用 Rustls Ring 和 bundled Mozilla WebPKI roots 实现 TLS 1.2/1.3；Server acceptor 仍为明文 HTTP/1.1 与 h2c。宏生成代码只通过版本化的 `fusen_rs::__macro::v1` ABI 使用 runtime internals。

## 逻辑调用与 Attempt

客户端一次调用固定执行：

```text
logical admission + tracing
-> ClientCall Middleware exactly once
-> freeze replayable RequestTemplate
-> service circuit breaker
-> AttemptExecutor
   -> latest DirectorySnapshot
   -> InstanceRouter -> breaker filter -> LoadBalancer
   -> endpoint bulkhead
   -> ClientAttempt Middleware
   -> send / decode
   -> AttemptOutcome -> retry decision / backoff
-> one logical terminal outcome
```

ClientCall Middleware 包围整个逻辑调用，ClientAttempt Middleware 包围每个物理 attempt。一个 absolute deadline 覆盖排队、Middleware、全部 attempts、退避与 decode。每次 attempt 重新读取发现快照；只要仍有未尝试 endpoint，就不能重复选择本次调用中已失败的 endpoint。调用方取消会直接取消当前 attempt，不产生 detached retry。

Endpoint breaker 记录真实 attempt；service breaker 只记录逻辑调用的最终结果。序列化、无实例、本地 admission、普通 4xx、Application error 和调用方取消不污染 breaker。自定义 `RetryPolicy` 仍受幂等性、三次 attempt、deadline 和 token budget 的硬上限约束。

HTTP/wire 成功但 typed `result` 无法解码时，调用以非重试的 `DataLoss`/`invalid_result` 结束；selected endpoint attempt 和 service 最终结果均记录为 `Protocol` failure。这类响应说明远端契约或部署版本已失配，属于健康度信号而不是本地序列化错误。

## 服务端请求管线

服务端执行顺序固定为：

```text
protocol / request-id / deadline / readiness
-> route head
-> fail-fast admission
-> content-type / content-length
-> ServerHead Middleware
-> byte budget + body read / decode
-> ServerCall Middleware / interface Handler
-> bounded response encode
```

因此未知路由、not-ready、draining、非法 head 和已知超限 `Content-Length` 都不会 poll body。请求 guard 至少持有到 decode 完成；响应 byte permit 由 HTTP body 和 Hyper 排队中的 payload 持有，直到 transport 消费或取消。Byte budget 约束 runtime 持有及排队的 body payload；HTTP framing、HPACK/H2 codec 和 OS socket buffer 是独立有界的 transport overhead，不计入 body budget。框架错误使用独立的最大 4 KiB 应急编码路径。

## Control Plane

`Registry::prepare_registration(RegistrationRequest)` 和 `prepare_subscription(SubscriptionRequest)` 同步返回已拥有 worker 与补偿状态的 handle。Runtime 先追踪 handle，再等待 `activate()`。Late success 若已经没有 waiter，会自动执行一次补偿关闭；`close()` 幂等且并发调用共享同一终态。

发现按 `(ServiceSelector, WireProtocol)` 共享唯一 supervisor。状态流为：

```text
Absent -> Starting -> Active -> Stale -> Closing -> Absent
                                  \-> Quarantined
```

旧 generation 未关闭前不能创建重叠订阅。关闭超时进入 `Quarantined`，新连接立即失败；只有后台 worker 到达终态后才能恢复。Directory 更新 latest-wins，状态或实例发生变化时 revision 严格递增。

## 生命周期

Client：

```text
Running -> Draining(deadline) -> Closed(result)
```

Server：

```text
Constructed -> Validated -> Bound -> AcceptingNotReady
            -> Registering -> Ready -> Draining -> Stopped
```

Client shutdown 先线性化关闭 admission，再并行排空逻辑调用、关闭订阅和连接池。Server bind 后即启动 accept loop，但 Ready 前拒绝业务请求；注册完成后 `start()` 才返回。停机先关闭 readiness 与 listener，再并行注销、通知连接 graceful shutdown 并排空请求。两者都使用唯一共享 deadline，取消 waiter 不取消后台 coordinator。

## Panic 与可观测性

发布 profile 使用 `panic=unwind`。Middleware、interface Handler、InstanceRouter、LoadBalancer、RetryPolicy 和 `MetricsRecorder` 分别隔离：请求扩展 panic 只终止当前请求；单个 H2 stream panic 不关闭同连接其他 stream；registry activation/close panic 转成生命周期错误并继续补偿；metrics recorder 首次 panic 后被原子禁用。

Core 直接产生结构化 tracing span/event，应用负责安装 subscriber。Metrics callback 必须同步且非阻塞，label 禁止包含 request ID、endpoint、错误文本、body、完整 headers 或凭据。同步死循环和阻塞无法被 async timeout 抢占，扩展实现应将阻塞工作移到 `spawn_blocking`。
