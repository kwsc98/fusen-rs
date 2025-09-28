
# `fusen-rs` High-performance RPC Framework

fusen-rs is a high-performance, lightweight microservice framework that uses Rust macros to solve the current problems of complex use and low performance of mainstream rpc frameworks. It does not need to generate RPC calling code through scripts and scaffolding, and compiles it through macros. It uses "reflection" to achieve high-performance calls and meet the simplicity of RPC calls. It also supports user-defined components and other functions.

[ [中文](./README_CN.md) ]

## Function List

- :white_check_mark: RPC call abstraction layer (Rust macro)
- :white_check_mark: Multi-protocol support (HTTP1, HTTP2)
- :white_check_mark: Service registration and discovery (Nacos)
- :white_check_mark: Microservice ecological compatibility (Dubbo3, SpringCloud)
- :white_check_mark: Custom components (custom load balancer, Aspect surround notification component)
- :white_check_mark: Graceful shutdown
- :construction: HTTP3 protocol support

## Quick Start

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

## Custom component

Microservice custom components include load balancers, service breaker/current limiting components, pre- and post-request processors, service link tracking and other components. Since the components are highly customized, this project is provided with reference to the concept of AOP Two custom components are provided to provide flexible request processing.

### LoadBalance

Load balancing component, LoadBalance provides a select interface to implement user-defined service balancing configuration.

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

I believe everyone is familiar with the concept of dynamic proxy. This is a technology used by Java to enhance classes, and the Spring framework uses this feature to encapsulate a more advanced model, which is the AOP aspect-first programming model. This component is a reference This model implements the wraparound notification model. Users can implement various component requirements based on this component, such as service circuit breaker/current limiting, request pre- and post-processing, link tracking, request response time monitoring and other requirements, and Aspect Components support multi-level nested calls and provide flexible definition methods to meet users' complex needs.

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