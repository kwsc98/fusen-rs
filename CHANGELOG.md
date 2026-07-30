# Changelog

## [Unreleased]

## [0.9.0] - YYYY-MM-DD

<!-- M0.11 冻结最终候选前必须将 YYYY-MM-DD 替换为实际发布日期；发布日期变动会使候选 SHA 失效并要求重跑全部证据。 -->

`0.9.0` 是 clean-slate 的首个兼容性 baseline，不兼容此前未发布的 Rust API、宏、配置或 wire 流量。

### Public Contract

- Workspace 统一为 Rust 1.97、Edition 2024、resolver 3、禁止 unsafe，并集中 lint policy。
- 接口声明统一为 `#[interface]` trait 宏；Client 与 Handler 实现同一个 trait，方法直接接收零到多个参数并返回 `Result<RpcResponse<T>, RpcError>`。每个方法必须用 `#[method(method = "...", path = "...")]` 声明请求语义；path/query/body field 按 method、path placeholder 与参数名确定性推断，`#[param(query/body/context/name)]` 只处理显式覆盖和调用上下文。
- 调用错误拆分为 `RpcError`、`ClientError`、`ServerError`、`RegistryError` 与 `ConfigError`，字段私有并提供稳定分类/getter。
- 公开扩展面收敛为 Middleware、Registry、InstanceRouter、LoadBalancer、RetryPolicy、ConfigSource 与 MetricsRecorder；transport/codec/acceptor/pool/lifecycle internals 全部私有。
- 所有配置采用私有字段、`Default`、builder/setter 与 getter；可扩展 enum/error 标记为 non-exhaustive。

### Protocol And Runtime

- 定义 Fusen V1：h2c、固定 v1 URI、按名称 arguments envelope 与 result envelope。
- 定义 Spring Cloud V1：HTTP/1.1、显式 method/path/query/body mapping 与 raw JSON success。
- Spring route 按 HTTP method 构建不可变的确定性 trie；每个 segment 优先匹配 literal，并在 literal 后续失败时回退同层 parameter，匹配结果不受 service 插入顺序影响，等价动态 shape 在构建时被拒绝。
- Spring query 通过公开的 `SpringCloudParameterCardinality::{Scalar, Repeated}` 固定基数语义：Scalar 缺失为 `null`、重复 key 返回 `duplicate_query_parameter`；Repeated 缺失为 `[]`，单值和多值始终为数组。宏将直接声明的 `Vec<T>` 标记为 Repeated、拒绝 `Option<Vec<T>>`，客户端空 Vec 不发送 key，其余元素按重复 key 编码。
- 两种协议统一 request ID、relative timeout、attempt headers 与 RFC 9457 Problem Details；内部 source/panic 永不进入 wire。
- Client 支持 canonical `http://`/`https://` endpoint；HTTPS 使用 Rustls Ring、TLS 1.2/1.3、bundled Mozilla WebPKI roots、严格证书/hostname 验证与 Fusen ALPN `h2`，不提供明文降级、自定义 CA 或 mTLS。
- Server listener 保持明文 HTTP/1.1/h2c；HTTPS advertised endpoint 只表示由 ingress、sidecar、反向代理或 service mesh 提供的外部 TLS 终止地址。
- Client 使用 logical invocation/attempt 分层，一个 deadline 覆盖 admission、Middleware、retries、backoff、transport 与 decode。
- 自动重试资格由标准 HTTP method 保守推导：GET、HEAD、OPTIONS、PUT、DELETE 允许重放，POST、PATCH 和没有 HTTP mapping 的方法禁止自动重试；不接受用户自报的幂等标记。
- 增加 retry token budget、endpoint/service circuit breaker、endpoint bulkhead、有界可选 queue 与全局 request/response byte budgets。
- Retry-After 同时支持 delta-seconds 与 HTTP-date；可重放请求模板和分段响应从序列化前到 Hyper transport 消费/取消 payload 全程持有 byte permit，framing/codec/socket buffer 作为独立有界 transport overhead。
- Typed result 解码失败以非重试 `DataLoss`/`invalid_result` 终止，并作为 endpoint attempt 与 service final outcome 的 protocol breaker failure 记账。
- Server 增加 not-ready accept、确定性并发注册、body-before-read head validation、bounded response encoder 与 accept backoff。

### Lifecycle And Control Plane

- Registry 改为同步 prepare 与取消安全的 `activate()`/`close()` handles，支持 late-success compensation 和共享幂等终态。
- Directory 增加 revision、observed time 与 Initializing/Ready/Stale/Unavailable/Closed 状态，更新采用 latest-wins。
- Client 和 Server shutdown 均由后台 coordinator 持有，使用一个 absolute deadline，并发 waiter 共享终态。
- `Server::start()` 只在 Ready 后返回 `RunningServer`；`ServerHandle` 提供幂等 shutdown，`serve()` 提供平台信号 convenience。
- `fusen-config` 提供 TOML/YAML 静态解析、last-good typed hot config 与显式关闭；Nacos naming/config 使用 listener-first setup 和取消补偿。

### Safety And Observability

- Middleware 在 ClientCall、ClientAttempt、ServerHead、ServerCall 四个阶段共享同一对象安全接口；Middleware、Handler、InstanceRouter、LoadBalancer、RetryPolicy、Registry、ConfigSource 与 MetricsRecorder 分别建立 panic boundary。
- Core 产生结构化 tracing event；MetricsRecorder 同步非阻塞，panic 后原子禁用，labels 受低 cardinality 与脱敏约束。
- 增加真实 H1/H2 golden fixtures、macro compile tests、paused-time lifecycle/resilience tests、资源预算测试和跨平台 CI/release gates。

### Removed

- 删除所有旧服务声明/实现入口、单体错误体系、observer 模型、公开 transport/codec 生命周期细节和旧 wire decoder。
- 删除旧的不完整协议与 TLS client stack、无限 queue 语义及兼容 facade；新的 HTTPS client 采用独立审计的 Rustls 边界。
- 删除独立配置 derive 宏 crate；敏感字段通过普通私有配置类型与显式安全 Debug 管理。
