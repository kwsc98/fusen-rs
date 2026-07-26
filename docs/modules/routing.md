# 路由行为

> English summary: server routes are immutable and client routing runs from a
> fresh directory snapshot for every physical attempt.

## Server Route

服务端启动时把每条 route 预绑定到静态 method descriptor、Middleware slice 与 service dispatch。Fusen V1 使用固定 `/_fusen/v1/{service}/{method}` identity；Spring Cloud V1 使用宏属性声明的 method/path。

Spring routes 按 HTTP method 存入不可变 trie，启动时拒绝完全重复以及 `/users/{id}` 与 `/users/{name}` 这类等价动态路径。每一段优先静态分支，再匹配参数分支，结果不依赖 service 插入顺序。路径逐 segment 严格百分号解码，编码后的 `/` 保持单一参数值；非法编码返回 400。

Route head 在 admission 和 body 读取前解析。未知 route 不 poll body。URI 最大 8 KiB，query 最多 128 pairs，headers 总计最大 32 KiB。

## Client Route And Selection

每个 physical attempt 都读取最新 `DirectorySnapshot`。多个 `Router` 按注册顺序执行，随后过滤 open endpoint breaker，再由 `LoadBalancer` 返回一个合法 index。默认 `WeightedRandom` 使用经过校验的正权重。

Router/LB panic 被隔离为当前逻辑调用的内部错误。空快照、Router 清空结果或非法 index fail fast，且不进入 breaker 失败统计。一次调用只要存在未尝试 endpoint，就不会再次选择先前失败的 endpoint。
