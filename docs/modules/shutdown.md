# 停机行为

> English summary: shutdown stops acceptance, deregisters instances, drains
> active Hyper connections, then aborts only after a deadline.

触发信号后 listener 不再接受连接，注册实例逆序摘除，连接收到 graceful shutdown 并完成在途请求。默认最多等待 30 秒；超时后 abort 并回收全部 JoinSet task。摘除错误被结构化记录，但不阻止其他实例摘除和连接排空。
