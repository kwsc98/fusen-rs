# 架构与调用链

## 边界

workspace 将稳定契约、注册发现、配置、Nacos、可观测性、RPC runtime 和过程宏分开。`fusen-contract` 定义唯一的 `ServiceDescriptor` 以及 selector/registration/instance/endpoint；`fusen-register` 只定义注册发现和不可变 `Directory` 快照；具体集成分别位于 `fusen-config`、`fusen-nacos` 与 `fusen-observability`。

## Invoker 模型

调用链借鉴 Dubbo 的 Invocation/Cluster/Protocol/Service 分层，但使用 Rust 静态描述、显式上下文和消费型 pipeline：

```text
ClientRuntime
  -> InvocationObserver
  -> Client Middleware
  -> ClusterInvoker
       -> Router -> LoadBalancer
       -> HTTP encode/send/decode

Server
  -> InvocationObserver
  -> admission -> decode -> route
  -> Provider Middleware
  -> MethodId service dispatch
  -> encode
```

没有 ThreadLocal RpcContext、字符串 attachment、动态 SPI 或可重复 Filter 调用。`RpcContext` 显式携带 request ID、静态 service/method、绝对 deadline、headers、metadata 与类型化 extensions。`Next` 借用不可变 Middleware slice 和私有 terminal，消费自身后才能进入下游。

## 静态描述与分派

`fusen_trait` 通过 `OnceLock` 初始化一份进程生命周期 `ServiceDescriptor`。每个方法按 trait 声明顺序获得 `MethodId(u16)`；客户端、服务端和注册信息引用同一描述对象。生成客户端直接按 ID 取静态描述，服务端隐藏 dispatch 使用整数 match 调用实现方法。

服务端启动时为每条 HTTP route 预绑定 `&'static MethodDescriptor + Middleware slice + service invoker`，请求热路径不查 service HashMap、不比较方法名、不重建路径元数据。

## Cluster

客户端 Middleware 在 ClusterInvoker 之前执行。未配置 Router 时，`InstanceSnapshot` 直接包住 Nacos `Arc<Vec<_>>`；Router 可以按顺序过滤或重排。调用链固定执行一次 LoadBalancer 和一次 transport，默认 `WeightedRandom` 返回一个索引；空结果和非法索引统一为 `ServiceUnavailable`。

## 生命周期与期限

客户端入口与服务端并发准入前创建 `InvocationGuard`。Observer 按注册顺序同步通知，每次调用恰好一次 finish，包含 side、request ID、service/method、阶段、耗时、HTTP 状态、错误 code 和 Success/Error/Timeout/Cancelled，不包含 body、凭据或完整 headers。

客户端一个绝对 deadline 覆盖参数构造、Middleware、快照、Cluster、连接和响应 decode。服务端一个绝对 deadline 覆盖 admission、decode、route、Middleware、service 与 encode。Future 被丢弃时 guard 报告 Cancelled；Middleware 后置代码不保证运行，资源释放必须依靠 RAII。

## 所有权与停机

`ClientRuntime` 统一拥有连接池、注册中心、全局 Middleware、Observer、订阅管理器和关闭状态。相同 selector/protocol 的 discovery client 共享 Directory 与 listener；最后一个 client lease 释放时后台关闭订阅。`shutdown()` 与并发 connect 原子交接所有权并强制清理全部 entry，Drop 只做后台兜底。Direct client 不进入订阅管理器。

服务端先校验、绑定并事务化注册；注册失败按相反顺序回滚。停机先停止 accept，在同一总期限内摘除服务并排空连接。
