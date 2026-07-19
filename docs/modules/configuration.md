# 配置行为

> English summary: runtime limits are typed Rust configuration; hot business
> configuration uses watch snapshots.

`ClientConfig` 和 `ServerConfig` 不读取隐式环境变量。限制值为零会在启动时失败。启用注册中心时 advertised URL 必填，避免注册不可访问的 bind 地址。

`fusen-common::ConfigManager` 解析 TOML/YAML，并通过 watch 原子发布新配置；解析失败保留上一版本并记录错误。Nacos listener 使用有界 channel，消费者关闭后停止更新。
