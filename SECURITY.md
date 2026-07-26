# Security Policy

安全更新覆盖最新发布的 0.9 patch。`0.9.0` 发布前的开发提交不构成受支持版本；发布后的更早 0.x line 只有在维护者明确声明时修复。

请使用 GitHub Security Advisory 私下报告漏洞，不要先创建公开 issue。报告应包含影响版本、复现步骤、攻击前提、潜在影响与建议修复。维护者目标是在 3 个工作日内确认，并在完成风险评估后协调披露。

## Transport Boundary

Core 只支持明文 `http://` 与 h2c。它不会提供传输加密，也不会加载证书。生产部署必须通过可信 ingress、sidecar、反向代理或 service mesh 终止 TLS，并保护 Core 到代理之间的网络边界。`https://` endpoint 会在网络 I/O 前被拒绝；禁止把明文 fallback 当作恢复策略。

Nacos provider 的控制面安全由 SDK 与部署配置负责。RPC Core 不得因此引入 native TLS/OpenSSL 依赖，provider credential 也不得泄漏到 contract/runtime 类型。

## Input And Resource Limits

Runtime 在读取 body 前验证 protocol、request ID、deadline、attempt、readiness、route head、headers、Content-Type 与已知 Content-Length。URI、query pairs、headers、body、并发、连接、H2 streams 和全局 byte budgets 都有硬上限；未知 route、not-ready、draining 与已知超限 body 不会被 poll。

默认 admission 与 byte budgets fail fast。可选队列必须有界且受 logical deadline 约束。Panic、timeout 和 cancellation 后 permits 必须归还；永久 pending work 必须在 shutdown deadline 后被取消并有界返回。

## Error And Telemetry Redaction

Problem Details 只能包含安全 message、稳定 code、request ID 与 retry hint。内部 source、panic payload、backtrace、credential 和完整 headers 永不返回。Internal error detail 固定隐藏实现细节，framework emergency response 最大 4 KiB。

Metrics label 禁止 request ID、endpoint、错误文本、body、完整 headers 与凭据；这些字段只有在必要、脱敏且受采样控制时才能进入 trace。Config password/secret 的 Debug、error、trace 和 metrics 输出必须脱敏。应用负责安全配置 tracing/OTel exporter endpoint 并持有其 shutdown guard。

请勿在漏洞报告中粘贴真实 Nacos 凭据、认证 header、完整生产 trace、用户 body 或内部 endpoint。使用最小化、可撤销的测试数据。
