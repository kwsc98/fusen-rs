# fusen-rs Roadmap

> 最后审阅：2026-07-27
>
> 当前远端基线：`fe2d43f`，状态为 0.9.0 发布候选，尚未形成兼容性 tag。

## Direction

当前唯一里程碑是发布可信的 `0.9.0` 首个兼容性基线。优先级固定为：
CI/正确性/安全 > 契约覆盖 > 发布证据 > 性能演进 > 新功能。

本文件只记录方向和迭代顺序；具体行为以架构、模块契约、ADR、
[兼容策略](compatibility.md)和[发布流程](releasing.md)为准。
状态使用 `NOW`、`NEXT`、`MANUAL`、`CANDIDATE`，任何时刻只能有一个 `NOW`。

## Current Snapshot

- 所有发布 crate 已使用 `0.9.0`，但 CHANGELOG 仍为 Unreleased，仓库没有 0.9 tag。
- [`main` 当前 CI](https://github.com/kwsc98/fusen-rs/actions/runs/30238267440)失败：
  MSRV、stable 三平台和 release-contracts 同源于 renamed-runtime 的环境相关诊断快照；
  security 独立失败于未声明许可证的私有测试 crate。
- 同一 CI run 的真实 Nacos lifecycle 已通过，后续进入 M0.3 时先复核是否仍需改造。
- M0.1 本地候选已消除对 `rust-src` 的诊断依赖，并通过目标包、全新 target、
  workspace、clippy 和格式检查，等待推送后的跨平台 CI 验证。
- 格式、feature matrix 的部分任务及 lifecycle repeat 已通过。
- 已确认 Spring route 实现没有完整满足“逐段静态优先且不依赖插入顺序”的契约。
- [性能规范](performance-baseline.md)与当前单场景 benchmark/baseline 覆盖范围不一致。

## Milestone M0: Publish 0.9.0

完成标准：同一干净候选 SHA 的 CI、nightly、Nacos、package、security、
性能门禁全部通过并留存证据，随后发布 crates 并创建唯一兼容性 tag。

| ID | 状态 | 独立迭代 | 验收标准 |
| --- | --- | --- | --- |
| M0.1 | NOW | 修复 renamed-runtime/workspace test 故障簇 | MSRV package/workspace tests 通过，stable 三平台恢复；不混入 deny 或 Nacos 修复 |
| M0.2 | NEXT | 修复 cargo-deny security gate | security job 通过；不使用无依据的 advisory/license 忽略 |
| M0.3 | NEXT | 修复真实 Nacos lifecycle gate | registration/discovery/config 与失败后 cleanup 全部通过 |
| M0.4 | NEXT | 修复 Spring route 逐段静态优先 | 正反插入顺序都选择更具体路由，并新增确定性回归测试 |
| M0.5 | NEXT | 锁定 repeated-query wire 契约 | `Vec<String>` 重复 query key 具有 client/server golden roundtrip |
| M0.6 | NEXT | 锁定 retry 外 Middleware exactly-once | 双 attempt 下 global/local middleware 均只执行一次 |
| M0.7 | NEXT | 锁定公开配置默认值 | Client/Server 默认值和关键预算关系有表驱动测试 |
| M0.8 | NEXT | 锁定 retry 时最新 DirectorySnapshot | 两次 attempt 间更新 snapshot，第二次使用新 revision |
| M0.9 | NEXT | 校准性能发布契约 | blocking matrix 与实现一致；baseline 记录不可变 SHA、机器信息和五轮原始结果 |
| M0.10 | NEXT | 补齐三 workspace 安全审计与发布 runbook | root/fuzz-support/fuzz lockfile 均审计；feature matrix、发布顺序、失败恢复与 tag 命名明确 |
| M0.11 | NEXT | 生成最终候选证据 | 同一 SHA 的 CI、nightly fuzz/E2E、Nacos、package consumer、固定机 benchmark 全绿 |
| M0.12 | MANUAL | 发布并建立兼容基线 | crates 按依赖顺序可见；GitHub Release 与 tag 指向已验证 SHA |

## Recommended Next Iteration

1. 推送 M0.1 的稳定 fixture 候选，并触发同一 SHA 的 CI。
2. 确认 renamed-runtime、MSRV workspace、stable 三平台和 release-contracts 全部通过。
3. 通过后将 M0.1 标记完成，把 M0.2 设为唯一 `NOW`。
4. 若仍有失败，只处理 M0.1 的同源根因，不扩大到 security/Nacos。

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
- Core 内 TLS、HTTP/3 或可替换 Transport/Codec SPI。
- 为未发布历史 API 增加兼容 facade 或旧 wire decoder。
- 在 0.9.0 基线形成前继续扩大功能面。

## Maintenance Rules

- 每轮只推进表中的一个 ID，并保持仓库绿色。
- 完成项必须附测试、CI run 或 release artifact 证据，不能凭代码存在标记完成。
- 新发现的问题先记录证据、风险和验收标准，再决定是否调整顺序。
- 每轮结束更新当前 SHA、`NOW` 项和完成状态，删除失效假设。
