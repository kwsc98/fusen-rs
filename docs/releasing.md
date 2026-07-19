# 发布流程

1. 更新所有 workspace crate 到同一版本并维护 CHANGELOG。
2. 执行 format、Clippy、workspace tests、doctest、文档构建、依赖审计和 Markdown 链接检查。
3. 使用 `cargo package --list` 检查每个发布 crate，确认不存在 registry 版 fusen 重复依赖。
4. 按内部公共类型、注册契约、宏、common、核心 RPC 的顺序发布。
5. 从干净环境运行 Direct 示例；设置 `NACOS_ADDR` 后运行 Nacos 验证。
6. 创建签名 tag `vX.Y.Z`，附带 CHANGELOG 和迁移链接。

发布失败时不得覆盖已发布 crate；提升 patch 版本重新执行流程。
