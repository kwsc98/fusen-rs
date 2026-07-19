# Contributing

感谢参与 fusen-rs。提交前请先阅读架构、模块行为文档和兼容性策略。

## 开发流程

1. 为行为变化创建 issue 或 ADR，说明公开契约和失败语义。
2. 保持改动集中，并为新增/修复行为添加测试和中文主文档；公开入口同时添加英文摘要。
3. 运行 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace --all-targets` 和 `cargo test --doc --workspace`。
4. PR 描述必须包含问题、方案、兼容影响、测试结果和文档链接。

不接受静默吞错、无界网络输入、无理由 unsafe、未记录的协议变更或只有示例没有断言的修复。
