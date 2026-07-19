# 错误契约

> English summary: every HTTP failure uses RFC 9457 and stable machine codes.

错误响应 content type 为 `application/problem+json`，标准字段为 type/title/status/detail/instance，扩展字段为 code/request_id。固定映射：400 参数、404 路由、413 body、415 协议、503 不可用、504 超时、500 内部错误。

内部 `source` 必须是 `Send + Sync + 'static`，且永不返回客户端；服务端日志通过 request_id 关联。应用可使用 `Application { status, code, message }` 表达稳定业务错误。
