# 0.9 性能基线

`0.9.0` 使用 direct、single-attempt、无 registry 的真实 loopback socket 矩阵。正式比较只能在带有 `fusen-benchmark-0-9-reference` label 的固定 self-hosted runner 上进行；GitHub hosted runner 只验证 benchmark 和 schema 可执行。

## Release Matrix

每轮依次执行以下场景，全部请求必须成功：

| Binding | Transport | Concurrency | Payload | Release threshold |
| --- | --- | ---: | ---: | --- |
| `http-json-v1` | h2c | 1 / 100 | small / 64 KiB | p50、p99 五轮中位数不得回退超过 10% |
| `http-json-v1` | HTTP/1.1 | 1 / 100 | small / 64 KiB | p50、p99 五轮中位数不得回退超过 10% |

默认每轮每个 small case 测量 10,000 次、每个 64 KiB case 测量 1,000 次，并在每个 case 前预热 500 次。参数会写入 summary 和 baseline；比较时必须完全一致。客户端显式配置一次 attempt，并分别使用 H1 与 h2c transport policy；binding bytes 保持相同。

每个 case 记录：

- `iterations`、`errors`、总 `duration_ns`；
- 双向 echo payload 的 UTF-8 `bytes`，不包含 JSON quoting、协议 envelope、HTTP framing 或 TCP/IP；
- 成功 `qps`，仅记录，不作为 0.9 的阻塞阈值；
- 成功请求 latency 的 nearest-rank `p50_ns` 和 `p99_ns`。

机器行固定为：

```text
benchmark-result case=... binding=... transport=... concurrency=... payload=... payload_bytes=... iterations=... bytes=... errors=... duration_ns=... qps=... p50_ns=... p99_ns=...
```

gate 要求每轮恰好包含 8 个唯一 case，拒绝缺失 case、重复 case、非零 errors、错误字节数、无效 percentile 或不一致 QPS。

case 名固定为 `http1-c{1|100}-{small|64k}` 和 `h2c-c{1|100}-{small|64k}`。输出中的 `binding=http-json-v1` 与 `transport=http1|h2c` 是独立字段，不再用协议名隐式绑定 payload 和 HTTP version。

## Baseline Schema

schema v3 baseline 的 `benchmark_suite` 固定为 `http-json-transport-matrix-v1`，并且必须包含：

- 产生测量的完整 `source_commit`，并能在完整 checkout 中解析为当前候选 `HEAD` 的 ancestor；
- 固定 `host.id`、CPU、OS，以及 toolchain 和完整 `rustc -Vv`；
- warmup、small/64 KiB iterations、concurrency 和 payload bytes；
- 8 个 case 各自连续五轮的原始 metrics 与聚合中位数；
- `required_runs = 5` 和固定为 10% 的 threshold。

比较器拒绝脏工作树、host/CPU/OS/rustc/参数不一致、不是恰好五轮、placeholder SHA、缺少原始样本或聚合值不能由原始样本复算的 baseline。它还要求 B 是 A 的唯一直接子提交、baseline 文件在 A 和 B 中均已提交，且 `A..B` 的差异必须恰好只有该 baseline JSON；没有 baseline 变化、存在额外 commit，或夹带任意源码、测试、workflow、Cargo metadata 或文档变化都会失败。HTTP/1.1 与 h2c 的全部 8 个 case 都是 blocking：p50 或 p99 任一中位数严格超过 baseline 的 110% 时失败，正好 110% 通过。全部 QPS 只记录，不参与判定。

## Two-Phase Calibration

baseline 必须在实现提交之后生成，不能在未提交代码上预填数字：

1. 将代码、测试、工具、workflow、发布文档和实际发布日期提交为干净候选 A，并将固定 `release/v0.9.0-calibration` ref 指向 A。
2. 在固定 runner 上从 `release/v0.9.0-calibration` 对 A 运行五轮 calibration；生成文件中的 `source_commit` 必须等于 A。
3. 从 workflow artifact 取出 `fusen-0.9-reference-macos-arm64.json`，审查五轮 log、机器指纹和 summary 后，只提交该 baseline JSON，形成最终候选 B；将固定 `release/v0.9.0-rc` ref 指向 B。
4. 从 `release/v0.9.0-rc` 对 B 重新运行 `compare`；gate 验证 B 是 A 的唯一直接子提交，且 `A..B` 只改变已提交的 baseline JSON，然后归档成功 artifact。

固定机本地 calibration 命令：

```bash
python3 .github/scripts/run-benchmark-gate.py \
  --host-id fusen-0.9-reference-macos-arm64 \
  --runs 5 \
  --write-baseline target/release-benchmark-gate/manual-calibrate-001/fusen-0.9-reference-macos-arm64.json \
  --output-dir target/release-benchmark-gate/manual-calibrate-001
```

等价的 GitHub Actions 入口是 `Release Benchmark Gate` 的 `calibrate` mode；workflow 会拒绝从固定 `release/v0.9.0-calibration` 以外的 ref 校准。当前 committed baseline 只要仍是 `calibration-required`，compare 就会明确失败，不能发布。

## Comparison

baseline 提交后运行：

```bash
python3 .github/scripts/run-benchmark-gate.py \
  --host-id fusen-0.9-reference-macos-arm64 \
  --runs 5 \
  --baseline .github/benchmarks/fusen-0.9-reference-macos-arm64.json \
  --output-dir target/release-benchmark-gate/manual-compare-001
```

每次命令必须使用全新或空的 output directory。固定 workflow 的 compare mode 只接受 `release/v0.9.0-rc`，并使用 `run_id-run_attempt` 唯一路径、完整 Git 历史、single-child check 和 baseline-only tree diff 验证 A/B provenance；随后上传五轮 log、`summary.json` 和 comparison。普通 CI 使用较短参数运行同一 8-case 矩阵，同时发现并执行 `.github/scripts/test_*.py` 的全部发布工具回归；托管机器结果不能更新 reference baseline。

Retry、breaker、admission、codec 和 interceptor 可以增加独立 microbenchmark，但不能代替这套真实 H1/h2c release gate。Benchmark 代码不得为了测量而暴露私有 transport 或 codec API。
