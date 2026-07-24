# 兼容性策略

`0.9.0` 发布前属于开发节点，公开 Rust API 可以直接破坏性调整，不承诺兼容层或迁移指南。正式发布后，0.x 的 minor 版本可引入破坏性变更，patch 版本不得主动破坏公开 Rust API 或已声明的线协议。

从 0.9 开始，Fusen JSON HTTP/2 和 SpringCloud JSON HTTP/1.1 子集是受支持协议。SpringCloud 子集包括路径/query 参数和最多一个 JSON request body；不声明完整 Spring MVC 注解兼容。未在协议测试中覆盖的能力不得出现在 README 功能列表。MSRV 为 Rust 1.97，提升 MSRV 属于 minor 变更。

弃用 API 至少保留一个 minor 周期；安全漏洞和错误协议实现可以直接移除，但必须在 SECURITY/CHANGELOG 中说明。
