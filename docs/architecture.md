# 架构与调用链

> English summary: fusen-rs separates service addressing, wire protocol,
> transport, routing, middleware, discovery, and generated service adapters.

## 边界

workspace 分为核心 RPC、注册契约、内部共享类型、通用基础设施和两组过程宏。`fusen-common` 管理 Nacos、配置与日志，但不反向依赖核心 RPC；`fusen-register` 只定义注册发现契约。

## 客户端数据流

生成客户端按 `ParameterInfo` 将参数分到 path/query/body，`ClientEndpoint` 决定 Direct 或 Discovery 寻址，负载均衡器选择快照中的实例，Aspect 链处理上下文。完整调用 timeout 包围负载均衡、中间件、transport 和响应体。非 2xx 响应解析为 `ProblemDetails`。

## 服务端数据流

监听器先绑定，再限时注册服务。连接和请求分别取得许可，Router 执行请求 deadline、所有媒体类型的有界解码、静态优先 trie 路由、Aspect 和服务调用。所有失败统一转换为 RFC 9457 响应，`x-request-id` 与上下文、日志保持一致。

## 生命周期

启动失败会限时回滚已经注册的实例。停机先停止 accept，再在同一个绝对 deadline 内从注册中心摘除并排空 Hyper 连接。发现与配置订阅支持显式 close 和 Drop 清理。高级重试、熔断和主动健康检查不属于 0.9。

关键测试位于路由、codec、Directory 和 server 模块的 `tests` 子模块。
