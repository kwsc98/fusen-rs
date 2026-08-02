# 兼容性策略

`0.9.0` 是 fusen-rs 的第一个兼容性 baseline。它发布前的开发提交不提供 API、宏、配置或 wire 迁移兼容；仓库不保留 alias、deprecated facade、旧 decoder 或公开版本过渡模块。

首个 HTTP binding 与 discovery/capability 边界见
[ADR 0009](adr/0009-http-binding-discovery-decoupling.md)。

`0.9.0` 发布后：

- patch 版本保持 0.9 公共 Rust API、宏语法和已声明 `http-json-v1` 行为兼容；
- 破坏 Rust API 或提升 MSRV 至少需要新的 minor 版本并记录在 CHANGELOG；
- HTTP 表示兼容由稳定的 binding ID 独立约束；`http-json-v1` 的有意破坏必须引入新的 binding ID 和 ADR；
- `ServiceEndpoint` 对 canonical `http://`/`https://` 的接受、HTTPS 严格验证与无明文降级属于公开 runtime 行为；内置 Server 不承诺 TLS termination；
- 未记录为公开扩展面的模块、隐藏宏 ABI 与实现细节不构成稳定契约；
- `HttpBindingId`、`HttpVersionPolicy` 与 endpoint capabilities 相互独立；新增 transport 能力不会隐式改变 binding bytes，也不会改变 registry subscription identity。

首个 baseline 将 `SensitiveShape::Fields` 固定为独立的 serialization/deserialization
字段表，并将 repeated query 固定为显式 `#[param(query, repeated)]`。这两项在
`v0.9.0` tag 之前不提供旧 API 或旧宏行为兼容层；tag 之后按上述公共 API、宏语法和
HTTP binding fixture 规则维护兼容性。

MSRV 为 Rust 1.97，Edition 为 2024。`v0.9.0` tag 形成基线后启用 `cargo-semver-checks`；在此之前不拿历史开发提交充当兼容参照。

语义 fixture 固定 binding ID、method、URI、header、raw JSON body、Problem Details 与参数映射，但不固定 JSON object key 顺序、TCP 分包、H2 frame 或 HPACK 编码。
