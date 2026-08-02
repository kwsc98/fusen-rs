# ADR 0009: HTTP binding 与 discovery 解耦

- 状态：已接受
- 日期：2026-08-02
- 决策者：fusen-rs 维护者
- 取代：[ADR 0005](0005-wire-v1-contract.md) 的 wire baseline，以及
  [ADR 0006](0006-client-tls-and-plaintext-server.md) 中按旧协议绑定 HTTP version 的决策

## 背景

开发期实现把表示格式、HTTP transport 和服务发现耦合在一个协议枚举中：
私有 envelope variant 同时表示固定 URI、`arguments`/`result` envelope 与 HTTP/2，
显式 HTTP mapping variant 同时表示参数 mapping 与 HTTP/1.1。Registry subscription
也把 protocol 当作服务身份的一部分。

这会让同一服务因 Client transport 偏好创建重复订阅，也无法准确表达“同一个 endpoint
支持多个 HTTP version 或 binding”。私有 Fusen envelope 还要求非标准 URI 与 content
type，却没有提供普通 HTTP operation + raw JSON 无法表达的服务调用能力。

`v0.9.0` 尚未发布，因此当前可以在第一个兼容基线前消除这组错误边界，而不永久维护
双 decoder、双 registration metadata 或已弃用 Rust API。

## 决策

### HTTP binding

- 删除开发期协议枚举、集合、variants 与专用 mapping 公共类型；
  不提供 alias 或 deprecated facade。
- 唯一内置表示为 `http-json-v1`，其 Rust 标识由 `HTTP_JSON_V1` 与
  `HttpBindingId` 表达。每个 method 都有必需的 `HttpOperation`，包含 method、route、
  参数来源以及缺省为 `application/json` 的 `consumes`/`produces`。
- Request 参数直接映射到 path、query、header、cookie、query/header map、JSON body
  field 或唯一 raw body；成功响应是 raw body。删除 `/_fusen/v1/...`、
  `application/fusen+json;version=1` 与 `arguments`/`result` envelope。
- RFC 9457 `application/problem+json` 错误结构、错误码安全边界、request ID 与资源限制
  继续由 binding codec 负责。Fusen timeout/attempt controls 是独立 endpoint capability，
  不是 binding identity。
- Client 可通过 `ClientRuntimeBuilder::http_binding(...)` 为其他 `HttpBindingId` 注册
  `RequestEncoder`、`ResponseDecoder` 与 `ErrorDecoder`。Codec 只处理有界 HTTP semantic
  data，不接管 transport/lifecycle；built-in Server 只实现 `http-json-v1`。

### Transport capabilities

- `EndpointCapabilities` 声明非空 `HttpVersionSet`、非空 `HttpBindingId` 集合及
  invocation-controls flag。默认值是 HTTP/1.1、`http-json-v1`、controls disabled。
- `HttpVersionPolicy::{Auto, Http1, Http2, H2c}` 只表达 Client transport 偏好。
  Binding ID 不编码 HTTP version，新增 HTTP transport 能力不改变 representation bytes。
- Direct endpoint 未显式声明 capabilities 时支持 Client 选中的 binding 并关闭
  controls；`http://` + `Auto` 使用 HTTP/1.1，`https://` + `Auto` 通过
  ALPN 协商 HTTP/2 或 HTTP/1.1。
- `ServiceRegistration` 和 `ServiceInstance` 都携带 capabilities；Client 每次 attempt
  先按目标 binding 与 version policy 过滤 endpoint，再进行 routing、load balancing 与
  transport connection。

### Discovery 与 provider convention

- `RegistrationRequest` 只包装完整 `ServiceRegistration`；`SubscriptionRequest` 只携带
  `ServiceSelector`。Subscription supervisor 以 selector 为唯一身份，不按 binding 或
  transport policy 分裂。
- Nacos service name 固定为 `selector.service_id`，group 使用 selector group 或
  `DEFAULT_GROUP`，version metadata 必须与 selector version 严格匹配。删除
  `fusen.protocol`，不双写也不双读。
- Canonical Nacos convention 严格要求 `fusen.http.bindings` 与
  `fusen.http.versions`；controls enabled 时另写 `fusen.invocation-controls=v1`。
  非法、重复、未知或 partial capability metadata 会使实例被过滤。
- `NacosConvention::SpringCloud` 只是 provider metadata 兼容策略，不是另一种 HTTP
  binding。仅当三项 capability key 全部缺失时，它把实例解释为
  `EndpointCapabilities::default()`；只要出现任一 key，就执行 canonical 严格解析。

## 迁移与兼容影响

这是 `v0.9.0` tag 前的一次有意 wire break。开发期私有 envelope peer
使用的固定 URI、content type 和 envelope 与 `http-json-v1` 不互通，
runtime 不探测旧流量，也不提供 fallback decoder。部署必须将 Client
与 Server 一起升级，或在升级窗口使用显式外部 HTTP adapter。

开发期显式 HTTP mapping 的 method/path/query/body 与 raw JSON 基础语义成为
`http-json-v1`，但 Rust 类型名、binding identity、capability metadata、额外参数来源和
media type 契约均以本 ADR 为准。Registry 数据需要重新注册；旧 `fusen.protocol`
metadata 不会被读取。Dashboard、日志、golden fixture、fuzz target 和外部 consumer
必须从 protocol 维度迁移到 binding + HTTP version/capabilities 维度。

`v0.9.0` 发布后，`http-json-v1` 的 method、URI、headers、body 和 Problem Details 语义
成为兼容基线；未来有意改变表示必须使用新的 binding ID。新增 HTTP version、registry
provider convention 或 capability 不要求新 binding，只要 representation bytes 不变。

## 后果

发现缓存不再因 transport 选择重复，endpoint 可以准确发布多种能力，普通 HTTP JSON
服务也可用保守默认 capabilities 接入。代价是旧开发 wire 与 registry metadata 必须一次性
迁移，运行时不能通过 protocol enum 隐式推导 HTTP version 或控制 header 支持。

## 备选方案

- 保留私有 envelope 作为高性能 binding：拒绝；私有 envelope 与固定 URI 没有证明其收益，
  却扩大互操作和兼容成本。
- 把 HTTP version 编进 binding ID：拒绝；表示和 transport 会再次耦合，同一 endpoint
  无法自然声明多版本能力。
- 按 `(ServiceSelector, HttpBindingId)` 订阅：拒绝；capabilities 属于实例快照，同一服务
  不应为每个调用策略创建远端 listener。
- 永久接受 capability metadata 缺失：拒绝；Canonical 必须 fail closed。兼容 fallback
  仅由显式 `NacosConvention::SpringCloud` 启用。
