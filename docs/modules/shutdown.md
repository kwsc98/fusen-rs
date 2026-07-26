# 停机行为

> English summary: on Unix, `Server::run` listens for SIGINT and SIGTERM; on
> other platforms it listens for Ctrl-C. Shutdown closes the listener first,
> then deregisters providers and drains active Hyper connections concurrently
> under one shared deadline.

`Server::run` 在 Unix 上由 SIGINT 或 SIGTERM 触发停机，在其他平台由 Ctrl-C 触发。信号监听失败时会记录错误并进入相同的 fail-safe 停机流程。`run_with_shutdown` 不叠加这些系统信号，只等待调用方传入的 shutdown future，适合嵌入式运行和确定性测试。该 future 在启动注册完成、进入 accept loop 后才开始轮询；启动注册不会被它中断，仍由 `registry_timeout` 约束。

停机开始后，Server 计算唯一的绝对 deadline，先关闭 listener，并立即通知所有 Hyper 连接 graceful shutdown。注册实例的逆序注销和在途连接排空随后并行进行，共享 `graceful_shutdown_timeout` 的总预算，而不是分别获得一段完整超时。期限到达时仍未结束的连接 task 会被强制取消，Server 不会为回收它们而无限等待。

accept 失败、注册中心注销失败或停机超时会由 Server 返回 `FusenError`，次要故障仍写入日志。若 `run_with_shutdown` 在资源被追踪后遭到取消，独立清理 worker 会触发一次有界的后台逆序注销；该补偿依赖承载 Server 的 Tokio runtime 在清理期间继续运行。
