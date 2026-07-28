# fusen-rs Roadmap

> 最后审阅：2026-07-28
>
> 最后外部检查候选：`9a33478`，尚未通过全部发布门禁，也未形成兼容性 tag。

## Direction

当前唯一里程碑是发布可信的 `0.9.0` 首个兼容性基线。优先级固定为：
CI/正确性/安全 > 契约覆盖 > 发布证据 > 性能演进 > 新功能。

维护者已将 Client HTTPS 明确纳入 0.9 基线；该迭代只扩展客户端出站传输，
不把 Server TLS、证书生命周期或 Transport SPI 带入范围。

本文件只记录方向和迭代顺序；具体行为以架构、模块契约、ADR、
[兼容策略](compatibility.md)和[发布流程](releasing.md)为准。
状态使用 `DONE`、`CANDIDATE`、`NOW`、`NEXT`、`MANUAL`，任何时刻只能有一个
`NOW`。`DONE` 表示该独立契约已在记录的候选 SHA 上通过；最终发布仍要在
M0.11 的冻结 SHA 上重跑全部门禁。

## Current Snapshot

- 所有发布 crate 已使用 `0.9.0`，但 CHANGELOG 仍为 Unreleased，仓库没有
  `v0.9.0` tag。
- [`9a33478` 的 CI](https://github.com/kwsc98/fusen-rs/actions/runs/30269435114) 中，format、
  MSRV clippy/test/doc、stable Linux/macOS、全部 feature matrix、release-contracts、
  lifecycle repeat 和真实 Nacos 已通过；stable Windows 的 workspace tests 失败。
- 同一 CI run 的 security 失败于 renamed-runtime 测试 crate 缺少 license 且
  path dependency 没有显式 version；后续 `cargo audit` 被跳过。
- [`9a33478` 的 Nightly](https://github.com/kwsc98/fusen-rs/actions/runs/30300364088) 中，
  100 轮真实 socket E2E 已通过；三个 fuzz job 均在 harness 步骤失败，且没有
  crash/timeout artifact。
- release-contracts 已证明 dependency policy、public API denylist、Markdown、fuzz-support
  clippy、renamed-runtime、benchmark smoke 和 package archive consumer 可执行。
- M0.HTTPS 已实现 Client HTTP/HTTPS、Rustls Ring、bundled roots、TLS 1.2/1.3、
  严格证书/hostname/ALPN 验证与无明文降级；Server listener 保持明文。
- M0.3 的真实 Nacos registration/discovery/config/cleanup 已在 `9a33478` 通过。
- 已确认 Spring route 实现没有完整满足“逐段静态优先且不依赖插入顺序”的契约。
- 已确认 Spring repeated query 的 `Vec<String>` 在 0/1 个值时不能稳定 roundtrip。
- [性能规范](performance-baseline.md)与当前单场景 benchmark/baseline 覆盖范围不一致。
- Release Benchmark Gate 尚无历史 run；最终门禁固定覆盖 Fusen 并发
  `1/100` × small/64 KiB，Spring 矩阵作为非阻塞 artifact。

## Milestone M0: Publish 0.9.0

完成标准：同一干净候选 SHA 的 CI、nightly、Nacos、package、security、
性能门禁全部通过并留存证据，随后发布 crates 并创建唯一兼容性 tag。

| ID | 状态 | 独立迭代 | 验收标准 |
| --- | --- | --- | --- |
| M0.1 | NOW | 恢复 Windows workspace tests | 定位具体失败，不跳过或放宽断言；同一 SHA 的 MSRV 与 stable 三平台全绿 |
| M0.HTTPS | CANDIDATE | 增加 Client 出站 HTTPS，保持 Server 明文 | Direct/Nacos HTTP 与 HTTPS 均可用；Fusen 为 h2c 或 TLS ALPN h2，Spring 为 H1；Rustls Ring、bundled roots、TLS 1.2/1.3、证书/hostname 拒绝及无降级有测试；HTTPS advertisement 只代表外部终止器 |
| M0.2 | NEXT | 修复 cargo-deny security gate | security job 通过；不使用无依据的 advisory/license 忽略 |
| M0.10a | NEXT | 补齐三 workspace 安全审计 | root/fuzz-support/fuzz 的 advisories/bans/licenses/sources/audit 均非空执行并阻断失败 |
| M0.FUZZ | NEXT | 恢复 Nightly fuzz | 三个 target 各运行 300 秒；真实 crash/timeout 先固化为确定性回归；100 轮 E2E 保持通过 |
| M0.3 | DONE | 锁定真实 Nacos lifecycle gate | registration/discovery/config 与失败后 cleanup 全部通过 |
| M0.4 | NEXT | 修复 Spring route 逐段静态优先 | 正反插入顺序都选择更具体路由，并新增确定性回归测试 |
| M0.5 | NEXT | 锁定 repeated-query wire 契约 | 显式 Scalar/Repeated cardinality；`Vec<String>` 的 0/1/N query key 具有 client/server golden roundtrip |
| M0.6 | NEXT | 锁定 retry 外 Middleware exactly-once | 双 attempt 下 global/local middleware 均只执行一次 |
| M0.7 | NEXT | 锁定公开配置默认值 | Client/Server 默认值和关键预算关系有表驱动测试 |
| M0.8 | NEXT | 锁定 retry 时最新 DirectorySnapshot | 两次 attempt 间更新 snapshot，第二次使用新 revision |
| M0.9 | NEXT | 校准性能发布契约 | Fusen 并发 1/100 × small/64 KiB 为 blocking matrix；baseline 记录不可变 SHA、机器信息和五轮原始结果 |
| M0.10b | NEXT | 冻结发布 runbook 与文档 | 发布顺序、registry 传播等待、部分发布失败/yank 恢复与 `v0.9.0` tag 命名明确 |
| M0.11 | NEXT | 生成最终候选证据 | 同一 SHA 的 CI、nightly fuzz/E2E、Nacos、package consumer、固定机 benchmark 全绿 |
| M0.12 | MANUAL | 发布并建立兼容基线 | crates 按依赖顺序可见；GitHub Release 与 `v0.9.0` tag 指向已验证 SHA |

## Recommended Next Iteration

1. 取得 `stable-windows-latest` 的具体失败日志；如果无法从 API 读取，为 Windows
   test step 增加失败日志 artifact。
2. 只修复跨平台根因，不跳过测试、放宽断言或混入 security/fuzz 改动。
3. 让同一新 SHA 的 MSRV、stable Linux/macOS/Windows 和 HTTPS/H1/h2 契约全绿，
   然后将 M0.1 与 M0.HTTPS 标记 `DONE`。

## Milestone M1: Maintain 0.9.x

- 基于 0.9 tag 接入 `cargo-semver-checks`，保护 Rust API、宏和 wire v1。
- patch 版本只接受兼容修复、安全修复、fuzz 回归和文档校正。
- 移除 package consumer 对 `0.9.0` 的硬编码，并加强 Actions/container 固定。
- 清理已被 0.9 实现取代的旧 issues；新需求必须重新按当前架构评估。

## Candidate, Not Committed

- 为 Spring Cloud V1、64 KiB payload 和高并发建立可重复性能基线。
- 为 retry、breaker、admission、codec、middleware 增加独立 microbench。
- 只有真实用户需求和 ADR 证明现有边界不足时，才考虑新 provider、SPI 或 wire 版本。

## Not Planned

- 完整 Spring MVC 兼容。
- Server 内 TLS、mTLS/自定义 CA、HTTP/3 或可替换 Transport/Codec SPI。
- 为未发布历史 API 增加兼容 facade 或旧 wire decoder。
- 在 0.9.0 基线形成前继续扩大功能面。

## Maintenance Rules

- 每轮只推进表中的一个 ID，并保持仓库绿色。
- 完成项必须附测试、CI run 或 release artifact 证据，不能凭代码存在标记完成。
- 新发现的问题先记录证据、风险和验收标准，再决定是否调整顺序。
- 每轮结束更新“最后外部检查 SHA”、`NOW` 项和完成状态，删除失效假设。
- 最终候选证据记录在 workflow summary 或 GitHub Release draft，不再为回填证据修改已冻结 SHA。
