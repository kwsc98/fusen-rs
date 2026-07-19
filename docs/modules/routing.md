# 路由行为

> English summary: routes are validated once and stored in an immutable table.

路由键由 HTTP method 和 path 组成。启动时拒绝完全重复路由及 `/users/{id}`、`/users/{name}` 这类等价动态路由。运行期先匹配静态路径，再按构建后的确定顺序匹配动态路径，不使用锁。

路径参数被合并到请求 query 参数后交给生成服务适配器。非法或空参数名在构建期失败。路由单元测试覆盖冲突和参数提取。
