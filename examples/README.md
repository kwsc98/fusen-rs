# fusen-rs examples

示例按寻址方式分为 Host 直连和 Nacos 注册发现，两组示例使用相同的 `DemoService` 接口与服务实现，便于直接比较 context、server 和 `ClientOptions` 的配置差异。

## Host 直连

Host 示例不依赖外部组件。先启动服务端：

```bash
cargo run -p examples --bin host-server
```

再在另一个终端运行客户端：

```bash
cargo run -p examples --bin host-client
```

压测使用独立的无日志服务端，避免逐请求日志和 tracing 影响结果。默认使用 HTTP/2，启动 100 个并发任务，每个任务请求 10,000 次：

```bash
cargo run --release -p examples --bin host-server-pt
cargo run --release -p examples --bin host-client-pt
```

设置 `PT_PROTOCOL=both` 会在相同并发和请求体下依次运行 HTTP/1.1 与 HTTP/2，并打印两者的 QPS、JSON 吞吐和倍率对比：

```bash
PT_PROTOCOL=both \
PT_CONCURRENCY=100 \
PT_REQUESTS_PER_TASK=10000 \
cargo run --release -p examples --bin host-client-pt
```

也可以通过环境变量调整监听地址和服务地址：

```bash
PT_BIND_ADDR=0.0.0.0:18081 \
cargo run --release -p examples --bin host-server-pt

PT_SERVER_URL=http://127.0.0.1:18081 \
PT_PROTOCOL=h2 \
PT_CONCURRENCY=200 \
PT_REQUESTS_PER_TASK=5000 \
cargo run --release -p examples --bin host-client-pt
```

结果会输出完成数、成功/失败数、总耗时、总 QPS、成功 QPS、请求与响应 JSON body 字节数及吞吐率。字节统计是应用层 JSON body 的实际序列化长度，不包含 HTTP/2 帧头、TCP/IP 和 TLS 开销；每种协议在计时前都会按配置并发度完成一轮预热，预热请求不计入结果。

`PT_PROTOCOL` 支持 `h1`、`h2` 和 `both`，默认值为 `h2`。在这个框架中，H1 使用 `WireProtocol::SpringCloud` 的 HTTP/1.1 transport，H2 使用 `WireProtocol::Fusen` 的 HTTP/2 transport。

| 对比项 | HTTP/1.1 | HTTP/2 |
| --- | --- | --- |
| 并发模型 | 一个连接同一时刻处理一个请求，连接池通过增加连接承载并发 | 一个连接可多路复用多个 stream |
| Header | 文本 header，每个请求重复发送 | 二进制帧并使用 HPACK 压缩 |
| 队头阻塞 | 同一连接上的后续请求需要等待前一个响应 | stream 之间独立，但底层 TCP 丢包仍会影响同一连接 |
| 典型场景 | 低并发、传统代理或 Spring Cloud 兼容 | 高并发、连接数受限、长连接 RPC |

两种协议传输相同请求时，JSON body 字节数应当一致。当前统计不包含 HTTP header 和协议帧，因此只能对比业务 payload 吞吐；如需比较真实线速字节，应在 transport 层增加计数或使用抓包工具。

## Nacos 注册发现

先启动可访问的 Nacos。`NACOS_ADDR` 默认使用 `127.0.0.1:8848`，也可以通过环境变量覆盖。服务端发布地址 `FUSEN_ADVERTISED_URL` 默认使用 `http://127.0.0.1:8081`：

```bash
NACOS_ADDR=127.0.0.1:8848 \
FUSEN_ADVERTISED_URL=http://127.0.0.1:8081 \
cargo run -p examples --bin nacos-server
```

然后启动发现客户端：

```bash
NACOS_ADDR=127.0.0.1:8848 \
cargo run -p examples --bin nacos-client
```

在 Docker、Kubernetes 或多机环境中，不能把 `FUSEN_ADVERTISED_URL` 设置为仅服务端自身可访问的 loopback 地址；该地址必须能被客户端实际访问。

## Nacos 热配置

将 `resource/nacos_config_export_20250928160704.zip` 导入 Nacos 后运行：

```bash
NACOS_ADDR=127.0.0.1:8848 \
cargo run -p examples --bin nacos-hot-config
```

程序会读取 `DEFAULT_GROUP` 下的 `application-config1`，随后每秒输出一次当前配置；在 Nacos 修改并发布配置即可观察热更新。

## 目录说明

```text
src/
├── lib.rs                 # DTO 与共享 RPC trait
├── service.rs             # Host/Nacos 共用的服务实现
├── handler/               # Aspect 与负载均衡扩展示例
├── host/
│   ├── server.rs          # 无注册中心的服务端
│   ├── client.rs          # ClientOptions::direct
│   ├── server_pt.rs       # 无日志的 Host 压测服务端
│   └── client_pt.rs       # 可配置并发和统计指标的压测客户端
└── nacos/
    ├── server.rs          # 注册服务并在停机时摘除
    ├── client.rs          # ClientOptions::discovery
    └── hot_config.rs      # Nacos 配置热更新
```
