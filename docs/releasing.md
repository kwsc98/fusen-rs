# 发布流程

1. 更新所有 workspace crate 到同一版本并维护 CHANGELOG。
2. 使用显式工具链命令执行本地检查，避免 `rust-toolchain.toml` 掩盖 stable 验证：

   ```shell
   cargo +1.97.0 fmt --all --check
   cargo +1.97.0 clippy --locked --workspace --all-targets --all-features -- -D warnings
   cargo +1.97.0 test --locked --workspace --all-features
   cargo +1.97.0 check --locked -p fusen-config --no-default-features
   cargo +1.97.0 check --locked -p fusen-observability --no-default-features
   cargo +1.97.0 test --locked --workspace --doc
   RUSTDOCFLAGS="-D warnings" cargo +1.97.0 doc --locked --workspace --all-features --no-deps
   cargo +stable check --locked --workspace --all-targets --all-features
   cargo +stable test --locked --workspace --all-features
   cargo +1.97.0 tree -p fusen-register -e normal,build -e features
   cargo deny check
   cargo audit
   ```

3. 按 `fusen-contract`、`fusen-register`、配置/可观测性、Nacos、宏、核心 RPC 的依赖顺序，对每个发布 crate 执行 `cargo +1.97.0 package --locked --list -p <crate>` 并检查归档内容。
4. 依赖顺序中的前置 crate 已能从 registry 解析后，逐个执行真实的 `cargo +1.97.0 publish --locked --dry-run -p <crate>`；前置 crate 尚不可用时，不得用跳过验证替代该步骤。
5. 从干净环境运行不依赖 Nacos/OTLP 的 Direct 示例。真实 Nacos 验证是可选的手工步骤，由维护者准备服务并显式运行：

   ```shell
   NACOS_ADDR=127.0.0.1:8848 cargo +1.97.0 test --locked -p fusen-nacos --all-features live_nacos_ -- --ignored
   ```
6. 发布和 tag 均由维护者在全部本地门禁通过后手工执行，本仓库不提供自动上传、tag 或远端流水线。

发布失败时不得覆盖已发布 crate；提升 patch 版本重新执行流程。
