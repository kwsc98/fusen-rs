# Security Policy

安全更新覆盖最新发布的 0.9 patch。`0.9.0` 发布前的开发提交不构成受支持版本；发布后的更早 0.x line 只有在维护者明确声明时修复。

请使用 GitHub Security Advisory 私下报告漏洞，不要先创建公开 issue。报告应包含影响版本、复现步骤、攻击前提、潜在影响与建议修复。维护者目标是在 3 个工作日内确认，并在完成风险评估后协调披露。

## Transport Boundary

Client 支持 `http://` 与 `https://` endpoint。HTTPS 使用 Rustls Ring、TLS 1.2/1.3 与 bundled Mozilla WebPKI roots，并验证证书链、有效期和 endpoint hostname。Fusen V1 要求 ALPN `h2`，Spring Cloud V1 使用 HTTP/1.1；证书或 ALPN 验证失败绝不回退到明文。Runtime 不提供跳过验证、自定义 CA、客户端证书或 mTLS API，也不读取系统 trust store；私有 CA 和自签名证书不受支持。

内置 Server 只监听明文 HTTP/1.1 与 h2c，不加载证书或私钥。生产入站 TLS 必须通过可信 ingress、sidecar、反向代理或 service mesh 终止，并保护 Server 到终止器之间的明文网络边界。`https://` advertised endpoint 只声明外部终止器的地址，不会让内置 listener 获得 TLS 能力。

Nacos provider 的控制面安全由 SDK 与部署配置负责。RPC client 的 TLS 栈只允许已审计的 Rustls Ring/bundled-roots 路径；`native-tls`、OpenSSL TLS backend、AWS-LC provider、native/system root loader、跳过验证和明文 fallback 仍被依赖策略禁止。Provider credential 不得泄漏到 contract/runtime 类型。

## Input And Resource Limits

Runtime 在读取 body 前验证 protocol、request ID、deadline、attempt、readiness、route head、headers、Content-Type 与已知 Content-Length。URI、query pairs、headers、body、并发、连接、H2 streams 和全局 byte budgets 都有硬上限；未知 route、not-ready、draining 与已知超限 body 不会被 poll。

默认 admission 与 byte budgets fail fast。可选队列必须有界且受 logical deadline 约束。Panic、timeout 和 cancellation 后 permits 必须归还；永久 pending work 必须在 shutdown deadline 后被取消并有界返回。

## Error And Telemetry Redaction

Problem Details 只能包含安全 message、稳定 code、request ID 与 retry hint。内部 source、panic payload、backtrace、credential 和完整 headers 永不返回。Internal error detail 固定隐藏实现细节，framework emergency response 最大 4 KiB。

Metrics label 禁止 request ID、endpoint、错误文本、body、完整 headers 与凭据；这些字段只有在必要、脱敏且受采样控制时才能进入 trace。Config password/secret 的 Debug、error、trace 和 metrics 输出必须脱敏。应用负责安全配置 tracing/OTel exporter endpoint 并持有其 shutdown guard。

请勿在漏洞报告中粘贴真实 Nacos 凭据、认证 header、完整生产 trace、用户 body 或内部 endpoint。使用最小化、可撤销的测试数据。
