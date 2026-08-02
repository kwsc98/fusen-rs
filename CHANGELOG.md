# Changelog

## [Unreleased]

- 调用 `Error` 改为正交的 `ErrorKind`、`ErrorOrigin` 与 `ErrorCategory`：删除 category/origin 中的 `Application`，新增 `Unknown` category，并以 `Error::local`、`Error::application` 和 `Error::application_status` 替换旧构造入口。Application 错误永不重试，但 remote 5xx 现在会计入 circuit breaker failure。
- `fusen-rs`、`fusen-register` 与 `fusen-config` 继续分别拥有调用/runtime、registry 与 config 领域错误，不新增全局错误 crate；公共错误的 `Debug` 不再展开 source、header 或 details 值。
- 公共调用 API 去除 `Rpc*` 前缀：统一使用 `Arguments`、`Body`、`Call`、`Context`、`Response`、`Side`、`Error`、`ErrorCategory`、`ErrorOrigin` 与 `ErrorDetails`；不提供旧名称 alias。
- `Middleware` SPI 更名为 `Interceptor`，入口统一为 `intercept`，builder 使用 `.interceptor(...)`、`.attempt_interceptor(...)` 与 `.head_interceptor(...)`；panic 错误码同步从 `middleware_panic` 改为 `interceptor_panic`，这是 wire error code breaking change。
- OpenTelemetry attempt histogram 从 `fusen.rpc.attempts` 改为 `fusen.invocation.attempts`，metric event/attribute 的 `protocol` 同步改为 `binding`；这是 dashboard/alert breaking change，相关规则需要同步迁移。
- 删除 `WireProtocol`、`ProtocolSet`、`FusenV1`、`SpringCloudV1` 与全部 `SpringCloud*` 公共 API；内置表示统一为 binding ID `http-json-v1`，使用必需 `HttpOperation` 与 raw JSON success。私有 `/_fusen/v1` URI、`application/fusen+json` 和 `arguments`/`result` envelope 不保留兼容 decoder，这是 `v0.9.0` tag 前的有意 wire break。
- HTTP binding、transport 与 discovery 解耦：新增 `HttpBindingId`、`HttpVersionSet`、`HttpVersionPolicy` 与 `EndpointCapabilities`；Registry subscription 只按 `ServiceSelector` 共享，Client 在每次 attempt 根据实例 capabilities 过滤 binding 与 HTTP version。
- `RegistrationRequest`/`SubscriptionRequest` 不再接收 protocol；`ServiceRegistration` 与 `ServiceInstance` 携带 endpoint capabilities。Nacos 删除 `fusen.protocol`，使用严格 capability metadata；`NacosConvention::SpringCloud` 仅在所有 capability key 都缺失时回退到 HTTP/1.1 + `http-json-v1` + controls disabled。
- 增加进程内 `SensitiveFields` DTO shape 与 method request/response 敏感度元数据；可选 `fusen-contract/derive` feature 提供 derive 宏，元数据不参与 wire、服务标识、注册或发现。
- `SensitiveFields` 结构 shape 分离 Serde serialization/deserialization 字段表；客户端/服务端请求与响应按实际方向投影，方向性 rename、alias 和 skip 不会跨分类泄漏。
- 接口参数支持显式 `#[param(path)]`，其 wire name 必须匹配同名 path placeholder；现有按名称推断保持兼容，且不同参数来源仍共享全局唯一的 wire name 空间。
- Spring Cloud repeated query 改为显式 `#[param(query, repeated)]`；primitive/newtype 解码不再依赖 Rust 类型 token 名称。

## [0.9.0] - YYYY-MM-DD

<!-- M0.11 冻结最终候选前必须将 YYYY-MM-DD 替换为实际发布日期；发布日期变动会使候选 SHA 失效并要求重跑全部证据。 -->

`0.9.0` 是 clean-slate 的首个兼容性 baseline，不兼容此前未发布的 Rust API、宏、配置或 wire 流量。

### Public Contract

- Workspace 统一为 Rust 1.97、Edition 2024、resolver 3、禁止 unsafe，并集中 lint policy。
- 接口声明统一为 `#[interface]` trait 宏；Client 与 Handler 实现同一个 trait，方法直接接收零到多个参数并返回 `Result<Response<T>, Error>`。每个方法必须用 `#[method(method = "...", path = "...")]` 声明请求语义，可用 `consumes`/`produces` 覆盖 media type；path/query/body field 按 method、path placeholder 与参数名确定性推断，`#[param(...)]` 支持 path、query、header、cookie、body、query/header map、context、name 与 repeated。
- 调用错误拆分为 `Error`、`ClientError`、`ServerError`、`RegistryError` 与 `ConfigError`，字段私有并提供稳定分类/getter。
- 公开扩展面收敛为 Interceptor、Registry、InstanceRouter、LoadBalancer、RetryPolicy、ConfigSource、MetricsRecorder，以及 Client-only `RequestEncoder`/`ResponseDecoder`/`ErrorDecoder` binding codec；transport、Server codec、acceptor、pool 与 lifecycle internals 全部私有。
- 所有配置采用私有字段、`Default`、builder/setter 与 getter；可扩展 enum/error 标记为 non-exhaustive。

