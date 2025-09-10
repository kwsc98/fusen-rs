
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
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ReqDto {
    pub str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResDto {
    pub str: String,
}

#[fusen_trait(id = "org.apache.dubbo.springboot.demo.DemoService")]
pub trait DemoService {
    async fn sayHello(&self, name: String) -> String;

    #[asset(path = "/sayHelloV2-http", method = POST)]
    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;

    #[asset(path = "/divide", method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> String;
}
```

### Server

```rust
#[derive(Debug)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_server(id = "org.apache.dubbo.springboot.demo.DemoService")]
impl DemoService for DemoServiceImpl {
    async fn sayHello(&self, req: String) -> FusenResult<String> {
        info!("res : {:?}", req);
        Ok("Hello ".to_owned() + &req)
    }
    #[asset(path="/sayHelloV2-http",method = POST)]
    async fn sayHelloV2(&self, req: ReqDto) -> FusenResult<ResDto> {
        info!("res : {:?}", req);
        Ok(ResDto::default().str("Hello ".to_owned() + req.get_str() + " V2"))
    }
    #[asset(path="/divide",method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> FusenResult<String> {
        info!("res : a={:?},b={:?}", a, b);
        Ok((a + b).to_string())
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() {
    fusen_common::logs::init_log();
    let server = DemoServiceImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    FusenApplicationContext::builder()
        //使用配置文件进行初始化
        .init(get_config_by_file("examples/server-config.yaml").unwrap())
        .add_fusen_server(Box::new(server))
        .add_handler(ServerLogAspect.load())
        .build()
        .run()
        .await;
}
```

### Client

```rust
#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() {
    fusen_common::logs::init_log();
    let context = FusenApplicationContext::builder()
        //使用配置文件进行初始化
        .init(get_config_by_file("examples/client-config.yaml").unwrap())
        .add_handler(CustomLoadBalance.load())
        .add_handler(ClientLogAspect.load())
        .build();
    //直接当HttpClient调用HTTP1 + JSON
    let client = DemoServiceClient::new(Arc::new(
        context.client(Type::Host("127.0.0.1:8081".to_string())),
    ));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev host msg : {:?}", res);
    //通过Fusen进行服务注册与发现，并且进行HTTP2+JSON进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::Fusen)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev fusen msg : {:?}", res);
    // //通过Dubbo进行服务注册与发现，并且进行HTTP2+Grpc进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::Dubbo)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev dubbo msg : {:?}", res);
    //通过SpringCloud进行服务注册与发现，并且进行HTTP1+JSON进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::SpringCloud)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev springcloud msg : {:?}", res);
}
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
        invokers: Arc<ResourceInfo>,
    ) -> Result<Arc<InvokerAssets>, fusen_rs::Error> {
        invokers
            .select()
            .ok_or("not find server : CustomLoadBalance".into())
    }
}
```

### Aspect

I believe everyone is familiar with the concept of dynamic proxy. This is a technology used by Java to enhance classes, and the Spring framework uses this feature to encapsulate a more advanced model, which is the AOP aspect-first programming model. This component is a reference This model implements the wraparound notification model. Users can implement various component requirements based on this component, such as service circuit breaker/current limiting, request pre- and post-processing, link tracking, request response time monitoring and other requirements, and Aspect Components support multi-level nested calls and provide flexible definition methods to meet users' complex needs.

```rust
#[handler(id = "ServerLogAspect")]
impl Aspect for ServerLogAspect {
    async fn aroud(
        &self,
        join_point: ProceedingJoinPoint,
    ) -> Result<fusen_common::FusenContext, fusen_rs::Error> {
        let start_time = get_now_date_time_as_millis();
        info!("server receive request : {:?}", join_point.get_context());
        let context = join_point.proceed().await;
        info!(
            "server dispose done RT : {:?}ms : {:?}",
            get_now_date_time_as_millis() - start_time,
            context
        );
        context
    }
}
```

## Dubbo3

This project is also compatible with the dubbo3 protocol, and can easily perform service registration discovery and intermodulation with the Java version of the Dubbo3 project through interface exposure.

Rust's Server and Client don't need to be modified at all, just like the above example.

The Java version of the Dubbo3 project does not need to be modified at the code level. It only needs to add some dependencies and configurations (because the way Dubbo3 uses interface exposure does not support the json serialization protocol by default, it uses the binary serialization format of fastjson2, so here we need to manually Add support for fastjson1)

Here we use duboo3’s official sample dubbo-samples-spring-boot project for demonstration.
<https://github.com/apache/dubbo-samples>

First, we need to add the maven dependencies of fastjson and nacos to the pom.xml of the Server and Client services.

```java
<dependency>
    <groupId>org.apache.dubbo</groupId>
    <artifactId>dubbo-serialization-fastjson</artifactId>
    <version>2.7.23</version>
