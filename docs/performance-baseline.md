# 0.9 性能基线

0.9 clean-slate 不把重构前的内部微基准当作发布基线。首个 baseline 由 `0.9.0` release candidate 的 direct、single-attempt、无日志真实 socket 场景建立，并与最终 tag 一起归档。

## Release Gate

同一机器、Rust 1.97、相同 release profile、相同 CPU 电源策略和相同 payload 下，对候选提交至少重复 5 轮，分别记录 p50/p99 latency、成功 QPS、错误数和应用层 JSON bytes。相对当前 0.9 baseline，direct single-attempt benchmark 的 p50 或 p99 不得回退超过 10%。参考机器由稳定 host id 与 self-hosted runner label 绑定；GitHub hosted runner 的结果只做 smoke artifact，禁止跨机器套用绝对延迟 baseline。

比较必须满足：

- Direct endpoint，单 logical invocation，关闭 retry 与外部 registry；
- 分别测试并发 1 与 100，至少覆盖小 payload 和 64 KiB payload；
- 服务端与客户端固定在同一组 CPU/网络条件，关闭逐请求日志与 exporter；
- 所有请求成功，permit、连接与 task 数量在测试后回到稳定值；
- 报告 before/after commit、操作系统、CPU、Rust 版本、命令、原始结果和中位数；
- 没有可执行 baseline 时不得声称通过回退门槛，应先建立并归档 baseline。

## E2E 命令

主 release benchmark 使用真实 loopback h2c socket，并直接报告 mean/p50/p99。正式 gate 运行五轮、保存逐轮 log 与 summary，并校验 committed baseline：

```bash
python3 .github/scripts/run-benchmark-gate.py \
  --host-id fusen-0.9-reference-macos-arm64 \
  --runs 5 \
  --baseline .github/benchmarks/fusen-0.9-reference-macos-arm64.json \
  --output-dir target/release-benchmark-gate
```

只查看单轮原始输出时仍可直接运行 `cargo +1.97.0 bench --locked -p fusen-rs --bench invocation`。机器可读行固定为 `direct/fusen-v1 iterations=... mean_ns=... p50_ns=... p99_ns=...`，gate 要求每轮恰好一行且所有数值为正。

双协议吞吐与成功率矩阵使用 examples runner：

```bash
cargo +1.97.0 run --release -p examples --bin host-server-pt

PT_PROTOCOL=both \
PT_CONCURRENCY=1,100 \
PT_ROUNDS=5 \
PT_REQUESTS_PER_TASK=10000 \
cargo +1.97.0 run --release -p examples --bin host-client-pt
```

`PT_PROTOCOL=h2` 测试 `FusenV1`，`PT_PROTOCOL=h1` 测试 `SpringCloudV1`。统计的 JSON bytes 不包含 HTTP framing、HPACK 或 TCP/IP；需要线速数据时使用独立 socket/packet instrumentation，不修改稳定 wire。

## Baseline Record

0.9 reference baseline 的机器可读记录保存在 [`.github/benchmarks/fusen-0.9-reference-macos-arm64.json`](../.github/benchmarks/fusen-0.9-reference-macos-arm64.json)。数据来自同一 macOS 26.5 arm64 参考机器、Rust 1.97.0、release profile 的连续五轮真实 loopback h2c 测量；baseline 文件与产生它的 0.9 clean-slate commit 一起冻结。

| Commit | Platform | Protocol | Payload | Concurrency | p50 | p99 | QPS | Errors |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline file commit | macOS 26.5 arm64 | Fusen V1 | small | 1 | 59,583 ns | 73,667 ns | 16,714.97 | 0 |

五轮 p50 为 `60,417 / 59,583 / 59,584 / 59,167 / 59,375 ns`，p99 为 `75,000 / 75,167 / 73,167 / 73,667 / 73,083 ns`。发布比较使用各列中位数，单轮 outlier 不直接决定 gate；任何一列中位数超过 baseline 的 110% 仍会失败。

Spring Cloud V1 的吞吐/成功率矩阵与大 payload 数据作为同一 release artifact 归档；扩展为稳定 latency gate 前，必须先为该 case 建立可重复 baseline。

Retry、breaker、admission、codec 和 middleware 可以增加独立 microbench，但不能代替真实 H1/H2 release gate。Benchmark 代码不得暴露私有 transport/codec API 只为方便测量。
