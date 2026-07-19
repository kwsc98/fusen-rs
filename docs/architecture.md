# 架构与调用链

> English summary: fusen-rs separates service addressing, wire protocol,
> transport, routing, middleware, discovery, and generated service adapters.

## 边界

workspace 分为核心 RPC、注册契约、内部共享类型、通用基础设施和两组过程宏。`fusen-common` 管理 Nacos、配置与日志，但不反向依赖核心 RPC；`fusen-register` 只定义注册发现契约。

## 客户端数据流

生成客户端将参数序列化为 JSON，`ClientEndpoint` 决定 Direct 或 Discovery 寻址，负载均衡器选择快照中的实例，Aspect 链处理上下文，HTTP transport 应用连接/请求超时并解码响应。非 2xx 响应解析为 `ProblemDetails`，不会作为成功值返回。

## 服务端数据流

监听器先绑定，再注册服务。Router 在读取 body 前取得并发许可，执行请求 deadline、有界解码、确定性路由、Aspect 和服务调用，最后编码 JSON。所有失败统一转换为 RFC 9457 响应并携带 `request_id`。

## 生命周期

启动失败会回滚已经注册的实例。停机先停止 accept，再从注册中心摘除，然后通知 Hyper 连接优雅结束；超出期限才取消任务。高级重试、熔断和主动健康检查不属于 0.9。

关键测试位于路由、codec、Directory 和 server 模块的 `tests` 子模块。
