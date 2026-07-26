# 服务端行为

## 职责与接口

`Server::bind(...)` 收集 registry、全局 `Middleware`、`InvocationObserver` 和服务。普通服务直接 `.service(Impl)`；需要局部 Middleware 时使用宏生成的 `ServiceNameServer::new(Impl).middleware(...)`。

全局 Middleware 先进入，服务局部 Middleware 后进入，退出顺序相反。启动时每条 route 会预绑定静态方法描述、不可变 Middleware slice 和 service invoker。

## 状态、并发与错误

启动顺序为校验服务/路由、绑定 listener、限时注册、对外服务。任一注册失败会逆序回滚；Server future 在已追踪注册资源后被取消时，也会由独立 worker 发起一次有界的后台逆序注销。该取消补偿依赖 Tokio runtime 在清理期间继续运行。Observer guard 在并发准入前创建；准入保持 fail-fast。一个绝对 deadline 覆盖 body decode、route、Middleware、`MethodId` service dispatch 与 response encode，Future 被丢弃时报告 Cancelled。

默认 body 上限 2 MiB、请求并发 1024、连接 2048、HTTP/2 每连接 128 streams、请求 timeout 30 秒、总停机 timeout 30 秒、单次注册操作 5 秒、HTTP/1 header timeout 10 秒。

`Server::run` 在 Unix 上监听 SIGINT 与 SIGTERM，在其他平台监听 Ctrl-C；信号监听失败会进入 fail-safe 停机。`run_with_shutdown` 用于嵌入式运行与确定性测试，只由调用方传入的 shutdown future 触发，不自动叠加系统信号。shutdown future 在启动注册完成、进入 accept loop 后才开始轮询，不中断启动注册；启动阶段继续由 `registry_timeout` 约束。

停机先关闭 listener 并通知 Hyper 连接 graceful shutdown，再并行执行逆序服务注销与在途连接排空。二者共享 `graceful_shutdown_timeout` 的同一个绝对 deadline；期限到达后强制取消剩余连接，不再无界等待 task 回收。accept、注销或停机超时会返回对应的 `FusenError`，而不是仅记录日志后返回成功。
