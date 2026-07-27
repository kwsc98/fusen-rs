# ADR 0006: 客户端 TLS 与明文服务端

- 状态：已接受
- 日期：2026-07-27
- 决策者：fusen-rs 维护者
- 取代：[ADR 0003](0003-plaintext-core-and-tls-termination.md)

## 背景

客户端需要直接调用 HTTPS 服务，包括第三方 Spring Cloud 服务和由 ingress、
sidecar、反向代理或 service mesh 暴露的 Fusen 服务。该需求只要求客户端拥有
出站 TLS；让内置服务端同时承担证书加载、私钥保护、SNI、mTLS 与轮换生命周期
会显著扩大本轮范围。

## 决策

`ServiceEndpoint` 接受 canonical absolute `http://` 与 `https://` URL，并继续拒绝
credentials、query、fragment、零端口、相对 URL 和其他 scheme。Direct endpoint、
Registry 提供的实例以及显式 server advertisement 使用同一验证规则。

客户端传输规则固定为：

- `FusenV1` 在 `http://` 上使用 HTTP/2 prior knowledge（h2c），在 `https://` 上
  使用 TLS ALPN `h2`；ALPN 未协商出 `h2` 时连接失败，不降级到 HTTP/1.1；
- `SpringCloudV1` 在 `http://` 与 `https://` 上都使用 HTTP/1.1；
- HTTPS 使用 Rustls Ring provider、TLS 1.2/1.3 与 bundled Mozilla WebPKI roots，
  并验证证书链、有效期与 endpoint hostname；
- 不提供自定义 CA、客户端证书/mTLS、跳过验证、明文 fallback 或用户可替换
  Transport SPI。构建 Runtime 不读取平台证书存储，也不依赖其可用性。

内置 Server listener 仍只接受明文 HTTP/1.1 与 h2c，不加载证书或私钥。未显式
配置时，它向 Registry 发布由 bound socket 生成的 `http://` endpoint。
`ServerBuilder::advertised_endpoint("https://...")` 只声明一个由外部 TLS 终止器
提供的可达地址；应用负责让该地址转发到明文 listener，Server 不验证或创建终止器。

Nacos adapter 必须保留 registration 的 `http`/`https` scheme，并允许 discovery
恢复两种 endpoint；未知 scheme 继续被过滤。TLS 不改变 Fusen/Spring 的 method、
URI、headers、JSON envelope 或 Problem Details，因此不新增 wire 版本。

依赖策略允许 `fusen-rs` 使用经过审计的 Rustls 客户端栈，但继续禁止
`native-tls`、`hyper-tls`、OpenSSL TLS backend、AWS-LC provider、平台 verifier、
native/system root loader 与 PEM loader。`fusen-contract` 和其他纯契约 crate 保持
network/runtime/backend-neutral；`webpki-roots` 的更新进入依赖与安全审计。

## 后果

Client 可以直接连接由 bundled Mozilla roots 信任的 HTTPS endpoint，HTTP 行为保持
兼容；证书验证失败不会触发明文恢复。私有 CA、自签名证书和只存在于系统 trust store
的根不受支持。根集合随 `webpki-roots` 依赖更新，Rustls 与根证书版本进入安全升级和
发布审计范围；Runtime 构建不再因为平台 Keychain/证书存储不可访问而失败。

Server 继续保持简单、跨平台且不持有私钥。需要进程内 TLS、mTLS、动态证书轮换或
直连内置 Server 的 HTTPS 部署仍必须使用外部终止器，或由未来独立 ADR 重新评估。

## 备选方案

- Client 与 Server 同时实现 TLS：拒绝，服务端证书生命周期不是客户端请求能力的前置条件。
- native-tls/OpenSSL：拒绝，会引入平台 backend 与 native toolchain 维护面。
- 系统/native roots：拒绝，在无可访问 Keychain/证书存储的环境会让整个 Runtime
  构建失败，并使纯 HTTP 调用无端依赖平台状态。
- 暴露自定义 connector、CA 或危险的跳过校验开关：拒绝，扩大 0.9 公开契约与误用面。
- HTTPS 失败后重试 HTTP：拒绝，会把传输机密性和服务身份验证静默降级。
