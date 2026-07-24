# 服务端行为

## 职责与接口

`Server::bind(...)` 收集 registry、全局 `Middleware`、`InvocationObserver` 和服务。普通服务直接 `.service(Impl)`；需要局部 Middleware 时使用宏生成的 `ServiceNameServer::new(Impl).middleware(...)`。

全局 Middleware 先进入，服务局部 Middleware 后进入，退出顺序相反。启动时每条 route 会预绑定静态方法描述、不可变 Middleware slice 和 service invoker。

## 状态、并发与错误

启动顺序为校验服务/路由、绑定 listener、限时注册、对外服务。任一注册失败会逆序回滚。Observer guard 在并发准入前创建；准入保持 fail-fast。一个绝对 deadline 覆盖 body decode、route、Middleware、`MethodId` service dispatch 与 response encode，Future 被丢弃时报告 Cancelled。

默认 body 上限 2 MiB、请求并发 1024、连接 2048、HTTP/2 每连接 128 streams、请求 timeout 30 秒、总停机 timeout 30 秒、单次注册操作 5 秒、HTTP/1 header timeout 10 秒。

`run_with_shutdown` 用于嵌入式运行与确定性测试。停机先停止 accept，再在总期限内摘除服务和排空连接。
