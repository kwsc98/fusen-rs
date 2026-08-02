# ADR 0008: Error ownership 与 invocation failure 分类

- 状态：已接受
- 日期：2026-08-02
- 决策者：fusen-rs 维护者

> Error ownership 与分类决策继续有效；旧协议专属的 wire 说明已由
> [ADR 0009](0009-http-binding-discovery-decoupling.md) 取代。

## 背景

Fusen 的调用、client/server lifecycle、注册发现和配置具有不同的恢复方式与依赖边界。
把这些错误集中成一个 workspace 级枚举，会迫使底层 crate 依赖 runtime 或让中央类型持续
感知每个 provider。旧调用错误还把 application 同时表达为 category 和 origin，导致来源、
语义与 retry/breaker 策略相互耦合。

远端 Problem Details 是不可信输入。若 Error 模型、wire decoder、retry 与两级 breaker
分别解释其 status、retryable 和 request ID，同一个失败可能得到不一致的决策。

## 决策

- 各 crate 拥有自己的领域错误：`fusen-rs` 拥有 invocation 与 client/server lifecycle
  错误，`fusen-register` 拥有 `RegistryError`，`fusen-config` 拥有 `ConfigError`。不新增
  `fusen-error` crate 或跨 workspace 总错误枚举。
- 单次调用的 `Error` 拆为正交的 `ErrorKind::{Application, Framework}`、
  `ErrorOrigin::{Local, Remote}` 与语义 `ErrorCategory`。`Application` 不再是 category
  或 origin，无法从 HTTP status 恢复的 category 使用 `Unknown`。
- 公共构造器按责任命名：service code 使用 `application`/`application_status`，扩展使用
  `local`。构造器统一返回 `ErrorConstructionError`；机器码严格验证为 lower snake case。
- Error 模型只保存已验证、已归一化的数据。Problem Details 的 URN、status、code、
  request ID、retry hint 与 4 KiB fallback 由 wire 层处理。
- 每个失败 attempt 只生成一次内部 failure class，并由 retry、metrics、endpoint breaker
  和 service breaker 共享。Application 永不自动重试；remote Application 5xx 仍计入
  breaker failure，4xx 不计入。
- 错误 Clone 使用共享内部数据与 copy-on-write mutation。公开错误的 `Debug` 不展开
  source、header 或 details 值；`RegistryError` 与 `ConfigError` 将安全 message 和
  provider source 分开保存，格式化只展示前者，跨 crate 因果关系使用标准 `source()` 保留。

## Wire 兼容性

`http-json-v1` 的 URI、header、content type 与 Problem Details 契约由 ADR 0009
和 wire golden fixtures 定义。解码器严格校验保留的 Fusen URN，并允许符合
RFC 9457 的外部 Problem type；矛盾或非法响应会被归一化为远端 protocol
error。远端 headers/details 不会在服务转发同一个 Error 时自动重新编码。

## 后果

调用方需要一次性迁移 `Error::new`、旧 application 构造签名、
`ErrorCategory::Application`、`ErrorOrigin::Application`、`ErrorCategory::status()` 和
生命周期错误的 `message_ref()`。`v0.9.0` 尚未建立兼容基线，因此不提供 deprecated
alias。合法 wire fixture 和 fuzz corpus 不需要迁移；非法或自相矛盾的响应会更早被拒绝。

## 备选方案

- 新增统一 `fusen-error` crate：拒绝，会反转 crate 依赖并把互不相关的恢复策略耦合到
  一个公共类型。
- 每个 consumer 自行解释 HTTP status：拒绝，会让 retry、metrics 与 breaker 对同一失败
  作出不同决定。
- 保留 `Application` category/origin alias：拒绝，三维模型仍会存在重叠状态，且 0.9 尚无
 兼容负担。