</dependency>
<!-- registry dependency -->
<dependency>
    <groupId>com.alibaba.nacos</groupId>
    <artifactId>nacos-client</artifactId>
    <version>2.2.0</version>
</dependency>
```

### Java-Server

```java
@DubboService
public class DemoServiceImpl implements DemoService {

    @Override
    public String sayHello(String name) {
        return "Hello " + name;
    }
}
```

### Server-application.yml

```java
dubbo:
  application:
    name: dubbo-springboot-demo-provider
  protocol:
    name: tri
    port: 50052
    //添加fastjson的支持
    prefer-serialization: fastjson
  registry:
    address: nacos://${nacos.address:127.0.0.1}:8848
```

### Java-Client

```java
@Component
public class Task implements CommandLineRunner {
    @DubboReference
    private DemoService demoService;

    @Override
    public void run(String... args) throws Exception {
        String result = demoService.sayHello("world");
        System.out.println("Receive result ======> " + result);

        new Thread(()-> {
            while (true) {
                try {
                    Thread.sleep(1000);
                    System.out.println(new Date() + " Receive result ======> " + demoService.sayHello("world"));
                } catch (InterruptedException e) {
                    e.printStackTrace();
                    Thread.currentThread().interrupt();
                }
            }
        }).start();
    }
}
```

### Client-application.yml

```java
dubbo:
  application:
    name: dubbo-springboot-demo-consumer
  registry:
    address: nacos://${nacos.address:127.0.0.1}:8848
```

## SpringCloud

At the same time, this project has also expanded the HTTP interface to be used as a WebServer framework, and also supports Spring Cloud service registration and discovery. Users can flexibly select and switch the protocols that need to be exposed, and support simultaneous exposure.

Here we use the spring-cloud-alibaba project for demonstration
<https://github.com/alibaba/spring-cloud-alibaba>

When calling SpringCloud on RustClient, you need to change fusen_trait_id to the target service id (application_name)

```rust
#[fusen_trait(id = "service-provider")]
```

There is no need to modify the Java server and client code. Just start it.

### SpringCloud-Server

Provider startup class
package com.alibaba.cloud.examples.ProviderApplication

```java
//EchoController
@RestController
public class EchoController {
...
    @GetMapping("/divide")
    public String divide(@RequestParam Integer a, @RequestParam Integer b) {
        if (b == 0) {
          return String.valueOf(0);
        } else {
          return String.valueOf(a / b);
        }
    }
...
}
```

### SpringCloud-Client

Consumer startup class
package com.alibaba.cloud.examples.ConsumerApplication

```java
//TestController
@RestController
public class TestController {
...
    @GetMapping("/divide-feign")
    public String divide(@RequestParam Integer a, @RequestParam Integer b) {
        return echoClient.divide(a, b);
    }
...
}

```

Test curl (curl => SpringCloud => fusen-rust)
<http://127.0.0.1:18083/divide-feign?a=1&b=2>

```rust
2024-04-10T06:52:32.737307Z  INFO ThreadId(07) server: 33: res : a=1,b=2
```

Test curl ( curl => fusen-rust )

<http://127.0.0.1:8081/divide?a=2&b=3>

```rust
2024-04-10T06:54:26.436416Z  INFO ThreadId(512) server: 33: res : a=2,b=3
```

Test curl ( curl => fusen-rust )

curl --location --request POST '<http://127.0.0.1:8081/sayHelloV2-http>' \
--header 'Content-Type: application/json' \
--header 'Connection: keep-alive' \
--data-raw '{
"str" ​​: "World"
}'

```rust
2024-04-10T07:02:50.138057Z  INFO ThreadId(03) server: 26: res : ReqDto { str: "World" }
```
