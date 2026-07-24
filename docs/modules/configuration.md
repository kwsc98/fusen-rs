# 配置行为

> English summary: runtime limits are typed Rust configuration; hot business
> configuration uses watch snapshots.

`ClientConfig` 和 `ServerConfig` 不读取隐式环境变量。必要限制或 timeout 为零会在 build/启动时失败；H1 的 `max_idle_per_host = 0` 是明确支持的例外，表示关闭连接复用。启用注册中心时 advertised URL 必填，避免注册不可访问的 bind 地址。

客户端通过 `Http1PoolConfig` 配置每个地址的空闲连接保留数和回收时间，通过 `Http2PoolConfig` 配置每个地址的连接分片数、回收时间和 ping 保活。H2 分片连接按需创建，因此配置为 4 不会在 runtime 构建时立即建立 4 条连接。

`fusen-config::ConfigManager` 解析 TOML，并在 `yaml` feature 下使用 `serde_yaml_ng`；watch 采用 latest-wins 原子发布，解析失败保留上一版本。`fusen-nacos` 提供 Nacos 配置 listener，支持显式 close，Drop 也会请求移除监听。日志与可选 OTel 初始化独立位于 `fusen-observability`。
