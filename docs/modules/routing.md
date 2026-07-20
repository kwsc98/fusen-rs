# 路由行为

> English summary: routes are validated once and stored in an immutable table.

路由按 HTTP method 存入不可变 trie。启动时拒绝完全重复路由及 `/users/{id}`、`/users/{name}` 这类等价动态路由。每一段都优先匹配静态分支，再匹配参数分支，因此结果不依赖 service HashMap 或插入顺序。

路径参数与 query 分开保存。非法、空、重复、没有对应方法参数的模板字段在宏展开或路由构建期失败。请求路径先按 `/` 分段，再对每段严格执行一次百分号解码；因此 `%2F` 保持在单个参数值内，非法百分号编码返回 400，静态和动态分支都使用同一份解码文本匹配。
