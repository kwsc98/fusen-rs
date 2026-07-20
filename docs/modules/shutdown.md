# 停机行为

> English summary: shutdown stops acceptance, deregisters instances, drains
> active Hyper connections, then aborts only after a deadline.

触发信号后 listener 不再接受连接并立即计算绝对 deadline。注册实例在剩余期限内逆序摘除，随后连接收到 graceful shutdown 并完成在途请求；摘除和排空共同使用默认 30 秒总预算，期限到达后 abort 并回收全部 JoinSet task。
