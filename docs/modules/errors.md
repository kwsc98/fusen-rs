# 错误契约

> English summary: invocation and lifecycle failures use separate error types;
> wire failures use bounded RFC 9457 Problem Details without internal sources.

## Rust API

- `RpcError` 是生成客户端方法与服务实现唯一的调用错误。
- `ClientError` 表示 runtime build、connect、discovery 与 shutdown 失败。
- `ServerError` 表示 validation、bind、startup、accept、registry 与 shutdown 失败。
- `RegistryError` 和 `ConfigError` 分别由对应 SPI crate 拥有。
- Attempt 与 transport 的内部失败分类不公开。

`RpcError` 字段私有，通过 getter 暴露 `RpcCategory`、经过校验的 `ErrorCode`、安全 message、origin、HTTP status、attempt count 与 retryable hint。可选 source 仅用于本地错误链，永不编码到 wire。`Application` error 始终 `retryable=false`。

`RpcCategory` 包括 `InvalidArgument`、`NotFound`、`Conflict`、`Unauthenticated`、`PermissionDenied`、`PayloadTooLarge`、`ResourceExhausted`、`Unavailable`、`DeadlineExceeded`、`Cancelled`、`Unimplemented`、`Internal`、`DataLoss` 和 `Application`。预期扩展的 error/enum 使用 `non_exhaustive`，调用方必须保留兜底分支。

## Wire Error

错误响应使用 `application/problem+json`，包含 RFC 9457 的 `type`、`title`、`status`、`detail`、`instance`，以及：

- `code`：经校验、稳定的机器码；
- `request_id`：与请求 header 和 trace 一致；
- `retryable`：只是一项受 runtime 硬约束的远端提示。

HTTP status 是最终权威；body 中冲突或非法的 status 作为上游协议错误处理。内部 source、backtrace、panic payload、凭据和完整 headers 永不返回。Internal category 的 detail 固定隐藏实现细节。框架错误的应急响应最大 4 KiB；完整文档超限时删除可选 detail/instance，但保留原始 HTTP status、type、code、request ID 与 retryable。

## Retry 与生命周期优先级

客户端先将 transport/HTTP/problem 解析为私有 attempt outcome，再由 hard guards 与 policy 决定 retry，最终只返回一个带总 attempt count 的 `RpcError`。Middleware、serialization、no-instance、本地 overload、cancellation 与 shutdown 不参与 retry 或 breaker 失败统计。

HTTP/wire 已成功但 typed `result` 无法解码时是契约级协议故障：客户端返回非重试的 `DataLoss`/`invalid_result`，并把 selected endpoint attempt 与 service 最终结果都记录为 `Protocol` breaker failure。

服务端终态优先级为 global deadline > fatal accept > registry aggregate。Registry aggregate 保留确定性顺序并尝试所有 close；更低优先级错误不会丢失，会写入结构化 trace event。
