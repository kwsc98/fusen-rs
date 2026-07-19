# 客户端行为

> English summary: clients separate endpoint selection from wire protocol and
> enforce connect and request deadlines.

## 职责与接口

`FusenClientContextBuilder` 装配注册中心、handler 和 `ClientConfig`。生成客户端通过 `ClientOptions` 指定 `Direct(Uri)` 或 `Discovery` 以及 `WireProtocol`。

## 状态、并发与错误

客户端和 Hyper 连接池可安全共享；Directory 读取不可变快照。Direct URI 必须包含 scheme 和 authority。默认连接超时 3 秒、请求超时 10 秒、响应体上限 2 MiB。无实例返回 503 语义，deadline 返回 504 语义，非 2xx Problem Details 还原为 `FusenError::Remote`。

## 扩展与测试

负载均衡和 Aspect 是扩展点；0.9 不自动重试。客户端错误还原由 codec/error 单元测试和 examples 验证。
