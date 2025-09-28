
# `fusen-rs` 一个最像RPC框架的Rust-RPC框架

fusen-rs是一个高性能，轻量级的微服务框架，通过使用Rust宏来解决目前主流rpc框架使用复杂，性能低等问题，不需要通过脚本和脚手架生成RPC调用代码，通过宏来进行编译期"反射"来实现高性能的调用，满足RPC调用的简易性，并且支持用户自定义组件等功能.

## 功能列表

- :white_check_mark: RPC调用抽象层(Rust宏)
- :white_check_mark: 多协议支持(HTTP1, HTTP2)
- :white_check_mark: 服务注册与发现(Nacos)
- :white_check_mark: 微服务生态兼容(Dubbo3, SpringCloud)
- :white_check_mark: 自定义组件(自定义负载均衡器,Aspect环绕通知组件)
- :white_check_mark: 配置中心(本地文件配置, Nacos)
- :white_check_mark: 优雅停机
- :construction: HTTP3协议支持

## 快速开始

### Common Interface

```rust
#[fusen_trait]
pub trait DemoService {
    async fn sayHello(&self, name: String) -> String;

    #[asset(path = "/sayHelloV2-http")]
    async fn sayHelloV2(&self, name: RequestDto) -> ResponseDto;

    #[asset(path = "/divide", method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> String;
}
```

### Server

```rust
#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn sayHello(&self, name: String) -> Result<String, FusenError> {
        Ok(format!("Hello {name}"))
    }

    #[asset(path = "/sayHelloV2-http")]
    async fn sayHelloV2(&self, name: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            str: format!("HelloV2 {}", name.str),
        })
    }

    #[asset(path = "/divide", method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> Result<String, FusenError> {
        Ok(format!("a + b = {}", a + b))
    }
}
```

### Client

```rust
let fusen_client = DemoServiceClient::init(
    &mut fusen_contet,
    Protocol::Fusen,
    Some(vec!["LogAspect", "TimeAspect"]),
).await.unwrap();
println!("{:?}", fusen_client.divideV2(1, 2).await);
println!("{:?}", fusen_client.sayHello("test1".to_owned()).await);
```

## 自定义组件

微服务自定义组件包括, 负载均衡器, 服务熔断/限流组件, 前置后置请求处理器, 服务链路追踪等组件. 由于组件的定制化程度较高, 所以本项目参考AOP的概念提供了两种自定义组件,来提供灵活的请求处理。

### LoadBalance

负载均衡组件, LoadBalance提供一个select接口来实现用户自定义服务均衡配置。

```rust
#[handler(id = "CustomLoadBalance")]
impl LoadBalance for CustomLoadBalance {
    async fn select(
        &self,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> Result<Option<Arc<ServiceResource>>, FusenError> {
        if invokers.is_empty() {
            return Ok(None);
        }
        let mut thread_rng = rand::rng();
        Ok(Some(
            invokers[thread_rng.random_range(0..invokers.len())].clone(),
        ))
    }
}
```

### Aspect

动态代理的概念相信大家都不陌生,这是Java对类进行增强的一种技术,而Spring框架利用此特性封装出了更高级的模型, 那就是AOP面先切面编程模型. 本组件就是参考了此模型,实现了环绕式通知模型, 用户可以基于此组件实现各种组件需求，比如说服务熔断/限流,请求的前置后置处理,链路追踪,请求响应时间监控等需求,并且Aspect组件支持多层嵌套调用,提供灵活的定义方式满足用户复杂需求.

```rust
#[handler(id = "TimeAspect")]
impl Aspect for TimeAspect {
    async fn aroud(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        debug!("开始处理时间 : {start_time:?}");
        let context = join_point.proceed().await;
        debug!(
            "结束处理时间 : {:?}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                - start_time
        );
        context
    }
}
```