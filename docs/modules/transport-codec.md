# Wire、Transport 与 Codec

> English summary: Fusen V1 is JSON over h2c or TLS/ALPN h2, Spring Cloud V1 is
> an explicit HTTP/1.1 subset, and all transport/codec machinery is private.

## Fusen V1

Fusen V1 在 `http://` endpoint 上使用 HTTP/2 prior knowledge（h2c），在
`https://` endpoint 上使用 TLS 1.2/1.3 并要求 ALPN `h2`：

```text
POST /_fusen/v1/{service}/{method}
Content-Type: application/fusen+json;version=1
x-fusen-service-group: ...       # optional
x-fusen-service-version: ...     # optional

{"arguments":{"parameter":...}}
{"result":...}
```

所有方法参数按名称进入 `arguments` object。Service 与 method 是经过验证并逐 segment 编码的身份，不依赖 Rust 的声明顺序或进程内 `MethodId`。

## Spring Cloud V1

Spring Cloud V1 在 `http://` 与 `https://` endpoint 上都使用 HTTP/1.1。Method 与 path 来自必需的 `method` 属性；参数按接口规则推断为 path、query 或 JSON body object field，也可显式声明 query 或唯一 raw JSON body。`#[param(query, repeated)]` 将序列化为 JSON array 的参数固定映射为重复 query key，普通 query 默认为 scalar。成功响应是 raw JSON，Content-Type 为 `application/json`。HEAD 映射仅允许 unit 成功类型，客户端不读取其响应 body；失败时因 HTTP 不传输 Problem Details body，客户端以 HTTP status 生成 `remote_head_error`。Path placeholder 与 query 值使用结构化 URI 编码；本协议不声明完整 Spring MVC 兼容。

## 控制 Header

- `x-request-id`：必须唯一，1-64 字节且仅 `[A-Za-z0-9._-]`；缺失时生成，重复或非法返回 400。
- `x-fusen-timeout-ms`：相对剩余毫秒，范围 `0..=86_400_000`；服务端采用 wire 与本地上限的较小者。
- `x-fusen-attempt`：从 1 开始；同一逻辑调用共享 request ID。非幂等方法收到大于 1 的值返回 400。

协议版本、控制 headers、deadline 与 readiness 在读取 body 前验证。两种 wire 的错误都使用 `application/problem+json`，包含 RFC 9457 字段和 `code`、`request_id`、`retryable`。

## Body 与预算

已知 `Content-Length` 超过单 body 上限时不 poll body。未知长度按最多 4 KiB 增量申请全局 byte budget，初始 buffer 不超过 16 KiB。Admission 和预算默认 fail-fast，取消、timeout 与 panic 后 permit 必须恢复。

Response writer 在扩展输出 buffer 前先增量申请全局 permit，并直接单次序列化，不经过通用 JSON Value 中间树；Middleware 短路响应也必须通过 `RpcContext::respond` 进入同一路径。Fusen result envelope 以 prefix/result/suffix 分段发送，不复制完整结果。响应超限返回非 retryable `500 response_too_large`；错误响应使用独立 4 KiB emergency path。Response permit 跟随 body payload，直到 Hyper transport 消费或取消对应 chunk。

Body byte budget 覆盖 runtime 持有的 decoded/encoded payload 以及 Hyper 尚未消费的排队 chunk。HTTP framing、HPACK/H2 codec staging 和 OS socket buffer 不计入 body budget；这些固定 transport overhead 由 header、TCP connection、H2 stream 限制及 transport 自身的有界 buffer 约束。因此该预算不是整个进程或内核网络栈的总内存上限。

## Transport 边界

Client 接受 canonical `http://` 与 `https://` endpoint。HTTPS 使用 Rustls Ring、bundled Mozilla WebPKI roots、TLS 1.2/1.3 及严格的证书/hostname 验证；不读取系统 trust store，也不提供自定义 CA、mTLS、跳过验证或明文 fallback。Server acceptor 仍只处理明文 HTTP/1.1 与 h2c，入站 TLS 由 sidecar、ingress、reverse proxy 或 service mesh 终止。Transport、Codec、Acceptor、pool、TLS config 与 socket state 均为私有实现，不提供替换 SPI。

Golden fixtures 固定 method、URI、header multimap、JSON envelope 与 Problem Details；不固定 JSON key order、TCP 分包、H2 frame 或 HPACK。
