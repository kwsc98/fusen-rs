# 客户端行为

## 职责与接口

`ClientRuntime::builder()` 装配 registry、全局 `Middleware`、`InvocationObserver` 和 `ClientConfig`。`fusen_trait` 为每个服务生成专属 Client Builder，普通用户通过 `.direct(...).connect()` 或 `.discover().connect()` 创建客户端；协议可用 `.protocol(WireProtocol)` 选择。

生成方法直接传入 `MethodId` 和精确容量参数 Vec，不做字符串方法查找。静态 `ServiceDescriptor` 通过 `OnceLock` 初始化一次，并由客户端、服务端和注册信息共享。

## Cluster

Client Middleware 在 Router/LB 之前执行，可写入 `RpcContext::metadata_mut()` 或 extensions。`client::cluster` 只提供同步 `Router`、`LoadBalancer` 和 `InstanceSnapshot`。多个 Router 按配置顺序执行；未配置时复用 Directory 快照。默认 `WeightedRandom`，每次调用只选择并访问一个 endpoint。

Direct endpoint 在 connect 时解析成 `ServiceEndpoint`，不登记订阅。Discovery 按 `ServiceSelector + WireProtocol` 复用订阅，调用时只 clone 当前不可变快照；最后一个 client lease 释放后关闭 listener。空实例、Router 清空结果和非法 LB 索引返回 `ServiceUnavailable`。

## Deadline、错误与关闭

默认连接超时 3 秒、调用超时 10 秒、发现超时 5 秒、订阅清理超时 5 秒、响应 body 上限 2 MiB。一个 `timeout_at` 覆盖关闭检查、构造、Middleware、Cluster、DNS/TLS、HTTP 响应头和 body。非 2xx Problem Details 还原为 `FusenError::Remote` 后再返回 Middleware。

`ClientRuntime::shutdown()` 幂等关闭所有订阅并拒绝新连接和 RPC。它会等待并发创建完成，超时项保留到下一次 shutdown，终态错误稳定返回；Drop 仅在最后一个 owner 释放时尽力启动后台清理。

## HTTP 连接池

`ClientRuntimeBuilder::http1_pool` 和 `http2_pool` 分别配置两个独立的 Hyper pool。H1 支持配置每个地址保留的最大空闲连接数和空闲回收时间；`max_idle_per_host = 0` 关闭连接复用，但它不是正在使用连接数的上限。

H2 的 `connections_per_host` 表示每个地址的独立多路复用连接分片数。每个分片拥有独立 pool，连接按需创建，请求根据 endpoint 和 request ID 做无锁稳定哈希分片。默认值为 1；CPU 密集的小消息高并发场景可从 2 或 4 开始压测，连接数增加也会同步增加 socket、握手和服务端连接状态成本。

H2 还可配置空闲回收、ping 间隔、ping 应答 timeout 以及是否在无活动 stream 时保活。`keep_alive_interval = None` 表示关闭 ping，不影响正常的 HTTP 连接复用。
