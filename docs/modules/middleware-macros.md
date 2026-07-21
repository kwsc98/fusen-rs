# 中间件与宏行为

> English summary: generated adapters serialize arguments and compose a
> deterministic aspect chain around transport or service execution.

handler ID 在同一 context 中必须唯一，引用未知 ID 会失败。每个服务最多选择一个 load balancer，多个配置时以最后一个为准；Aspect 保持声明顺序并允许调用 `proceed`。`handler` 默认从实现的 trait 名推断 `Aspect` 或 `LoadBalance`；trait 使用别名时通过 `kind = Aspect` 或 `kind = LoadBalance` 明确类型。泛型 handler 实现会保留原有 generics 和 where clause。

`fusen_trait` 是 id/group/version/path/method/参数来源的唯一元数据来源，并生成返回 `Result<_, FusenError>` 的客户端。RPC trait 必须是非泛型安全 trait，只包含没有默认实现的 `async fn(&self, ...)`；参数和返回值必须是拥有所有权的具体类型，不能包含引用、生命周期、`impl Trait`、关联项或方法泛型。普通 supertrait 和仅约束 `Self` 的非泛型 where clause 会被保留。

`id/version/group/path` 使用非空字符串字面量；`method` 接受 `GET` 或 `"GET"` 并统一为大写。推荐使用限定形式 `#[fusen_rs::fusen_procedural_macro::asset(...)]`，依赖重命名时替换为实际 crate 名。重复字段、重复 `asset`、query/fragment route 和不匹配的 placeholder 均在宏展开阶段失败。

`fusen_service` 只接受对应 trait impl 并复用隐藏元数据入口。实现侧重复 `asset` 或 service 元数据会生成编译错误；实现参数可以使用 `_`、`mut` 或解构 pattern。泛型实现类型会保留 generics 和 where clause。一个具体类型只能标注一个 `fusen_service`；多个 RPC trait 应使用不同实现类型。宏通过 `proc-macro-crate` 支持运行时依赖重命名。
