# ADR 0004: Tracing 与 MetricsRecorder

- 状态：已接受
- 日期：2026-07-26
- 决策者：fusen-rs 维护者

## 背景

库直接安装全局 subscriber/exporter 会夺取应用级策略。把 trace 与 metric 塞进同一个 observer 也无法分别约束敏感字段和 label cardinality，并让取消时的终态报告依赖用户 callback 正确性。

## 决策

- Core 直接产生结构化 `tracing` span/event，但从不安装进程级 subscriber；
- 唯一公开 metrics 扩展为同步、非阻塞、无失败返回值的 `MetricsRecorder`；
- `fusen-observability` 的 base feature 只包含 backend-neutral event/SPI，可选 tracing/OTel adapter 由应用初始化并持有 flush/shutdown guard；
- recorder callback 分别被 panic boundary 包围，首次 panic 后原子禁用该 recorder，service invocation 与生命周期继续运行；
- metrics label 只允许 side、binding、HTTP version、service、method、outcome、status、failure class 等有界值；
- request ID、endpoint、错误文本、body、完整 headers 和凭据禁止成为 metric label；其中必要字段只可进入脱敏 trace；
- success/error/timeout/cancellation 由 runtime-owned RAII guard 形成唯一终态，不依赖用户后置代码执行。

Recorder 不得阻塞、递归进入 runtime 或执行异步 I/O。批处理、背压和 exporter 失败由 adapter/backend 负责。

## 后果

应用可以自由选择 tracing subscriber 和 metrics backend，Core 不固定 OTel runtime 或全局状态。Metrics callback panic 不会击穿请求，但 recorder 会被永久禁用并产生结构化诊断事件。

## 备选方案

- 通用 invocation observer：拒绝，字段安全、panic 和 cardinality 契约不清晰。
- Core 直接依赖 OTel backend：拒绝，会固定 exporter、runtime 和版本节奏。
- 公开 trace sink：拒绝，`tracing` 已提供成熟的应用侧组合边界。
