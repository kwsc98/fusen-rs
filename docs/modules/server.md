# 服务端行为

> English summary: server startup is transactional and all request processing
> is bounded by concurrency, body, and time limits.

## 职责与接口

`FusenServerBuilder` 收集服务、handler 和注册中心，`ServerConfig` 定义监听及资源边界。重复服务和未知 handler 在启动前失败。

## 状态、并发与错误

状态顺序为校验、绑定、注册、服务、摘除、排空。默认 body 上限 2 MiB、并发 1024、请求 timeout 30 秒、排空 timeout 30 秒。监听失败不会注册；部分注册失败逆序回滚。过载返回 503，超时返回 504。

## 扩展与测试

`run_with_shutdown` 用于嵌入式运行和确定性测试。server tests 覆盖绑定顺序与注册回滚。
