# HTTP Binding、Transport 与 Codec

> English summary: `http-json-v1` defines the HTTP representation independently
> from endpoint discovery and HTTP version selection; client binding codecs are extensible,
> while transport and server codecs remain private.

## `http-json-v1`

每个 interface method 都必须声明 `HttpOperation`，包含 HTTP method、canonical route、
参数位置以及 `consumes`/`produces` media type。缺省 media type 为
`application/json`。Binding ID 的稳定 registry/telemetry 表示为 `http-json-v1`；
它不包含服务名、HTTP version 或 provider convention。

参数可映射到 path、query、header、cookie、query map、header map、一个 synthesized
JSON body object 的字段，或唯一 raw JSON body。`#[param(query, repeated)]` 把 JSON
array 编码为 0/1/N 个同名 query key。Path、query、header 与 cookie 通过对应的结构化
HTTP API 编码，不拼接未验证字符串。成功响应直接包含声明 media type 的 raw body：

```text
POST /users
Content-Type: application/json
Accept: application/json

{"name":"Ada","audit":true}
{"id":"42","name":"Ada"}
```

Binding 不使用 `/_fusen/v1/...` 路由、`application/fusen+json` content type，或
`arguments`/`result` envelope。HEAD 仅允许 unit 成功类型，客户端不读取成功 body；
失败时若 HTTP 语义不传输 Problem Details body，客户端按 status 生成
`remote_head_error`。

## Capabilities 与 HTTP Version

`EndpointCapabilities` 分别声明 `HttpVersionSet`、非空 `HttpBindingId` 集合，以及
是否支持 Fusen invocation controls。`HttpVersionPolicy::{Auto, Http1, Http2, H2c}`
是 Client transport 偏好，不改变 `http-json-v1` 表示。Discovery 只按
`ServiceSelector` 订阅；每次 attempt 在路由阶段过滤不支持目标 binding 或 version
policy 的 endpoint。

默认 capabilities 是 HTTP/1.1、`http-json-v1`、controls disabled。这个缺省值允许
普通 HTTP JSON endpoint 和未携带 Fusen capability metadata 的兼容 provider 接入，
同时避免把 h2c 或私有控制 header 当作隐式能力。
Direct endpoint 未显式声明 capabilities 时使用 Client 选中的 binding 并关闭
controls；`http://` + `Auto` 使用 HTTP/1.1，`https://` + `Auto` 通过 ALPN
协商 HTTP/2 或 HTTP/1.1。

## 控制 Header

- `x-request-id`：必须唯一，1-64 字节且仅 `[A-Za-z0-9._-]`；缺失时生成，重复或非法返回 400。
- `x-fusen-timeout-ms`：相对剩余毫秒，范围 `0..=86_400_000`；服务端采用 wire 与本地上限的较小者。
- `x-fusen-attempt`：从 1 开始；同一逻辑调用共享 request ID。非幂等方法收到大于 1 的值返回 400。

Binding、控制 headers、deadline 与 readiness 在读取 body 前验证。Built-in Server 错误使用 `application/problem+json`，包含 RFC 9457 字段和 `code`、`request_id`、`retryable`；Client 也接受合法的外部 RFC Problem type。`x-fusen-timeout-ms` 与 `x-fusen-attempt` 只在 endpoint capabilities 声明 invocation controls 时发送。

## Body 与预算

已知 `Content-Length` 超过单 body 上限时不 poll body。未知长度按最多 4 KiB 增量申请全局 byte budget，初始 buffer 不超过 16 KiB。Admission 和预算默认 fail-fast，取消、timeout 与 panic 后 permit 必须恢复。

Response writer 在扩展输出 buffer 前先增量申请全局 permit，并直接单次序列化，不经过通用 JSON Value 中间树；Interceptor 短路响应也必须通过 `Context::respond` 进入同一路径。Raw success body 不经过私有 result envelope。响应超限返回非 retryable `500 response_too_large`；错误响应使用独立 4 KiB emergency path。Response permit 跟随 body payload，直到 Hyper transport 消费或取消对应 chunk。

Body byte budget 覆盖 runtime 持有的 decoded/encoded payload 以及 Hyper 尚未消费的排队 chunk。HTTP framing、HPACK/H2 codec staging 和 OS socket buffer 不计入 body budget；这些固定 transport overhead 由 header、TCP connection、H2 stream 限制及 transport 自身的有界 buffer 约束。因此该预算不是整个进程或内核网络栈的总内存上限。

## Transport 边界

Client 接受 canonical `http://` 与 `https://` endpoint。HTTPS 使用 Rustls Ring、bundled Mozilla WebPKI roots、TLS 1.2/1.3 及严格的证书/hostname 验证；不读取系统 trust store，也不提供自定义 CA、mTLS、跳过验证或明文 fallback。Server acceptor 仍只处理明文 HTTP/1.1 与 h2c，入站 TLS 由 sidecar、ingress、reverse proxy 或 service mesh 终止。

Client binding 扩展可实现 `RequestEncoder`、`ResponseDecoder` 与 `ErrorDecoder`，只处理已验证、受 byte limit 约束的 HTTP semantic parts，不拥有 socket、pool、TLS 或 lifecycle。`EncodedRequest` 在网络 I/O 前再次经过 method/URI/header/body validation；`BufferedResponse` 不自动向 decoder 暴露 hop-by-hop 与 runtime control headers。HTTP Transport、Server codec、Acceptor、pool、TLS config 与 socket state 均为私有实现，不提供替换 SPI。

Golden fixtures 固定 binding ID、method、URI、header multimap、raw JSON body 与 Problem Details；不固定 JSON key order、TCP 分包、H2 frame 或 HPACK。
