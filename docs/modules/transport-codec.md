# 传输与编解码行为

> English summary: JSON is the only body codec in 0.9; body frames are read
> incrementally with a hard byte limit.

HTTP/2 用于 Fusen，HTTP/1.1 用于 SpringCloud。路径模板参数始终进入 URL path；GET/DELETE/HEAD 的其余参数进入 query；其他参数进入 body。Fusen 的零/单/多 body 参数分别编码为空、原始 JSON、精确长度 JSON 数组；SpringCloud 最多允许一个 body 参数。

所有请求和响应 body 都逐 frame 读取并受硬字节上限约束。`application/json` 与 `application/problem+json` 通过 MIME 解析；请求中的重复、非法或非 JSON Content-Type（包括 `application/grpc`）返回 415，响应中的对应问题统一映射为 `InvalidResponse`。URI 使用结构化 URL API，路径参数按 segment 编码并在服务端解码，query 支持重复字段。
