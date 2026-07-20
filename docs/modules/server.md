# 服务端行为

> English summary: server startup is transactional and all request processing
> is bounded by concurrency, body, and time limits.

## 职责与接口

`FusenServerBuilder` 收集服务、handler 和注册中心，`ServerConfig` 定义监听及资源边界。重复服务和未知 handler 在启动前失败。

## 状态、并发与错误

状态顺序为校验、绑定、限时注册、服务、限时摘除、排空。默认 body 上限 2 MiB、请求并发 1024、连接 2048、HTTP/2 每连接 128 streams、请求 timeout 30 秒、总停机 timeout 30 秒、单次注册操作 5 秒。HTTP/1 header 最多读取 10 秒。

## 扩展与测试

`run_with_shutdown` 用于嵌入式运行和确定性测试。server tests 覆盖绑定顺序、注册回滚、连接排空和挂起注册中心不能延长停机期限。
