# 开发期性能基线

性能验证分为进程内微基准和端到端 HTTP 压测。共享 CI 不设置耗时硬门槛；同机、同工具链、同负载比较时，重构后的 median QPS 或吞吐不得比重构前回退超过 5%。

## 微基准

运行：

```bash
cargo bench --locked -p fusen-rs --bench invocation
```

基准覆盖 0/1/4/8 个空 middleware、0/1 个 Observer 回调、客户端声明序号分派，以及 1 KiB/64 KiB JSON request encode/decode。结果报告 `ns/op` 和 `ops/s`；正式对比应固定 CPU 电源策略、关闭逐请求日志，并至少重复 5 次关注中位数。

## HTTP 压测

先启动无日志服务端，再运行客户端矩阵：

```bash
cargo run --release -p examples --bin host-server-pt
PT_PROTOCOL=both PT_CONCURRENCY=1,100 PT_ROUNDS=5 \
PT_REQUESTS_PER_TASK=10000 \
cargo run --release -p examples --bin host-client-pt
```

客户端逐轮报告成功/失败、QPS 和 JSON body 吞吐，并按协议与并发度输出中位数。HTTP/1.1 与 HTTP/2 顺序执行；字节统计不包含 HTTP framing、TCP/IP 或 TLS。比较前后版本时必须使用相同 Rust 版本、release profile、机器、服务地址和环境变量。

## 当前开发节点记录

2026-07-24 在 arm64 macOS 26.5、Rust/Cargo 1.97.0 上完成本轮质量改造后连续运行 5 次，以下为 `ns/op` 中位数。chain 数值包含每次构造 `RpcContext`；Observer 数值只测同步回调 fan-out。变化比例相对同机、同工具链和同负载的上一份开发节点记录计算；`ns/op` 为负值表示更快。

| Case | Current median | Previous median | Change |
| --- | ---: | ---: | ---: |
| chain: 0 middleware | 126.76 ns | 161.05 ns | -21.3% |
| chain: 1 middleware | 199.29 ns | 204.81 ns | -2.7% |
| chain: 4 middleware | 392.37 ns | 394.80 ns | -0.6% |
| chain: 8 middleware | 624.54 ns | 632.66 ns | -1.3% |
| observer fan-out: 0 | 0.33 ns | 0.33 ns | 0.0% |
| observer fan-out: 1 | 2.24 ns | 2.27 ns | -1.3% |
| client dispatch: `MethodId` index | 0.96 ns | 9.72 ns | -90.1% |
| codec encode: 1 KiB | 1133.66 ns | 1133.15 ns | +0.0% |
| codec decode: 1 KiB | 727.68 ns | 722.09 ns | +0.8% |
| codec encode: 64 KiB | 24246.54 ns | 24757.42 ns | -2.1% |
| codec decode: 64 KiB | 11873.54 ns | 11915.92 ns | -0.4% |

端到端矩阵使用 release server/client、每任务 10,000 次 RPC、每组 5 轮，所有请求均成功：

| Protocol | Concurrency | Median QPS | Previous QPS | QPS change | JSON throughput |
| --- | ---: | ---: | ---: | ---: | ---: |
| HTTP/1.1 | 1 | 22,496.98 | 22,498.34 | +0.0% | 0.99 MiB/s |
| HTTP/1.1 | 100 | 115,354.61 | 116,195.83 | -0.7% | 5.06 MiB/s |
| HTTP/2 | 1 | 13,590.27 | 13,789.70 | -1.4% | 0.60 MiB/s |
| HTTP/2 | 100 | 34,136.38 | 34,699.06 | -1.6% | 1.50 MiB/s |

本轮所有中位数变化均未超过 5% 回退门槛，端到端请求全部成功。这些数字是当前开发节点的基线，不代表 HTTP/1.1 与 HTTP/2 的普遍性能关系。后续比较必须记录前后 commit，并复用相同机器、工具链、协议顺序与环境变量；没有可执行的旧构建时不能声称满足或违反 5% 回退门槛。
