# 错误契约

> English summary: every HTTP failure uses RFC 9457 and stable machine codes.

错误响应 content type 为 `application/problem+json`，标准字段为 type/title/status/detail/instance，扩展字段为 code/request_id。固定映射：400 参数、404 路由、413 body、415 协议、502 非法上游响应、503 不可用、504 超时、500 内部错误。

内部 `source` 必须是 `Send + Sync + 'static` 且永不返回客户端。`x-request-id` 在客户端、上下文、响应 header、日志和 Problem Details 中保持一致。应用错误只能通过 `FusenError::application(StatusCode, code, message)` 构造，1xx、2xx、3xx 会被拒绝，`ApplicationError` 的字段保持私有。远端 Problem Details 的 body status 与 HTTP status 冲突时以真实 HTTP status 为准；错误响应构建失败或遇到防御性无效状态时只能回退 500，不能回退 200。
