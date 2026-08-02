# 错误契约

> English summary: each crate owns its domain errors; invocation failures use
> orthogonal kind, origin, and category dimensions, while untrusted Problem
> Details are validated at the HTTP binding boundary.

## Ownership 与 Rust API

错误由产生并理解该失败的 crate 维护，不建立 workspace 级总错误枚举：

| Owner | Public errors | Responsibility |
| --- | --- | --- |
| `fusen-rs` | `Error` | 单次 service invocation 的应用或框架失败 |
| `fusen-rs` | `ClientError`, `ServerError` | client/server runtime 的构建、启动、运行和关闭失败 |
| `fusen-register` | `RegistryError` | registry provider 与 registration/subscription lifecycle 失败 |
| `fusen-config` | `ConfigError` | 静态配置及配置 provider lifecycle 失败 |

crate 边界通过标准 `std::error::Error::source` 保留因果链，不通过一个依赖所有
crate 的 `FusenError` 聚合。Attempt、transport 与 breaker 的内部分类不公开。

`Error` 使用三个正交维度：

- `ErrorKind::{Application, Framework}` 表示错误由业务语义还是框架行为定义；
- `ErrorOrigin::{Local, Remote}` 表示本进程还是远端 peer 产生错误；
- `ErrorCategory` 表示 `InvalidArgument`、`NotFound`、`Conflict`、
  `Unauthenticated`、`PermissionDenied`、`PayloadTooLarge`、
  `ResourceExhausted`、`Unavailable`、`DeadlineExceeded`、`Cancelled`、
  `Unimplemented`、`Internal`、`DataLoss` 或无法映射的 `Unknown` 语义。

`ErrorCategory::canonical_status()` 返回已知 category 的标准 HTTP status；`Unknown`
没有 canonical status。服务实现用 `Error::application(category, code, message)` 创建
canonical application error，用 `Error::application_status(status, code, message)` 表达
418、422 等非 canonical application status。Interceptor、Router、LoadBalancer 等扩展
用 `Error::local(category, code, message)` 创建 local framework error。构造失败统一返回
`ErrorConstructionError`，调用方不能绕过 error code 或 status 校验。

`ErrorCode` 必须匹配 `[a-z][a-z0-9]*(?:_[a-z0-9]+)*` 且最长 64 字节。
`Error` 初始 attempt count 为 0，进入一次物理 attempt 后才增加。Application error 的
retry hint 始终归一化为 `Never`。所有可扩展 error/enum 都使用 `non_exhaustive`，调用方
匹配时必须保留兜底分支。

`Error`、`ClientError`、`ServerError`、`RegistryError` 与 `ConfigError` 的 `Debug` 只包含
安全分类和公开 message，不展开 source、header 或 details 的值。`RegistryError` 与
`ConfigError` 的 `Display` 同样只使用 `safe_message()` 返回的安全文案；provider/parser source
仅通过 `source_ref()` 和标准 `source()` 供本地诊断系统显式遍历。

详细决策见 [ADR 0008](../adr/0008-error-ownership-and-classification.md)。

## HTTP Error

错误响应继续使用 `application/problem+json`，RFC 9457 字段和现有扩展字段保持不变：

- `type`、`title`、`status`、`detail`、`instance`；
- `code`：经过校验的稳定机器码；
- `request_id`：与逻辑调用 ID 一致；
- `retryable`：受 runtime hard guard 约束的远端提示；
- `details`：可选的 application JSON object。

`http-json-v1` 接受外部 RFC Problem type；一旦使用
`urn:fusen:error:<category>:<code>` 就严格校验 URN、body code、body status 与 HTTP
status。Endpoint 声明 invocation controls 时，response request-ID header 与 Problem
request ID 都必须出现、合法且匹配本地逻辑调用 ID；controls disabled 时按普通外部 HTTP
服务处理这些可选字段，不把缺失视为协议损坏。

非法 JSON、URN、status、code 或 request ID 转为
`Framework + Remote + DataLoss`，并作为 `Protocol` failure 处理。内部 source、
backtrace、panic payload、凭据与完整 headers 永不进入 wire。框架错误的应急响应最大
4 KiB；超限时删除可选 detail/instance，保留 status、type、code、request ID 与
retryable。

远端 headers 和 application details 可供当前调用方检查，但服务通过 `?` 返回远端
`Error` 时不会自动重新编码这些值。只有 `Application + Local` details 会写入响应，
避免把不可信上游元数据透传到下一跳。

## Retry 与 Breaker

HTTP binding 层先将 transport、status 与 Problem Details 归一化成 `Error`，随后每个失败 attempt
只计算一次私有 failure class。retry、attempt metrics、endpoint breaker 与 service
breaker 复用同一结果，不各自解释 status 或 HTTP binding。

- Application error 永不自动重试；remote Application 4xx 不惩罚 breaker，5xx 记为
  `RemoteFailure` 并惩罚 breaker。
- Remote framework error 只在 transient status 与 HTTP method replay guard 允许时重试；
  `Retry-After` 只解析一次，且只影响已经允许重试的错误。Endpoint capabilities 中的
  invocation-controls flag 决定控制 header 与 request ID 的严格协商，不替代本地 retry
  hard guard。
- 协议破损，以及 remote HTTP 响应的 typed raw body 解码失败，记为 `Protocol` breaker
  failure；Interceptor 本地替换响应后的解码失败仍是 local error，不惩罚 breaker。
- Interceptor、serialization、no-instance、本地 overload、cancellation、shutdown、
  客户端读取 response limit 与 byte budget rejection 不重试，也不惩罚 breaker。

响应 framing、Content-Type 或 Content-Length 破损是 remote protocol failure；超过本地
response limit 是 local `PayloadTooLarge`，耗尽本地 byte budget 是 local
`ResourceExhausted`。HTTP HEAD 在 Content-Type 和 body 校验前完成分支，不要求
Problem body。

服务端终态优先级保持 global deadline > fatal accept > registry aggregate。Registry
aggregate 使用确定性顺序并尝试所有 close；低优先级错误通过结构化 trace event 保留。
