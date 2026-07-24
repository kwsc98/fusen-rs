# Middleware 与宏行为

## Middleware

用户直接实现统一的 `Middleware` trait，并可使用 `async fn handle`；runtime 的 blanket adapter 在内部完成对象擦除，不要求宏或 `BoxFuture`。

`RpcContext` 只表示请求，提供 request ID、静态 service/method、`MethodId`、deadline/remaining、headers、metadata 和类型化 extensions。成功返回 `RpcResponse`，错误返回 `FusenError`。Middleware 可以短路，也可以调用消费型 `Next::run`；`Next` 不可克隆，下游和框架私有 terminal 最多执行一次。

客户端 Middleware 位于 Cluster 之前；服务端 Middleware 位于 route 之后。全局配置先进入，服务局部配置后进入。完整日志、指标、timeout 和 cancellation 应由 `InvocationObserver` 观察。

## fusen_trait

`fusen_trait` 是 id/group/version/path/method/参数来源的唯一元数据来源。它生成：

- 唯一静态 `ServiceDescriptor` 与声明顺序 `MethodId`；
- 类型安全的 `ServiceNameClient` 和专属 Builder；
- `ServiceNameServer<T>` 局部 Middleware wrapper；
- 隐藏的 O(1) service dispatch。

RPC trait 必须非泛型，只包含无默认实现的 `async fn(&self, ...)`；参数和返回值必须是拥有所有权的具体类型。方法 future 在生成契约中要求 `Send`。route、placeholder、HTTP method 和参数来源在宏展开及启动校验阶段验证。

## fusen_service

`fusen_service` 只绑定实现类型到 trait 生成的静态描述和 dispatch。实现方法顺序不影响 `MethodId`；实现侧不得重复 service 或 asset 元数据。泛型实现保留 generics 与 where clause，一个具体类型只能绑定一个 RPC service。

运行时依赖重命名由 `proc-macro-crate` 解析。旧 `handler` 宏、字符串 ID、Aspect、HandlerLoad 和动态 handler controller 不存在。
