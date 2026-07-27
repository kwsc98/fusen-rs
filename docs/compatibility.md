# 兼容性策略

`0.9.0` 是 fusen-rs 的第一个兼容性 baseline。它发布前的开发提交不提供 API、宏、配置或 wire 迁移兼容；仓库不保留 alias、deprecated facade、旧 decoder 或公开版本过渡模块。

`0.9.0` 发布后：

- patch 版本保持 0.9 公共 Rust API、宏语法和已声明 wire v1 行为兼容；
- 破坏 Rust API 或提升 MSRV 至少需要新的 minor 版本并记录在 CHANGELOG；
- wire 兼容由 `FusenV1`、`SpringCloudV1` 的协议版本独立约束，有意破坏语义必须引入新 wire 版本和 ADR；
- `ServiceEndpoint` 对 canonical `http://`/`https://` 的接受、HTTPS 严格验证与无明文降级属于公开 runtime 行为；内置 Server 不承诺 TLS termination；
- 未记录为公开扩展面的模块、隐藏宏 ABI 与实现细节不构成稳定契约；
- 不承诺完整 Spring MVC 兼容，只承诺 golden fixtures 覆盖的 Spring Cloud V1 子集。

MSRV 为 Rust 1.97，Edition 为 2024。0.9 发布 tag 形成基线后启用 `cargo-semver-checks`；在此之前不拿历史开发提交充当兼容参照。

语义 fixture 固定 method、URI、header、JSON envelope、Problem Details 与参数映射，但不固定 JSON object key 顺序、TCP 分包、H2 frame 或 HPACK 编码。
