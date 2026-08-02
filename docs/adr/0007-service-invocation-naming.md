# ADR 0007: Service invocation 公共命名

- 状态：已接受
- 日期：2026-08-02
- 决策者：fusen-rs 维护者

> 本 ADR 只定义 Rust API 命名；HTTP binding 与 wire baseline 由
> [ADR 0009](0009-http-binding-discovery-decoupling.md) 独立定义。

## 背景

Fusen 同时提供服务接口生成、注册发现、路由、负载均衡、韧性、配置与可观测性，定位不只是一个 RPC codec 或 transport。原有 `Rpc*` 类型前缀和 `Middleware` 名称让公共 API 过度绑定 RPC/Web 术语，也弱化了 service invocation 的整体生命周期语义。

## 决策

- 产品定位统一为 Rust 微服务与 service invocation 框架；当前公共文档使用 service interface、service method 和 service invocation。
- 调用 API 去除 `Rpc` 前缀，使用 `Arguments`、`Body`、`Call`、`Context`、`Response`、`Side`、`Error`、`ErrorCategory`、`ErrorOrigin` 与 `ErrorDetails`。
- `Middleware` SPI 改为 `Interceptor`，执行入口为 `intercept`；四个阶段及 global-before-local、短路、重试和取消语义保持不变。
- `v0.9.0` 是首个兼容基线，因此不提供旧类型、模块或 builder 方法的 alias 与 deprecated facade。
- API 命名不自身定义或改变 wire 契约；`http-json-v1` 的语义以 ADR 0009 为准。
- 两个有意的外部可观察名称同步调整：panic code 使用 `interceptor_panic`，OpenTelemetry histogram 使用 `fusen.invocation.attempts`。

“RPC”只用于引用外部标准、历史 API 或专门拒绝旧 `#[rpc(...)]` 语法的诊断和测试，不再描述 Fusen 当前的产品定位或公共 API。

## 后果

应用在进入 0.9 基线前必须一次性迁移 Rust import、trait 实现、builder 调用、错误码匹配以及 dashboard/alert instrument 名称。单独的 wire 迁移影响记录在 ADR 0009。

## 备选方案

- 保留 `Rpc*` alias：拒绝，会在首个兼容基线中永久保留与项目定位不符的双入口。
- 使用 `Filter`：拒绝，容易被理解为只过滤请求；`Interceptor` 更准确表达包围调用、修改上下文、继续下游或短路。
- 同时重命名 wire 协议：拒绝，Rust API 术语变化不应制造无关的协议破坏。
