# ADR 0003: Core 只提供明文 HTTP

- 状态：已接受
- 日期：2026-07-26
- 决策者：fusen-rs 维护者

## 背景

进程内 TLS 会把证书加载、轮换、SNI、平台 crypto backend 和供应链升级耦合到 RPC transport 与生命周期。该项目的部署目标已经具备 ingress、sidecar、反向代理或 service mesh，可以在独立边界终止 TLS。

## 决策

`fusen-rs` Core 只实现明文 HTTP/1.1 与 HTTP/2 prior knowledge（h2c）：

- 删除 `hyper-tls`、native TLS 与 OpenSSL 依赖；
- `ServiceEndpoint` 只接受 canonical absolute `http://` URL；
- `https://` 及其他 scheme 在任何 DNS、connect 或 socket I/O 前返回 validation/connect error，绝不静默降级；
- 服务端不加载证书或私钥；生产 TLS 在进程外终止；
- Transport、Codec、Acceptor、连接池和 socket 状态全部私有，不提供用户替换 SPI；
- Registry 只能向 Core runtime 提供可直接调用的明文 RPC endpoint。

Nacos provider SDK 的控制面连接不属于 Core RPC transport；其安全配置由 adapter 和部署环境负责，但相关依赖不能泄漏进 `fusen-rs`。

## 后果

Core 的跨平台构建、依赖审计和连接语义显著收敛。需要进程内 TLS 或自定义 socket 的应用必须在 fusen-rs 外建立代理边界；0.9 不承诺 transport 插件机制。

## 备选方案

- 默认 native TLS：拒绝，扩大平台和安全维护边界。
- 同时维护 native-tls/rustls features：拒绝，仍使 Core 拥有证书和握手语义。
- 公开 Transport SPI：拒绝，会暴露 retry、pool、deadline 与 cancellation 的内部契约。
