# 配置行为

> English summary: runtime limits are typed Rust configuration; hot business
> configuration uses watch snapshots.

`ClientConfig` 和 `ServerConfig` 不读取隐式环境变量。限制或 timeout 为零会在 build/启动时失败。启用注册中心时 advertised URL 必填，避免注册不可访问的 bind 地址。

`fusen-common::ConfigManager` 解析 TOML，并在 `yaml` feature 下使用 `serde_yaml_ng`；watch 采用 latest-wins 原子发布，解析失败保留上一版本。Nacos listener 可显式 close，Drop 也会请求移除监听。`nacos`、`otel`、`yaml` 均为可选 feature。
