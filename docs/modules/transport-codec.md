# 传输与编解码行为

> English summary: JSON is the only body codec in 0.9; body frames are read
> incrementally with a hard byte limit.

HTTP/2 用于 Fusen，HTTP/1.1 用于 SpringCloud。JSON 请求支持单参数对象和多参数数组；GET/DELETE/HEAD 使用 query，其余方法使用 body。读取 frame 失败立即终止，累计大小超过限制返回 413。

`application/json` 与 `application/problem+json` 可解码；`application/grpc` 固定返回 415。路径参数和 query 均进行百分号编码。codec tests 覆盖超限与禁用协议。
