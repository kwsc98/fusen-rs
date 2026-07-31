# ADR 0005: Wire v1 首个兼容基线

- 状态：已接受
- 日期：2026-07-26
- 决策者：fusen-rs 维护者

## 背景

0.9 clean-slate 明确允许重置旧开发 wire。新的重试、request ID、deadline、参数来源与错误安全契约需要不依赖历史实现偶然行为的版本化格式。

## 决策

`0.9.0` 定义两个首版协议：

### Fusen V1

- `http://` 使用 HTTP/2 prior knowledge（h2c）；
- `https://` 使用 TLS ALPN `h2`，不得降级到 HTTP/1.1；
- `POST /_fusen/v1/{service}/{method}`；
- `Content-Type: application/fusen+json;version=1`；
- request 为 `{"arguments":{"name":...}}`；
- success 为 `{"result":...}`；
- group/version 由 `x-fusen-service-group`、`x-fusen-service-version` 区分。

### Spring Cloud V1

- `http://` 与 `https://` 都使用 HTTP/1.1；
- 使用方法属性显式声明的 method/path/query/body；
- query 默认为 Scalar，只有 `#[param(query, repeated)]` 声明 Repeated；Repeated 的 0/1/N 个值分别编码为 0/1/N 个同名 key；
- route template literal 仅接受 ASCII RFC3986 `pchar` 或以大写百分号编码的合法 UTF-8 非 ASCII 字符；拒绝 raw Unicode、空白、控制字符、反斜线、坏或非规范 `%`、编码 ASCII、点段，以及解码后属于 Unicode 空白或控制字符的 literal；
- 最多一个 JSON body，success 为 raw JSON；
- 只承诺 fixtures 覆盖的子集，不声明完整 Spring MVC 兼容。

两种协议共享经验证的 `x-request-id`、`x-fusen-timeout-ms`、`x-fusen-attempt`。错误统一为 RFC 9457 `application/problem+json`，扩展 `code`、`request_id`、`retryable`。Source 和 panic payload 不进入 wire。

HTTPS 的信任根、验证与服务端终止边界由
[ADR 0006](0006-client-tls-and-plaintext-server.md) 定义。TLS 不改变 method、URI、
header multimap、JSON shape 或 Problem Details，因此不引入新的 wire 版本。

Golden fixtures 固定 method、URI、header multimap、JSON shape、Problem Details、path/query/body mapping、request ID、deadline 和 attempt。Fixtures 不固定 H2 frame、HPACK、TCP 分包或 JSON object key order。

现有 v1 fixture 不得为迁就实现而改写。有意破坏语义必须新增 wire 版本与 ADR；runtime 不提供旧格式猜测或双 decoder。

## 后果

Client、Server 与第三方实现可以依赖可审查的语义 fixture。重写 transport/codec 不影响协议，只要双向 golden 和真实 socket tests 继续通过。

## 备选方案

- 沿用旧开发 wire：拒绝，无法表达新错误、deadline 与参数契约。
- 冻结原始 H2 字节：拒绝，frame/HPACK 不是业务语义。
- 只做同仓 client/server roundtrip：拒绝，同一错误实现可能两端互相抵消。
