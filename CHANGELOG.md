# Changelog

## [Unreleased]

## [0.9.0] - 2026-08-02

`0.9.0` 是 clean-slate 的首个兼容性 baseline，不兼容此前未发布的 Rust API、宏、配置或 wire 流量。

### Public Contract

- Workspace 统一为 Rust 1.97、Edition 2024、resolver 3、禁止 unsafe，并集中 lint policy。
- 接口声明统一为 `#[interface]` trait 宏；Client 与 Handler 实现同一个 trait，方法直接接收零到多个参数并返回 `Result<Response<T>, Error>`。每个方法必须用 `#[method(method = "...", path = "...")]` 声明请求语义，可用 `consumes`/`produces` 覆盖 media type；path/query/body field 按 method、path placeholder 与参数名确定性推断，`#[param(...)]` 支持 path、query、header、cookie、显式 `body_field`、raw `body`、query/header map、context、name 与 repeated。`body_field` 允许 `name` 但禁止 `repeated`；显式 path 的 wire name 必须匹配完整 segment placeholder，repeated query 不再依赖 Rust type token 推断。
- 公共调用 API 去除 `Rpc*` 前缀，统一使用 `Arguments`、`Body`、`Call`、`Context`、`Response`、`Side`、`Error`、`ErrorCategory`、`ErrorOrigin` 与 `ErrorDetails`，不提供旧名称 alias。
- 调用错误改为正交的 `ErrorKind`、`ErrorOrigin` 与 `ErrorCategory`；Application 错误永不重试，但 remote Application 5xx 计入 circuit breaker failure。`fusen-rs`、`fusen-register` 与 `fusen-config` 分别拥有 invocation/runtime、registry 与 config 错误，安全 `Debug` 不展开 source、header 或 details 值；`Call`、`Context`、`Response`、配置快照以及 registration/discovery metadata carrier 同样只输出安全元数据，不展开载荷、header、extension 或 metadata 值。
- 公开扩展面收敛为 Interceptor、Registry、InstanceRouter、LoadBalancer、RetryPolicy、ConfigSource、MetricsRecorder、Sanitizer，以及 Client-only `RequestEncoder`/`ResponseDecoder`/`ErrorDecoder` binding codec；`ConfigSource` 与安全 provider lifecycle 构造器是稳定 SPI，transport、Server codec、acceptor、pool 与 lifecycle internals 全部私有。
- `Middleware` SPI 更名为 `Interceptor`，入口为 `intercept`，builder 使用 `.interceptor(...)`、`.attempt_interceptor(...)` 与 `.head_interceptor(...)`；panic wire error code 从 `middleware_panic` 改为 `interceptor_panic`，这是需要同步更新客户端匹配逻辑的 wire error-code breaking change。
- 生成代码只依赖 doc-hidden 的版本化 `fusen_rs::__macro::v1` ABI；它不是应用扩展 API，但 Cargo 允许组合的 0.9.x macro/runtime 必须保持编译兼容并支持 renamed runtime dependency。
- 所有配置采用私有字段、`Default`、builder/setter 与 getter；可扩展 enum/error 标记为 non-exhaustive。

### HTTP Binding And Runtime

- 定义 `http-json-v1`：每个 method 都有显式 `HttpOperation`，参数映射到 path/query/header/cookie/query map/header map/body，成功响应为 raw body，`consumes`/`produces` 缺省为 `application/json`。`HttpOperation` 接受通用 MIME；内置 JSON binding 在网络 I/O 前只接受 `application/json` 或具体的 `application/<subtype>+json`。
- HTTP route 按 method 构建不可变的确定性 trie；每个 segment 优先匹配 literal，并在 literal 后续失败时回退同层 parameter，匹配结果不受 service 插入顺序影响，等价动态 shape 在构建时被拒绝。
- Query 通过公开的 `HttpParameterCardinality::{Scalar, Repeated}` 固定基数语义：Scalar 缺失为 `null`、重复 key 返回 `duplicate_query_parameter`；Repeated 缺失为 `[]`，单值和多值始终为数组。宏仅按显式 `#[param(query, repeated)]` 生成 Repeated，客户端空 array 不发送 key，其余元素按重复 key 编码。
- HTTP binding、transport 与 discovery 解耦：Endpoint 通过 `EndpointCapabilities` 分别声明 `HttpBindingId`、HTTP version 与 invocation controls；Client 用独立 `HttpVersionPolicy` 选择 HTTP/1.1、HTTP/2 或 h2c，不从 binding ID 推导 transport。Registry subscription 只按 `ServiceSelector` 共享，并在每次 attempt 过滤 endpoint capabilities。
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
- `fusen-config` 提供 TOML/YAML 静态解析、last-good typed hot config、稳定 `ConfigSource` provider SPI 与显式关闭；Nacos naming/config 使用 listener-first setup 和取消补偿。
- `RegistrationRequest`/`SubscriptionRequest` 不再携带 protocol；registration/instance 改为携带 endpoint capabilities。Nacos 删除 `fusen.protocol` metadata，并提供显式 `NacosConvention::SpringCloud`，仅在全部 capability key 缺失时合成为 HTTP/1.1 + `http-json-v1` + controls disabled。

### Safety And Observability

- Interceptor 在 ClientCall、ClientAttempt、ServerHead、ServerCall 四个阶段共享同一对象安全接口；Interceptor、Handler、InstanceRouter、LoadBalancer、RetryPolicy、Registry、ConfigSource 与 MetricsRecorder 分别建立 panic boundary。
- 增加进程内 `SensitiveFields` DTO shape 与 method request/response metadata；结构 shape 分离 Serde serialization/deserialization 字段表，方向性 rename、alias 与 skip 不会跨投影方向泄漏，metadata 不参与 wire、服务标识或发现。
- Core 产生结构化 tracing event；MetricsRecorder 同步非阻塞，panic 后原子禁用，labels 受低 cardinality 与脱敏约束。OpenTelemetry attempt histogram 从 `fusen.rpc.attempts` 改为 `fusen.invocation.attempts`，metric 的 `protocol` attribute 拆为 `http.binding` 与实际可观测时才出现的 `network.protocol.version`；现有 dashboard、告警和查询必须迁移到新 instrument 与 attribute 名称。
- 增加 `http-json-v1` 真实 H1/H2 golden fixtures、macro compile tests、paused-time lifecycle/resilience tests、资源预算测试和跨平台 CI/release gates。

### Removed

- 删除所有旧服务声明/实现入口、单体错误体系、observer 模型、公开 transport/codec 生命周期细节和旧 wire decoder。
- 删除 `WireProtocol`、`ProtocolSet`、`FusenV1`、`SpringCloudV1`、全部 `SpringCloud*` API、`/_fusen/v1` URI、`application/fusen+json` 与 `arguments`/`result` envelope，不保留旧 decoder；同时删除不完整 TLS client stack、无限 queue 语义及兼容 facade，新的 HTTPS client 采用独立审计的 Rustls 边界。
- 删除独立配置 derive 宏 crate；敏感字段通过普通私有配置类型与显式安全 Debug 管理。
