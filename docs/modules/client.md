# 客户端行为

> English summary: clients separate endpoint selection from wire protocol and
> enforce connect and request deadlines.

## 职责与接口

`FusenClientContextBuilder` 装配注册中心、handler 和 `ClientConfig`。生成客户端通过 `ClientOptions` 指定 `Direct(Uri)` 或 `Discovery` 以及 `WireProtocol`。

## 状态、并发与错误

客户端和 Hyper 连接池可安全共享；Directory 只暴露不可变快照，更新权限由注册 provider 单独持有。Direct URL 只允许 HTTP(S)，可包含 base path，不允许 query/fragment。默认连接超时 3 秒、完整调用超时 10 秒、发现超时 5 秒、订阅关闭超时 5 秒、响应体上限 2 MiB；配置零值在 `build` 时失败。

完整调用 timeout 覆盖负载均衡、Aspect、DNS/TLS、响应头和响应体。非 2xx Problem Details 还原为 `FusenError::Remote`。发现客户端通过生成的 `close` 显式取消订阅，最后引用释放也会触发后台清理。关闭超时只停止调用方等待，不取消 provider cleanup；关闭开始后客户端拒绝新的 RPC，避免继续使用陈旧快照。

## 扩展与测试

负载均衡和 Aspect 是扩展点；0.9 不自动重试。测试覆盖慢流式响应、Spring HTTP/1.1、path/query 编码和错误还原。
