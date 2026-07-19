# 0.8 到 0.9 迁移指南

> English summary: 0.9 intentionally breaks endpoint, server, error, and
> registry APIs to establish reliable runtime contracts.

## 客户端

将 `Protocol::Host(url)` 替换为 `ClientOptions::direct(url.parse()?)`；注册发现使用 `ClientOptions::discovery(WireProtocol::Fusen)`。`FusenClientContextBuilder::builder()` 更名为 `build()`，handler 注册现在返回 `Result`。

## 服务端

将 `FusenServerContext::new(port)` 替换为 `FusenServerBuilder::new(SocketAddr)`。启用注册时在 `ServerConfig` 中设置 `advertised_base_url`。`handler` 和 `service` 返回 `Result<Self, FusenError>`。

## 错误与协议

删除 `ErrorMessage`、`Impossible`、`HttpError` 和不安全的任意 error variant。使用语义化 `FusenError`；远端错误为 RFC 9457 Problem Details。`Protocol` 拆分为 `ClientEndpoint` 与 `WireProtocol`。

Dubbo/Triple、`Protocol::Dubbo` 和 Prost codec 已删除。调用方必须使用 JSON HTTP，或继续停留在明确支持 Triple 的旧版本。

## 注册中心

`Register` 接受 `WireProtocol`，错误源要求 `Send + Sync`。`Directory::snapshot/replace` 是首选 API；旧的异步 `get/change` 暂时作为迁移辅助保留。
