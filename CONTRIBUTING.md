# Contributing

感谢参与 fusen-rs。提交前请先阅读架构、模块行为文档和兼容性策略。

## 开发流程

1. 为公开 API、wire、crate 边界或生命周期变化创建 ADR，写明失败语义和资源所有权。
2. 保持改动集中，并为新增/修复行为添加确定性测试和中文主文档；公开入口同时添加英文摘要。
3. 不使用预占端口或 correctness sleep。并发与时间行为使用一次绑定的 listener、paused time、Barrier、oneshot 或 Semaphore。
4. 运行 `cargo +1.97.0 fmt --all --check`、`cargo +1.97.0 clippy --locked --workspace --all-targets --all-features -- -D warnings`、`cargo +1.97.0 test --locked --workspace --all-features` 和 doc checks。
5. PR 描述必须包含问题、方案、兼容影响、测试结果、性能影响和文档链接。

不接受静默吞错、无界网络输入、unsafe、未记录的协议变更、公开泄漏 provider/runtime internals，或只有示例没有断言的修复。新增扩展点必须证明现有六个 SPI 无法表达需求。

修改 wire、route 或 Problem Details 时，同时运行 `cargo +1.97.0 check --locked --offline --manifest-path fuzz-support/Cargo.toml`，并按 [`fuzz/README.md`](fuzz/README.md) 运行对应 fuzz target。生命周期或双协议 E2E 变化可用 `.github/scripts/repeat-core-e2e-tests.sh <次数>` 做本地重复验证；nightly 固定执行 100 轮。
