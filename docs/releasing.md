# 0.9 发布流程

`0.9.0` 是第一个 compatibility baseline。发布前不运行相对历史开发提交的 semver 检查；tag 完成后以该 tag 配置后续 `cargo-semver-checks`。

## Required Checks

```shell
cargo +1.97.0 fmt --all --check
cargo +1.97.0 fmt --manifest-path fuzz-support/Cargo.toml -- --check
cargo +1.97.0 fmt --manifest-path fuzz/Cargo.toml -- --check
cargo +1.97.0 clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo +1.97.0 test --locked --workspace --all-features
cargo +1.97.0 test --locked --workspace --all-features --doc
RUSTDOCFLAGS="-D warnings" cargo +1.97.0 doc --locked --workspace --all-features --no-deps

cargo +1.97.0 check --locked -p fusen-config --no-default-features
cargo +1.97.0 check --locked -p fusen-observability --no-default-features
cargo +stable test --locked --workspace --all-features
cargo deny check advisories bans licenses sources
cargo audit
bash .github/scripts/check-dependency-policy.sh
bash .github/scripts/check-public-api-denylist.sh
bash .github/scripts/check-package-consumer.sh
cargo +1.97.0 clippy --locked --offline --manifest-path fuzz-support/Cargo.toml --all-targets -- -D warnings
```

CI 还必须通过 Linux/macOS/Windows、feature matrix、renamed-runtime macro consumer、`nacos-live-container`、HTTP/HTTPS client socket tests、lifecycle 重复测试、Markdown links 与从 `.crate` archive 构建的 package consumer。`repeat-lifecycle-tests.sh` 和 `run-live-nacos-tests.sh` 都先列举预期测试并在匹配数为零时失败，Cargo 的空过滤器不得被当成成功。
定时流水线还必须通过三个真实私有源码 harness 的 `cargo-fuzz` 任务，以及 `runtime_e2e` 和 `wire_v1_contract` 的 100 轮真实 socket 重复测试。fuzz corpus 与运行方法见 [`fuzz/README.md`](../fuzz/README.md)。

## Contract Audit

发布负责人必须确认：

- `check-public-api-denylist.sh` 从干净 rustdoc 输出确认旧入口为零，只有 `service`/`method` 宏和约定的六个扩展 SPI；
- root、fuzz-support 和 fuzz 的解析后 dependency graph 与 lockfile 只包含批准的 Rustls Ring/bundled WebPKI client TLS 栈，不含 `hyper-tls`、`native-tls`、OpenSSL TLS backend、AWS-LC、native/system root loader、platform verifier 或 PEM loader；
- Core 不依赖 Nacos、subscriber 或 OTel backend；
- Fusen V1/Spring Cloud V1 golden fixtures、真实明文 H1/h2c sockets、HTTPS H1/ALPN h2 sockets、证书/hostname 拒绝、Problem Details 和 macro trybuild 全部通过；
- HTTPS 测试确认 Rustls Ring、TLS 1.2/1.3、bundled Mozilla WebPKI roots、无明文 fallback；Server listener 仍不加载证书或私钥；
- 永久 pending request/registry/config cleanup 在 deadline 内有界返回；
- lifecycle、retry、breaker 与 byte-budget tests 不依赖 correctness sleep 或预占端口；
- 在绑定参考机器上执行 `Release Benchmark Gate`，direct single-attempt p50/p99 相对 committed baseline 回退不超过 10%，原始五轮日志与 JSON summary 已归档。

真实 Nacos container tests 是 required release gate。CI 固定启动 Nacos `v2.4.3` standalone service container，测试同时覆盖 registration/discovery 和 config publish/listen，并在主流程失败后继续关闭 handle、注销 instance 和删除 config。资源名由 GitHub run id、进程 id 和时间戳组成，不复用共享名称。

在已经运行并 ready 的 Nacos 上可复现同一个非空 gate：

```shell
NACOS_ADDR=127.0.0.1:8848 \
NACOS_TEST_RUN_ID=manual-$(date +%s) \
bash .github/scripts/run-live-nacos-tests.sh
```

脚本在执行前要求至少发现一个 `live_nacos_` ignored test；零测试、provider error、cleanup error 或 deadline 均使 gate 失败。CI job 是容器启动和 finally cleanup 的规范入口，本地命令不会代替调用方关闭自己启动的 Nacos 容器。

## Performance Gate

普通托管 CI 会真实运行一轮 benchmark smoke sample 并上传 `target/benchmark-smoke`，用于确认 socket benchmark 可执行和输出格式稳定；不同托管机器之间的绝对延迟不用于 10% 判定。

正式比较由 [release-benchmark.yml](../.github/workflows/release-benchmark.yml) 在带有 `fusen-benchmark-0-9-reference` label 的固定 self-hosted runner 上执行。该 runner 必须是建立 [committed baseline](../.github/benchmarks/fusen-0.9-reference-macos-arm64.json) 的同一参考机器：

```shell
python3 .github/scripts/run-benchmark-gate.py \
  --host-id fusen-0.9-reference-macos-arm64 \
  --runs 5 \
  --baseline .github/benchmarks/fusen-0.9-reference-macos-arm64.json \
  --output-dir target/release-benchmark-gate
```

比较器拒绝 host id、toolchain、schema、run count 不匹配和缺失 baseline；p50 或 p99 任一中位数回退超过 10% 即非零退出。发布负责人必须保留 workflow artifact，并确认 checkout SHA 是待发布 commit。

## Packaging Order

按以下依赖顺序检查 archive 内容并执行 dry run：

```text
fusen-contract
fusen-register / fusen-config / fusen-observability / macro crates
fusen-nacos
fusen-rs
```

`check-package-consumer.sh` 对每个发布 crate 执行不带 `--no-verify` 的 `cargo package`，随后解包全部 `.crate` 并在七个独立外部 workspace 中逐项编译；`fusen-rs` consumer 同时展开服务宏。前置 crate 可从 registry 解析后再执行 `cargo +1.97.0 publish --locked --dry-run -p <crate>`。不得使用跳过依赖验证伪造成功。

确认 README、crate README、CHANGELOG、SECURITY、兼容性、性能记录和文档链接已更新后，维护者手工发布并创建 `0.9.0` tag。发布失败不得覆盖已发布 artifact，应提升 patch 版本重新走完整流程。