### HTTP Binding And Runtime

- 定义 `http-json-v1`：每个 method 都有显式 `HttpOperation`，参数映射到 path/query/header/cookie/query map/header map/body，成功响应为 raw body，`consumes`/`produces` 缺省为 `application/json`。
- HTTP route 按 method 构建不可变的确定性 trie；每个 segment 优先匹配 literal，并在 literal 后续失败时回退同层 parameter，匹配结果不受 service 插入顺序影响，等价动态 shape 在构建时被拒绝。
- Query 通过公开的 `HttpParameterCardinality::{Scalar, Repeated}` 固定基数语义：Scalar 缺失为 `null`、重复 key 返回 `duplicate_query_parameter`；Repeated 缺失为 `[]`，单值和多值始终为数组。宏仅按显式 `#[param(query, repeated)]` 生成 Repeated，客户端空 array 不发送 key，其余元素按重复 key 编码。
- Endpoint 通过 `EndpointCapabilities` 分别声明 binding、HTTP version 与 invocation controls；Client 用独立 `HttpVersionPolicy` 选择 HTTP/1.1、HTTP/2 或 h2c，不从 binding ID 推导 transport。
- Binding 使用 request ID 与 RFC 9457 Problem Details；relative timeout 与 attempt headers 只对声明 invocation-controls capability 的 endpoint 启用。内部 source/panic 永不进入 wire。
- Client 支持 canonical `http://`/`https://` endpoint；HTTPS 使用 Rustls Ring、TLS 1.2/1.3、bundled Mozilla WebPKI roots 和严格证书/hostname 验证，不提供明文降级、自定义 CA 或 mTLS。
- Server listener 保持明文 HTTP/1.1/h2c；HTTPS advertised endpoint 只表示由 ingress、sidecar、反向代理或 service mesh 提供的外部 TLS 终止地址。
- Client 使用 logical invocation/attempt 分层，一个 deadline 覆盖 admission、Interceptor、retries、backoff、transport 与 decode。
- 自动重试资格由标准 HTTP method 保守推导：GET、HEAD、OPTIONS、PUT、DELETE 允许重放，POST、PATCH 禁止自动重试；不接受用户自报的幂等标记。
- 增加 retry token budget、endpoint/service circuit breaker、endpoint bulkhead、有界可选 queue 与全局 request/response byte budgets。
- Retry-After 同时支持 delta-seconds 与 HTTP-date；可重放请求模板和分段响应从序列化前到 Hyper transport 消费/取消 payload 全程持有 byte permit，framing/codec/socket buffer 作为独立有界 transport overhead。
- Typed raw response 解码失败以非重试 `DataLoss`/`invalid_result` 终止，并作为 endpoint attempt 与 service final outcome 的 protocol breaker failure 记账。
- Server 增加 not-ready accept、确定性并发注册、body-before-read head validation、bounded response encoder 与 accept backoff。

### Lifecycle And Control Plane

- Registry 改为同步 prepare 与取消安全的 `activate()`/`close()` handles，支持 late-success compensation 和共享幂等终态。
- Directory 增加 revision、observed time 与 Initializing/Ready/Stale/Unavailable/Closed 状态，更新采用 latest-wins。
- Client 和 Server shutdown 均由后台 coordinator 持有，使用一个 absolute deadline，并发 waiter 共享终态。
- `Server::start()` 只在 Ready 后返回 `RunningServer`；`ServerHandle` 提供幂等 shutdown，`serve()` 提供平台信号 convenience。
- `fusen-config` 提供 TOML/YAML 静态解析、last-good typed hot config 与显式关闭；Nacos naming/config 使用 listener-first setup 和取消补偿。

### Safety And Observability

- Interceptor 在 ClientCall、ClientAttempt、ServerHead、ServerCall 四个阶段共享同一对象安全接口；Interceptor、Handler、InstanceRouter、LoadBalancer、RetryPolicy、Registry、ConfigSource 与 MetricsRecorder 分别建立 panic boundary。
- Core 产生结构化 tracing event；MetricsRecorder 同步非阻塞，panic 后原子禁用，labels 受低 cardinality 与脱敏约束。
- 增加 `http-json-v1` 真实 H1/H2 golden fixtures、macro compile tests、paused-time lifecycle/resilience tests、资源预算测试和跨平台 CI/release gates。

### Removed

- 删除所有旧服务声明/实现入口、单体错误体系、observer 模型、公开 transport/codec 生命周期细节和旧 wire decoder。
- 删除旧协议枚举、私有 Fusen envelope、不完整 TLS client stack、无限 queue 语义及兼容 facade；新的 HTTPS client 采用独立审计的 Rustls 边界。
- 删除独立配置 derive 宏 crate；敏感字段通过普通私有配置类型与显式安全 Debug 管理。
