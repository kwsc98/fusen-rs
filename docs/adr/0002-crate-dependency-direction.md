# ADR 0002: Crate 职责与依赖方向

- 状态：已接受
- 日期：2026-07-26
- 决策者：fusen-rs 维护者

## 背景

稳定契约、运行时、provider adapter 和可观测性 backend 必须保持单向依赖，否则具体 SDK 或进程级设施会渗入核心 RPC 生命周期，并形成难以独立测试和发布的循环。

## 决策

不新增生产 crate。依赖层固定为：

```text
L0  fusen-procedural-macro  fusen-config  fusen-observability
L1  fusen-contract
L2  fusen-register
L3  fusen-rs  fusen-nacos
L4  examples and test consumers
```

规则如下：

- `fusen-procedural-macro` 私有持有属性解析、`SensitiveFields` derive 和接口代码生成实现；生成代码引用 contract/runtime ABI，但过程宏 crate 自身不依赖它们；
- `fusen-config` 与 `fusen-observability` 当前都不依赖其他 workspace crate；前者拥有静态解析、last-good typed hot config 和取消安全 lifecycle，后者拥有 backend-neutral `MetricsRecorder` SPI 与可选 telemetry adapter；
- `fusen-contract` 只拥有 wire、service、registry 和进程内 sensitivity schema 共用的稳定值对象，不拥有 executor、provider 或 backend；其可选 `derive` feature 只重导出 L0 的 `SensitiveFields` derive，因此发布时必须在过程宏之后；
- `fusen-register` 依赖 contract，拥有 registry、registration/subscription lifecycle 和 Directory SPI；
- `fusen-rs` 只依赖 contract、register、observability SPI 和过程宏；
- `fusen-nacos` 依赖 register、config 和 contract，核心 crate 永不反向依赖 Nacos；
- Core 自行产生 `tracing` span/event；不再发布单独的 macro-support crate；
- tracing subscriber、OpenTelemetry backend、Nacos SDK 和其他 provider 依赖不得通过公共类型泄漏到下层 crate。

workspace 使用 Cargo resolver 3 和统一 lint policy。跨 crate 类型只有一个 canonical 定义路径；便捷 re-export 不应制造第二个公共契约入口。

## 后果

底层 crate 可以独立执行最小 feature 检查，provider 和 backend 能在不修改 core 的情况下替换。依赖树检查将成为 CI 和发布门禁的一部分。

## 备选方案

- 将所有 SPI 放回 `fusen-rs`：拒绝，因为 adapter 会为实现一个小接口而依赖完整 runtime。
- 为每个 backend 新建 workspace crate：当前拒绝；只有真实出现多个独立实现且职责无法由 feature 隔离时再立 ADR。
